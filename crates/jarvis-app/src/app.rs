use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::SystemTime;

use jarvis_core::logging;
use jarvis_core::models;
use jarvis_core::paths::AppPaths;
use jarvis_core::settings::{AppSettings, ThemeChoice};
use jarvis_core::setup::{SetupAction, SetupItem, SetupKind, SetupReport, SetupStatus};
use rune::{Tx, Waker};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Assistant,
    Voice,
    Routines,
    Insights,
    Appearance,
    Settings,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Assistant,
        Section::Voice,
        Section::Routines,
        Section::Insights,
        Section::Appearance,
        Section::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Assistant => "Assistant",
            Section::Voice => "Voice",
            Section::Routines => "Routines",
            Section::Insights => "Insights",
            Section::Appearance => "Appearance",
            Section::Settings => "Settings",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Section::Assistant => "assistant",
            Section::Voice => "voice",
            Section::Routines => "routines",
            Section::Insights => "insights",
            Section::Appearance => "appearance",
            Section::Settings => "settings",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    Setup,
    General,
    About,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 3] = [
        SettingsPage::Setup,
        SettingsPage::General,
        SettingsPage::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsPage::Setup => "Setup",
            SettingsPage::General => "General",
            SettingsPage::About => "About",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            SettingsPage::Setup => "setup",
            SettingsPage::General => "general",
            SettingsPage::About => "about",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeTone {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub message: String,
    pub tone: NoticeTone,
}

impl Notice {
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            tone: NoticeTone::Info,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            tone: NoticeTone::Error,
        }
    }
}

pub struct Jarvis {
    pub paths: AppPaths,
    pub settings: AppSettings,
    pub section: Section,
    pub settings_page: SettingsPage,
    pub setup: SetupReport,
    pub setup_running: bool,
    pub setup_error: Option<String>,
    pub notice: Option<Notice>,
    pub version: &'static str,
    rx: Option<Receiver<SetupReport>>,
    waker: Option<Waker>,
}

impl Jarvis {
    pub fn new() -> Self {
        let paths = AppPaths::resolve();
        let settings = AppSettings::load(&paths.settings_file);
        let mut notice = None;
        if let Err(error) = paths.ensure_dirs() {
            notice = Some(Notice::error(format!(
                "Could not create the data folder: {error}"
            )));
        }
        logging::info(
            &paths.log_file,
            &format!(
                "Jarvis {} starting · data {}",
                env!("CARGO_PKG_VERSION"),
                paths.data_dir.display()
            ),
        );

        let (section, settings_page) = if settings.setup_completed {
            (Section::Assistant, SettingsPage::General)
        } else {
            (Section::Settings, SettingsPage::Setup)
        };

        Self {
            paths,
            settings,
            section,
            settings_page,
            setup: SetupReport::checking(),
            setup_running: false,
            setup_error: None,
            notice,
            version: env!("CARGO_PKG_VERSION"),
            rx: None,
            waker: None,
        }
    }

    pub fn for_test() -> Self {
        let suffix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let data_dir = std::env::temp_dir().join(format!("jarvis-win-test-{suffix}"));
        let mut state = Self::with_state(AppPaths::with_data_dir(data_dir), AppSettings::default());
        state.setup = Self::demo_report(false);
        state
    }

    pub fn with_state(paths: AppPaths, settings: AppSettings) -> Self {
        let completed = settings.setup_completed;
        Self {
            paths,
            settings,
            section: if completed {
                Section::Assistant
            } else {
                Section::Settings
            },
            settings_page: if completed {
                SettingsPage::General
            } else {
                SettingsPage::Setup
            },
            setup: SetupReport::checking(),
            setup_running: false,
            setup_error: None,
            notice: None,
            version: env!("CARGO_PKG_VERSION"),
            rx: None,
            waker: None,
        }
    }

    pub fn dark(&self, system_dark: bool) -> bool {
        self.settings.theme.is_dark(system_dark)
    }

