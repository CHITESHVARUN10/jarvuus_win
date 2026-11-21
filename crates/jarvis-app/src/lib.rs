mod app;
mod command;
mod ui;
mod updates;

pub use app::{HotkeyTarget, Jarvis, Notice, NoticeTone, Section, SettingsPage};
pub use command::{CommandRecord, CommandUi};
pub use updates::{DownloadState, UpdateEvent, UpdatePhase, UpdatesUi};

use jarvis_core::insights::day_key_ago;
use jarvis_core::Insights;
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
    ("routines", |state| {
        state.section = Section::Routines;
        state.setup = Jarvis::demo_report(true);
        state.setup_running = false;
        state
            .routines
            .add("Focus time", "open chrome and open vscode")
            .unwrap();
        state
            .routines
            .add("Wind down", "close chrome and pause")
            .unwrap();
        state
            .routines
            .add("Morning brief", "what time is it and mute")
            .unwrap();
    }),
    ("insights", |state| {
        state.section = Section::Insights;
        state.setup = Jarvis::demo_report(true);
        state.setup_running = false;
        state.insights = Insights::default();
        for (offset, count, source, ok) in [
            (6, 3, "Rules", true),
            (5, 5, "Rules", true),
            (4, 2, "Intent model", true),
            (3, 6, "Rules", true),
            (2, 4, "Ollama", true),
            (1, 7, "Rules", false),
            (0, 5, "Rules", true),
        ] {
            for _ in 0..count {
                state.insights.record_on(&day_key_ago(offset), source, ok);
            }
        }
    }),
    ("appearance", |state| {
        state.section = Section::Appearance;
        state.setup = Jarvis::demo_report(true);
    }),
    ("settings", |state| {
        state.section = Section::Settings;
        state.settings_page = SettingsPage::General;
        state.setup = Jarvis::demo_report(true);
        state.settings.launch_at_login = true;
        state.settings.close_to_tray = true;
    }),
    ("updates", |state| {
        state.section = Section::Settings;
        state.settings_page = SettingsPage::Updates;
        state.setup = Jarvis::demo_report(true);
        state.auto_update_attempted = true;
        state.updates.latest = Some(jarvis_core::Release {
            version: semver::Version::new(0, 2, 0),
            tag: "v0.2.0".into(),
            notes: "Packaging, signed releases and the update flow.".into(),
            notes_url: String::new(),
            published_at: "2026-01-24T00:00:00Z".into(),
            assets: vec![jarvis_core::ReleaseAsset {
                name: "Jarvis-0.2.0-x64-setup.exe".into(),
                url: String::new(),
                bytes: 24_100_000,
                sha256: Some("a".repeat(64)),
            }],
        });
        state.updates.phase = crate::updates::UpdatePhase::Available;
        state.updates.signature_required = true;
    }),
    ("about", |state| {
        state.section = Section::Settings;
        state.settings_page = SettingsPage::About;
        state.setup = Jarvis::demo_report(true);
    }),
];

pub fn run() {
    let app = rune::App::new(AppSpec {
        setup: Some(app::setup_hook),
        poll: Some(app::poll),
        on_key: Some(app::on_key),
        scenes: SCENES,
        ..AppSpec::new("Jarvis", (1120.0, 760.0), Jarvis::new, ui::view)
    })
    .keep_running()
    .on_close(app::on_close)
    .on_ready(|handle| app::remember_window(handle.clone()));
    app::remember_app_handle(app.handle());
    app.run();
}
