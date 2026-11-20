use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use jarvis_commands::actions::PlannedAction;
use jarvis_commands::{
    alias, intent_mapper, ollama, planner, safety, validate_action, validate_command,
};
use jarvis_core::{logging, AppPaths};
use jarvis_models::{intent_store, IntentEngine};
use jarvis_platform::{execute, FileHit};
use rune::TextEdit;

pub type Waker = Arc<dyn Fn() + Send + Sync>;

const INTENT_CONFIDENCE_THRESHOLD: f64 = 0.75;
const MAX_COMMAND_RECORDS: usize = 10;

#[derive(Clone, Debug, Default)]
pub struct CommandRecord {
    pub input: String,
    pub source: String,
    pub actions: Vec<String>,
    pub messages: Vec<String>,
    pub files: Vec<FileHit>,
    pub ok: bool,
}

#[derive(Clone, Debug)]
pub enum CommandEvent {
    Started { input: String },
    Status { message: String },
    Done { record: CommandRecord },
}

pub struct CommandUi {
    pub input: TextEdit,
    pub busy: bool,
    pub status: String,
    pub active_input: String,
    pub records: Vec<CommandRecord>,
    pub hotkey_error: Option<String>,
}

impl CommandUi {
    pub fn new() -> Self {
        Self {
            input: TextEdit::default(),
            busy: false,
            status: String::new(),
            active_input: String::new(),
            records: Vec::new(),
            hotkey_error: None,
        }
    }

    pub fn apply(&mut self, event: CommandEvent) -> bool {
        match event {
            CommandEvent::Started { input } => {
                self.busy = true;
                self.status = "Planning...".into();
                self.active_input = input;
            }
            CommandEvent::Status { message } => {
                self.status = message;
            }
            CommandEvent::Done { record } => {
                self.busy = false;
                self.status.clear();
                self.active_input.clear();
                self.records.insert(0, record);
                self.records.truncate(MAX_COMMAND_RECORDS);
            }
        }
        true
    }
}

