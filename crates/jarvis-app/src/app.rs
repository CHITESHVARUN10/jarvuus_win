use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::OnceLock;
use std::time::SystemTime;

use jarvis_core::hotkey::{Hotkey, HotkeyKey};
use jarvis_core::insights::Insights;
use jarvis_core::logging;
use jarvis_core::models;
use jarvis_core::paths::AppPaths;
use jarvis_core::routines::RoutineStore;
use jarvis_core::settings::{AppSettings, ThemeChoice};
use jarvis_core::setup::{SetupAction, SetupItem, SetupKind, SetupReport, SetupStatus};
use jarvis_core::update;
use jarvis_models::manager::{ModelKind, ModelManager, ModelPhase, ModelUpdate};
use jarvis_platform::{HotkeyEvent, HotkeyRegistration, InsertionOutcome};
use jarvis_speech::{DictationController, DictationState, SpeechEvent};
use rune::{AppHandle, Key, KeyEvent, TextEdit, Tx, Waker, WindowHandle};

static WINDOW_HANDLE: OnceLock<WindowHandle> = OnceLock::new();
static APP_HANDLE: OnceLock<AppHandle<Jarvis>> = OnceLock::new();

pub fn remember_window(handle: WindowHandle) {
    let _ = WINDOW_HANDLE.set(handle);
    if let Some(app) = APP_HANDLE.get() {
        app.wake();
    }
}

pub fn remember_app_handle(handle: AppHandle<Jarvis>) {
    let _ = APP_HANDLE.set(handle);
}

pub fn on_close(state: &mut Jarvis, tx: &mut Tx) {
    if !state.settings.close_to_tray || state.tray.is_none() {
        tx.quit();
    }
}

pub fn on_key(state: &mut Jarvis, event: &KeyEvent, _tx: &mut Tx) -> bool {
    if state.hotkey_target.is_none() {
        return false;
    }
    if matches!(event.key, Key::Escape) {
        state.cancel_hotkey_capture();
        return true;
    }
    match hotkey_from_event(event) {
        Some(hotkey) => state.apply_hotkey(hotkey),
        None => {
            state.hotkey_message =
                Some("Use Ctrl, Shift, Alt or Win plus a letter, digit, F-key or Space.".into());
        }
    }
    true
}

fn hotkey_from_event(event: &KeyEvent) -> Option<Hotkey> {
    let ctrl = event.mods.ctrl || event.mods.cmd;
    let shift = event.mods.shift;
    let alt = event.mods.alt;
    let win = event.mods.logo;
    if !(ctrl || shift || alt || win) {
        return None;
    }
    let key = match &event.key {
        Key::Command(character) => {
            if *character == ' ' {
                HotkeyKey::Space
            } else if character.is_ascii_alphanumeric() {
                HotkeyKey::Character(character.to_ascii_uppercase())
            } else {
                return None;
            }
        }
        Key::Text(text) => {
            let mut characters = text.chars();
            let character = characters.next()?;
            if characters.next().is_some() {
                return None;
            }
            if character == ' ' {
                HotkeyKey::Space
            } else if character.is_ascii_alphanumeric() {
                HotkeyKey::Character(character.to_ascii_uppercase())
            } else {
                return None;
            }
        }
        Key::Function(number) if (1..=24).contains(number) => HotkeyKey::Function(*number),
        _ => return None,
    };
    Some(Hotkey {
        ctrl,
        shift,
        alt,
        win,
        key,
    })
}