    pub fn navigate(&mut self, section: Section) {
        self.section = section;
        self.notice = None;
    }

    pub fn open_settings(&mut self, page: SettingsPage) {
        self.section = Section::Settings;
        self.settings_page = page;
        self.notice = None;
    }

    pub fn set_theme(&mut self, theme: ThemeChoice) {
        self.settings.theme = theme;
        self.save_settings();
    }

    pub fn set_accent(&mut self, accent: u8) {
        self.settings.accent = accent.min(5);
        self.save_settings();
    }

    pub fn refresh_setup(&mut self) {
        let waker: Waker = self
            .waker
            .clone()
            .unwrap_or_else(|| std::sync::Arc::new(|| {}));
        self.start_setup_probe(waker);
    }

    pub fn start_setup_probe(&mut self, waker: Waker) {
        if self.setup_running {
            return;
        }
        let paths = self.paths.clone();
        let log_file = self.paths.log_file.clone();
        let (sender, receiver) = mpsc::channel();
        self.rx = Some(receiver);
        self.setup_running = true;
        self.setup_error = None;
        self.setup = SetupReport::checking();

        std::thread::spawn(move || {
            let report = SetupReport::run(&paths, &jarvis_platform::Probe);
            logging::info(
                &log_file,
                &format!(
                    "setup probe complete · {}/{} essentials ready",
                    report.essentials_ready, report.essentials_total
                ),
            );
            let _ = sender.send(report);
            waker();
        });
    }

    pub fn apply_report(&mut self, report: SetupReport) {
        self.setup_running = false;
        self.setup = report;
        self.rx = None;
    }

    pub fn finish_setup(&mut self) {
        self.settings.setup_completed = true;
        self.settings_page = SettingsPage::General;
        self.section = Section::Assistant;
        self.save_settings();
        logging::info(
            &self.paths.log_file,
            &format!(
                "setup marked complete · {}/{} essentials ready",
                self.setup.essentials_ready, self.setup.essentials_total
            ),
        );
    }

    pub fn run_action(&mut self, action: SetupAction) {
        let result = match action {
            SetupAction::OpenMicrophoneSettings => jarvis_platform::open_microphone_settings(),
            SetupAction::OpenOllamaWebsite => {
                jarvis_platform::open_url(jarvis_core::ollama::OLLAMA_DOWNLOAD_URL)
            }
            SetupAction::OpenModelFolder => jarvis_platform::open_path(&self.paths.models_dir),
            SetupAction::None => Ok(()),
        };
        if let Err(error) = result {
            logging::warn(
                &self.paths.log_file,
                &format!("open action failed: {error}"),
            );
            self.notice = Some(Notice::error(error));
        } else {
            self.notice = None;
        }
    }

    pub fn open_data_folder(&mut self) {
        if let Err(error) = jarvis_platform::open_path(&self.paths.data_dir) {
            self.notice = Some(Notice::error(error));
        }
    }

    pub fn open_intent_model_page(&mut self) {
        if let Err(error) = jarvis_platform::open_url(models::INTENT_MODEL_URL) {
            self.notice = Some(Notice::error(error));
        }
    }

    pub fn open_ollama_page(&mut self) {
        if let Err(error) = jarvis_platform::open_url(jarvis_core::ollama::OLLAMA_DOWNLOAD_URL) {
            self.notice = Some(Notice::error(error));
        }
    }

    pub fn dismiss_notice(&mut self) {
        self.notice = None;
    }

    fn save_settings(&mut self) {
        if let Err(error) = self.settings.save_atomic(&self.paths.settings_file) {
            logging::error(
                &self.paths.log_file,
                &format!("settings save failed: {error}"),
            );
            self.notice = Some(Notice::error(format!("Could not save settings: {error}")));
        }
    }

