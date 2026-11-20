mod app;
mod command;
mod ui;

pub use app::{Jarvis, Notice, NoticeTone, Section, SettingsPage};
pub use command::{CommandRecord, CommandUi};

use jarvis_models::manager::{ModelKind, ModelPhase, ModelUpdate};
use jarvis_speech::DictationState;
use rune::AppSpec;

type Scene = (&'static str, fn(&mut Jarvis));

const SCENES: &[Scene] = &[
    ("setup", |state| {
        state.section = Section::Settings;
        state.settings_page = SettingsPage::Setup;
        state.setup = Jarvis::demo_report(false);
        state.setup_running = false;
        state.models.speech = ModelUpdate::new(
            ModelKind::Speech,
            ModelPhase::Missing,
            "Not installed · 1.6 GB download",
        );
        state.models.intent = ModelUpdate::new(
            ModelKind::Intent,
            ModelPhase::Missing,
            "Not installed · about 206 MB download",
        );
    }),
    ("downloading", |state| {
        state.section = Section::Settings;
        state.settings_page = SettingsPage::Setup;
        state.setup = Jarvis::demo_report(false);
        state.setup_running = false;
        state.models.speech = ModelUpdate::new(
            ModelKind::Speech,
            ModelPhase::Downloading,
            "742 MB of 1.5 GB · 24.1 MB/s · 32s left",
        )
        .with_fraction(0.48);
        state.models.intent = ModelUpdate::new(
            ModelKind::Intent,
            ModelPhase::Missing,
            "Not installed · about 206 MB download",
        );
    }),
    ("ready", |state| {
        state.settings.setup_completed = true;
        state.section = Section::Assistant;
        state.setup = Jarvis::demo_report(true);
        state.setup_running = false;
        state.command.records.push(CommandRecord {
            input: "open chrome and set volume to 30".into(),
            source: "Rules".into(),
            actions: vec!["Open 'Google Chrome'".into(), "Volume: set to 30%".into()],
            messages: vec![
                "Opened Google Chrome.".into(),
                "Volume set to 30 percent.".into(),
            ],
            files: Vec::new(),
            ok: true,
        });
        state.command.hotkey_error = None;
        state.models.speech = ModelUpdate::new(
            ModelKind::Speech,
            ModelPhase::Ready,
            "Whisper loaded on vulkan and passed a silent warm-up.",
        )
        .with_fraction(1.0)
        .with_backend("vulkan");
        state.models.intent = ModelUpdate::new(
            ModelKind::Intent,
            ModelPhase::Ready,
            "Intent model v1.0.2 loaded on directml and passed its warm-up.",
        )
        .with_fraction(1.0)
        .with_backend("directml")
        .with_version("1.0.2");
    }),
    ("voice", |state| {
        state.section = Section::Voice;
        state.setup = Jarvis::demo_report(true);
        state.speech.state = DictationState::Listening;
        state.speech.detail = "Listening...".into();
        state.speech.level = 0.62;
        state.speech.partial = "remind me to send the design review before the standup".into();
        state.speech.device = Some("Microphone Array (Realtek Audio)".into());
        state.speech.model_loaded = true;
        state.speech.backend = Some("vulkan".into());
        state.speech.model_detail = "Whisper loaded on vulkan.".into();
        state.speech.hotkey_error = None;
        state.models.speech = ModelUpdate::new(
            ModelKind::Speech,
            ModelPhase::Ready,
            "Whisper loaded on vulkan and passed a silent warm-up.",
        )
        .with_fraction(1.0)
        .with_backend("vulkan");
        state.models.intent = ModelUpdate::new(
            ModelKind::Intent,
            ModelPhase::Ready,
            "Intent model v1.0.2 loaded on directml and passed its warm-up.",
        )
        .with_fraction(1.0)
        .with_backend("directml")
        .with_version("1.0.2");
    }),
    ("appearance", |state| {
        state.section = Section::Appearance;
        state.setup = Jarvis::demo_report(true);
    }),
    ("about", |state| {
        state.section = Section::Settings;
        state.settings_page = SettingsPage::About;
        state.setup = Jarvis::demo_report(true);
    }),
];

pub fn run() {
    rune::run(AppSpec {
        setup: Some(app::setup_hook),
        poll: Some(app::poll),
        scenes: SCENES,
        ..AppSpec::new("Jarvis", (1120.0, 760.0), Jarvis::new, ui::view)
    });
}