use crate::command::{CommandEvent, CommandService, CommandUi};
use crate::updates::{self as update_ui, UpdateEvent, UpdateService, UpdatesUi};

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
    Updates,
    About,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 4] = [
        SettingsPage::Setup,
        SettingsPage::General,
        SettingsPage::Updates,
        SettingsPage::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsPage::Setup => "Setup",
            SettingsPage::General => "General",
            SettingsPage::Updates => "Updates",
            SettingsPage::About => "About",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            SettingsPage::Setup => "setup",
            SettingsPage::General => "general",
            SettingsPage::Updates => "updates",
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechMode {
    Dictate,
    Command,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyTarget {
    Dictation,
    Command,
}

impl HotkeyTarget {
    pub fn label(self) -> &'static str {
        match self {
            HotkeyTarget::Dictation => "Dictation",
            HotkeyTarget::Command => "Command",
        }
    }

    pub fn setting(self) -> &'static str {
        match self {
            HotkeyTarget::Dictation => "dictation",
            HotkeyTarget::Command => "command",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SpeechUi {
    pub session: u64,
    pub mode: SpeechMode,
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
            mode: SpeechMode::Dictate,
            state: DictationState::Idle,
            detail: "Press the dictation hotkey or Start to dictate.".into(),
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
    pub command: CommandUi,
    pub routines: RoutineStore,
    pub routine_name: TextEdit,
    pub routine_command: TextEdit,
    pub routine_error: Option<String>,
    pub insights: Insights,
    pub updates: UpdatesUi,
    update_service: UpdateService,
    pub auto_update_attempted: bool,
    pub hotkey_target: Option<HotkeyTarget>,
    pub hotkey_message: Option<String>,
    pub hotkey: Option<HotkeyRegistration>,
    pub command_hotkey: Option<HotkeyRegistration>,
    pub tray: Option<rune::Tray>,
    pub tray_error: Option<String>,
    #[allow(dead_code)]
    tray_tx: mpsc::Sender<String>,
    tray_rx: Receiver<String>,
    #[allow(dead_code)]
    tray_attempted: bool,
    command_service: CommandService,
    command_rx: Receiver<CommandEvent>,
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
        let command = CommandUi::new();
        let (command_service, command_rx) = CommandService::new(paths.clone());
        let routines = RoutineStore::load(&paths.routines_file);
        let insights = Insights::load(&paths.insights_file);
        let (tray_tx, tray_rx) = mpsc::channel();
        let update_service = UpdateService::new(paths.clone());

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
            command,
            routines,
            routine_name: TextEdit::default(),
            routine_command: TextEdit::default(),
            routine_error: None,
            insights,
            updates: UpdatesUi::new(update::current_version()),
            update_service,
            auto_update_attempted: false,
            hotkey_target: None,
            hotkey_message: None,
            hotkey: None,
            command_hotkey: None,
            tray: None,
            tray_error: None,
            tray_tx,
            tray_rx,
            tray_attempted: false,
            command_service,
            command_rx,
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
        let command = CommandUi::new();
        let (command_service, command_rx) = CommandService::new(paths.clone());
        let routines = RoutineStore::load(&paths.routines_file);
        let insights = Insights::load(&paths.insights_file);
        let (tray_tx, tray_rx) = mpsc::channel();
        let update_service = UpdateService::new(paths.clone());
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
            command,
            routines,
            routine_name: TextEdit::default(),
            routine_command: TextEdit::default(),
            routine_error: None,
            insights,
            updates: UpdatesUi::new(update::current_version()),
            update_service,
            auto_update_attempted: false,
            hotkey_target: None,
            hotkey_message: None,
            hotkey: None,
            command_hotkey: None,
            tray: None,
            tray_error: None,
            tray_tx,
            tray_rx,
            tray_attempted: false,
            command_service,
            command_rx,
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

    pub fn set_update_channel(&mut self, channel: jarvis_core::UpdateChannel) {
        self.settings.update_channel = channel;
        self.save_settings();
    }

    pub fn set_check_for_updates(&mut self, enabled: bool) {
        self.settings.check_for_updates = enabled;
        self.save_settings();
        if !enabled {
            self.updates.phase = crate::updates::UpdatePhase::Idle;
            self.updates.latest = None;
            self.updates.error = None;
        }
    }

    pub fn set_auto_download_updates(&mut self, enabled: bool) {
        self.settings.auto_download_updates = enabled;
        self.save_settings();
    }

    pub fn check_for_updates(&mut self) {
        if self.updates.phase.is_busy() {
            return;
        }
        let Some(waker) = self.waker.clone() else {
            self.notice = Some(Notice::info("Updater waits for the first frame."));
            return;
        };
        self.update_service.check(waker);
    }

    pub fn download_update(&mut self) {
        let (Some(release), Some(asset)) =
            (self.updates.latest.clone(), self.updates.asset().cloned())
        else {
            return;
        };
        if !self.updates.can_download() {
            return;
        }
        let Some(waker) = self.waker.clone() else {
            self.notice = Some(Notice::info("Updater waits for the first frame."));
            return;
        };
        self.update_service.download(release, asset, waker);
    }

    /// Starts the verified installer and quits so it can replace `jarvis.exe`.
    /// `AppHandle::quit` is used because UI callbacks only receive `&mut Jarvis`.
    pub fn install_update(&mut self) {
        let Some(path) = self.updates.installer.clone() else {
            return;
        };
        match jarvis_platform::launch_installer(&path, false) {
            Ok(()) => {
                let version = self
                    .updates
                    .latest_version()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                logging::info(
                    &self.paths.log_file,
                    &format!("update installer started · {version}"),
                );
                let current = self.updates.current.clone();
                self.updates.apply(
                    UpdateEvent::InstallStarted,
                    self.settings.update_channel,
                    &current,
                );
                self.notice = Some(Notice::info(format!(
                    "Installing Jarvis {version}. Jarvis will restart when the installer finishes."
                )));
                if let Some(app) = app_handle() {
                    app.quit();
                }
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!(
                    "Could not start the installer: {error}"
                )));
            }
        }
    }

    pub fn reveal_update(&mut self) {
        let Some(path) = self.updates.installer.clone() else {
            return;
        };
        if let Err(error) = jarvis_platform::reveal_path(&path) {
            self.notice = Some(Notice::error(format!(
                "Could not open the update folder: {error}"
            )));
        }
    }

    fn maybe_auto_check(&mut self) {
        if self.auto_update_attempted || !update_ui::should_check(&self.settings, &self.paths) {
            return;
        }
        self.auto_update_attempted = true;
        self.check_for_updates();
    }

    fn apply_update_event(&mut self, event: UpdateEvent) {
        let channel = self.settings.update_channel;
        let current = self.updates.current.clone();
        let changed = self.updates.apply(event, channel, &current);
        if !changed {
            return;
        }
        if matches!(
            self.updates.phase,
            crate::updates::UpdatePhase::Available | crate::updates::UpdatePhase::UpToDate
        ) {
            let checked_at = chrono::Utc::now().to_rfc3339();
            self.updates.last_checked = Some(checked_at.clone());
            let receipt = update_ui::UpdateReceipt {
                version: self
                    .updates
                    .latest_version()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| self.version.to_string()),
                checked_at,
            };
            update_ui::save_receipt(&self.paths, &receipt);
        }
        logging::info(
            &self.paths.log_file,
            &format!(
                "updates · {} · {}",
                self.updates.phase.label(),
                self.updates
                    .latest_version()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "no release".into())
            ),
        );
        if self.settings.auto_download_updates
            && self.updates.can_download()
            && self
                .updates
                .asset()
                .is_some_and(|asset| asset.sha256.is_some())
        {
            self.download_update();
        }
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
        self.speech.mode = SpeechMode::Dictate;
        self.start_capture_dictation();
    }

    pub fn begin_command_dictation(&mut self) {
        if self.command.busy {
            return;
        }
        if self.speech.mode == SpeechMode::Command && self.speech.state == DictationState::Listening
        {
            self.stop_dictation();
            return;
        }
        if self.speech.state.is_active() {
            return;
        }
        self.section = Section::Assistant;
        self.speech.mode = SpeechMode::Command;
        self.start_capture_dictation();
    }

    fn start_capture_dictation(&mut self) {
        if let Err(error) = self.dictation.start() {
            self.speech.mode = SpeechMode::Dictate;
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
        self.speech.mode = SpeechMode::Dictate;
        self.dictation.cancel();
    }

    pub fn toggle_dictation(&mut self) {
        match self.speech.state {
            DictationState::Listening => self.stop_dictation(),
            DictationState::Processing => self.cancel_dictation(),
            _ => self.start_dictation(),
        }
    }

    pub fn submit_command(&mut self) {
        if self.command.busy {
            return;
        }
        let text = self.command.input.text.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.command.input = TextEdit::new("");
        self.run_command_text(text);
    }

    pub fn run_command_text(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let text = self.resolve_routine_command(&text).unwrap_or(text);
        self.enqueue_command_text(text);
    }

    fn resolve_routine_command(&self, text: &str) -> Option<String> {
        let lower = text.trim().to_ascii_lowercase();
        let candidate = lower.strip_prefix("run ").unwrap_or(&lower);
        self.routines
            .routines
            .iter()
            .find(|routine| routine.name.eq_ignore_ascii_case(candidate))
            .map(|routine| routine.command.clone())
    }

    fn enqueue_command_text(&mut self, text: String) {
        if self.command.busy {
            return;
        }
        self.section = Section::Assistant;
        match self.command_service.run(text.clone()) {
            Ok(()) => {
                self.command.busy = true;
                self.command.status = "Planning...".into();
                self.command.active_input = text;
            }
            Err(error) => {
                self.command.status = error;
            }
        }
    }

    pub fn open_command_file(&mut self, index: usize, reveal: bool) {
        let Some(record) = self.command.records.first() else {
            return;
        };
        let Some(file) = record.files.get(index) else {
            return;
        };
        if let Err(error) = jarvis_platform::open_file(&file.path, reveal) {
            self.notice = Some(Notice::error(format!(
                "Could not open {}: {error}",
                file.name
            )));
        }
    }

    pub fn add_routine(&mut self) {
        let name = self.routine_name.text.trim().to_string();
        let command = self.routine_command.text.trim().to_string();
        match self.routines.add(&name, &command) {
            Ok(_) => {
                self.routine_name = TextEdit::new("");
                self.routine_command = TextEdit::new("");
                self.routine_error = None;
                self.save_routines();
                self.notice = Some(Notice::info(format!("Saved the routine '{name}'.")));
            }
            Err(error) => self.routine_error = Some(error),
        }
    }

    pub fn delete_routine(&mut self, index: usize) {
        if let Some(routine) = self.routines.remove(index) {
            self.save_routines();
            self.notice = Some(Notice::info(format!(
                "Removed the routine '{}'.",
                routine.name
            )));
        }
    }

    pub fn run_routine(&mut self, index: usize) {
        let Some(routine) = self.routines.routines.get(index).cloned() else {
            return;
        };
        if self.command.busy {
            self.notice = Some(Notice::info("Another command is still running."));
            return;
        }
        self.enqueue_command_text(routine.command);
    }

    fn save_routines(&mut self) {
        if let Err(error) = self.routines.save_atomic(&self.paths.routines_file) {
            self.routine_error = Some(format!("Could not save routines: {error}"));
        }
    }

    fn save_insights(&mut self) {
        if let Err(error) = self.insights.save_atomic(&self.paths.insights_file) {
            logging::warn(
                &self.paths.log_file,
                &format!("insights save failed: {error}"),
            );
        }
    }

    pub fn set_launch_at_login(&mut self, enabled: bool) {
        match jarvis_platform::set_launch_at_login(enabled) {
            Ok(()) => {
                self.settings.launch_at_login = enabled;
                self.save_settings();
                self.notice = Some(Notice::info(if enabled {
                    "Jarvis will start when you sign in."
                } else {
                    "Jarvis will no longer start when you sign in."
                }));
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!(
                    "Could not update launch at login: {error}"
                )));
            }
        }
    }

    pub fn set_close_to_tray(&mut self, enabled: bool) {
        self.settings.close_to_tray = enabled;
        self.save_settings();
        self.notice = Some(Notice::info(if enabled {
            "Closing the window keeps Jarvis in the tray."
        } else {
            "Closing the window will quit Jarvis."
        }));
    }

    pub fn begin_hotkey_capture(&mut self, target: HotkeyTarget) {
        self.section = Section::Settings;
        self.settings_page = SettingsPage::General;
        self.hotkey_target = Some(target);
        self.hotkey_message = Some(format!(
            "Press the new {} hotkey. Escape cancels.",
            target.label().to_lowercase()
        ));
    }

    pub fn cancel_hotkey_capture(&mut self) {
        self.hotkey_target = None;
        self.hotkey_message = None;
    }

    pub fn apply_hotkey(&mut self, hotkey: Hotkey) {
        let Some(target) = self.hotkey_target else {
            return;
        };
        let chord = hotkey.format();
        let other = match target {
            HotkeyTarget::Dictation => &self.settings.command_hotkey,
            HotkeyTarget::Command => &self.settings.dictation_hotkey,
        };
        if Hotkey::parse(other).is_some_and(|other| other.format().eq_ignore_ascii_case(&chord)) {
            self.hotkey_message = Some("Dictation and command hotkeys must be different.".into());
            return;
        }
        let current = match target {
            HotkeyTarget::Dictation => &self.settings.dictation_hotkey,
            HotkeyTarget::Command => &self.settings.command_hotkey,
        };
        if Hotkey::parse(current)
            .is_some_and(|current| current.format().eq_ignore_ascii_case(&chord))
        {
            self.hotkey_target = None;
            self.hotkey_message = Some("Hotkey unchanged.".into());
            return;
        }

        let waker: Waker = self
            .waker
            .clone()
            .unwrap_or_else(|| std::sync::Arc::new(|| {}));
        let result = match target {
            HotkeyTarget::Dictation => {
                jarvis_platform::start_dictation_hotkey(self.hotkey_tx.clone(), waker, hotkey)
            }
            HotkeyTarget::Command => {
                jarvis_platform::start_command_hotkey(self.hotkey_tx.clone(), waker, hotkey)
            }
        };
        match result {
            Ok(handle) => {
                match target {
                    HotkeyTarget::Dictation => {
                        self.hotkey = Some(handle);
                        self.settings.dictation_hotkey = chord.clone();
                        self.speech.hotkey_error = None;
                    }
                    HotkeyTarget::Command => {
                        self.command_hotkey = Some(handle);
                        self.settings.command_hotkey = chord.clone();
                        self.command.hotkey_error = None;
                    }
                }
                self.hotkey_target = None;
                self.hotkey_message = Some(format!("Hotkey set to {}.", hotkey.format()));
                self.save_settings();
                self.notice = Some(Notice::info(format!(
                    "{} hotkey set to {}.",
                    target.label(),
                    hotkey.format()
                )));
            }
            Err(error) => {
                self.hotkey_message = Some(error.clone());
                let has_handle = match target {
                    HotkeyTarget::Dictation => self.hotkey.is_some(),
                    HotkeyTarget::Command => self.command_hotkey.is_some(),
                };
                if !has_handle {
                    match target {
                        HotkeyTarget::Dictation => self.speech.hotkey_error = Some(error),
                        HotkeyTarget::Command => self.command.hotkey_error = Some(error),
                    }
                }
            }
        }
    }

    pub fn register_command_hotkey(&mut self) {
        let Some(hotkey) = Hotkey::parse(&self.settings.command_hotkey) else {
            self.command.hotkey_error = Some("The saved command hotkey is not valid.".into());
            return;
        };
        let waker: Waker = self
            .waker
            .clone()
            .unwrap_or_else(|| std::sync::Arc::new(|| {}));
        match jarvis_platform::start_command_hotkey(self.hotkey_tx.clone(), waker, hotkey) {
            Ok(handle) => {
                self.command_hotkey = Some(handle);
                self.command.hotkey_error = None;
            }
            Err(error) => self.command.hotkey_error = Some(error),
        }
    }

    pub fn register_hotkey(&mut self) {
        let Some(hotkey) = Hotkey::parse(&self.settings.dictation_hotkey) else {
            self.speech.hotkey_error = Some("The saved dictation hotkey is not valid.".into());
            return;
        };
        let waker: Waker = self
            .waker
            .clone()
            .unwrap_or_else(|| std::sync::Arc::new(|| {}));
        match jarvis_platform::start_dictation_hotkey(self.hotkey_tx.clone(), waker, hotkey) {
            Ok(handle) => {
                self.hotkey = Some(handle);
                self.speech.hotkey_error = None;
            }
            Err(error) => self.speech.hotkey_error = Some(error),
        }
    }

    fn ensure_tray(&mut self) -> bool {
        #[cfg(not(windows))]
        {
            let _ = self.tray_attempted;
            false
        }
        #[cfg(windows)]
        {
            if self.tray.is_some() || self.tray_attempted {
                return false;
            }
            let Some(handle) = window_handle() else {
                return false;
            };
            self.tray_attempted = true;
            let menu = vec![
                rune::TrayItem::action("open", "Open Jarvis"),
                rune::TrayItem::Separator,
                rune::TrayItem::action("dictate", "Dictate"),
                rune::TrayItem::action("command", "Command"),
                rune::TrayItem::action("settings", "Settings"),
                rune::TrayItem::Separator,
                rune::TrayItem::action("quit", "Quit Jarvis"),
            ];
            let spec = rune::TraySpec::new("com.chiteshvarun.jarvis", "Jarvis", tray_icon())
                .tooltip("Jarvis — local voice assistant")
                .menu(menu);
            let sender = std::sync::Arc::new(std::sync::Mutex::new(self.tray_tx.clone()));
            let waker = self.waker.clone();
            let app_handle = app_handle();
            let result = rune::Tray::new(spec, move |event| match event {
                rune::TrayEvent::Activate { .. } => handle.open(),
                rune::TrayEvent::Menu(id) => {
                    if id == "open" {
                        handle.open();
                        return;
                    }
                    if id == "quit" {
                        if let Some(app) = &app_handle {
                            app.quit();
                            return;
                        }
                    }
                    if id == "settings" {
                        handle.open();
                    }
                    if let Ok(sender) = sender.lock() {
                        let _ = sender.send(id);
                    }
                    if let Some(waker) = &waker {
                        waker();
                    }
                }
                _ => {}
            });
            match result {
                Ok(tray) => {
                    self.tray = Some(tray);
                    self.tray_error = None;
                }
                Err(error) => {
                    self.tray_error = Some(error.to_string());
                    logging::warn(&self.paths.log_file, &format!("tray unavailable: {error}"));
                }
            }
            true
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
            if self.speech.mode == SpeechMode::Command {
                self.speech.mode = SpeechMode::Dictate;
                self.run_command_text(text);
            } else {
                self.start_insertion(text);
            }
        }
        if audio_limit {
            self.stop_dictation();
        }
        if error {
            self.speech.mode = SpeechMode::Dictate;
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
    state.maybe_auto_check();
    state.model_manager.set_notifier(waker.clone());
    state.dictation.set_notifier(waker.clone());
    state.command_service.set_notifier(waker.clone());
    if state.setup.checked_at.is_some() {
        return;
    }
    if state.hotkey.is_none() && state.speech.hotkey_error.is_none() {
        state.register_hotkey();
    }
    if state.command_hotkey.is_none() && state.command.hotkey_error.is_none() {
        state.register_command_hotkey();
    }
    state.start_setup_probe(waker);
}

pub fn poll(state: &mut Jarvis, _clock: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    changed |= state.ensure_tray();
    for event in state.update_service.drain() {
        state.apply_update_event(event);
        changed = true;
    }
    while let Ok(update) = state.model_rx.try_recv() {
        state.apply_model_update(update);
        changed = true;
    }
    while let Ok(event) = state.speech_rx.try_recv() {
        state.apply_speech_event(event);
        changed = true;
    }
    while let Ok(event) = state.hotkey_rx.try_recv() {
        match event {
            HotkeyEvent::Pressed => state.toggle_dictation(),
            HotkeyEvent::CommandPressed => state.begin_command_dictation(),
        }
        changed = true;
    }
    while let Ok(result) = state.insertion_rx.try_recv() {
        state.apply_insertion_result(result);
        changed = true;
    }
    while let Ok(event) = state.command_rx.try_recv() {
        if let CommandEvent::Done { record } = &event {
            state.insights.record(&record.source, record.ok);
            state.save_insights();
        }
        state.command.apply(event);
        changed = true;
    }
    while let Ok(action) = state.tray_rx.try_recv() {
        match action.as_str() {
            "dictate" => state.toggle_dictation(),
            "command" => state.begin_command_dictation(),
            "settings" => state.open_settings(SettingsPage::General),
            "quit" => tx.quit(),
            _ => {}
        }
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

#[cfg(windows)]
fn window_handle() -> Option<WindowHandle> {
    WINDOW_HANDLE.get().cloned()
}

fn app_handle() -> Option<AppHandle<Jarvis>> {
    APP_HANDLE.get().cloned()
}

#[cfg(windows)]
fn tray_icon() -> rune::TrayIcon {
    const SIZE: u32 = 32;
    let mut pixels = vec![0_u8; (SIZE * SIZE * 4) as usize];
    let center = (SIZE as f32 - 1.0) / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            let alpha = ((center - distance).clamp(0.0, 1.0) * 255.0) as u8;
            if alpha == 0 {
                continue;
            }
            let offset = ((y * SIZE + x) * 4) as usize;
            let (red, green, blue) = if distance <= 4.5 {
                (255, 255, 255)
            } else {
                (74, 143, 247)
            };
            pixels[offset] = red;
            pixels[offset + 1] = green;
            pixels[offset + 2] = blue;
            pixels[offset + 3] = alpha;
        }
    }
    rune::TrayIcon::rgba(SIZE, SIZE, pixels)
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
    fn hotkey_capture_normalizes_chords() {
        let event = KeyEvent {
            key: Key::Command('d'),
            mods: rune::Mods {
                ctrl: true,
                cmd: true,
                ..Default::default()
            },
        };
        assert_eq!(hotkey_from_event(&event).unwrap().format(), "Ctrl+D");

        let function = KeyEvent {
            key: Key::Function(5),
            mods: rune::Mods {
                ctrl: true,
                alt: true,
                ..Default::default()
            },
        };
        assert_eq!(
            hotkey_from_event(&function).unwrap().format(),
            "Ctrl+Alt+F5"
        );

        let plain = KeyEvent {
            key: Key::Command('x'),
            mods: rune::Mods::default(),
        };
        assert!(hotkey_from_event(&plain).is_none());
    }

    #[test]
    fn routines_and_tray_settings_persist() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(dir.path());
        let mut state = Jarvis::with_state(paths.clone(), AppSettings::default());
        state.routine_name = TextEdit::new("Focus");
        state.routine_command = TextEdit::new("open chrome and open vscode");
        state.add_routine();
        assert_eq!(state.routines.routines.len(), 1);
        assert_eq!(
            jarvis_core::RoutineStore::load(&paths.routines_file)
                .routines
                .len(),
            1
        );

        state.set_close_to_tray(false);
        assert!(!AppSettings::load(&paths.settings_file).close_to_tray);
        assert_eq!(
            state.resolve_routine_command("focus").as_deref(),
            Some("open chrome and open vscode")
        );
        assert_eq!(
            state.resolve_routine_command("run Focus").as_deref(),
            Some("open chrome and open vscode")
        );
        assert!(state.resolve_routine_command("something else").is_none());
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
