use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_RANGE, RANGE};

use crate::{ModelError, Result};

pub const RANGE_BYTES: u64 = 8 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Progress {
    pub downloaded: u64,
    pub total: u64,
    pub fraction: f32,
    pub speed_bps: f64,
    pub eta_secs: u64,
}

pub fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(15))
        .user_agent(concat!("JarvisWindows/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(ModelError::from)
}

#[allow(clippy::too_many_arguments)]
pub fn download_ranged(
    client: &Client,
    url: &str,
    destination: &std::path::Path,
    expected_bytes: u64,
    expected_sha256: &str,
    cancel: &AtomicBool,
    allow_hugging_face_redirects: bool,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    if expected_bytes == 0 {
        return Err(ModelError::message("model file has no expected size"));
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut downloaded = std::fs::metadata(destination)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if downloaded > expected_bytes {
        std::fs::remove_file(destination)?;
        downloaded = 0;
    }
    if downloaded == expected_bytes {
        if sha256_file(destination)? == expected_sha256 {
            progress(Progress {
                downloaded,
                total: expected_bytes,
                fraction: 1.0,
                speed_bps: 0.0,
                eta_secs: 0,
            });
            return Ok(());
        }
        std::fs::remove_file(destination)?;
        downloaded = 0;
    }

    let initial = downloaded;
    let started = Instant::now();
    let mut last_report = Instant::now();

    while downloaded < expected_bytes {
        if cancel.load(Ordering::SeqCst) {
            return Err(ModelError::Cancelled);
        }

        let end = (downloaded + RANGE_BYTES - 1).min(expected_bytes - 1);
        let mut response = client
            .get(url)
            .header(RANGE, format!("bytes={downloaded}-{end}"))
            .header(ACCEPT_ENCODING, "identity")
            .send()?
            .error_for_status()?;

        if allow_hugging_face_redirects {
            validate_hugging_face_url(response.url())?;
        }

        let status = response.status().as_u16();
        let expected_body = match status {
            206 => {
                let value = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| ModelError::message("model server omitted Content-Range"))?;
                let (first, last, total) = parse_content_range(value)?;
                if first != downloaded || last > end || total != expected_bytes {
                    return Err(ModelError::message(
                        "model server returned an unexpected Content-Range",
                    ));
                }
                last - first + 1
            }
            200 => {
                if response.content_length() != Some(expected_bytes) {
                    return Err(ModelError::message(
                        "model server ignored Range with an unexpected full size",
                    ));
                }
                downloaded = 0;
                expected_bytes
            }
            _ => {
                return Err(ModelError::message(format!(
                    "model server returned HTTP {status}"
                )))
            }
        };

        if let Some(length) = response.content_length() {
            if length != expected_body {
                return Err(ModelError::message(
                    "model response length disagrees with Content-Range",
                ));
            }
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(destination)?;
        if status == 200 {
            file.set_len(0)?;
        }

        let mut received = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(ModelError::Cancelled);
            }
            let read = std::io::Read::read(&mut response, &mut buffer)?;
            if read == 0 {
                break;
            }
            if received + read as u64 > expected_body {
                return Err(ModelError::message(
                    "model response body exceeded the declared range",
                ));
            }
            std::io::Write::write_all(&mut file, &buffer[..read])?;
            received += read as u64;
            downloaded += read as u64;

            if last_report.elapsed() >= PROGRESS_INTERVAL {
                progress(progress_of(
                    downloaded,
                    expected_bytes,
                    initial,
                    started.elapsed(),
                ));
                last_report = Instant::now();
            }
        }
        file.sync_data()?;

        if received != expected_body {
            return Err(ModelError::message(
                "model response ended early; partial file retained for resume",
            ));
        }
    }

    let digest = sha256_file(destination)?;
    if digest != expected_sha256 {
        return Err(ModelError::message(
            "model checksum verification failed; delete the partial file and retry",
        ));
    }

    progress(Progress {
        downloaded,
        total: expected_bytes,
        fraction: 1.0,
        speed_bps: 0.0,
        eta_secs: 0,
    });
    Ok(())
}

