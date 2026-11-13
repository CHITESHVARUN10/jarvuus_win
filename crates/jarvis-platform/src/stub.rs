use std::path::Path;
use std::process::Command;

use jarvis_core::setup::{Consent, GpuInfo, MicrophoneAccess};

pub fn microphone() -> MicrophoneAccess {
    MicrophoneAccess {
        consent: Consent::Unknown,
        detail: "Microphone privacy checks run on Windows; this is a UI preview build.".into(),
    }
}

pub fn gpu() -> GpuInfo {
    GpuInfo {
        adapters: Vec::new(),
        note: Some("GPU detection runs on Windows; this is a UI preview build.".into()),
    }
}

pub fn open_microphone_settings() -> Result<(), String> {
    Err("Windows settings are only available on Windows".into())
}

pub fn open_target(target: &str) -> Result<(), String> {
    Command::new("open")
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn open_path(path: &Path) -> Result<(), String> {
    Command::new("open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}
