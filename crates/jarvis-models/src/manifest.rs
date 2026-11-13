use serde::{Deserialize, Serialize};

use jarvis_core::models::{INTENT_MODEL_REPO, INTENT_REQUIRED_FILES};

use crate::{ModelError, Result};

pub const ALLOWED_FILES: &[&str] = &[
    "encoder_model_quantized.onnx",
    "decoder_model_merged_quantized.onnx",
    "config.json",
    "generation_config.json",
    "added_tokens.json",
    "spiece.model",
    "special_tokens_map.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "model_meta.json",
    "LICENSE",
    "NOTICE",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseFile {
    pub name: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentRelease {
    pub format_version: u32,
    pub runtime: String,
    #[serde(default)]
    pub license: Option<String>,
    pub version: String,
    pub intent_schema_version: u32,
    pub max_source_len: u32,
    pub max_target_len: u32,
    pub files: Vec<ReleaseFile>,
}

impl IntentRelease {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size_bytes).sum()
    }

    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1
            || self.runtime != "jarvis-t5-onnx-v1"
            || !(1..=5).contains(&self.intent_schema_version)
            || self.max_source_len != 128
            || self.max_target_len != 448
        {
            return Err(ModelError::message(
                "this model release needs a newer Jarvis build",
            ));
        }
        if !valid_version(&self.version)
            || !(6..=12).contains(&self.files.len())
            || self.files.len() > ALLOWED_FILES.len()
        {
            return Err(ModelError::message(
                "the model release has an invalid version or file count",
            ));
        }

        let mut names = std::collections::HashSet::new();
        for file in &self.files {
            if !names.insert(file.name.as_str()) {
                return Err(ModelError::message("the model release repeats a file"));
            }
            if !ALLOWED_FILES.contains(&file.name.as_str()) {
                return Err(ModelError::message(format!(
                    "the model release contains an unexpected file: {}",
                    file.name
                )));
            }
            if file.size_bytes == 0 || file.size_bytes > 256_000_000 {
                return Err(ModelError::message(
                    "the model release has an invalid file size",
                ));
            }
            if !hex(&file.sha256, 64) {
                return Err(ModelError::message(
                    "the model release has an invalid checksum",
                ));
            }
        }
        if !INTENT_REQUIRED_FILES
            .iter()
            .all(|required| names.contains(required))
        {
            return Err(ModelError::message(
                "the model release is missing required files",
            ));
        }
        if self.license.as_deref() == Some("apache-2.0")
            && !(names.contains("LICENSE") && names.contains("NOTICE"))
        {
            return Err(ModelError::message(
                "the model release is missing its license notices",
            ));
        }
        if self.total_bytes() > 350_000_000 {
            return Err(ModelError::message(
                "the model release exceeds the supported size",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct RemoteRelease {
    pub commit: String,
    pub manifest: IntentRelease,
    pub manifest_data: Vec<u8>,
}

impl RemoteRelease {
    pub fn validate(&self) -> Result<()> {
        if !hex(&self.commit, 40) {
            return Err(ModelError::message(
                "the model server returned an invalid revision",
            ));
        }
        self.manifest.validate()?;
        let decoded: IntentRelease = serde_json::from_slice(&self.manifest_data)?;
        if decoded != self.manifest {
            return Err(ModelError::message(
                "the model manifest changed during download",
            ));
        }
        Ok(())
    }
}

pub fn repository() -> &'static str {
    INTENT_MODEL_REPO
}

pub fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn valid_version(version: &str) -> bool {
    if version.is_empty() || version.len() > 80 {
        return false;
    }
    let (core, prerelease) = match version.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (version, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return false;
    }
    prerelease.is_none_or(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
    })
}

/// True when `local` is a numerically newer release than `remote`.
pub fn local_is_newer(local: &str, remote: &str) -> bool {
    let local = numeric_version(local);
    let remote = numeric_version(remote);
    match (local, remote) {
        (Some(local), Some(remote)) => local > remote,
        _ => false,
    }
}

fn numeric_version(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.split_once('-').map_or(version, |(core, _)| core);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(files: Vec<serde_json::Value>) -> IntentRelease {
        serde_json::from_value(json!({
            "format_version": 1,
            "runtime": "jarvis-t5-onnx-v1",
            "license": "apache-2.0",
            "version": "1.2.3",
            "intent_schema_version": 5,
            "max_source_len": 128,
            "max_target_len": 448,
            "files": files
        }))
        .unwrap()
    }

    fn valid_files() -> Vec<serde_json::Value> {
        INTENT_REQUIRED_FILES
            .iter()
            .map(|name| json!({"name": name, "size_bytes": 10, "sha256": "a".repeat(64)}))
            .chain([
                json!({"name": "LICENSE", "size_bytes": 10, "sha256": "a".repeat(64)}),
                json!({"name": "NOTICE", "size_bytes": 10, "sha256": "a".repeat(64)}),
            ])
            .collect()
    }

    #[test]
    fn accepts_the_expected_release_shape() {
        release(valid_files()).validate().unwrap();
    }

    #[test]
    fn rejects_missing_files_and_notices() {
        let mut files = valid_files();
        files.retain(|file| file["name"] != "spiece.model");
        assert!(release(files).validate().is_err());

        let mut files = valid_files();
        files.retain(|file| file["name"] != "NOTICE");
        assert!(release(files).validate().is_err());
    }

    #[test]
    fn rejects_unknown_files_and_bad_sizes() {
        let mut files = valid_files();
        files.push(json!({"name": "evil.onnx", "size_bytes": 10, "sha256": "a".repeat(64)}));
        assert!(release(files).validate().is_err());

        let mut files = valid_files();
        files[0]["size_bytes"] = json!(0);
        assert!(release(files).validate().is_err());
    }

    #[test]
    fn rejects_wrong_runtime_and_schema() {
        let mut value = release(valid_files());
        value.runtime = "other".into();
        assert!(value.validate().is_err());

        let mut value = release(valid_files());
        value.intent_schema_version = 6;
        assert!(value.validate().is_err());
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(local_is_newer("1.10.0", "1.9.9"));
        assert!(!local_is_newer("1.0.2", "1.0.2"));
        assert!(!local_is_newer("1.0.2", "2.0.0"));
        assert!(valid_version("1.2.3-rc.1"));
        assert!(!valid_version("1.2"));
    }
}