impl Default for CommandUi {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CommandService {
    jobs: Sender<Job>,
    notifier: Arc<Mutex<Option<Waker>>>,
}

enum Job {
    Run(String),
}

impl CommandService {
    pub fn new(paths: AppPaths) -> (Self, Receiver<CommandEvent>) {
        let (jobs, job_rx) = mpsc::channel();
        let (events, event_rx) = mpsc::channel();
        let notifier = Arc::new(Mutex::new(None));
        let worker_notifier = notifier.clone();
        thread::spawn(move || {
            let mut worker = Worker {
                paths,
                events,
                notifier: worker_notifier,
                intent: None,
                intent_failed: false,
                last_files: Vec::new(),
                last_used: Instant::now(),
            };
            loop {
                match job_rx.recv_timeout(Duration::from_secs(30)) {
                    Ok(Job::Run(input)) => {
                        worker.last_used = Instant::now();
                        worker.emit(CommandEvent::Started {
                            input: input.clone(),
                        });
                        let record = worker.run(&input);
                        worker.last_used = Instant::now();
                        worker.emit(CommandEvent::Done { record });
                    }
                    Err(RecvTimeoutError::Timeout) => worker.release_idle(),
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        (Self { jobs, notifier }, event_rx)
    }

    pub fn set_notifier(&self, waker: Waker) {
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(waker);
        }
    }

    pub fn run(&self, input: String) -> Result<(), String> {
        self.jobs
            .send(Job::Run(input))
            .map_err(|_| "The command worker stopped. Restart Jarvis.".into())
    }
}

struct Worker {
    paths: AppPaths,
    events: Sender<CommandEvent>,
    notifier: Arc<Mutex<Option<Waker>>>,
    intent: Option<IntentEngine>,
    intent_failed: bool,
    last_files: Vec<FileHit>,
    last_used: Instant,
}

impl Worker {
    fn emit(&self, event: CommandEvent) {
        let _ = self.events.send(event);
        let waker = self.notifier.lock().ok().and_then(|slot| slot.clone());
        if let Some(waker) = waker {
            waker();
        }
    }

    fn status(&self, message: impl Into<String>) {
        self.emit(CommandEvent::Status {
            message: message.into(),
        });
    }

    fn release_idle(&mut self) {
        if self.intent.is_some() && self.last_used.elapsed() >= Duration::from_secs(300) {
            self.intent = None;
            logging::info(
                &self.paths.log_file,
                "intent command model unloaded after idle",
            );
        }
    }

    fn run(&mut self, input: &str) -> CommandRecord {
        let mut record = CommandRecord {
            input: input.to_string(),
            source: "Local".into(),
            ..CommandRecord::default()
        };

        let Some(cleaned) = validate_command(input) else {
            record
                .messages
                .push("That doesn't look like a command.".into());
            return record;
        };
        let mut actions = planner::plan(&cleaned);
        if actions.is_empty() {
            record
                .messages
                .push("I didn't catch a command. Try 'open notepad' or 'what time is it'.".into());
            return record;
        }
        record.source = "Rules".into();

        if is_single_ai(&actions) {
            if let Some(PlannedAction::AiQuery { query }) = actions.first() {
                if let Some(reason) = query.strip_prefix("blocked: ") {
                    record.messages.push(reason.to_string());
                    return record;
                }
            }
        }

        if is_single_ai(&actions) && !looks_like_question(&cleaned) {
            self.status("Checking the local intent model...");
            if let Some(engine) = self.local_intent() {
                match engine.generate(&cleaned) {
                    Ok(generation) => {
                        logging::info(
                            &self.paths.log_file,
                            &format!(
                                "intent command: confidence={:.3} raw={}",
                                generation.confidence, generation.raw
                            ),
                        );
                        if generation.confidence < INTENT_CONFIDENCE_THRESHOLD {
                            logging::info(
                                &self.paths.log_file,
                                &format!(
                                    "intent model skipped: low confidence {:.3}",
                                    generation.confidence
                                ),
                            );
                        } else if let Some(mut mapped) =
                            intent_mapper::map_intents_json(&generation.raw)
                        {
                            if mapped.iter().all(actions_allowed) && !mapped.iter().all(is_ai) {
                                normalize_actions(&mut mapped);
                                actions = mapped;
                                record.source = "Intent model".into();
                            }
                        }
                    }
                    Err(error) => {
                        logging::warn(
                            &self.paths.log_file,
                            &format!("intent model failed: {error}"),
                        );
                    }
                }
            }
        }

        if actions.iter().any(is_ai) && actions.len() > 1 {
            record
                .messages
                .push("I couldn't turn every part of that into a safe action.".into());
            return record;
        }

        if is_single_ai(&actions) {
            self.status("Asking Ollama...");
            if !ollama::ollama_available() {
                record.messages.push(
                    "I couldn't match that locally. Try a command like 'open notepad', or start Ollama for free-form answers."
                        .into(),
                );
                return record;
            }
            let planned = ollama::plan_with_timeout(&cleaned, 20);
            if planned.parsed
                && !planned.actions.is_empty()
                && !planned.actions.iter().all(is_ai)
                && planned.actions.iter().all(actions_allowed)
            {
                let mut planned_actions = planned.actions;
                normalize_actions(&mut planned_actions);
                actions = planned_actions;
                record.source = "Ollama".into();
            }
            if is_single_ai(&actions) {
                let answer = ollama::answer_with_timeout(&cleaned, 30);
                if answer.error.is_none() && !answer.text.trim().is_empty() {
                    record.ok = true;
                    record.source = "Ollama".into();
                    record.messages.push(answer.text.trim().to_string());
                    return record;
                }
                record.messages.push(
                    answer
                        .error
                        .unwrap_or_else(|| "Ollama didn't return an answer.".into()),
                );
                return record;
            }
        }

        let mut all_ok = true;
        let mut any_action = false;
        for action in &actions {
            record.actions.push(action.description());
            match validate_action(action) {
                safety::SafetyVerdict::Allowed => {}
                safety::SafetyVerdict::Blocked { reason } => {
                    record.messages.push(reason);
                    all_ok = false;
                    break;
                }
                safety::SafetyVerdict::InstallPreview { command, .. } => {
                    record.messages.push(format!(
                        "{command} needs confirmation and is disabled in this build."
                    ));
                    all_ok = false;
                    break;
                }
            }
            any_action = true;
            let outcome = execute(action, &self.last_files);
            if !outcome.files.is_empty() {
                self.last_files = outcome.files.clone();
                record.files = outcome.files.clone();
            }
            all_ok &= outcome.ok;
            record.messages.push(outcome.message);
        }
        record.ok = any_action && all_ok;
        record
    }

    fn local_intent(&mut self) -> Option<&mut IntentEngine> {
        if self.intent_failed {
            return None;
        }
        if self.intent.is_none() {
            intent_store::receipt(&self.paths.intent_dir)?;
            self.status("Loading the local intent model...");
            match IntentEngine::load(&self.paths) {
                Ok(engine) => {
                    logging::info(
                        &self.paths.log_file,
                        &format!(
                            "intent model loaded: v{} on {}",
                            engine.version(),
                            engine.backend()
                        ),
                    );
                    self.intent = Some(engine);
                }
                Err(error) => {
                    logging::warn(
                        &self.paths.log_file,
                        &format!("intent model unavailable: {error}"),
                    );
                    self.intent_failed = true;
                    return None;
                }
            }
        }
        self.intent.as_mut()
    }
}

fn is_ai(action: &PlannedAction) -> bool {
    matches!(action, PlannedAction::AiQuery { .. })
}

fn is_single_ai(actions: &[PlannedAction]) -> bool {
    actions.len() == 1 && is_ai(&actions[0])
}

fn looks_like_question(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "explain ",
        "what ",
        "what's ",
        "who ",
        "why ",
        "how ",
        "tell me ",
        "describe ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn actions_allowed(action: &PlannedAction) -> bool {
    matches!(validate_action(action), safety::SafetyVerdict::Allowed)
}

fn normalize_actions(actions: &mut [PlannedAction]) {
    for action in actions {
        match action {
            PlannedAction::OpenApp { name } | PlannedAction::CloseApp { name } => {
                *name = alias::resolve(name);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_skip_the_intent_model() {
        assert!(looks_like_question("what is the capital of France?"));
        assert!(looks_like_question("explain rust lifetimes"));
        assert!(!looks_like_question("open notepad"));
        assert!(!looks_like_question("set volume to 30"));
    }

    #[test]
    fn command_ui_collects_records_and_clears_busy() {
        let mut ui = CommandUi::new();
        ui.apply(CommandEvent::Started {
            input: "open notepad".into(),
        });
        assert!(ui.busy);
        ui.apply(CommandEvent::Status {
            message: "Planning...".into(),
        });
        assert_eq!(ui.status, "Planning...");
        ui.apply(CommandEvent::Done {
            record: CommandRecord {
                input: "open notepad".into(),
                source: "Rules".into(),
                ok: true,
                messages: vec!["Opened Notepad.".into()],
                ..CommandRecord::default()
            },
        });
        assert!(!ui.busy);
        assert_eq!(ui.records.len(), 1);
        assert_eq!(ui.records[0].source, "Rules");
    }

    #[test]
    fn command_history_is_bounded() {
        let mut ui = CommandUi::new();
        for index in 0..MAX_COMMAND_RECORDS + 5 {
            ui.apply(CommandEvent::Done {
                record: CommandRecord {
                    input: format!("command {index}"),
                    source: "Rules".into(),
                    ok: true,
                    ..CommandRecord::default()
                },
            });
        }
        assert_eq!(ui.records.len(), MAX_COMMAND_RECORDS);
        assert_eq!(ui.records[0].input, "command 14");
        assert_eq!(ui.records[MAX_COMMAND_RECORDS - 1].input, "command 5");
    }
}
