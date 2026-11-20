use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::SystemTime;

use jarvis_core::logging;
use jarvis_core::models;
use jarvis_core::paths::AppPaths;
use jarvis_core::settings::{AppSettings, ThemeChoice};
use jarvis_core::setup::{SetupAction, SetupItem, SetupKind, SetupReport, SetupStatus};
use jarvis_models::manager::{ModelKind, ModelManager, ModelPhase, ModelUpdate};
use jarvis_platform::{HotkeyEvent, HotkeyRegistration, InsertionOutcome};
use jarvis_speech::{DictationController, DictationState, SpeechEvent};
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

#[derive(Clone, Debug)]
pub struct ModelsUi {
    pub speech: ModelUpdate,
    pub intent: ModelUpdate,
}

impl ModelsUi {
    pub fn get(&self, kind: ModelKind) -> &ModelUpdate {
        match kind {
            ModelKind::Speech => &self.speech,
            ModelKind::Intent => &self.intent,
        }
    }

    pub fn apply(&mut self, update: ModelUpdate) {
        match update.kind {
            ModelKind::Speech => self.speech = update,
            ModelKind::Intent => self.intent = update,
        }
    }

    pub fn has_active_job(&self, kind: ModelKind) -> bool {
        let update = self.get(kind);
        update.phase.is_busy()
            || matches!(
                update.phase,
                ModelPhase::Ready | ModelPhase::Error | ModelPhase::Cancelled
            )
    }
}

#[derive(Clone, Debug)]
pub struct SpeechUi {
    pub session: u64,
    pub state: DictationState,
    pub detail: String,
    pub level: f32,
    pub partial: String,
    pub transcript: Option<String>,
    pub backend: Option<String>,
    pub model_loaded: bool,
    pub model_detail: String,
    pub device: Option<String>,
    pub hotkey_error: Option<String>,
    pub insertion: Option<String>,
}

impl SpeechUi {
    pub fn new() -> Self {
        Self {
            session: 0,
            state: DictationState::Idle,
            detail: "Press Ctrl + Shift + D or Start to dictate.".into(),
            level: 0.0,
            partial: String::new(),
            transcript: None,
            backend: None,
            model_loaded: false,
            model_detail: "The speech model loads on first use.".into(),
            device: None,
            hotkey_error: None,
            insertion: None,
        }
    }

    pub fn apply(&mut self, event: SpeechEvent) -> bool {
        match event {
            SpeechEvent::State {
                session,
                state,
                detail,
            } => {
                self.session = session;
                self.state = state;
                self.detail = detail;
                if state != DictationState::Listening {
                    self.level = 0.0;
                }
                if state == DictationState::Listening {
                    self.partial.clear();
                    self.transcript = None;
                    self.insertion = None;
                }
                if matches!(state, DictationState::Processing | DictationState::Idle) {
                    self.partial.clear();
                }
                true
            }
            SpeechEvent::Device { session, name } if session == self.session => {
                self.device = Some(name);
                true
            }
            SpeechEvent::Level { session, level } if session == self.session => {
                self.level = level.clamp(0.0, 1.0);
                true
            }
            SpeechEvent::Partial { session, text } if session == self.session => {
                self.partial = text;
                true
            }
            SpeechEvent::Final {
                session,
                text,
                backend,
            } if session == self.session => {
                self.state = DictationState::Ready;
                self.detail = "Transcript ready.".into();
                self.transcript = Some(text);
                self.partial.clear();
                self.level = 0.0;
                self.backend = Some(backend);
                self.insertion = None;
                true
            }
            SpeechEvent::Model {
                session,
                loaded,
                backend,
                detail,
            } if session == 0 || session == self.session => {
                self.model_loaded = loaded;
                self.model_detail = detail;
                if let Some(backend) = backend {
                    self.backend = Some(backend);
                } else if !loaded {
                    self.backend = None;
                }
                true
            }
            SpeechEvent::AudioLimit { session } if session == self.session => {
                self.detail = "Maximum dictation length reached. Stopping.".into();
                true
            }
            SpeechEvent::Error { session, message } if session == 0 || session == self.session => {
                self.state = DictationState::Error;
                self.detail = message;
                self.partial.clear();
                self.level = 0.0;
                self.insertion = None;
                true
            }
            _ => false,
        }
    }
}

