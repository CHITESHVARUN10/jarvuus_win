use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use chrono::Local;

pub fn append(path: &Path, level: &str, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let line = format!(
        "{} {:<5} {}\n",
        Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z"),
        level,
        message
    );
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(line.as_bytes()));
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
}
