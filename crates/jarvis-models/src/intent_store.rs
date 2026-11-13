use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::{hex, IntentRelease, RemoteRelease, ALLOWED_FILES};
use crate::{download, ModelError, Result};

pub const RECEIPT_LIMIT: u64 = 8192;
pub const MANIFEST_LIMIT: u64 = 64_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub commit: String,
    pub directory: String,
    pub version: String,
    pub manifest_sha256: String,
    pub generation: String,
}

pub fn root(paths: &jarvis_core::AppPaths) -> PathBuf {
    paths.intent_dir.clone()
}

pub fn receipt_from(root: &Path, name: &str) -> Option<Receipt> {
    let receipt: Receipt =
        serde_json::from_slice(&read_bounded(&root.join(name), RECEIPT_LIMIT).ok()?).ok()?;
    if !receipt_is_well_formed(&receipt) {
        return None;
    }
    Some(receipt)
}

pub fn receipt(root: &Path) -> Option<Receipt> {
    receipt_from(root, "current.json")
}

pub fn receipt_is_well_formed(receipt: &Receipt) -> bool {
    hex(&receipt.commit, 40)
        && hex(&receipt.manifest_sha256, 64)
        && receipt.directory.len() == 73
        && receipt
            .directory
            .starts_with(&format!("{}-", receipt.commit))
        && hex(&receipt.directory[41..], 32)
        && !receipt.version.is_empty()
        && !receipt.generation.is_empty()
}

pub fn read_manifest(folder: &Path) -> Result<(Vec<u8>, IntentRelease)> {
    let path = folder.join("model-release.json");
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| ModelError::message("the intent model manifest is missing"))?;
    if !metadata.file_type().is_file() {
        return Err(ModelError::message("the intent model manifest is invalid"));
    }
    let data = read_bounded(&path, MANIFEST_LIMIT)?;
    let manifest: IntentRelease = serde_json::from_slice(&data)?;
    manifest.validate()?;
    Ok((data, manifest))
}

pub fn verify(root: &Path, selected: &Receipt) -> Result<PathBuf> {
    if !receipt_is_well_formed(selected) {
        return Err(ModelError::message(
            "invalid installed intent model receipt",
        ));
    }
    let versions = root.join("versions");
    let folder = versions.join(&selected.directory);
    let metadata = fs::symlink_metadata(&folder)
        .map_err(|_| ModelError::message("the installed intent model is missing"))?;
    if !metadata.file_type().is_dir() {
        return Err(ModelError::message("the installed intent model is invalid"));
    }
    if folder.canonicalize()?.parent() != Some(versions.canonicalize()?.as_path()) {
        return Err(ModelError::message(
            "the installed intent model is outside its folder",
        ));
    }

    let (data, manifest) = read_manifest(&folder)?;
    if download::sha256_bytes(&data) != selected.manifest_sha256 {
        return Err(ModelError::message(
            "the installed intent model manifest checksum does not match",
        ));
    }
    if manifest.version != selected.version {
        return Err(ModelError::message(
            "the installed intent model version is inconsistent",
        ));
    }
    verify_folder(&folder, &manifest)?;
    Ok(folder)
}

pub fn verify_folder(folder: &Path, manifest: &IntentRelease) -> Result<()> {
    manifest.validate()?;
    let names: HashSet<&str> = manifest
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect();
    if names.len() != manifest.files.len() || !names.iter().all(|name| ALLOWED_FILES.contains(name))
    {
        return Err(ModelError::message(
            "the model release has an invalid file list",
        ));
    }

    let mut total = 0_u64;
    for file in &manifest.files {
        let path = folder.join(&file.name);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| ModelError::message(format!("the model file {} is missing", file.name)))?;
        if !metadata.file_type().is_file() || metadata.len() != file.size_bytes {
            return Err(ModelError::message(format!(
                "the model file {} is incomplete",
                file.name
            )));
        }
        if download::sha256_file(&path)? != file.sha256 {
            return Err(ModelError::message(format!(
                "checksum verification failed for {}",
                file.name
            )));
        }
        total += file.size_bytes;
        if total > 350_000_000 {
            return Err(ModelError::message(
                "the model release exceeds the supported size",
            ));
        }
    }

    let config = read_bounded(&folder.join("config.json"), 64_000)?;
    let config: Value = serde_json::from_slice(&config)?;
    if config.get("model_type").and_then(Value::as_str) != Some("t5") {
        return Err(ModelError::message(
            "the model architecture is incompatible with this build",
        ));
    }
    Ok(())
}

pub fn staging_dir(root: &Path, commit: &str) -> Result<PathBuf> {
    if !hex(commit, 40) {
        return Err(ModelError::message("invalid intent model revision"));
    }
    fs::create_dir_all(root)?;
    let staging = root.join(format!("download-{commit}"));
    fs::create_dir_all(&staging)?;
    Ok(staging)
}

