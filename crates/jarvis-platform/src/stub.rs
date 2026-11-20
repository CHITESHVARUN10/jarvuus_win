use std::path::Path;
use std::process::Command;

use std::sync::mpsc::Sender;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed,
    CommandPressed,
}

pub struct HotkeyRegistration;

impl HotkeyRegistration {
    pub fn stop(&mut self) {}
}

pub fn start_dictation_hotkey(
    _sender: Sender<HotkeyEvent>,
    _waker: crate::Wake,
    _hotkey: jarvis_core::hotkey::Hotkey,
) -> Result<HotkeyRegistration, String> {
    Err("Global dictation hotkeys are Windows-only in this preview.".into())
}

pub fn start_command_hotkey(
    _sender: Sender<HotkeyEvent>,
    _waker: crate::Wake,
    _hotkey: jarvis_core::hotkey::Hotkey,
) -> Result<HotkeyRegistration, String> {
    Err("Global command hotkeys are Windows-only in this preview.".into())
}

pub fn set_launch_at_login(_enabled: bool) -> Result<(), String> {
    Err("Launch at login is Windows-only in this preview.".into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertionOutcome {
    Pasted,
    ClipboardOnly,
}

pub fn insert_text(_text: &str) -> Result<InsertionOutcome, String> {
    Err("Automatic text insertion is Windows-only in this preview.".into())
}

pub fn copy_text(_text: &str) -> Result<(), String> {
    Err("Clipboard copy is Windows-only in this preview.".into())
}
