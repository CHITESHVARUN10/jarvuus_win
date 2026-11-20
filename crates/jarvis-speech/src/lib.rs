pub mod audio;
pub mod controller;
pub mod stitch;
pub mod vad;
mod worker;

use std::sync::Arc;

pub use controller::DictationController;

pub type Notifier = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DictationState {
    Idle,
    Listening,
    Processing,
    Ready,
    Error,
}

impl DictationState {
    pub fn label(self) -> &'static str {
        match self {
            DictationState::Idle => "Idle",
            DictationState::Listening => "Listening",
            DictationState::Processing => "Transcribing",
            DictationState::Ready => "Ready",
            DictationState::Error => "Error",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, DictationState::Listening | DictationState::Processing)
    }
}

#[derive(Clone, Debug)]
pub enum SpeechEvent {
    State {
        session: u64,
        state: DictationState,
        detail: String,
    },
    Device {
        session: u64,
        name: String,
    },
    Level {
        session: u64,
        level: f32,
    },
    Partial {
        session: u64,
        text: String,
    },
    Final {
        session: u64,
        text: String,
        backend: String,
    },
    Model {
        session: u64,
        loaded: bool,
        backend: Option<String>,
        detail: String,
    },
    AudioLimit {
        session: u64,
    },
    Error {
        session: u64,
        message: String,
    },
}
