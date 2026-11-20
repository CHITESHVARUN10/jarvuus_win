use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local};
use jarvis_commands::actions::{MediaAction, PlannedAction, SystemInfoAction, VolumeAction};
use jarvis_commands::command_policy::FileQuery;
use jarvis_commands::execution::{ActionOutcome, FileHit};
use jarvis_commands::local_search::{
    SearchDate, SearchKind, SearchMode, SearchRequest, SelectionOperation,
};
use jarvis_commands::workspace::{WorkspaceApp, WorkspaceOperation};
use windows::core::{w, GUID, HSTRING};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_SCAN_ENTRIES: usize = 20_000;
const MAX_SCAN_DEPTH: usize = 8;
const MAX_RESULTS: usize = 20;
const MAX_CONTENT_FILES: usize = 200;
const MAX_CONTENT_BYTES: u64 = 2 * 1024 * 1024;

pub fn execute(action: &PlannedAction, last_files: &[FileHit]) -> ActionOutcome {
    match action {
        PlannedAction::Workspace { command } => workspace(command),
        PlannedAction::OpenUrlInBrowser { url, browser } => open_in_browser(url, browser),
        PlannedAction::FileQuery { query } => file_query(query),
        PlannedAction::SearchFiles { request } => search_files(request),
        PlannedAction::FileResult { selection } => file_result(selection, last_files),
        PlannedAction::OpenSettings { pane } => open_settings(pane),
        PlannedAction::OpenApp { name } => open_app(name),
        PlannedAction::CloseApp { name } => close_app(name),
        PlannedAction::SystemInfo { info } => system_info(info),
        PlannedAction::OpenUrl { url } => open_url(url),
        PlannedAction::SearchWeb { engine, query } => search_web(engine, query),
        PlannedAction::OpenFolder { path } => open_folder(path),
        PlannedAction::OpenLatestFile { folder } => open_latest_file(folder),
        PlannedAction::MediaControl { action } => media(action),
        PlannedAction::VolumeControl { action } => volume(action),
        PlannedAction::CreateFile { name } => create_file(name),
        PlannedAction::CreateFolder { name } => create_folder(name),
        PlannedAction::AiQuery { .. } => ActionOutcome::error(
            "That command needs a local AI answer. Start Ollama and try again.",
        ),
        PlannedAction::InstallPreview { package, .. } => {
            ActionOutcome::error(format!("Installing '{package}' is disabled in this build."))
        }
        PlannedAction::DisplayControl { action } => ActionOutcome::error(format!(
            "{} isn't available yet. Windows display control arrives in a later phase.",
            action.description()
        )),
    }
}

pub fn open_file(path: &str, reveal: bool) -> Result<(), String> {
    if reveal {
        Command::new("explorer.exe")
            .arg(format!("/select,{path}"))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    } else {
        shell_open(path)
    }
}

fn open_app(name: &str) -> ActionOutcome {
    let launch = match name {
        "Google Chrome" => "chrome",
        "Microsoft Edge" => "msedge",
        "Brave Browser" => "brave",
        "Firefox" => "firefox",
        "Arc" => "Arc",
        "Spotify" => "spotify:",
        "WhatsApp" => "whatsapp:",
        "Visual Studio Code" => {
            return spawn_fixed("cmd", &["/C", "code"], "Opened Visual Studio Code.")
        }
        "Windows Terminal" => "wt",
        "File Explorer" => "explorer",
        "Notepad" => "notepad",
        "Calculator" => "calc",
        "Settings" => "ms-settings:",
        "Paint" => "mspaint",
        "Task Manager" => "taskmgr",
        "Command Prompt" => "cmd",
        "GitHub Desktop" => "GitHubDesktop",
        other => {
            if other.contains(['\\', '/', ':']) || other.contains("..") {
                return ActionOutcome::error("That app name isn't valid.");
            }
            other
        }
    };
    match shell_open(launch) {
        Ok(()) => ActionOutcome::ok(format!("Opened {name}.")),
        Err(error) => ActionOutcome::error(error),
    }
}

