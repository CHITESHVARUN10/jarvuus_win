use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, LPARAM, WPARAM};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, SendInput, UnregisterHotKey, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VIRTUAL_KEY,
    VK_CONTROL, VK_D, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, PostThreadMessageW, MSG, WM_HOTKEY, WM_QUIT,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed,
}

pub struct HotkeyRegistration {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl HotkeyRegistration {
    pub fn stop(&mut self) {
        if let Some(join) = self.join.take() {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            let _ = join.join();
        }
    }
}

impl Drop for HotkeyRegistration {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start_dictation_hotkey(
    sender: Sender<HotkeyEvent>,
    waker: crate::Wake,
) -> Result<HotkeyRegistration, String> {
    let (ready, readiness) = mpsc::channel();
    let join = thread::spawn(move || {
        let thread_id = unsafe { GetCurrentThreadId() };
        let modifiers = MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT;
        if let Err(error) = unsafe { RegisterHotKey(None, 1, modifiers, VK_D.0 as u32) } {
            let _ = ready.send(Err(format!(
                "Could not register Ctrl+Shift+D: {error}. Another app may already use it."
            )));
            return;
        }
        let _ = ready.send(Ok(thread_id));

        let mut message = MSG::default();
        loop {
            let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
            if result.0 == 0 || result.0 == -1 {
                break;
            }
            if message.message == WM_HOTKEY && message.wParam.0 == 1 {
                if sender.send(HotkeyEvent::Pressed).is_err() {
                    break;
                }
                waker();
            }
        }
        unsafe {
            let _ = UnregisterHotKey(None, 1);
        }
    });

    match readiness.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(thread_id)) => Ok(HotkeyRegistration {
            thread_id,
            join: Some(join),
        }),
        Ok(Err(error)) => {
            let _ = join.join();
            Err(error)
        }
        Err(_) => Err("Timed out while registering the dictation hotkey.".into()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertionOutcome {
    Pasted,
    ClipboardOnly,
}

pub fn copy_text(text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("There is no transcript to copy.".into());
    }
    write_clipboard_text(text)
}

pub fn insert_text(text: &str) -> Result<InsertionOutcome, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("There is no transcript to paste.".into());
    }

    let previous = read_clipboard_text();
    write_clipboard_text(text)?;
    thread::sleep(Duration::from_millis(80));

    let inputs = [
        key_input(VK_CONTROL, false),
        key_input(VK_V, false),
        key_input(VK_V, true),
        key_input(VK_CONTROL, true),
    ];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == 0 {
        return Ok(InsertionOutcome::ClipboardOnly);
    }

    thread::sleep(Duration::from_millis(900));
    if read_clipboard_text().as_deref() == Some(text) {
        match previous {
            Some(previous) => {
                let _ = write_clipboard_text(&previous);
            }
            None => {
                let _ = clear_clipboard();
            }
        }
    }
    Ok(InsertionOutcome::Pasted)
}

fn key_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn open_clipboard() -> Result<(), String> {
    for _ in 0..10 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err("The clipboard is busy. Try again.".into())
}

fn write_clipboard_text(text: &str) -> Result<(), String> {
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = units.len() * std::mem::size_of::<u16>();

    open_clipboard()?;
    let result = (|| {
        unsafe { EmptyClipboard() }.map_err(|error| error.to_string())?;
        let handle =
            unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }.map_err(|error| error.to_string())?;
        let pointer = unsafe { GlobalLock(handle) } as *mut u16;
        if pointer.is_null() {
            unsafe {
                let _ = GlobalFree(Some(handle));
            }
            return Err("Could not lock the clipboard memory.".into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(units.as_ptr(), pointer, units.len());
            let _ = GlobalUnlock(handle);
        }
        match unsafe { SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(handle.0))) } {
            Ok(_) => Ok(()),
            Err(error) => {
                unsafe {
                    let _ = GlobalFree(Some(handle));
                }
                Err(format!("Could not write the clipboard: {error}"))
            }
        }
    })();
    unsafe {
        let _ = CloseClipboard();
    }
    result
}

fn read_clipboard_text() -> Option<String> {
    open_clipboard().ok()?;
    let result = (|| {
        let handle = unsafe { GetClipboardData(CF_UNICODETEXT.0 as u32) }.ok()?;
        if handle.0.is_null() {
            return None;
        }
        let global = HGLOBAL(handle.0);
        let pointer = unsafe { GlobalLock(global) } as *const u16;
        if pointer.is_null() {
            return None;
        }
        let mut length = 0;
        while unsafe { *pointer.add(length) } != 0 {
            length += 1;
        }
        let text = unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length)) };
        unsafe {
            let _ = GlobalUnlock(global);
        }
        Some(text)
    })();
    unsafe {
        let _ = CloseClipboard();
    }
    result
}

fn clear_clipboard() -> Result<(), String> {
    open_clipboard()?;
    let result = unsafe { EmptyClipboard() }.map_err(|error| error.to_string());
    unsafe {
        let _ = CloseClipboard();
    }
    result
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
