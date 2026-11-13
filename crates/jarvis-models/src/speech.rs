use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use jarvis_core::models::{
    WHISPER_MODEL_BYTES, WHISPER_MODEL_FILE, WHISPER_MODEL_SHA256, WHISPER_MODEL_URL,
};
use jarvis_core::AppPaths;

use crate::download::{self, Progress};
use crate::{ModelError, Result};

#[derive(Clone, Debug)]
pub struct SpeechSpec {
    pub url: String,
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
}

pub fn production_spec() -> SpeechSpec {
    SpeechSpec {
        url: WHISPER_MODEL_URL.into(),
        filename: WHISPER_MODEL_FILE.into(),
        bytes: WHISPER_MODEL_BYTES,
        sha256: WHISPER_MODEL_SHA256.into(),
    }
}

pub fn model_path(paths: &AppPaths) -> PathBuf {
    paths.whisper_model_dir.join(WHISPER_MODEL_FILE)
}

pub fn path_for(paths: &AppPaths, spec: &SpeechSpec) -> PathBuf {
    paths.whisper_model_dir.join(&spec.filename)
}

pub fn cheap_verify(path: &Path, spec: &SpeechSpec) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.len() == spec.bytes)
        .unwrap_or(false)
        && ggml_magic(path)
}

pub fn verify(path: &Path, spec: &SpeechSpec) -> Result<()> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return Err(ModelError::message("the speech model is not installed")),
    };
    if metadata.len() != spec.bytes {
        return Err(ModelError::message("the speech model is incomplete"));
    }
    if !ggml_magic(path) {
        return Err(ModelError::message(
            "the speech model has an invalid ggml header",
        ));
    }
    if download::sha256_file(path)? != spec.sha256 {
        return Err(ModelError::message(
            "the speech model failed checksum verification",
        ));
    }
    Ok(())
}

pub fn install(
    paths: &AppPaths,
    spec: &SpeechSpec,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let model = path_for(paths, spec);
    let model_dir = model
        .parent()
        .ok_or_else(|| ModelError::message("invalid speech model path"))?;
    fs::create_dir_all(model_dir)?;

    if verify(&model, spec).is_ok() {
        progress(Progress {
            downloaded: spec.bytes,
            total: spec.bytes,
            fraction: 1.0,
            speed_bps: 0.0,
            eta_secs: 0,
        });
        return Ok(());
    }

    let partial = model.with_extension("bin.part");
    if model.exists() {
        let valid_partial_exists = partial.exists();
        let shorter = fs::metadata(&model)
            .map(|metadata| metadata.len() < spec.bytes)
            .unwrap_or(false);
        if !valid_partial_exists && shorter {
            fs::rename(&model, &partial)?;
        } else {
            quarantine(&model)?;
        }
    }

    if let Ok(metadata) = fs::metadata(&partial) {
        if metadata.len() >= spec.bytes {
            if verify(&partial, spec).is_ok() {
                fs::rename(&partial, &model)?;
                progress(Progress {
                    downloaded: spec.bytes,
                    total: spec.bytes,
                    fraction: 1.0,
                    speed_bps: 0.0,
                    eta_secs: 0,
                });
                return Ok(());
            }
            fs::remove_file(&partial)?;
        }
    }

    let client = download::http_client()?;
    download::download_ranged(
        &client,
        &spec.url,
        &partial,
        spec.bytes,
        &spec.sha256,
        cancel,
        spec.url.contains("huggingface.co"),
        progress,
    )?;

    if let Err(error) = verify(&partial, spec) {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    fs::rename(&partial, &model)?;
    Ok(())
}

fn ggml_magic(path: &Path) -> bool {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut header = [0_u8; 8];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    let magic = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    magic == 0x6767_6d6c || magic == 0x6767_6a74
}

fn quarantine(path: &Path) -> Result<()> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    fs::rename(path, path.with_extension(format!("invalid-{stamp}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::{Duration, Instant};

    fn bytes() -> Vec<u8> {
        let mut bytes = vec![42_u8; 1024];
        bytes[..4].copy_from_slice(&0x6767_6d6c_u32.to_le_bytes());
        bytes
    }

    fn spec(url: String) -> SpeechSpec {
        SpeechSpec {
            url,
            filename: "tiny.bin".into(),
            bytes: 1024,
            sha256: digest(&bytes()),
        }
    }

    fn digest(bytes: &[u8]) -> String {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(bytes))
    }

    fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut data =
            format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n").into_bytes();
        data.extend_from_slice(body);
        data
    }

    fn server(responses: Vec<Vec<u8>>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/tiny.bin", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream: TcpStream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(Instant::now() < deadline, "client never connected");
                    thread::sleep(Duration::from_millis(10));
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                requests.push(String::from_utf8(request).unwrap());
                stream.write_all(&response).unwrap();
            }
            requests
        });
        (url, handle)
    }

    #[test]
    fn a_verified_model_needs_no_network() {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(directory.path());
        paths.ensure_dirs().unwrap();
        fs::write(
            path_for(&paths, &spec("http://127.0.0.1:9/none".into())),
            bytes(),
        )
        .unwrap();

        let mut fraction = 0.0;
        install(
            &paths,
            &spec("http://127.0.0.1:9/none".into()),
            &AtomicBool::new(false),
            &mut |progress| fraction = progress.fraction,
        )
        .unwrap();
        assert_eq!(fraction, 1.0);
        assert!(cheap_verify(
            &path_for(&paths, &spec("unused".into())),
            &spec("unused".into())
        ));
    }

    #[test]
    fn resumes_a_partial_and_publishes_only_after_verification() {
        let body = bytes();
        let (url, server) = server(vec![response(
            "206 Partial Content",
            "Content-Range: bytes 8-1023/1024\r\nContent-Length: 1016\r\n",
            &body[8..],
        )]);
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(directory.path());
        paths.ensure_dirs().unwrap();
        let model = path_for(&paths, &spec(url.clone()));
        fs::write(&model, &body[..8]).unwrap();

        install(&paths, &spec(url), &AtomicBool::new(false), &mut |_| {}).unwrap();

        assert_eq!(fs::read(&model).unwrap(), body);
        let requests = server.join().unwrap();
        assert!(requests[0].to_lowercase().contains("range: bytes=8-1023"));
    }

    #[test]
    fn a_corrupt_full_file_is_quarantined() {
        let body = bytes();
        let (url, server) = server(vec![response("200 OK", "Content-Length: 1024\r\n", &body)]);
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(directory.path());
        paths.ensure_dirs().unwrap();
        let model = path_for(&paths, &spec(url.clone()));
        let mut corrupt = body.clone();
        corrupt[100] ^= 1;
        fs::write(&model, seed_corrupt(corrupt)).unwrap();

        install(&paths, &spec(url), &AtomicBool::new(false), &mut |_| {}).unwrap();

        assert_eq!(fs::read(&model).unwrap(), body);
        assert!(fs::read_dir(paths.whisper_model_dir.clone())
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("invalid-")));
        server.join().unwrap();
    }

    fn seed_corrupt(mut bytes: Vec<u8>) -> Vec<u8> {
        bytes[..4].copy_from_slice(&0x6767_6d6c_u32.to_le_bytes());
        bytes
    }

    #[test]
    fn cancellation_does_not_publish_the_model() {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(directory.path());
        paths.ensure_dirs().unwrap();
        let spec = spec("http://127.0.0.1:9/none".into());
        let model = path_for(&paths, &spec);

        let error = install(&paths, &spec, &AtomicBool::new(true), &mut |_| {}).unwrap_err();
        assert!(error.is_cancelled());
        assert!(!model.exists());
    }
}