fn close_app(name: &str) -> ActionOutcome {
    let image = match name {
        "Google Chrome" => "chrome.exe",
        "Microsoft Edge" => "msedge.exe",
        "Brave Browser" => "brave.exe",
        "Firefox" => "firefox.exe",
        "Arc" => "Arc.exe",
        "Spotify" => "Spotify.exe",
        "WhatsApp" => "WhatsApp.exe",
        "Visual Studio Code" => "Code.exe",
        "Windows Terminal" => "WindowsTerminal.exe",
        "Notepad" => "notepad.exe",
        "Calculator" => "CalculatorApp.exe",
        "Paint" => "mspaint.exe",
        "Task Manager" => "Taskmgr.exe",
        "GitHub Desktop" => "GitHubDesktop.exe",
        "File Explorer" => {
            return ActionOutcome::error("Windows manages File Explorer; I won't close it.")
        }
        "Command Prompt" => return ActionOutcome::error("I won't close a command prompt window."),
        _ => return ActionOutcome::error("I can only close apps I know."),
    };
    match Command::new("taskkill")
        .args(["/IM", image])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
    {
        Ok(status) if status.success() => ActionOutcome::ok(format!("Closed {name}.")),
        Ok(_) => ActionOutcome::error(format!("{name} wasn't running.")),
        Err(error) => ActionOutcome::error(format!("Could not close {name}: {error}")),
    }
}

fn open_url(url: &str) -> ActionOutcome {
    if let Err(error) = validate_url(url) {
        return ActionOutcome::error(error);
    }
    match shell_open(url) {
        Ok(()) => ActionOutcome::ok(format!("Opened {url}.")),
        Err(error) => ActionOutcome::error(error),
    }
}

fn open_in_browser(url: &str, browser: &str) -> ActionOutcome {
    if let Err(error) = validate_url(url) {
        return ActionOutcome::error(error);
    }
    let Some(executable) = browser_executable(browser) else {
        return ActionOutcome::error(format!("I don't know how to open {browser}."));
    };
    match shell_open_with(executable, url) {
        Ok(()) => ActionOutcome::ok(format!("Opened {url} in {browser}.")),
        Err(error) => ActionOutcome::error(error),
    }
}

fn open_settings(pane: &str) -> ActionOutcome {
    let target = match pane.to_ascii_lowercase().as_str() {
        "bluetooth" => "ms-settings:bluetooth",
        "network" => "ms-settings:network",
        "wifi" => "ms-settings:network-wifi",
        "sound" => "ms-settings:sound",
        "display" => "ms-settings:display",
        "keyboard" => "ms-settings:keyboard",
        "mouse" => "ms-settings:mouse",
        "accessibility" => "ms-settings:easeofaccess",
        "privacy" => "ms-settings:privacy",
        "microphone" => "ms-settings:privacy-microphone",
        "camera" => "ms-settings:privacy-webcam",
        "battery" => "ms-settings:batterysaver",
        "notifications" => "ms-settings:notifications",
        "storage" => "ms-settings:storagesense",
        "windows_update" => "ms-settings:windowsupdate",
        "apps" => "ms-settings:appsfeatures",
        "about" => "ms-settings:about",
        "personalization" => "ms-settings:personalization",
        "time" => "ms-settings:dateandtime",
        "language" => "ms-settings:regionlanguage",
        "accounts" => "ms-settings:yourinfo",
        "gaming" => "ms-settings:gaming-gamebar",
        "multitasking" => "ms-settings:multitasking",
        "search" => "ms-settings:search",
        "system" => "ms-settings:system",
        other => {
            return ActionOutcome::error(format!(
                "I don't know the Windows settings page for '{other}'."
            ))
        }
    };
    match shell_open(target) {
        Ok(()) => ActionOutcome::ok(format!("Opened {pane} settings.")),
        Err(error) => ActionOutcome::error(error),
    }
}