impl Default for SpeechUi {
    fn default() -> Self {
        Self::new()
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
    pub models: ModelsUi,
    pub model_manager: ModelManager,
    pub speech: SpeechUi,
    pub dictation: DictationController,
    pub hotkey: Option<HotkeyRegistration>,
    rx: Option<Receiver<SetupReport>>,
    model_rx: Receiver<ModelUpdate>,
    speech_rx: Receiver<SpeechEvent>,
    insertion_tx: mpsc::Sender<Result<InsertionOutcome, String>>,
    insertion_rx: Receiver<Result<InsertionOutcome, String>>,
    hotkey_tx: mpsc::Sender<HotkeyEvent>,
    hotkey_rx: Receiver<HotkeyEvent>,
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

        let models = ModelsUi {
            speech: ModelUpdate::initial(&paths, ModelKind::Speech),
            intent: ModelUpdate::initial(&paths, ModelKind::Intent),
        };
        let (model_sender, model_rx) = mpsc::channel();
        let model_manager = ModelManager::new(paths.clone(), model_sender);
        let speech = SpeechUi::new();
        let (speech_sender, speech_rx) = mpsc::channel();
        let dictation = DictationController::new(paths.clone(), speech_sender);
        let (insertion_tx, insertion_rx) = mpsc::channel();
        let (hotkey_tx, hotkey_rx) = mpsc::channel();

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
            models,
            model_manager,
            speech,
            dictation,
            hotkey: None,
            rx: None,
            model_rx,
            speech_rx,
            insertion_tx,
            insertion_rx,
            hotkey_tx,
            hotkey_rx,
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
        let models = ModelsUi {
            speech: ModelUpdate::initial(&paths, ModelKind::Speech),
            intent: ModelUpdate::initial(&paths, ModelKind::Intent),
        };
        let (model_sender, model_rx) = mpsc::channel();
        let model_manager = ModelManager::new(paths.clone(), model_sender);
        let speech = SpeechUi::new();
        let (speech_sender, speech_rx) = mpsc::channel();
        let dictation = DictationController::new(paths.clone(), speech_sender);
        let (insertion_tx, insertion_rx) = mpsc::channel();
        let (hotkey_tx, hotkey_rx) = mpsc::channel();
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
            models,
            model_manager,
            speech,
            dictation,
            hotkey: None,
            rx: None,
            model_rx,
            speech_rx,
            insertion_tx,
            insertion_rx,
            hotkey_tx,
            hotkey_rx,
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

        let active: Vec<ModelUpdate> = [self.models.speech.clone(), self.models.intent.clone()]
            .into_iter()
            .filter(|update| self.models.has_active_job(update.kind))
            .collect();
        for update in active {
            self.apply_model_update(update);
        }
    }

    pub fn install_model(&mut self, kind: ModelKind) {
        self.model_manager.install(kind);
    }

    pub fn load_model(&mut self, kind: ModelKind) {
        self.model_manager.load(kind);
    }

    pub fn cancel_model(&mut self, kind: ModelKind) {
        self.model_manager.cancel(kind);
    }

    pub fn apply_model_update(&mut self, update: ModelUpdate) {
        let kind = match update.kind {
            ModelKind::Speech => SetupKind::SpeechModel,
            ModelKind::Intent => SetupKind::IntentModel,
        };
        let status = match update.phase {
            ModelPhase::Missing => SetupStatus::Missing,
            ModelPhase::Installed | ModelPhase::Ready => SetupStatus::Ready,
            ModelPhase::Downloading | ModelPhase::Verifying | ModelPhase::Loading => {
                SetupStatus::Checking
            }
            ModelPhase::Error => SetupStatus::Incomplete,
            ModelPhase::Cancelled => SetupStatus::Warning,
        };
        self.setup.update_item(kind, status, update.detail.clone());
        self.models.apply(update);
    }

    pub fn start_dictation(&mut self) {
        if let Err(error) = self.dictation.start() {
            self.speech.state = DictationState::Error;
            self.speech.detail = error;
            self.speech.partial.clear();
            self.speech.insertion = None;
        }
    }

    pub fn stop_dictation(&mut self) {
        self.dictation.stop();
    }

    pub fn cancel_dictation(&mut self) {
        self.dictation.cancel();
    }

    pub fn toggle_dictation(&mut self) {
        match self.speech.state {
            DictationState::Listening => self.stop_dictation(),
            DictationState::Processing => self.cancel_dictation(),
            _ => self.start_dictation(),
        }
    }

    pub fn register_hotkey(&mut self) {
        let waker: Waker = self
            .waker
            .clone()
            .unwrap_or_else(|| std::sync::Arc::new(|| {}));
        match jarvis_platform::start_dictation_hotkey(self.hotkey_tx.clone(), waker) {
            Ok(handle) => {
                self.hotkey = Some(handle);
                self.speech.hotkey_error = None;
            }
            Err(error) => self.speech.hotkey_error = Some(error),
        }
    }

