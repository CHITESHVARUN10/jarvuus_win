use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    Auto,
    Light,
    #[default]
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::Auto, ThemeChoice::Light, ThemeChoice::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::Auto => "Auto",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }

    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            ThemeChoice::Auto => system_dark,
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Prerelease,
}

impl UpdateChannel {
    pub const ALL: [UpdateChannel; 2] = [UpdateChannel::Stable, UpdateChannel::Prerelease];

    pub fn label(self) -> &'static str {
        match self {
            UpdateChannel::Stable => "Stable",
            UpdateChannel::Prerelease => "Prerelease",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            UpdateChannel::Stable => "Final releases only.",
            UpdateChannel::Prerelease => "Include beta and release-candidate builds.",
        }
    }

    pub fn accepts(self, version: &semver::Version) -> bool {
        match self {
            UpdateChannel::Stable => version.pre.is_empty(),
            UpdateChannel::Prerelease => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub onboarding_version: u32,
    pub setup_completed: bool,
    pub theme: ThemeChoice,
    pub accent: u8,
    pub dictation_hotkey: String,
    pub command_hotkey: String,
    pub launch_at_login: bool,
    pub close_to_tray: bool,
    pub microphone: Option<String>,
    pub update_channel: UpdateChannel,
    pub check_for_updates: bool,
    pub auto_download_updates: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            onboarding_version: 1,
            setup_completed: false,
            theme: ThemeChoice::Dark,
            accent: 0,
            dictation_hotkey: "Ctrl+Shift+D".into(),
            command_hotkey: "Ctrl+Shift+A".into(),
            launch_at_login: false,
            close_to_tray: true,
            microphone: None,
            update_channel: UpdateChannel::Stable,
            check_for_updates: true,
            auto_download_updates: false,
        }
    }
}

impl AppSettings {
    pub fn load(path: &Path) -> Self {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| Self::default()),
            Err(_) => Self::default(),
        }
    }

    pub fn save_atomic(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_fit_a_fresh_install() {
        let settings = AppSettings::default();
        assert!(!settings.setup_completed);
        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.accent, 0);
        assert_eq!(settings.dictation_hotkey, "Ctrl+Shift+D");
        assert_eq!(settings.command_hotkey, "Ctrl+Shift+A");
        assert!(settings.close_to_tray);
        assert!(!settings.launch_at_login);
        assert!(settings.check_for_updates);
        assert!(!settings.auto_download_updates);
        assert_eq!(settings.update_channel, UpdateChannel::Stable);
    }

    #[test]
    fn update_channels_filter_prereleases() {
        let stable = semver::Version::parse("0.2.0").unwrap();
        let beta = semver::Version::parse("0.2.0-beta.1").unwrap();
        assert!(UpdateChannel::Stable.accepts(&stable));
        assert!(!UpdateChannel::Stable.accepts(&beta));
        assert!(UpdateChannel::Prerelease.accepts(&stable));
        assert!(UpdateChannel::Prerelease.accepts(&beta));
    }

    #[test]
    fn round_trips_and_tolerates_missing_or_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        assert_eq!(AppSettings::load(&path), AppSettings::default());

        let settings = AppSettings {
            setup_completed: true,
            theme: ThemeChoice::Light,
            accent: 3,
            ..AppSettings::default()
        };
        settings.save_atomic(&path).unwrap();

        assert_eq!(AppSettings::load(&path), settings);

        fs::write(&path, b"{not json").unwrap();
        assert_eq!(AppSettings::load(&path), AppSettings::default());
    }

    #[test]
    fn partial_files_keep_their_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, br#"{"setup_completed":true}"#).unwrap();

        let settings = AppSettings::load(&path);
        assert!(settings.setup_completed);
        assert_eq!(settings.theme, ThemeChoice::Dark);
    }
}
