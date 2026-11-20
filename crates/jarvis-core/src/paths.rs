use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const DATA_DIR_ENV: &str = "JARVIS_DATA_DIR";
pub const STT_MODEL_DIR_ENV: &str = "JARVIS_STT_MODEL_DIR";
pub const INTENT_MODEL_DIR_ENV: &str = "JARVIS_INTENT_MODEL_DIR";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub models_dir: PathBuf,
    pub intent_dir: PathBuf,
    pub intent_override: Option<PathBuf>,
    pub whisper_model_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub settings_file: PathBuf,
    pub routines_file: PathBuf,
    pub insights_file: PathBuf,
    pub log_file: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Self {
        Self::resolve_with(|key| env::var_os(key))
    }

    pub fn resolve_with(get: impl Fn(&str) -> Option<OsString>) -> Self {
        let data_dir = get(DATA_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(default_data_dir);
        let models_dir = data_dir.join("models");
        let whisper_model_dir = get(STT_MODEL_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| models_dir.clone());
        let intent_override = get(INTENT_MODEL_DIR_ENV).map(PathBuf::from);
        let logs_dir = data_dir.join("logs");

        Self {
            settings_file: data_dir.join("settings.json"),
            routines_file: data_dir.join("routines.json"),
            insights_file: data_dir.join("insights.json"),
            log_file: logs_dir.join("commands.log"),
            intent_dir: models_dir.join("intent"),
            models_dir,
            whisper_model_dir,
            logs_dir,
            intent_override,
            data_dir,
        }
    }

    pub fn with_data_dir(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir: PathBuf = data_dir.into();
        Self::resolve_with(move |key| match key {
            DATA_DIR_ENV => Some(data_dir.clone().into_os_string()),
            _ => None,
        })
    }

    pub fn ensure_dirs(&self) -> io::Result<()> {
        for dir in [
            &self.data_dir,
            &self.models_dir,
            &self.logs_dir,
            &self.whisper_model_dir,
        ] {
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn relative_to_data(&self, path: &Path) -> String {
        path.strip_prefix(&self.data_dir)
            .unwrap_or(path)
            .display()
            .to_string()
    }
}

#[cfg(windows)]
fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Jarvis")
}

#[cfg(not(windows))]
fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("JarvisWindows")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_override_moves_all_derived_paths() {
        let paths = AppPaths::resolve_with(|key| match key {
            DATA_DIR_ENV => Some(OsString::from("/tmp/jarvis-test")),
            STT_MODEL_DIR_ENV => Some(OsString::from("/tmp/models")),
            INTENT_MODEL_DIR_ENV => Some(OsString::from("/tmp/intent")),
            _ => None,
        });

        assert_eq!(paths.data_dir, PathBuf::from("/tmp/jarvis-test"));
        assert_eq!(paths.models_dir, PathBuf::from("/tmp/jarvis-test/models"));
        assert_eq!(paths.whisper_model_dir, PathBuf::from("/tmp/models"));
        assert_eq!(
            paths.intent_dir,
            PathBuf::from("/tmp/jarvis-test/models/intent")
        );
        assert_eq!(paths.intent_override, Some(PathBuf::from("/tmp/intent")));
        assert_eq!(
            paths.settings_file,
            PathBuf::from("/tmp/jarvis-test/settings.json")
        );
        assert_eq!(
            paths.routines_file,
            PathBuf::from("/tmp/jarvis-test/routines.json")
        );
        assert_eq!(
            paths.insights_file,
            PathBuf::from("/tmp/jarvis-test/insights.json")
        );
        assert_eq!(
            paths.log_file,
            PathBuf::from("/tmp/jarvis-test/logs/commands.log")
        );
    }

    #[test]
    fn data_dir_defaults_under_the_platform_data_dir() {
        let paths = AppPaths::resolve_with(|_| None);
        assert!(paths.data_dir.is_absolute() || paths.data_dir == Path::new("."));
        assert!(paths.data_dir.ends_with(if cfg!(windows) {
            "Jarvis"
        } else {
            "JarvisWindows"
        }));
    }
}
