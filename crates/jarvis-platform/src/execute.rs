#[cfg(not(windows))]
mod stub;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
pub use stub::{execute, open_file};
#[cfg(windows)]
pub use windows::{execute, open_file};

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;
    use jarvis_commands::actions::PlannedAction;

    #[test]
    fn preview_execution_is_explicitly_unavailable() {
        let outcome = execute(
            &PlannedAction::OpenApp {
                name: "Notepad".into(),
            },
            &[],
        );
        assert!(!outcome.ok);
        assert!(outcome.message.contains("Windows-only"));
    }
}