fn search_web(engine: &str, query: &str) -> ActionOutcome {
    let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let url = if engine.eq_ignore_ascii_case("youtube") {
        format!("https://www.youtube.com/results?search_query={encoded}")
    } else {
        format!("https://www.google.com/search?q={encoded}")
    };
    match shell_open(&url) {
        Ok(()) => ActionOutcome::ok(format!("Searched {engine} for '{query}'.")),
        Err(error) => ActionOutcome::error(error),
    }
}

fn open_folder(path: &str) -> ActionOutcome {
    let Some(path) = resolve_folder(path) else {
        return ActionOutcome::error(format!("I couldn't find the folder '{path}'."));
    };
    match shell_open(&path.to_string_lossy()) {
        Ok(()) => ActionOutcome::ok(format!("Opened {}.", path.display())),
        Err(error) => ActionOutcome::error(error),
    }
}

fn open_latest_file(folder: &str) -> ActionOutcome {
    let Some(root) = resolve_folder(folder) else {
        return ActionOutcome::error(format!("I couldn't find the folder '{folder}'."));
    };
    let (entries, _) = collect_entries(&root);
    let Some(entry) = entries
        .into_iter()
        .filter(|entry| !entry.is_dir)
        .max_by_key(|entry| entry.modified)
    else {
        return ActionOutcome::error(format!("No files found in {}.", root.display()));
    };
    match shell_open(&entry.path.to_string_lossy()) {
        Ok(()) => ActionOutcome::ok(format!("Opened {}.", entry.path.display())),
        Err(error) => ActionOutcome::error(error),
    }
}

fn create_file(name: &str) -> ActionOutcome {
    let Some(name) = safe_name(name) else {
        return ActionOutcome::error("That file name isn't valid.");
    };
    let Some(base) = documents_dir() else {
        return ActionOutcome::error("I couldn't find your Documents folder.");
    };
    let path = base.join(&name);
    if path.exists() {
        return ActionOutcome::error(format!("{} already exists.", path.display()));
    }
    match fs::write(&path, b"") {
        Ok(()) => ActionOutcome::ok(format!("Created {}.", path.display())),
        Err(error) => ActionOutcome::error(format!("Could not create the file: {error}")),
    }
}

fn create_folder(name: &str) -> ActionOutcome {
    let Some(name) = safe_name(name) else {
        return ActionOutcome::error("That folder name isn't valid.");
    };
    let Some(base) = documents_dir() else {
        return ActionOutcome::error("I couldn't find your Documents folder.");
    };
    let path = base.join(&name);
    if path.exists() {
        return ActionOutcome::error(format!("{} already exists.", path.display()));
    }
    match fs::create_dir(&path) {
        Ok(()) => ActionOutcome::ok(format!("Created {}.", path.display())),
        Err(error) => ActionOutcome::error(format!("Could not create the folder: {error}")),
    }
}

fn media(action: &MediaAction) -> ActionOutcome {
    let (key, label) = match action {
        MediaAction::Play | MediaAction::Pause => (VK_MEDIA_PLAY_PAUSE, "Toggled playback."),
        MediaAction::NextTrack => (VK_MEDIA_NEXT_TRACK, "Skipped to the next track."),
        MediaAction::PreviousTrack => (VK_MEDIA_PREV_TRACK, "Went to the previous track."),
        MediaAction::PlaySong { .. }
        | MediaAction::PlayPlaylist { .. }
        | MediaAction::PlayLikedSongs
        | MediaAction::SetShuffle { .. }
        | MediaAction::PlayLikedWithShuffle { .. }
        | MediaAction::PlayPlaylistWithShuffle { .. } => {
            return ActionOutcome::error(
                "Named songs, playlists and shuffle control aren't available yet. Media transport works.",
            )
        }
    };
    match send_media_key(key) {
        Ok(()) => ActionOutcome::ok(label),
        Err(error) => ActionOutcome::error(error),
    }
}

fn volume(action: &VolumeAction) -> ActionOutcome {
    match apply_volume(action) {
        Ok(message) => ActionOutcome::ok(message),
        Err(error) => ActionOutcome::error(error),
    }
}

