mod app;
mod ui;

pub use app::{Jarvis, Notice, NoticeTone, Section, SettingsPage};

use jarvis_models::manager::{ModelKind, ModelPhase, ModelUpdate};
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
