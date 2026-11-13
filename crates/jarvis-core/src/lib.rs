pub mod logging;
pub mod models;
pub mod ollama;
pub mod paths;
pub mod settings;
pub mod setup;

pub use paths::AppPaths;
pub use settings::{AppSettings, ThemeChoice};
pub use setup::{SetupAction, SetupItem, SetupKind, SetupReport, SetupStatus};