fn apply_volume(action: &VolumeAction) -> Result<String, String> {
    let endpoint = endpoint()?;
    let current = unsafe { endpoint.GetMasterVolumeLevelScalar() }.unwrap_or(0.5);
    let current_percent = (current * 100.0).round() as i32;
    match action {
        VolumeAction::SetLevel { level } => {
            let level = (*level).clamp(0, 100);
            unsafe { endpoint.SetMasterVolumeLevelScalar(level as f32 / 100.0, std::ptr::null()) }
                .map_err(|error| error.to_string())?;
            Ok(format!("Volume set to {level} percent."))
        }
        VolumeAction::Increase { by } => {
            let level = (current_percent + by).clamp(0, 100);
            unsafe { endpoint.SetMasterVolumeLevelScalar(level as f32 / 100.0, std::ptr::null()) }
                .map_err(|error| error.to_string())?;
            Ok(format!("Volume increased to {level} percent."))
        }
        VolumeAction::Decrease { by } => {
            let level = (current_percent - by).clamp(0, 100);
            unsafe { endpoint.SetMasterVolumeLevelScalar(level as f32 / 100.0, std::ptr::null()) }
                .map_err(|error| error.to_string())?;
            Ok(format!("Volume decreased to {level} percent."))
        }
        VolumeAction::Mute => {
            unsafe { endpoint.SetMute(true, std::ptr::null()) }
                .map_err(|error| error.to_string())?;
            Ok("Sound muted.".into())
        }
        VolumeAction::Unmute => {
            unsafe { endpoint.SetMute(false, std::ptr::null()) }
                .map_err(|error| error.to_string())?;
            Ok("Sound unmuted.".into())
        }
    }
}

fn system_info(info: &SystemInfoAction) -> ActionOutcome {
    match info {
        SystemInfoAction::CurrentTime => {
            ActionOutcome::ok(Local::now().format("%I:%M %p").to_string())
        }
        SystemInfoAction::CurrentDate => {
            ActionOutcome::ok(Local::now().format("%A, %B %e, %Y").to_string())
        }
        SystemInfoAction::CurrentDay => ActionOutcome::ok(Local::now().format("%A").to_string()),
        SystemInfoAction::CurrentMonth => {
            ActionOutcome::ok(Local::now().format("%B %Y").to_string())
        }
        SystemInfoAction::CurrentYear => ActionOutcome::ok(Local::now().format("%Y").to_string()),
        SystemInfoAction::BatteryStatus => battery_status(),
        SystemInfoAction::WifiStatus => wifi_status(),
        SystemInfoAction::BluetoothDevices => {
            ActionOutcome::error("Bluetooth status isn't available yet.")
        }
        SystemInfoAction::SystemVolume => match endpoint() {
            Ok(endpoint) => {
                let level = unsafe { endpoint.GetMasterVolumeLevelScalar() }.unwrap_or(0.0);
                let muted = unsafe { endpoint.GetMute() }
                    .map(|value| value.as_bool())
                    .unwrap_or(false);
                ActionOutcome::ok(if muted {
                    format!("Volume is {:.0} percent and muted.", level * 100.0)
                } else {
                    format!("Volume is {:.0} percent.", level * 100.0)
                })
            }
            Err(error) => ActionOutcome::error(error),
        },
        SystemInfoAction::DisplayBrightness => {
            ActionOutcome::error("Brightness readback isn't available yet.")
        }
        SystemInfoAction::DisplayContrast => {
            ActionOutcome::error("Contrast readback isn't available yet.")
        }
    }
}

