pub mod download;
pub mod engines;
pub mod intent_remote;
pub mod intent_store;
pub mod manager;
pub mod manifest;
pub mod speech;

#[cfg(feature = "inference")]
pub use engines::whisper::WhisperEngine;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("{0}")]
    Message(String),
    #[error("download cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

impl ModelError {
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

pub type Result<T> = std::result::Result<T, ModelError>;
