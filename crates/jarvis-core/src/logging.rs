use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use chrono::Local;

pub const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
static LOG_LOCK: Mutex<()> = Mutex::new(());

pub fn append(path: &Path, level: &str, message: &str) {
    append_with_limit(path, level, message, MAX_LOG_BYTES);
}

fn append_with_limit(path: &Path, level: &str, message: &str, limit: u64) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let line = format!(
        "{} {:<5} {}\n",
        Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z"),
        level,
        message
    );

    // Logging happens from the UI, command, speech and setup threads. Keep the
    // rotate-and-append pair atomic so a rotation cannot lose a line or append
    // to the old file after it moved.
    let _guard = LOG_LOCK.lock().ok();
    rotate_if_needed(path, line.len() as u64, limit);
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(line.as_bytes()));
}

fn rotate_if_needed(path: &Path, incoming: u64, limit: u64) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len().saturating_add(incoming) <= limit {
        return;
    }
    let Some(name) = path.file_name() else {
        return;
    };
    let backup = path.with_file_name(format!("{}.1", name.to_string_lossy()));
    let _ = fs::remove_file(&backup);
    let _ = fs::rename(path, backup);
}

pub fn info(path: &Path, message: &str) {
    append(path, "INFO", message);
}

pub fn warn(path: &Path, message: &str) {
    append(path, "WARN", message);
}

pub fn error(path: &Path, message: &str) {
    append(path, "ERROR", message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_timestamped_lines_and_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("commands.log");

        info(&path, "first");
        warn(&path, "second");

        let contents = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("INFO"));
        assert!(lines[0].ends_with("first"));
        assert!(lines[1].contains("WARN"));
    }

    #[test]
    fn rotates_the_previous_file_when_the_limit_is_reached() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commands.log");

        append_with_limit(&path, "INFO", "first", 1);
        append_with_limit(&path, "INFO", "second", 1);

        let backup = dir.path().join("commands.log.1");
        assert!(backup.exists());
        assert!(fs::read_to_string(&backup).unwrap().contains("first"));
        assert!(fs::read_to_string(&path).unwrap().contains("second"));
        assert!(!fs::read_to_string(&path).unwrap().contains("first"));
    }
}