fn file_query(query: &FileQuery) -> ActionOutcome {
    let Some(root) = resolve_folder(&query.folder) else {
        return ActionOutcome::error(format!("I couldn't find the folder '{}'.", query.folder));
    };
    let (entries, _) = collect_entries(&root);
    let mut files: Vec<&Entry> = entries
        .iter()
        .filter(|entry| {
            !entry.is_dir
                && (query.ext.is_empty()
                    || entry
                        .path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case(&query.ext)))
        })
        .collect();
    let folders: Vec<&Entry> = entries.iter().filter(|entry| entry.is_dir).collect();

    match query.op.as_str() {
        "count" => ActionOutcome::ok(format!("{} files in {}.", files.len(), root.display())),
        "countFolders" => {
            ActionOutcome::ok(format!("{} folders in {}.", folders.len(), root.display()))
        }
        "list" => {
            files.sort_by_key(|entry| entry.name.to_lowercase());
            files.truncate(12);
            ActionOutcome::with_files(
                format!(
                    "{} files in {}{}.",
                    files.len(),
                    root.display(),
                    if files.len() == 12 { " (first 12)" } else { "" }
                ),
                files.iter().map(|entry| entry.hit()).collect(),
            )
        }
        "listFolders" => {
            let mut folders = folders;
            folders.sort_by_key(|entry| entry.name.to_lowercase());
            folders.truncate(12);
            ActionOutcome::with_files(
                format!("{} folders in {}.", folders.len(), root.display()),
                folders.iter().map(|entry| entry.hit()).collect(),
            )
        }
        "largest" => {
            files.sort_by_key(|entry| std::cmp::Reverse(entry.size));
            files.truncate(10);
            ActionOutcome::with_files(
                "Largest files:".to_string(),
                files.iter().map(|entry| entry.hit()).collect(),
            )
        }
        "newest" => {
            files.sort_by_key(|entry| std::cmp::Reverse(entry.modified));
            files.truncate(10);
            ActionOutcome::with_files(
                "Newest files:".to_string(),
                files.iter().map(|entry| entry.hit()).collect(),
            )
        }
        "oldest" => {
            files.sort_by_key(|entry| entry.modified);
            files.truncate(10);
            ActionOutcome::with_files(
                "Oldest files:".to_string(),
                files.iter().map(|entry| entry.hit()).collect(),
            )
        }
        "totalSize" => {
            let bytes: u64 = files.iter().map(|entry| entry.size).sum();
            ActionOutcome::ok(format!(
                "{} files use {} in {}.",
                files.len(),
                human_size(bytes),
                root.display()
            ))
        }
        "openNewest" | "openOldest" => {
            if files.is_empty() {
                return ActionOutcome::error(format!("No files found in {}.", root.display()));
            }
            files.sort_by_key(|entry| entry.modified);
            let entry = if query.op == "openNewest" {
                files.last().unwrap()
            } else {
                files.first().unwrap()
            };
            match shell_open(&entry.path.to_string_lossy()) {
                Ok(()) => ActionOutcome::ok(format!("Opened {}.", entry.path.display())),
                Err(error) => ActionOutcome::error(error),
            }
        }
        other => ActionOutcome::error(format!("Unknown file query '{other}'.")),
    }
}

fn search_files(request: &SearchRequest) -> ActionOutcome {
    let mut roots = Vec::new();
    if request.scopes.is_empty() {
        for name in ["downloads", "documents", "desktop"] {
            if let Some(path) = resolve_folder(name) {
                roots.push(path);
            }
        }
    } else {
        for scope in &request.scopes {
            match resolve_folder(scope) {
                Some(path) => roots.push(path),
                None => {
                    return ActionOutcome::error(format!(
                        "I couldn't find the search location '{scope}'."
                    ))
                }
            }
        }
    }
    if roots.is_empty() {
        return ActionOutcome::error("I couldn't find a folder to search.");
    }

    let needle = request.query.to_lowercase();
    let mut results = Vec::new();
    let mut content_checked = 0usize;
    for root in roots {
        let (entries, _) = collect_entries(&root);
        for entry in entries {
            if !kind_match(&request.kind, &entry) {
                continue;
            }
            if entry.is_dir && request.mode == SearchMode::Content {
                continue;
            }
            if !size_match(entry.size, request.minimum_bytes, request.maximum_bytes) {
                continue;
            }
            if !date_match(entry.modified, &request.date) {
                continue;
            }
            if request.mode == SearchMode::Content {
                if content_checked >= MAX_CONTENT_FILES {
                    break;
                }
                content_checked += 1;
                if entry.size > MAX_CONTENT_BYTES || !content_match(&entry.path, &needle) {
                    continue;
                }
            } else if !needle.is_empty() && !entry.name.to_lowercase().contains(&needle) {
                continue;
            }
            results.push(entry);
            if results.len() >= 200 {
                break;
            }
        }
    }
    results.sort_by_key(|entry| std::cmp::Reverse(entry.modified));
    results.truncate(MAX_RESULTS);
    if results.is_empty() {
        return ActionOutcome::ok("No matching files found.".to_string());
    }
    let files: Vec<FileHit> = results.iter().map(Entry::hit).collect();
    ActionOutcome::with_files(format!("Found {} matching file(s).", files.len()), files)
}