    pub fn copy_transcript(&mut self) {
        let Some(text) = self.speech.transcript.clone() else {
            return;
        };
        match jarvis_platform::copy_text(&text) {
            Ok(()) => {
                self.speech.insertion = Some("Transcript copied to the clipboard.".into());
                self.notice = Some(Notice::info("Transcript copied to the clipboard."));
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!(
                    "Could not copy the transcript: {error}"
                )));
            }
        }
    }

    pub fn apply_speech_event(&mut self, event: SpeechEvent) {
        let audio_limit = matches!(event, SpeechEvent::AudioLimit { .. });
        let error = matches!(event, SpeechEvent::Error { .. });
        let final_text = match &event {
            SpeechEvent::Final { text, .. } => Some(text.clone()),
            _ => None,
        };
        if !self.speech.apply(event) {
            return;
        }
        if let Some(text) = final_text {
            self.start_insertion(text);
        }
        if audio_limit {
            self.stop_dictation();
        }
        if error {
            self.dictation.abort();
        }
    }

    fn start_insertion(&mut self, text: String) {
        self.speech.insertion = Some("Pasting at the cursor...".into());
        let sender = self.insertion_tx.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let result = jarvis_platform::insert_text(&text);
            let _ = sender.send(result);
            if let Some(waker) = waker {
                waker();
            }
        });
    }

    fn apply_insertion_result(&mut self, result: Result<InsertionOutcome, String>) {
        match result {
            Ok(InsertionOutcome::Pasted) => {
                self.speech.insertion = Some("Pasted at the cursor.".into());
            }
            Ok(InsertionOutcome::ClipboardOnly) => {
                self.speech.insertion =
                    Some("Copied to the clipboard. Press Ctrl+V in the target window.".into());
                self.notice = Some(Notice::info(
                    "The active window blocks automated input. The transcript is on the clipboard.",
                ));
            }
            Err(error) => {
                self.speech.insertion = Some(format!(
                    "Automatic paste unavailable: {error}. Use Copy transcript."
                ));
                self.notice = Some(Notice::error(format!(
                    "Automatic paste unavailable: {error}"
                )));
            }
        }
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
    state.model_manager.set_notifier(waker.clone());
    state.dictation.set_notifier(waker.clone());
    if state.setup.checked_at.is_some() {
        return;
    }
    if state.hotkey.is_none() && state.speech.hotkey_error.is_none() {
        state.register_hotkey();
    }
    state.start_setup_probe(waker);
}

pub fn poll(state: &mut Jarvis, _clock: f64, _tx: &mut Tx) -> bool {
    let mut changed = false;
    while let Ok(update) = state.model_rx.try_recv() {
        state.apply_model_update(update);
        changed = true;
    }
    while let Ok(event) = state.speech_rx.try_recv() {
        state.apply_speech_event(event);
        changed = true;
    }
    while let Ok(event) = state.hotkey_rx.try_recv() {
        if event == HotkeyEvent::Pressed {
            state.toggle_dictation();
            changed = true;
        }
    }
    while let Ok(result) = state.insertion_rx.try_recv() {
        state.apply_insertion_result(result);
        changed = true;
    }

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
        Some(Err(TryRecvError::Empty)) | None => changed,
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

    #[test]
    fn speech_events_drive_the_voice_state() {
        let mut ui = SpeechUi::new();
        assert!(ui.apply(SpeechEvent::State {
            session: 1,
            state: DictationState::Listening,
            detail: "Listening".into(),
        }));
        assert_eq!(ui.state, DictationState::Listening);

        assert!(ui.apply(SpeechEvent::Level {
            session: 1,
            level: 0.55
        }));
        assert_eq!(ui.level, 0.55);
        assert!(!ui.apply(SpeechEvent::Level {
            session: 2,
            level: 1.0
        }));

        assert!(ui.apply(SpeechEvent::Partial {
            session: 1,
            text: "hello".into()
        }));
        assert_eq!(ui.partial, "hello");

        assert!(ui.apply(SpeechEvent::Final {
            session: 1,
            text: "hello world".into(),
            backend: "cpu".into()
        }));
        assert_eq!(ui.state, DictationState::Ready);
        assert_eq!(ui.transcript.as_deref(), Some("hello world"));
        assert_eq!(ui.backend.as_deref(), Some("cpu"));
    }

    #[test]
    fn model_updates_change_the_setup_rows_and_essentials() {
        let mut state = Jarvis::for_test();
        state.apply_report(Jarvis::demo_report(false));
        assert_eq!(
            state.setup.item(SetupKind::SpeechModel).unwrap().status,
            SetupStatus::Missing
        );

        state.apply_model_update(ModelUpdate::new(
            ModelKind::Speech,
            ModelPhase::Downloading,
            "50%",
        ));
        assert_eq!(
            state.setup.item(SetupKind::SpeechModel).unwrap().status,
            SetupStatus::Checking
        );

        state.apply_model_update(ModelUpdate::new(
            ModelKind::Speech,
            ModelPhase::Ready,
            "loaded",
        ));
        assert_eq!(
            state.setup.item(SetupKind::SpeechModel).unwrap().status,
            SetupStatus::Ready
        );
        assert_eq!(state.setup.essentials_ready, 2);
    }
}
