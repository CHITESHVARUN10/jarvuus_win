use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use reqwest::blocking::Client;
use serde_json::Value;

use crate::download::{self, Progress};
use crate::intent_store::{self, Receipt};
use crate::manifest::{self, IntentRelease, RemoteRelease};
use crate::{ModelError, Result};

pub const HF_BASE: &str = "https://huggingface.co";

#[derive(Clone, Debug)]
pub struct IntentRemote {
    pub base_url: String,
    pub repository: String,
    pub validate_redirects: bool,
}

impl Default for IntentRemote {
    fn default() -> Self {
        Self {
            base_url: HF_BASE.into(),
            repository: manifest::repository().into(),
            validate_redirects: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed(Receipt),
    UpToDate(Receipt),
    KeptNewer(Receipt),
}

impl InstallOutcome {
    pub fn receipt(&self) -> &Receipt {
        match self {
            InstallOutcome::Installed(receipt)
            | InstallOutcome::UpToDate(receipt)
            | InstallOutcome::KeptNewer(receipt) => receipt,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            InstallOutcome::Installed(_) => "installed",
            InstallOutcome::UpToDate(_) => "up to date",
            InstallOutcome::KeptNewer(_) => "a newer model is already installed",
        }
    }
}

impl IntentRemote {
    pub fn latest(&self, client: &Client) -> Result<RemoteRelease> {
        let api = format!(
            "{}/api/models/{}/revision/main",
            self.base_url.trim_end_matches('/'),
            self.repository
        );
        let body = self.get_bounded(client, &api, 2_000_000)?;
        let value: Value = serde_json::from_slice(&body)?;
        let commit = value
            .get("sha")
            .and_then(Value::as_str)
            .filter(|commit| manifest::hex(commit, 40))
            .ok_or_else(|| {
                ModelError::message("could not identify the latest published intent model")
            })?
            .to_string();

        let manifest_url = format!(
            "{}/{}/resolve/{commit}/model-release.json",
            self.base_url.trim_end_matches('/'),
            self.repository
        );
        let manifest_data = self.get_bounded(client, &manifest_url, 64_000)?;
        let intent_manifest: IntentRelease = serde_json::from_slice(&manifest_data)?;
        let release = RemoteRelease {
            commit,
            manifest: intent_manifest,
            manifest_data,
        };
        release.validate()?;
        Ok(release)
    }

    fn get_bounded(&self, client: &Client, url: &str, limit: usize) -> Result<Vec<u8>> {
        let response = client.get(url).send()?.error_for_status()?;
        if self.validate_redirects {
            download::validate_hugging_face_url(response.url())?;
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err(ModelError::message(
                "the model server response is too large",
            ));
        }
        let body = response.bytes()?;
        if body.len() > limit {
            return Err(ModelError::message(
                "the model server response is too large",
            ));
        }
        Ok(body.to_vec())
    }

    pub fn install(
        &self,
        root: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<InstallOutcome> {
        if cancel.load(Ordering::SeqCst) {
            return Err(ModelError::Cancelled);
        }

        let client = download::http_client()?;
        let release = self.latest(&client)?;
        if cancel.load(Ordering::SeqCst) {
            return Err(ModelError::Cancelled);
        }

        let manifest_sha256 = {
            use sha2::Digest;
            format!("{:x}", sha2::Sha256::digest(&release.manifest_data))
        };
        let current = intent_store::receipt(root);

        if let Some(current) = &current {
            if current.manifest_sha256 == manifest_sha256
                && intent_store::verify(root, current).is_ok()
            {
                return Ok(InstallOutcome::UpToDate(current.clone()));
            }
            if manifest::local_is_newer(&current.version, &release.manifest.version)
                && intent_store::verify(root, current).is_ok()
            {
                return Ok(InstallOutcome::KeptNewer(current.clone()));
            }
        }

        let staging = intent_store::staging_dir(root, &release.commit)?;
        intent_store::clear_other_staging(root, &staging)?;

        let total = release.manifest.total_bytes();
        let mut completed = 0_u64;
        for file in &release.manifest.files {
            if cancel.load(Ordering::SeqCst) {
                return Err(ModelError::Cancelled);
            }
            let url = format!(
                "{}/{}/resolve/{}/{}",
                self.base_url.trim_end_matches('/'),
                self.repository,
                release.commit,
                file.name
            );
            let base = completed;
            let mut callback = |file_progress: Progress| {
                let downloaded = base + file_progress.downloaded;
                let speed = file_progress.speed_bps;
                let remaining = total.saturating_sub(downloaded);
                progress(Progress {
                    downloaded,
                    total,
                    fraction: downloaded as f32 / total as f32,
                    speed_bps: speed,
                    eta_secs: if speed > 1.0 {
                        (remaining as f64 / speed) as u64
                    } else {
                        0
                    },
                });
            };
            download::download_ranged(
                &client,
                &url,
                &staging.join(&file.name),
                file.size_bytes,
                &file.sha256,
                cancel,
                self.validate_redirects,
                &mut callback,
            )?;
            completed += file.size_bytes;
        }

        if cancel.load(Ordering::SeqCst) {
            return Err(ModelError::Cancelled);
        }
        let receipt = intent_store::activate(root, &staging, &release)?;
        progress(Progress {
            downloaded: total,
            total,
            fraction: 1.0,
            speed_bps: 0.0,
            eta_secs: 0,
        });
        Ok(InstallOutcome::Installed(receipt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ALLOWED_FILES;
    use serde_json::json;
    use std::collections::HashMap;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn digest(bytes: &[u8]) -> String {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(bytes))
    }

    struct Fixture {
        repository: String,
        files: HashMap<String, Vec<u8>>,
        manifest: Vec<u8>,
    }

    fn fixture() -> Fixture {
        let repository = "owner/tiny-intent".to_string();
        let config = br#"{"model_type":"t5"}"#.to_vec();
        let files: Vec<(&str, Vec<u8>)> = vec![
            ("config.json", config),
            ("encoder_model_quantized.onnx", b"encoder".to_vec()),
            ("decoder_model_merged_quantized.onnx", b"decoder".to_vec()),
            ("added_tokens.json", b"{}".to_vec()),
            ("spiece.model", b"spiece".to_vec()),
            ("model_meta.json", b"{}".to_vec()),
            ("LICENSE", b"license".to_vec()),
            ("NOTICE", b"notice".to_vec()),
        ];
        let entries: Vec<Value> = files
            .iter()
            .map(|(name, data)| {
                json!({
                    "name": name,
                    "size_bytes": data.len(),
                    "sha256": digest(data)
                })
            })
            .collect();
        let manifest = serde_json::to_vec(&json!({
            "format_version": 1,
            "runtime": "jarvis-t5-onnx-v1",
            "license": "apache-2.0",
            "version": "1.0.2",
            "intent_schema_version": 5,
            "max_source_len": 128,
            "max_target_len": 448,
            "files": entries
        }))
        .unwrap();
        let mut map = HashMap::new();
        for (name, data) in files {
            map.insert(format!("/{repository}/resolve/{COMMIT}/{name}"), data);
        }
        map.insert(
            format!("/{repository}/resolve/{COMMIT}/model-release.json"),
            manifest.clone(),
        );
        map.insert(
            format!("/api/models/{repository}/revision/main"),
            serde_json::to_vec(&json!({"sha": COMMIT})).unwrap(),
        );
        Fixture {
            repository,
            files: map,
            manifest,
        }
    }

    fn serve(fixture: &Fixture, expected: usize) -> (String, thread::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = fixture.files.clone();
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut served = 0;
            while served < expected && Instant::now() < deadline {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                    let mut byte = [0];
                    if stream.read_exact(&mut byte).is_err() {
                        break;
                    }
                    request.push(byte[0]);
                }
                let request = String::from_utf8_lossy(&request).to_string();
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let range = request
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("range:"))
                    .and_then(|line| line.split_once('='))
                    .map(|(_, value)| value);
                let body = routes.get(path).cloned().unwrap_or_default();
                let (status, headers, payload) = if let Some(range) = range {
                    let (first, last) = range.split_once('-').unwrap();
                    let first: usize = first.trim().parse().unwrap();
                    let last: usize = last.trim().parse().unwrap_or(body.len() - 1);
                    let last = last.min(body.len() - 1);
                    (
                        "206 Partial Content",
                        format!(
                            "Content-Range: bytes {first}-{last}/{}\r\nContent-Length: {}\r\n",
                            body.len(),
                            last - first + 1
                        ),
                        body[first..=last].to_vec(),
                    )
                } else {
                    (
                        "200 OK",
                        format!("Content-Length: {}\r\n", body.len()),
                        body,
                    )
                };
                let response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n");
                let mut response = response.into_bytes();
                response.extend_from_slice(&payload);
                let _ = stream.write_all(&response);
                served += 1;
            }
            served
        });
        (base, handle)
    }

    #[test]
    fn installs_a_release_from_a_pinned_manifest() {
        let fixture = fixture();
        let (base, server) = serve(&fixture, fixture.files.len());
        let root = tempfile::tempdir().unwrap();
        let remote = IntentRemote {
            base_url: base,
            repository: fixture.repository.clone(),
            validate_redirects: false,
        };
        let cancel = AtomicBool::new(false);
        let mut fraction = 0.0;

        let outcome = remote
            .install(root.path(), &cancel, &mut |progress| {
                fraction = progress.fraction;
            })
            .unwrap();

        assert!(matches!(outcome, InstallOutcome::Installed(_)));
        assert_eq!(fraction, 1.0);
        let receipt = outcome.receipt().clone();
        assert_eq!(receipt.version, "1.0.2");
        assert_eq!(intent_store::receipt(root.path()), Some(receipt.clone()));
        assert!(intent_store::verify(root.path(), &receipt).is_ok());
        assert!(fixture.manifest.starts_with(b"{"));

        // One request for the API, one for the manifest, one per model file.
        assert_eq!(server.join().unwrap(), fixture.files.len());
    }

    #[test]
    fn a_matching_receipt_is_up_to_date() {
        let fixture = fixture();
        let (base, server) = serve(&fixture, 2);
        let root = tempfile::tempdir().unwrap();
        let remote = IntentRemote {
            base_url: base,
            repository: fixture.repository.clone(),
            validate_redirects: false,
        };
        let directory = format!("{COMMIT}-{}", "b".repeat(32));
        let folder = root.path().join("versions").join(&directory);
        fs::create_dir_all(&folder).unwrap();
        for (path, data) in &fixture.files {
            let Some(name) = path.rsplit('/').next() else {
                continue;
            };
            if name == "model-release.json" || name == "main" {
                continue;
            }
            fs::write(folder.join(name), data).unwrap();
        }
        fs::write(folder.join("model-release.json"), &fixture.manifest).unwrap();
        let receipt = Receipt {
            commit: COMMIT.into(),
            directory,
            version: "1.0.2".into(),
            manifest_sha256: digest(&fixture.manifest),
            generation: "test".into(),
        };
        fs::write(
            root.path().join("current.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();

        let outcome = remote
            .install(root.path(), &AtomicBool::new(false), &mut |_| {})
            .unwrap();
        assert!(matches!(outcome, InstallOutcome::UpToDate(_)));
        assert_eq!(outcome.receipt().version, "1.0.2");
        server.join().unwrap();
    }

    #[test]
    fn cancellation_before_network_returns_immediately() {
        let root = tempfile::tempdir().unwrap();
        let remote = IntentRemote {
            base_url: "http://127.0.0.1:9".into(),
            repository: "owner/tiny".into(),
            validate_redirects: false,
        };
        let error = remote
            .install(root.path(), &AtomicBool::new(true), &mut |_| {})
            .unwrap_err();
        assert!(error.is_cancelled());
    }

    #[test]
    fn allowed_files_cover_the_fixture() {
        assert!(ALLOWED_FILES.contains(&"config.json"));
    }
}