fn file_result(
    selection: &jarvis_commands::local_search::FileSelection,
    files: &[FileHit],
) -> ActionOutcome {
    if files.is_empty() {
        return ActionOutcome::error("There are no recent file results to open.");
    }
    let index = if selection.index == 0 {
        0
    } else {
        selection.index as usize - 1
    };
    let Some(file) = files.get(index) else {
        return ActionOutcome::error("That file number is out of range.");
    };
    let reveal = matches!(selection.operation, SelectionOperation::Reveal);
    match open_file(&file.path, reveal) {
        Ok(()) => ActionOutcome::ok(if reveal {
            format!("Revealed {}.", file.name)
        } else {
            format!("Opened {}.", file.name)
        }),
        Err(error) => ActionOutcome::error(error),
    }
}

fn workspace(command: &jarvis_commands::workspace::WorkspaceRequest) -> ActionOutcome {
    let app_name = match command.app {
        WorkspaceApp::Brave => "Brave Browser",
        WorkspaceApp::Chrome => "Google Chrome",
        WorkspaceApp::Vscode => "Visual Studio Code",
        WorkspaceApp::Explorer => "File Explorer",
        WorkspaceApp::Terminal => "Windows Terminal",
    };
    match command.operation {
        WorkspaceOperation::NewWindow
        | WorkspaceOperation::NewTab
        | WorkspaceOperation::NewTerminalTab => open_app(app_name),
        WorkspaceOperation::OpenProject => ActionOutcome::error(
            "Project opening needs a folder path in this build. Try 'open folder <path>'.",
        ),
    }
}

fn battery_status() -> ActionOutcome {
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return ActionOutcome::error("Battery status isn't available.");
    }
    if status.BatteryLifePercent == 255 || status.BatteryFlag == 128 {
        return ActionOutcome::ok("No battery detected; this PC is on AC power.");
    }
    let charging = status.BatteryFlag & 8 != 0 || status.ACLineStatus == 1;
    ActionOutcome::ok(format!(
        "Battery is at {}%{}.",
        status.BatteryLifePercent.min(100),
        if charging { " and charging" } else { "" }
    ))
}

fn wifi_status() -> ActionOutcome {
    let output = Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(output) = output else {
        return ActionOutcome::error("Wi-Fi status isn't available.");
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let state = text
        .lines()
        .find(|line| line.trim_start().starts_with("State"))
        .and_then(|line| line.split(':').nth(1))
        .map(str::trim)
        .unwrap_or("");
    if !state.eq_ignore_ascii_case("connected") {
        return ActionOutcome::ok("Wi-Fi is not connected.");
    }
    let ssid = text
        .lines()
        .find(|line| line.trim_start().starts_with("SSID"))
        .and_then(|line| line.split(':').nth(1))
        .map(str::trim)
        .unwrap_or("an unknown network");
    let signal = text
        .lines()
        .find(|line| line.trim_start().starts_with("Signal"))
        .and_then(|line| line.split(':').nth(1))
        .map(str::trim)
        .unwrap_or("");
    ActionOutcome::ok(format!(
        "Wi-Fi is connected to {ssid}{}.",
        if signal.is_empty() {
            String::new()
        } else {
            format!(" ({signal})")
        }
    ))
}

fn endpoint() -> Result<IAudioEndpointVolume, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let clsid = GUID::from_u128(0xbcde0395_e52f_467c_8e3d_c4579291692e);
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&clsid, None, CLSCTX_ALL) }
        .map_err(|error| format!("Audio endpoint unavailable: {error}"))?;
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
        .map_err(|error| format!("No default playback device: {error}"))?;
    unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }
        .map_err(|error| format!("Could not control volume: {error}"))
}

