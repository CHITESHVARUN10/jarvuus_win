use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths::AppPaths;

pub const WHISPER_MODEL_FILE: &str = "ggml-large-v3-turbo.bin";
pub const WHISPER_MODEL_BYTES: u64 = 1_624_555_275;
pub const WHISPER_MODEL_SHA256: &str =
    "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69";
pub const WHISPER_MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo.bin";

pub const INTENT_MODEL_REPO: &str = "chitesh12545/jarvis-intent-t5-small";
pub const INTENT_MODEL_URL: &str = "https://huggingface.co/chitesh12545/jarvis-intent-t5-small";
pub const INTENT_ENCODER_FILE: &str = "encoder_model_quantized.onnx";
pub const INTENT_DECODER_FILE: &str = "decoder_model_merged_quantized.onnx";
pub const INTENT_REQUIRED_FILES: &[&str] = &[
    INTENT_ENCODER_FILE,
    INTENT_DECODER_FILE,
    "config.json",
    "added_tokens.json",
    "spiece.model",
    "model_meta.json",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelState {
    Installed,
    Missing,
    Incomplete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelStatus {
    pub state: ModelState,
    pub version: Option<String>,
    pub detail: String,
}

impl ModelStatus {
    pub fn missing(detail: impl Into<String>) -> Self {
        Self {
            state: ModelState::Missing,
            version: None,
            detail: detail.into(),
        }
    }

    pub fn incomplete(detail: impl Into<String>) -> Self {
        Self {
            state: ModelState::Incomplete,
            version: None,
            detail: detail.into(),
        }
    }

    pub fn installed(detail: impl Into<String>, version: Option<String>) -> Self {
        Self {
            state: ModelState::Installed,
            version,
            detail: detail.into(),
        }
    }

    pub fn is_installed(&self) -> bool {
        self.state == ModelState::Installed
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct IntentReceipt {
    pub commit: String,
    pub version: String,
    pub directory: String,
    pub manifest_sha256: String,
    pub generation: String,
}

pub fn whisper_model_path(paths: &AppPaths) -> std::path::PathBuf {
    paths.whisper_model_dir.join(WHISPER_MODEL_FILE)
}

pub fn whisper_status(paths: &AppPaths) -> ModelStatus {
    let path = whisper_model_path(paths);
    status_for_file(&path, WHISPER_MODEL_BYTES, "ggml-large-v3-turbo.bin")
}

fn status_for_file(path: &Path, expected_bytes: u64, label: &str) -> ModelStatus {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() == expected_bytes => ModelStatus::installed(
            format!("{label} · {} installed", human_bytes(expected_bytes)),
            None,
        ),
        Ok(metadata) => ModelStatus::incomplete(format!(
            "{label} is incomplete ({} of {})",
            human_bytes(metadata.len()),
            human_bytes(expected_bytes)
        )),
        Err(_) => ModelStatus::missing(format!("{label} is not installed")),
    }
}

pub fn intent_status(paths: &AppPaths) -> ModelStatus {
    if let Some(directory) = &paths.intent_override {
        if INTENT_REQUIRED_FILES
            .iter()
            .all(|file| directory.join(file).is_file())
        {
            return ModelStatus::installed(
                format!("Custom model directory · {}", directory.display()),
                None,
            );
        }
        return ModelStatus::incomplete(format!(
            "Custom model directory is missing required files · {}",
            directory.display()
        ));
    }

    let receipt_path = paths.intent_dir.join("current.json");
    let bytes = match fs::read(&receipt_path) {
        Ok(bytes) if bytes.len() <= 8192 => bytes,
        Ok(_) => return ModelStatus::incomplete("Intent model receipt is too large"),
        Err(_) => return ModelStatus::missing("Managed intent model is not installed"),
    };
    let receipt: IntentReceipt = match serde_json::from_slice(&bytes) {
        Ok(receipt) => receipt,
        Err(_) => return ModelStatus::incomplete("Intent model receipt is invalid"),
    };

    if !valid_directory_name(&receipt.directory) || !receipt.directory.starts_with(&receipt.commit)
    {
        return ModelStatus::incomplete("Intent model receipt has an invalid directory");
    }

    let folder = paths.intent_dir.join("versions").join(&receipt.directory);
    let missing: Vec<&str> = INTENT_REQUIRED_FILES
        .iter()
        .copied()
        .filter(|file| !folder.join(file).is_file())
        .collect();
    if !missing.is_empty() {
        return ModelStatus::incomplete(format!(
            "Managed release v{} is incomplete (missing {})",
            receipt.version,
            missing.join(", ")
        ));
    }

    ModelStatus::installed(
        format!("Managed release v{} installed", receipt.version),
        Some(receipt.version),
    )
}

fn valid_directory_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !name.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
        && name.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_incomplete_models_are_reported_without_hashing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(dir.path());

        assert_eq!(whisper_status(&paths).state, ModelState::Missing);
        assert_eq!(intent_status(&paths).state, ModelState::Missing);

        paths.ensure_dirs().unwrap();
        fs::write(whisper_model_path(&paths), b"partial").unwrap();
        assert_eq!(whisper_status(&paths).state, ModelState::Incomplete);
        assert!(whisper_status(&paths).detail.contains("incomplete"));
    }

    #[test]
    fn a_managed_intent_release_is_recognised_by_its_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(dir.path());
        let release = "0123456789abcdef0123456789abcdef01234567-0123456789abcdef0123456789abcdef";
        let folder = paths.intent_dir.join("versions").join(release);
        fs::create_dir_all(&folder).unwrap();
        for file in INTENT_REQUIRED_FILES {
            fs::write(folder.join(file), b"x").unwrap();
        }
        let receipt = IntentReceipt {
            commit: release[..40].to_string(),
            version: "1.2.3".into(),
            directory: release.into(),
            manifest_sha256: "0".repeat(64),
            generation: "test".into(),
        };
        fs::create_dir_all(&paths.intent_dir).unwrap();
        fs::write(
            paths.intent_dir.join("current.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();

        let status = intent_status(&paths);
        assert_eq!(status.state, ModelState::Installed);
        assert_eq!(status.version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn receipt_paths_cannot_escape_the_managed_directory() {
        assert!(valid_directory_name("abc-123"));
        assert!(!valid_directory_name("../abc"));
        assert!(!valid_directory_name("abc/def"));
        assert!(!valid_directory_name("abc\\def"));
        assert!(!valid_directory_name(""));
    }

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1_624_555_275), "1.5 GB");
    }
}
