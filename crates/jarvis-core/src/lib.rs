pub mod hotkey;
pub mod insights;
pub mod logging;
pub mod models;
pub mod ollama;
pub mod paths;
pub mod routines;
pub mod settings;
pub mod setup;

pub use hotkey::Hotkey;
pub use insights::{DayInsight, Insights};
pub use paths::AppPaths;
pub use routines::{Routine, RoutineStore};
pub use settings::{AppSettings, ThemeChoice};
pub use setup::{SetupAction, SetupItem, SetupKind, SetupReport, SetupStatus};