fn send_media_key(key: VIRTUAL_KEY) -> Result<(), String> {
    let inputs = [media_input(key, false), media_input(key, true)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == 0 {
        Err("Windows blocked the media key.".into())
    } else {
        Ok(())
    }
}

fn media_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
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

fn shell_open(target: &str) -> Result<(), String> {
    let target = HSTRING::from(target);
    let result = unsafe { ShellExecuteW(None, w!("open"), &target, None, None, SW_SHOWNORMAL) };
    if result.0 as isize <= 32 {
        Err("Windows could not open that target.".into())
    } else {
        Ok(())
    }
}

fn shell_open_with(program: &str, parameter: &str) -> Result<(), String> {
    let program = HSTRING::from(program);
    let parameter = HSTRING::from(parameter);
    let result =
        unsafe { ShellExecuteW(None, w!("open"), &program, &parameter, None, SW_SHOWNORMAL) };
    if result.0 as isize <= 32 {
        Err(format!("Windows could not open {parameter}."))
    } else {
        Ok(())
    }
}

fn spawn_fixed(program: &str, args: &[&str], message: &str) -> ActionOutcome {
    match Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        Ok(_) => ActionOutcome::ok(message.to_string()),
        Err(error) => ActionOutcome::error(format!("Could not open that app: {error}")),
    }
}

fn browser_executable(browser: &str) -> Option<&'static str> {
    Some(match browser {
        "Google Chrome" => "chrome",
        "Microsoft Edge" => "msedge",
        "Brave Browser" => "brave",
        "Firefox" => "firefox",
        "Arc" => "Arc",
        _ => return None,
    })
}

fn validate_url(url: &str) -> Result<(), String> {
    let parsed =
        url::Url::parse(url).map_err(|_| "That doesn't look like a web address.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only http and https links are allowed.".into());
    }
    if parsed.host_str().is_none() || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("That web address isn't allowed.".into());
    }
    Ok(())
}

fn resolve_folder(raw: &str) -> Option<PathBuf> {
    let value = raw
        .trim()
        .trim_matches(|character| character == '"' || character == '\'');
    let lower = value.to_ascii_lowercase();
    let known = match lower.as_str() {
        "downloads" | "download" => dirs::download_dir(),
        "documents" | "docs" => dirs::document_dir(),
        "desktop" => dirs::desktop_dir(),
        "pictures" | "photos" | "images" => dirs::picture_dir(),
        "music" => dirs::audio_dir(),
        "videos" | "movies" => dirs::video_dir(),
        "home" | "home folder" | "home directory" => dirs::home_dir(),
        _ => None,
    };
    if let Some(path) = known.filter(|path| path.is_dir()) {
        return Some(path);
    }
    let path = PathBuf::from(value);
    if path.is_dir() && !path_is_protected(&path) {
        return Some(path);
    }
    let home = dirs::home_dir()?;
    let candidate = home.join(value);
    (candidate.is_dir() && !path_is_protected(&candidate)).then_some(candidate)
}

fn path_is_protected(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    jarvis_commands::safety::PROTECTED_PATHS
        .iter()
        .any(|protected| lower.starts_with(protected))
}

fn documents_dir() -> Option<PathBuf> {
    dirs::document_dir()
        .filter(|path| path.is_dir())
        .or_else(dirs::home_dir)
}

fn safe_name(raw: &str) -> Option<String> {
    let name = raw
        .trim()
        .trim_matches(|character| character == '"' || character == '\'')
        .to_string();
    if name.is_empty()
        || name.len() > 120
        || name == "."
        || name == ".."
        || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|'])
    {
        return None;
    }
    Some(name)
}

struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    size: u64,
    modified: SystemTime,
}

