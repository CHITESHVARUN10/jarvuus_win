use std::path::Path;

pub use jarvis_core::setup::{Consent, GpuAdapter, GpuInfo, MicrophoneAccess, SystemProbe};

#[derive(Clone, Copy, Debug, Default)]
pub struct Probe;

#[cfg(not(windows))]
mod stub;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
use stub as imp;
#[cfg(windows)]
use windows as imp;

impl SystemProbe for Probe {
    fn microphone(&self) -> MicrophoneAccess {
        imp::microphone()
    }

    fn gpu(&self) -> GpuInfo {
        imp::gpu()
    }
}

pub fn open_microphone_settings() -> Result<(), String> {
    imp::open_microphone_settings()
}

pub fn open_url(url: &str) -> Result<(), String> {
    let supported = url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with("ms-settings:");
    if !supported {
        return Err("Unsupported link".into());
    }
    imp::open_target(url)
}

pub fn open_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }
    imp::open_path(path)
}