pub fn clear_other_staging(root: &Path, keep: &Path) -> Result<()> {
    let keep = keep.file_name().and_then(|name| name.to_str());
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("download-") && Some(name.as_ref()) != keep {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}

pub fn activate(root: &Path, staging: &Path, release: &RemoteRelease) -> Result<Receipt> {
    release.validate()?;
    let staging_parent = staging
        .parent()
        .ok_or_else(|| ModelError::message("invalid model staging directory"))?;
    if staging_parent != root
        || !staging
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("download-"))
    {
        return Err(ModelError::message("invalid model staging directory"));
    }

    verify_folder(staging, &release.manifest)?;
    write_bytes(&staging.join("model-release.json"), &release.manifest_data)?;

    let directory = format!("{}-{}", release.commit, nonce32());
    let versions = root.join("versions");
    fs::create_dir_all(&versions)?;
    let destination = versions.join(&directory);
    if destination.exists() {
        return Err(ModelError::message(
            "an intent model release with this name already exists",
        ));
    }
    fs::rename(staging, &destination)?;

    let receipt = Receipt {
        commit: release.commit.clone(),
        directory,
        version: release.manifest.version.clone(),
        manifest_sha256: download::sha256_file(&destination.join("model-release.json"))?,
        generation: nonce32(),
    };

    if let Err(error) = write_json(&root.join("current.json"), &receipt) {
        let _ = fs::remove_dir_all(&destination);
        return Err(error);
    }
    Ok(receipt)
}

pub fn remember_good(root: &Path, selected: &Receipt) -> Result<()> {
    fs::create_dir_all(root)?;
    write_json(&root.join("last-good.json"), selected)
}

pub fn load_selection(root: &Path) -> Result<(PathBuf, Receipt)> {
    for name in ["current.json", "last-good.json"] {
        if let Some(selected) = receipt_from(root, name) {
            if let Ok(folder) = verify(root, &selected) {
                return Ok((folder, selected));
            }
        }
    }
    Err(ModelError::message(
        "the intent model is not installed or failed verification",
    ))
}

pub fn load_folder(root: &Path) -> Result<PathBuf> {
    load_selection(root).map(|(folder, _)| folder)
}

pub fn current_info(root: &Path) -> Option<Receipt> {
    receipt(root)
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        return Err(ModelError::message("model metadata file is too large"));
    }
    Ok(data)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    write_bytes(path, &bytes)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", nonce32()));
    fs::write(&temporary, bytes)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn nonce32() -> String {
    use sha2::{Digest, Sha256};
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let mixed = format!("{stamp}-{}-{}", std::process::id(), unique_counter());
    let digest = Sha256::digest(mixed.as_bytes());
    format!("{digest:x}")[..32].to_string()
}