impl Entry {
    fn hit(&self) -> FileHit {
        FileHit {
            name: self.name.clone(),
            path: self.path.to_string_lossy().into_owned(),
            size: self.size,
            modified: DateTime::<Local>::from(self.modified)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
        }
    }
}

fn collect_entries(root: &Path) -> (Vec<Entry>, bool) {
    let mut entries = Vec::new();
    let mut capped = false;
    collect_into(root, 0, &mut entries, &mut capped);
    (entries, capped)
}

fn collect_into(directory: &Path, depth: usize, entries: &mut Vec<Entry>, capped: &mut bool) {
    if depth > MAX_SCAN_DEPTH || *capped {
        return;
    }
    let Ok(read) = fs::read_dir(directory) else {
        return;
    };
    for item in read.flatten() {
        if entries.len() >= MAX_SCAN_ENTRIES {
            *capped = true;
            return;
        }
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(file_type) = item.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = item.path();
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let is_dir = file_type.is_dir();
        entries.push(Entry {
            path: path.clone(),
            name,
            is_dir,
            size: if is_dir { 0 } else { metadata.len() },
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
        if is_dir {
            collect_into(&path, depth + 1, entries, capped);
        }
    }
}

fn kind_match(kind: &SearchKind, entry: &Entry) -> bool {
    if matches!(kind, SearchKind::Folder) {
        return entry.is_dir;
    }
    if entry.is_dir {
        return false;
    }
    let extension = entry
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match kind {
        SearchKind::Any => true,
        SearchKind::Pdf => extension == "pdf",
        SearchKind::Presentation => matches!(extension.as_str(), "ppt" | "pptx"),
        SearchKind::Document => matches!(extension.as_str(), "doc" | "docx" | "txt" | "rtf" | "md"),
        SearchKind::Spreadsheet => matches!(extension.as_str(), "xls" | "xlsx" | "csv"),
        SearchKind::Image => matches!(
            extension.as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "heic"
        ),
        SearchKind::Audio => matches!(
            extension.as_str(),
            "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg"
        ),
        SearchKind::Video => matches!(extension.as_str(), "mp4" | "mov" | "mkv" | "avi" | "webm"),
        SearchKind::Mail => matches!(extension.as_str(), "eml" | "msg"),
        SearchKind::Note => matches!(extension.as_str(), "txt" | "md"),
        SearchKind::Folder => entry.is_dir,
    }
}

fn size_match(size: u64, minimum: i64, maximum: i64) -> bool {
    if minimum > 0 && size < minimum as u64 {
        return false;
    }
    if maximum > 0 && size > maximum as u64 {
        return false;
    }
    true
}

fn date_match(modified: SystemTime, date: &SearchDate) -> bool {
    if matches!(date, SearchDate::Any) {
        return true;
    }
    let modified: DateTime<Local> = modified.into();
    let now = Local::now();
    let today = now.date_naive();
    let start_of_week = today - ChronoDuration::days(now.weekday().num_days_from_monday() as i64);
    match date {
        SearchDate::Any => true,
        SearchDate::Today => modified.date_naive() == today,
        SearchDate::Yesterday => modified.date_naive() == today - ChronoDuration::days(1),
        SearchDate::ThisWeek => modified.date_naive() >= start_of_week,
        SearchDate::LastWeek => {
            modified.date_naive() >= start_of_week - ChronoDuration::days(7)
                && modified.date_naive() < start_of_week
        }
        SearchDate::ThisMonth => modified.year() == now.year() && modified.month() == now.month(),
        SearchDate::LastMonth => {
            let (year, month) = if now.month() == 1 {
                (now.year() - 1, 12)
            } else {
                (now.year(), now.month() - 1)
            };
            modified.year() == year && modified.month() == month
        }
        SearchDate::Last7Days => modified >= now - ChronoDuration::days(7),
        SearchDate::Last30Days => modified >= now - ChronoDuration::days(30),
    }
}

fn content_match(path: &Path, needle: &str) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    text.to_lowercase().contains(needle)
}

fn human_size(bytes: u64) -> String {
    const KB: u64 = 1_000;
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} bytes")
    }
}