fn progress_of(downloaded: u64, total: u64, initial: u64, elapsed: Duration) -> Progress {
    let speed = downloaded.saturating_sub(initial) as f64 / elapsed.as_secs_f64().max(0.001);
    let remaining = total.saturating_sub(downloaded);
    Progress {
        downloaded,
        total,
        fraction: downloaded as f32 / total as f32,
        speed_bps: speed,
        eta_secs: if speed > 1.0 {
            (remaining as f64 / speed) as u64
        } else {
            0
        },
    }
}

fn parse_content_range(value: &str) -> Result<(u64, u64, u64)> {
    let value = value
        .strip_prefix("bytes ")
        .ok_or_else(|| ModelError::message("invalid Content-Range unit"))?;
    let (interval, total) = value
        .split_once('/')
        .ok_or_else(|| ModelError::message("invalid Content-Range"))?;
    let (first, last) = interval
        .split_once('-')
        .ok_or_else(|| ModelError::message("invalid Content-Range bounds"))?;
    let first = first
        .parse::<u64>()
        .map_err(|_| ModelError::message("invalid Content-Range start"))?;
    let last = last
        .parse::<u64>()
        .map_err(|_| ModelError::message("invalid Content-Range end"))?;
    let total = total
        .parse::<u64>()
        .map_err(|_| ModelError::message("invalid Content-Range total"))?;
    if first > last || last >= total {
        return Err(ModelError::message("invalid Content-Range bounds"));
    }
    Ok((first, last, total))
}

pub fn sha256_file(path: &std::path::Path) -> Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        use sha2::Digest;
        hasher.update(&buffer[..read]);
    }
    use sha2::Digest;
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

pub(crate) fn validate_hugging_face_url(url: &reqwest::Url) -> Result<()> {
    let host = url
        .host_str()
        .ok_or_else(|| ModelError::message("model download has no host"))?;
    let allowed = url.scheme() == "https"
        && (host == "huggingface.co"
            || host.ends_with(".huggingface.co")
            || host.ends_with(".hf.co"));
    if !allowed {
        return Err(ModelError::message(
            "model download was redirected outside Hugging Face",
        ));
    }
    Ok(())
}