fn unique_counter() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest_data(version: &str, files: &[(&str, &[u8], String)]) -> Vec<u8> {
        let files: Vec<Value> = files
            .iter()
            .map(|(name, data, sha)| {
                json!({
                    "name": name,
                    "size_bytes": data.len(),
                    "sha256": sha
                })
            })
            .collect();
        serde_json::to_vec(&json!({
            "format_version": 1,
            "runtime": "jarvis-t5-onnx-v1",
            "license": "apache-2.0",
            "version": version,
            "intent_schema_version": 5,
            "max_source_len": 128,
            "max_target_len": 448,
            "files": files
        }))
        .unwrap()
    }

    fn fixture(root: &Path) -> (Receipt, PathBuf) {
        let commit = "a".repeat(40);
        let directory = format!("{commit}-{}", "b".repeat(32));
        let folder = root.join("versions").join(&directory);
        fs::create_dir_all(&folder).unwrap();
        let config = br#"{"model_type":"t5"}"#;
        let files: Vec<(&str, &[u8], String)> = vec![
            ("config.json", config, digest(config)),
            (
                "encoder_model_quantized.onnx",
                b"encoder",
                digest(b"encoder"),
            ),
            (
                "decoder_model_merged_quantized.onnx",
                b"decoder",
                digest(b"decoder"),
            ),
            ("added_tokens.json", b"{}", digest(b"{}")),
            ("spiece.model", b"spiece", digest(b"spiece")),
            ("model_meta.json", b"{}", digest(b"{}")),
            ("LICENSE", b"license", digest(b"license")),
            ("NOTICE", b"notice", digest(b"notice")),
        ];
        for (name, data, _) in &files {
            fs::write(folder.join(name), data).unwrap();
        }
        let manifest = manifest_data("1.0.2", &files);
        fs::write(folder.join("model-release.json"), &manifest).unwrap();
        let receipt = Receipt {
            commit,
            directory,
            version: "1.0.2".into(),
            manifest_sha256: digest(&manifest),
            generation: "test".into(),
        };
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join("current.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        (receipt, folder)
    }

    fn digest(bytes: &[u8]) -> String {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(bytes))
    }

    #[test]
    fn verifies_an_installed_release_and_last_good() {
        let directory = tempfile::tempdir().unwrap();
        let (receipt, folder) = fixture(directory.path());
        assert_eq!(
            receipt_from(directory.path(), "current.json"),
            Some(receipt.clone())
        );
        assert_eq!(verify(directory.path(), &receipt).unwrap(), folder);
        remember_good(directory.path(), &receipt).unwrap();
        assert_eq!(
            receipt_from(directory.path(), "last-good.json"),
            Some(receipt)
        );
        assert_eq!(load_folder(directory.path()).unwrap(), folder);
    }

    #[test]
    fn rejects_tampered_files_and_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let (receipt, folder) = fixture(directory.path());
        fs::write(folder.join("config.json"), "damaged").unwrap();
        assert!(verify(directory.path(), &receipt).is_err());

        let directory = tempfile::tempdir().unwrap();
        let (receipt, folder) = fixture(directory.path());
        fs::write(folder.join("model-release.json"), "{}").unwrap();
        assert!(verify(directory.path(), &receipt).is_err());
    }

    #[test]
    fn rejects_traversal_and_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let (mut receipt, _folder) = fixture(directory.path());
        receipt.directory = "../../elsewhere".into();
        assert!(verify(directory.path(), &receipt).is_err());

        let directory = tempfile::tempdir().unwrap();
        let (receipt, folder) = fixture(directory.path());
        fs::remove_file(folder.join("spiece.model")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(folder.join("config.json"), folder.join("spiece.model"))
            .unwrap();
        assert!(verify(directory.path(), &receipt).is_err());
    }

    #[test]
    fn rejects_unknown_schema_and_oversized_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let (mut receipt, _folder) = fixture(directory.path());
        let path = _folder.join("model-release.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["intent_schema_version"] = json!(6);
        fs::write(&path, manifest.to_string()).unwrap();
        receipt.manifest_sha256 = digest(&fs::read(&path).unwrap());
        assert!(verify(directory.path(), &receipt).is_err());

        let directory = tempfile::tempdir().unwrap();
        fixture(directory.path());
        fs::write(
            directory.path().join("current.json"),
            " ".repeat(RECEIPT_LIMIT as usize + 1),
        )
        .unwrap();
        assert!(receipt_from(directory.path(), "current.json").is_none());
    }

    #[test]
    fn activates_a_verified_staging_directory_and_writes_a_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let staging = staging_dir(directory.path(), &"c".repeat(40)).unwrap();
        let config = br#"{"model_type":"t5"}"#;
        let files: Vec<(&str, &[u8], String)> = vec![
            ("config.json", config, digest(config)),
            (
                "encoder_model_quantized.onnx",
                b"encoder",
                digest(b"encoder"),
            ),
            (
                "decoder_model_merged_quantized.onnx",
                b"decoder",
                digest(b"decoder"),
            ),
            ("added_tokens.json", b"{}", digest(b"{}")),
            ("spiece.model", b"spiece", digest(b"spiece")),
            ("model_meta.json", b"{}", digest(b"{}")),
            ("LICENSE", b"license", digest(b"license")),
            ("NOTICE", b"notice", digest(b"notice")),
        ];
        for (name, data, _) in &files {
            fs::write(staging.join(name), data).unwrap();
        }
        let manifest_data = manifest_data("1.2.3", &files);
        let manifest: IntentRelease = serde_json::from_slice(&manifest_data).unwrap();
        let release = RemoteRelease {
            commit: "c".repeat(40),
            manifest,
            manifest_data,
        };

        let receipt = activate(directory.path(), &staging, &release).unwrap();
        assert_eq!(receipt.version, "1.2.3");
        assert_eq!(
            receipt_from(directory.path(), "current.json"),
            Some(receipt.clone())
        );
        assert!(verify(directory.path(), &receipt).is_ok());
        assert!(!staging.exists());
    }

    #[test]
    fn activation_rejects_incompatible_configs_and_leaves_no_pointer() {
        let directory = tempfile::tempdir().unwrap();
        let staging = staging_dir(directory.path(), &"d".repeat(40)).unwrap();
        let config = br#"{"model_type":"bert"}"#;
        let files: Vec<(&str, &[u8], String)> = vec![
            ("config.json", config, digest(config)),
            (
                "encoder_model_quantized.onnx",
                b"encoder",
                digest(b"encoder"),
            ),
            (
                "decoder_model_merged_quantized.onnx",
                b"decoder",
                digest(b"decoder"),
            ),
            ("added_tokens.json", b"{}", digest(b"{}")),
            ("spiece.model", b"spiece", digest(b"spiece")),
            ("model_meta.json", b"{}", digest(b"{}")),
            ("LICENSE", b"license", digest(b"license")),
            ("NOTICE", b"notice", digest(b"notice")),
        ];
        for (name, data, _) in &files {
            fs::write(staging.join(name), data).unwrap();
        }
        let manifest_data = manifest_data("1.2.3", &files);
        let manifest: IntentRelease = serde_json::from_slice(&manifest_data).unwrap();
        let release = RemoteRelease {
            commit: "d".repeat(40),
            manifest,
            manifest_data,
        };

        assert!(activate(directory.path(), &staging, &release).is_err());
        assert!(!directory.path().join("current.json").exists());
    }
}