    pub fn demo_report(all_ready: bool) -> SetupReport {
        let ready = |kind, detail: &str| SetupItem {
            kind,
            detail: detail.into(),
            status: SetupStatus::Ready,
            action: SetupAction::None,
        };
        let missing = |kind, detail: &str, action| SetupItem {
            kind,
            detail: detail.into(),
            status: SetupStatus::Missing,
            action,
        };

        let items = vec![
            ready(
                SetupKind::Microphone,
                "Windows privacy settings allow desktop apps to use the microphone.",
            ),
            if all_ready {
                ready(
                    SetupKind::SpeechModel,
                    "ggml-large-v3-turbo.bin · 1.5 GB installed",
                )
            } else {
                missing(
                    SetupKind::SpeechModel,
                    "Not installed. Jarvis downloads and verifies the 1.6 GB model.",
                    SetupAction::None,
                )
            },
            if all_ready {
                ready(SetupKind::IntentModel, "Managed release v1.0.2 installed")
            } else {
                missing(
                    SetupKind::IntentModel,
                    "Not installed. Jarvis downloads and verifies the ONNX release.",
                    SetupAction::None,
                )
            },
            ready(
                SetupKind::Gpu,
                "NVIDIA GeForce RTX 4060 (8 GB) · Vulkan for speech, DirectML for intent, CPU fallback",
            ),
            if all_ready {
                ready(
                    SetupKind::Ollama,
                    "Running · 1 model · optional local answers",
                )
            } else {
                missing(
                    SetupKind::Ollama,
                    "Not running · optional local LLM answers",
                    SetupAction::OpenOllamaWebsite,
                )
            },
        ];
        SetupReport::new(items)
    }
}

impl Default for Jarvis {
    fn default() -> Self {
        Self::new()
    }
}

pub fn setup_hook(state: &mut Jarvis, waker: Waker) {
    state.waker = Some(waker.clone());
    if state.setup.checked_at.is_some() {
        return;
    }
    state.start_setup_probe(waker);
}

pub fn poll(state: &mut Jarvis, _clock: f64, _tx: &mut Tx) -> bool {
    let result = state.rx.as_ref().map(Receiver::try_recv);
    match result {
        Some(Ok(report)) => {
            state.apply_report(report);
            true
        }
        Some(Err(TryRecvError::Disconnected)) => {
            state.rx = None;
            state.setup_running = false;
            state.setup_error = Some("Setup checks stopped unexpectedly. Try again.".into());
            true
        }
        Some(Err(TryRecvError::Empty)) | None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_starts_on_setup() {
        let state = Jarvis::with_state(
            AppPaths::with_data_dir(tempfile::tempdir().unwrap().path()),
            AppSettings::default(),
        );
        assert_eq!(state.section, Section::Settings);
        assert_eq!(state.settings_page, SettingsPage::Setup);
        assert!(!state.settings.setup_completed);
    }

    #[test]
    fn completed_setups_start_on_the_assistant() {
        let settings = AppSettings {
            setup_completed: true,
            ..AppSettings::default()
        };
        let state = Jarvis::with_state(
            AppPaths::with_data_dir(tempfile::tempdir().unwrap().path()),
            settings,
        );
        assert_eq!(state.section, Section::Assistant);
        assert_eq!(state.settings_page, SettingsPage::General);
    }

    #[test]
    fn finishing_setup_persists_the_flag() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(dir.path());
        let mut state = Jarvis::with_state(paths.clone(), AppSettings::default());
        state.apply_report(Jarvis::demo_report(true));

        state.finish_setup();

        assert_eq!(state.section, Section::Assistant);
        assert!(state.settings.setup_completed);
        assert!(AppSettings::load(&paths.settings_file).setup_completed);
    }

    #[test]
    fn demo_reports_cover_both_sides_of_setup() {
        let partial = Jarvis::demo_report(false);
        let ready = Jarvis::demo_report(true);
        assert_eq!(partial.essentials_ready, 1);
        assert!(ready.essentials_complete());
        assert_eq!(ready.essentials_total, 3);
    }
}
