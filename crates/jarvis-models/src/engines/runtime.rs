use std::sync::OnceLock;

use jarvis_core::AppPaths;
use ort::environment::Environment;

use crate::{ModelError, Result};

static ENVIRONMENT: OnceLock<std::result::Result<Environment, String>> = OnceLock::new();

pub fn environment(paths: &AppPaths) -> Result<&'static Environment> {
    ENVIRONMENT
        .get_or_init(|| build_environment(paths))
        .as_ref()
        .map_err(|error| ModelError::message(error.clone()))
}

#[cfg(windows)]
fn build_environment(_paths: &AppPaths) -> std::result::Result<Environment, String> {
    ort::init()
        .with_telemetry(false)
        .build()
        .map_err(|error| error.to_string())
}

#[cfg(not(windows))]
fn build_environment(paths: &AppPaths) -> std::result::Result<Environment, String> {
    let library = ort_library(paths).map_err(|error| error.to_string())?;
    ort::init_from(library)
        .map_err(|error| error.to_string())?
        .with_telemetry(false)
        .build()
        .map_err(|error| error.to_string())
}

#[cfg(not(windows))]
fn ort_library(paths: &AppPaths) -> Result<std::path::PathBuf> {
    let name = if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    };

    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("JARVIS_ORT_DYLIB") {
        candidates.push(std::path::PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(name));
            candidates.push(parent.join("runtime").join(name));
        }
    }
    candidates.push(paths.data_dir.join("runtime").join(name));

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            ModelError::message(format!(
                "ONNX Runtime is not installed. Put {name} next to Jarvis or set JARVIS_ORT_DYLIB."
            ))
        })
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn missing_runtime_has_a_helpful_error() {
        let paths = AppPaths::with_data_dir(tempfile::tempdir().unwrap().path());
        let error = ort_library(&paths).unwrap_err().to_string();
        assert!(error.contains("ONNX Runtime"));
    }
}
