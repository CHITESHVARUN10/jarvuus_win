use jarvis_commands::actions::PlannedAction;
use jarvis_commands::execution::ActionOutcome;

pub fn execute(
    _action: &PlannedAction,
    _last_files: &[jarvis_commands::execution::FileHit],
) -> ActionOutcome {
    ActionOutcome::error("Command actions are Windows-only in this preview build.")
}

pub fn open_file(_path: &str, _reveal: bool) -> Result<(), String> {
    Err("Opening files is Windows-only in this preview build.".into())
}
