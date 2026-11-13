mod app;
mod ui;

pub use app::{Jarvis, Notice, NoticeTone, Section, SettingsPage};

use rune::AppSpec;

type Scene = (&'static str, fn(&mut Jarvis));

const SCENES: &[Scene] = &[
    ("setup", |state| {
        state.section = Section::Settings;
        state.settings_page = SettingsPage::Setup;
        state.setup = Jarvis::demo_report(false);
        state.setup_running = false;
    }),
    ("ready", |state| {
        state.settings.setup_completed = true;
        state.section = Section::Assistant;
        state.setup = Jarvis::demo_report(true);
        state.setup_running = false;
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