pub fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_000_000.0 {
        format!("{:.1} MB/s", bytes_per_sec / 1_000_000.0)
    } else if bytes_per_sec >= 1_000.0 {
        format!("{:.0} KB/s", bytes_per_sec / 1_000.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

pub fn format_duration(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    } else if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::AtomicBool;
    use std::thread;

    fn payload() -> Vec<u8> {
        let mut bytes = vec![42_u8; 1024];
        bytes[..4].copy_from_slice(&0x6767_6d6c_u32.to_le_bytes());
        bytes
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
        let url = format!("http://{}/model.bin", listener.local_addr().unwrap());
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

    fn client() -> Client {
        Client::builder().no_proxy().build().unwrap()
    }

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn resumes_from_the_actual_partial_size() {
        let body = payload();
        let (url, server) = server(vec![response(
            "206 Partial Content",
            "Content-Range: bytes 8-1023/1024\r\nContent-Length: 1016\r\n",
            &body[8..],
        )]);
        let directory = dir();
        let path = directory.path().join("model.bin");
        std::fs::write(&path, &body[..8]).unwrap();

        download_ranged(
            &client(),
            &url,
            &path,
            1024,
            &digest(&body),
            &AtomicBool::new(false),
            false,
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), body);
        let requests = server.join().unwrap();
        assert!(requests[0].to_lowercase().contains("range: bytes=8-1023"));
    }

    #[test]
    fn ignored_range_restarts_instead_of_appending() {
        let body = payload();
        let (url, server) = server(vec![response("200 OK", "Content-Length: 1024\r\n", &body)]);
        let directory = dir();
        let path = directory.path().join("model.bin");
        std::fs::write(&path, &body[..8]).unwrap();

        download_ranged(
            &client(),
            &url,
            &path,
            1024,
            &digest(&body),
            &AtomicBool::new(false),
            false,
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), body);
        server.join().unwrap();
    }

    #[test]
    fn errors_keep_the_partial_for_resume() {
        for response in [
            response("404 Not Found", "Content-Length: 4\r\n", b"html"),
            response(
                "206 Partial Content",
                "Content-Range: bytes 0-1023/1024\r\nContent-Length: 1024\r\n",
                &payload(),
            ),
        ] {
            let (url, server) = server(vec![response]);
            let directory = dir();
            let path = directory.path().join("model.bin");
            std::fs::write(&path, &payload()[..8]).unwrap();

            assert!(download_ranged(
                &client(),
                &url,
                &path,
                1024,
                &digest(&payload()),
                &AtomicBool::new(false),
                false,
                &mut |_| {},
            )
            .is_err());
            assert_eq!(std::fs::read(&path).unwrap(), payload()[..8]);
            server.join().unwrap();
        }
    }

    #[test]
    fn interrupted_response_resumes_on_the_next_attempt() {
        let body = payload();
        let (url, server) = server(vec![
            response(
                "206 Partial Content",
                "Content-Range: bytes 8-1023/1024\r\nContent-Length: 1016\r\n",
                &body[8..101],
            ),
            response(
                "206 Partial Content",
                "Content-Range: bytes 101-1023/1024\r\nContent-Length: 923\r\n",
                &body[101..],
            ),
        ]);
        let directory = dir();
        let path = directory.path().join("model.bin");
        std::fs::write(&path, &body[..8]).unwrap();

        assert!(download_ranged(
            &client(),
            &url,
            &path,
            1024,
            &digest(&body),
            &AtomicBool::new(false),
            false,
            &mut |_| {},
        )
        .is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 101);

        download_ranged(
            &client(),
            &url,
            &path,
            1024,
            &digest(&body),
            &AtomicBool::new(false),
            false,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), body);

        let requests = server.join().unwrap();
        assert!(requests[1].to_lowercase().contains("range: bytes=101-1023"));
    }

    #[test]
    fn cancellation_keeps_partial_bytes() {
        let body = payload();
        let directory = dir();
        let path = directory.path().join("model.bin");
        std::fs::write(&path, &body[..8]).unwrap();
        let cancel = AtomicBool::new(true);

        let error = download_ranged(
            &client(),
            "http://127.0.0.1:9/unreachable",
            &path,
            1024,
            &digest(&body),
            &cancel,
            false,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(error.is_cancelled());
        assert_eq!(std::fs::read(&path).unwrap(), body[..8]);
    }

    #[test]
    fn a_complete_verified_file_needs_no_request() {
        let body = payload();
        let directory = dir();
        let path = directory.path().join("model.bin");
        std::fs::write(&path, &body).unwrap();

        download_ranged(
            &client(),
            "http://127.0.0.1:9/unreachable",
            &path,
            1024,
            &digest(&body),
            &AtomicBool::new(false),
            false,
            &mut |_| {},
        )
        .unwrap();
    }

    #[test]
    fn checksum_mismatch_is_rejected() {
        let body = payload();
        let (url, server) = server(vec![response("200 OK", "Content-Length: 1024\r\n", &body)]);
        let directory = dir();
        let path = directory.path().join("model.bin");

        let error = download_ranged(
            &client(),
            &url,
            &path,
            1024,
            &"0".repeat(64),
            &AtomicBool::new(false),
            false,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(error.to_string().contains("checksum"));
        server.join().unwrap();
    }

    fn digest(bytes: &[u8]) -> String {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(bytes))
    }
}
