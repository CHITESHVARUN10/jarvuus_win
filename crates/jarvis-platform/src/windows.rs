use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

use jarvis_core::setup::{Consent, GpuAdapter, GpuInfo, MicrophoneAccess};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MICROPHONE_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

pub fn microphone() -> MicrophoneAccess {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let device = read_consent(&hkcu, MICROPHONE_KEY);
    let desktop = read_consent(&hkcu, &format!(r"{MICROPHONE_KEY}\NonPackaged"));

    if device == Some(false) || desktop == Some(false) {
        return MicrophoneAccess {
            consent: Consent::Denied,
            detail:
                "Microphone access is blocked in Windows privacy settings. Allow desktop apps to use the microphone."
                    .into(),
        };
    }

    MicrophoneAccess {
        consent: Consent::Allowed,
        detail: "Windows privacy settings allow desktop apps to use the microphone.".into(),
    }
}

fn read_consent(root: &RegKey, path: &str) -> Option<bool> {
    let key = root.open_subkey(path).ok()?;
    let value: String = key.get_value("Value").ok()?;
    match value.as_str() {
        "Allow" => Some(true),
        "Deny" => Some(false),
        _ => None,
    }
}

pub fn gpu() -> GpuInfo {
    match unsafe { enumerate_adapters() } {
        Ok(mut adapters) => {
            adapters.sort_by_key(|adapter| std::cmp::Reverse(adapter.vram_mb));
            adapters.dedup_by(|a, b| a.name == b.name);
            let note = if adapters.is_empty() {
                Some("No hardware display adapter detected; Whisper will use the CPU.".into())
            } else {
                None
            };
            GpuInfo { adapters, note }
        }
        Err(error) => GpuInfo {
            adapters: Vec::new(),
            note: Some(format!(
                "GPU probe unavailable ({error}); CPU fallback will be used."
            )),
        },
    }
}

unsafe fn enumerate_adapters() -> windows::core::Result<Vec<GpuAdapter>> {
    let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
    let mut adapters = Vec::new();
    let mut index = 0;

    loop {
        let adapter = match factory.EnumAdapters1(index) {
            Ok(adapter) => adapter,
            Err(_) => break,
        };
        let desc = adapter.GetDesc1()?;
        let name = String::from_utf16_lossy(&desc.Description)
            .trim_end_matches('\0')
            .trim()
            .to_string();
        let software = desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;

        if !name.is_empty() && !software && !is_software_adapter(&name) {
            let vram_mb = (desc.DedicatedVideoMemory / (1024 * 1024)) as u64;
            adapters.push(GpuAdapter {
                name,
                vram_mb: (vram_mb > 0).then_some(vram_mb),
            });
        }
        index += 1;
    }

    Ok(adapters)
}

fn is_software_adapter(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("microsoft basic")
        || name.contains("microsoft remote")
        || name.contains("hyper-v video")
        || name.contains("virtual display")
}

pub fn open_microphone_settings() -> Result<(), String> {
    open_target("ms-settings:privacy-microphone")
}

pub fn open_target(target: &str) -> Result<(), String> {
    Command::new("cmd")
        .args(["/C", "start", "", target])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn open_path(path: &Path) -> Result<(), String> {
    Command::new("explorer.exe")
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_adapter_names_are_filtered() {
        assert!(is_software_adapter("Microsoft Basic Display Adapter"));
        assert!(is_software_adapter("Microsoft Basic Render Driver"));
        assert!(is_software_adapter("Hyper-V Video"));
        assert!(!is_software_adapter("Intel(R) UHD Graphics"));
        assert!(!is_software_adapter("NVIDIA GeForce RTX 4060"));
    }
}
