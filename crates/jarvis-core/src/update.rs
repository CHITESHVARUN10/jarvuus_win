use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, USER_AGENT};
use semver::Version;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const RELEASES_API: &str =
    "https://api.github.com/repos/CHITESHVARUN10/jarvuss/releases/latest";
pub const CHECKSUMS_ASSET: &str = "SHA256SUMS.txt";
pub const REBASE_API_ENV: &str = "JARVIS_RELEASES_API";
pub const PUBLISHER_ENV: &str = "JARVIS_UPDATE_PUBLISHER";
pub const ALLOW_UNSIGNED_ENV: &str = "JARVIS_ALLOW_UNSIGNED_UPDATE";

pub const MAX_RELEASE_BYTES: usize = 1_048_576;
pub const MAX_CHECKSUMS_BYTES: usize = 256 * 1024;
pub const MAX_UPDATE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("could not reach the release server: {0}")]
    Network(#[from] reqwest::Error),
    #[error("release metadata is invalid: {0}")]
    Invalid(String),
    #[error("the update was larger than allowed")]
    TooLarge,
    #[error("checksum verification failed for {0}")]
    Checksum(String),
    #[error("the release has no SHA-256 for {0}, so the download was refused")]
    Unchecked(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactKind {
    Installer,
    Portable,
    Msix,
}

impl ArtifactKind {
    pub fn label(self) -> &'static str {
        match self {
            ArtifactKind::Installer => "Installer",
            ArtifactKind::Portable => "Portable zip",
            ArtifactKind::Msix => "MSIX package",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: Option<String>,
}

impl ReleaseAsset {
    pub fn kind(&self) -> Option<ArtifactKind> {
        if self.name.ends_with("-setup.exe") {
            Some(ArtifactKind::Installer)
        } else if self.name.ends_with(".msix") {
            Some(ArtifactKind::Msix)
        } else if self.name.ends_with(".zip") {
            Some(ArtifactKind::Portable)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub notes: String,
    pub notes_url: String,
    pub published_at: String,
    pub assets: Vec<ReleaseAsset>,
}

impl Release {
    pub fn preferred_asset(&self) -> Option<&ReleaseAsset> {
        let rank = |asset: &ReleaseAsset| match asset.kind() {
            Some(ArtifactKind::Installer) => 0,
            Some(ArtifactKind::Msix) => 1,
            Some(ArtifactKind::Portable) => 2,
            None => 3,
        };
        self.assets
            .iter()
            .filter(|asset| asset.kind().is_some())
            .min_by_key(|asset| rank(asset))
    }
}

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is valid semver")
}

pub fn parse_version(tag: &str) -> Result<Version, UpdateError> {
    let trimmed = tag.trim().trim_start_matches(['v', 'V']);
    Version::parse(trimmed).map_err(|error| {
        UpdateError::Invalid(format!("release tag '{tag}' is not semver: {error}"))
    })
}

pub fn is_newer(candidate: &Version, current: &Version) -> bool {
    candidate > current
}

pub fn publisher_pin() -> Option<String> {
    env::var(PUBLISHER_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn allow_unsigned() -> bool {
    env::var(ALLOW_UNSIGNED_ENV).is_ok_and(|value| value.trim() == "1")
}

pub fn publisher_matches(publisher: Option<&str>, pin: &str) -> bool {
    let Some(publisher) = publisher else {
        return false;
    };
    pin.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            publisher.eq_ignore_ascii_case(entry)
                || publisher
                    .to_ascii_lowercase()
                    .contains(&entry.to_ascii_lowercase())
        })
}

pub fn parse_checksums(text: &str) -> BTreeMap<String, String> {
    let mut checksums = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else { continue };
        let Some(name) = parts.next() else { continue };
        let hash = hash.trim_start_matches('*');
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let name = name.trim_start_matches('*');
        checksums.insert(name.to_string(), hash.to_ascii_lowercase());
    }
    checksums
}

pub fn sha256_file(path: &Path) -> Result<String, UpdateError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub struct UpdateClient {
    client: Client,
    api: String,
}

impl UpdateClient {
    pub fn new() -> Result<Self, UpdateError> {
        let api = env::var(REBASE_API_ENV).unwrap_or_else(|_| RELEASES_API.into());
        Self::with_client(api, default_client()?)
    }

    pub fn with_client(api: impl Into<String>, client: Client) -> Result<Self, UpdateError> {
        Ok(Self {
            client,
            api: api.into(),
        })
    }

    pub fn latest(&self) -> Result<Release, UpdateError> {
        let body = self.get_bounded(&self.api, MAX_RELEASE_BYTES)?;
        let value: Value = serde_json::from_slice(&body)
            .map_err(|error| UpdateError::Invalid(format!("release JSON: {error}")))?;

        let tag = value
            .get("tag_name")
            .and_then(Value::as_str)
            .ok_or_else(|| UpdateError::Invalid("release has no tag".into()))?
            .to_string();
        let version = parse_version(&tag)?;

        let mut assets: Vec<ReleaseAsset> = value
            .get("assets")
            .and_then(Value::as_array)
            .map(|assets| {
                assets
                    .iter()
                    .filter_map(|asset| {
                        let name = asset.get("name").and_then(Value::as_str)?;
                        let url = asset.get("browser_download_url").and_then(Value::as_str)?;
                        if url.is_empty() {
                            return None;
                        }
                        Some(ReleaseAsset {
                            name: name.to_string(),
                            url: url.to_string(),
                            bytes: asset.get("size").and_then(Value::as_u64).unwrap_or(0),
                            sha256: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        if let Some(checksums) = assets.iter().find(|asset| asset.name == CHECKSUMS_ASSET) {
            if let Ok(body) = self.get_bounded(&checksums.url, MAX_CHECKSUMS_BYTES) {
                let map = parse_checksums(&String::from_utf8_lossy(&body));
                for asset in &mut assets {
                    asset.sha256 = map.get(&asset.name).cloned();
                }
            }
        }

        Ok(Release {
            version,
            tag,
            notes: value
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            notes_url: value
                .get("html_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            published_at: value
                .get("published_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            assets,
        })
    }

    pub fn download(
        &self,
        asset: &ReleaseAsset,
        directory: &Path,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<PathBuf, UpdateError> {
        if asset.bytes > MAX_UPDATE_BYTES {
            return Err(UpdateError::TooLarge);
        }
        let expected = asset
            .sha256
            .clone()
            .ok_or_else(|| UpdateError::Unchecked(asset.name.clone()))?;
        if !valid_asset_name(&asset.name) {
            return Err(UpdateError::Invalid(format!(
                "release asset name '{}' is unsafe",
                asset.name
            )));
        }

        fs::create_dir_all(directory)?;
        let destination = directory.join(&asset.name);
        if destination.is_file() && sha256_file(&destination)? == expected {
            let size = fs::metadata(&destination)?.len();
            progress(size, size);
            return Ok(destination);
        }

        let partial = directory.join(format!("{}.part", asset.name));
        let _ = fs::remove_file(&partial);
        let mut response = self
            .client
            .get(&asset.url)
            .header(ACCEPT, "application/octet-stream")
            .send()?
            .error_for_status()?;

        let declared = asset.bytes.max(response.content_length().unwrap_or(0));
        let mut file = fs::File::create(&partial)?;
        let mut downloaded = 0u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = response.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            downloaded = downloaded.saturating_add(read as u64);
            if downloaded > MAX_UPDATE_BYTES || (declared > 0 && downloaded > declared) {
                drop(file);
                let _ = fs::remove_file(&partial);
                return Err(UpdateError::TooLarge);
            }
            file.write_all(&buffer[..read])?;
            progress(downloaded, declared);
        }
        file.sync_data()?;
        drop(file);

        let digest = sha256_file(&partial)?;
        if digest != expected {
            let _ = fs::remove_file(&partial);
            return Err(UpdateError::Checksum(asset.name.clone()));
        }
        fs::rename(&partial, &destination)?;
        Ok(destination)
    }

    fn get_bounded(&self, url: &str, limit: usize) -> Result<Vec<u8>, UpdateError> {
        let mut response: Response = self
            .client
            .get(url)
            .header(ACCEPT, "application/vnd.github+json")
            .header(USER_AGENT, user_agent())
            .send()?
            .error_for_status()?;
        let mut body = Vec::new();
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = response.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            if body.len() + read > limit {
                return Err(UpdateError::TooLarge);
            }
            body.extend_from_slice(&buffer[..read]);
        }
        Ok(body)
    }
}

fn default_client() -> Result<Client, UpdateError> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .user_agent(user_agent())
        .build()?)
}

fn user_agent() -> String {
    format!("JarvisWindows/{}", env!("CARGO_PKG_VERSION"))
}

fn valid_asset_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 160
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    fn response(status: &str, body: &[u8]) -> Vec<u8> {
        let mut data = format!(
            "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        data.extend_from_slice(body);
        data
    }

    fn serve(
        responses: impl FnOnce(&str) -> Vec<Vec<u8>>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let responses = responses(&base);
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let mut stream: TcpStream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "client never connected"
                    );
                    thread::sleep(Duration::from_millis(10));
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                requests.push(String::from_utf8(request).unwrap());
                stream.write_all(&response).unwrap();
            }
            requests
        });
        (base, handle)
    }

    fn client(base: &str) -> UpdateClient {
        let http = Client::builder().no_proxy().build().unwrap();
        UpdateClient::with_client(format!("{base}/releases/latest"), http).unwrap()
    }

    fn release_json(base: &str, tag: &str, asset: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","body":"Local-first release.","html_url":"https://example.test/releases/{tag}","published_at":"2026-01-24T00:00:00Z","assets":[{{"name":"{asset}","browser_download_url":"{base}/{asset}","size":5}},{{"name":"SHA256SUMS.txt","browser_download_url":"{base}/SHA256SUMS.txt","size":80}}]}}"#
        )
    }

    fn body() -> Vec<u8> {
        b"jarvi".to_vec()
    }

    fn checksums(name: &str, bytes: &[u8]) -> Vec<u8> {
        format!("{:x}  {name}\n", Sha256::digest(bytes)).into_bytes()
    }

    #[test]
    fn checksums_are_parsed_from_the_published_sum_file() {
        let hash = "a".repeat(64);
        let text =
            format!("# generated\n{hash}  Jarvis-0.2.0-x64-setup.exe\r\nnot-a-hash  other.bin\n");
        let parsed = parse_checksums(&text);
        assert_eq!(
            parsed.get("Jarvis-0.2.0-x64-setup.exe"),
            Some(&hash.clone())
        );
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn versions_compare_without_the_leading_v() {
        let current = parse_version("v0.1.0").unwrap();
        assert!(is_newer(&parse_version("0.2.0").unwrap(), &current));
        assert!(!is_newer(&current, &current));
        assert!(!is_newer(&parse_version("0.1.0-beta.1").unwrap(), &current));
        assert!(parse_version("0.1.0-beta.1").is_ok());
        assert!(parse_version("latest").is_err());
    }

    #[test]
    fn publisher_pins_match_case_insensitively() {
        assert!(publisher_matches(
            Some("Jarvis contributors"),
            "jarvis contributors"
        ));
        assert!(publisher_matches(
            Some("CN=Jarvis contributors, O=Jarvis"),
            "CN=Jarvis contributors"
        ));
        assert!(publisher_matches(
            Some("Contoso"),
            "Contoso, Jarvis contributors"
        ));
        assert!(!publisher_matches(Some("Contoso"), "Jarvis"));
        assert!(!publisher_matches(None, "Jarvis"));
    }

    #[test]
    fn the_latest_release_carries_checksums_and_the_installer() {
        let name = "Jarvis-0.2.0-x64-setup.exe";
        let body = body();
        let (base, server) = serve(|base| {
            vec![
                response("200 OK", release_json(base, "v0.2.0", name).as_bytes()),
                response("200 OK", &checksums(name, &body)),
            ]
        });
        let release = client(&base).latest().unwrap();
        assert_eq!(release.version, Version::new(0, 2, 0));
        assert_eq!(release.notes, "Local-first release.");
        assert_eq!(
            release.preferred_asset().and_then(|asset| asset.kind()),
            Some(ArtifactKind::Installer)
        );
        assert_eq!(
            release
                .preferred_asset()
                .and_then(|asset| asset.sha256.clone()),
            Some(format!("{:x}", Sha256::digest(&body)))
        );
        server.join().unwrap();
    }

    #[test]
    fn downloads_are_verified_and_saved_under_the_release_name() {
        let name = "Jarvis-0.2.0-x64-setup.exe";
        let body = body();
        let (base, server) = serve(|base| {
            vec![
                response("200 OK", release_json(base, "v0.2.0", name).as_bytes()),
                response("200 OK", &checksums(name, &body)),
                response("200 OK", &body),
            ]
        });
        let client = client(&base);
        let release = client.latest().unwrap();
        let asset = release.preferred_asset().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut seen = 0;
        let path = client
            .download(asset, dir.path(), |downloaded, _| seen = downloaded)
            .unwrap();
        assert_eq!(path.file_name().unwrap().to_string_lossy(), name);
        assert_eq!(fs::read(&path).unwrap(), body);
        assert_eq!(seen, body.len() as u64);
        assert!(!dir.path().join(format!("{name}.part")).exists());
        server.join().unwrap();
    }

    #[test]
    fn a_checksum_mismatch_is_refused() {
        let name = "Jarvis-0.2.0-x64-setup.exe";
        let body = body();
        let (base, server) = serve(|base| {
            vec![
                response("200 OK", release_json(base, "v0.2.0", name).as_bytes()),
                response("200 OK", &checksums(name, b"other")),
                response("200 OK", &body),
            ]
        });
        let client = client(&base);
        let release = client.latest().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let error = client
            .download(release.preferred_asset().unwrap(), dir.path(), |_, _| {})
            .unwrap_err();
        assert!(matches!(error, UpdateError::Checksum(_)));
        assert!(dir.path().read_dir().unwrap().next().is_none());
        server.join().unwrap();
    }

    #[test]
    fn an_unlisted_asset_is_never_downloaded() {
        let unsigned = ReleaseAsset {
            name: "Jarvis-0.2.0-x64-setup.exe".into(),
            url: "http://127.0.0.1:1/never".into(),
            bytes: 5,
            sha256: None,
        };
        let client = UpdateClient::with_client(
            "http://127.0.0.1:1/latest",
            Client::builder().no_proxy().build().unwrap(),
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            client.download(&unsigned, dir.path(), |_, _| {}),
            Err(UpdateError::Unchecked(_))
        ));
    }

    #[test]
    fn unsafe_asset_names_are_rejected() {
        for name in ["../escape.exe", "nested/file.exe", ".hidden", ""] {
            assert!(!valid_asset_name(name));
        }
        assert!(valid_asset_name("Jarvis-0.2.0-x64-setup.exe"));
    }
}
