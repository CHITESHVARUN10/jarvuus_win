use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use jarvis_core::models;
use jarvis_core::{logging, AppPaths};

use crate::download::{format_duration, format_speed, Progress};
use crate::engines;
use crate::intent_remote::{InstallOutcome, IntentRemote};
use crate::intent_store;
use crate::speech;
use crate::Result;

pub type Notifier = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    Speech,
    Intent,
}

impl ModelKind {
    pub fn key(self) -> &'static str {
        match self {
            ModelKind::Speech => "speech",
            ModelKind::Intent => "intent",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ModelKind::Speech => "Speech model",
            ModelKind::Intent => "Intent model",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelPhase {
    Missing,
    Installed,
    Downloading,
    Verifying,
    Loading,
    Ready,
    Error,
    Cancelled,
}

impl ModelPhase {
    pub fn is_busy(self) -> bool {
        matches!(
            self,
            ModelPhase::Downloading | ModelPhase::Verifying | ModelPhase::Loading
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            ModelPhase::Missing => "Not installed",
            ModelPhase::Installed => "Installed",
            ModelPhase::Downloading => "Downloading",
            ModelPhase::Verifying => "Verifying",
            ModelPhase::Loading => "Loading",
            ModelPhase::Ready => "Ready",
            ModelPhase::Error => "Error",
            ModelPhase::Cancelled => "Cancelled",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModelUpdate {
    pub kind: ModelKind,
    pub phase: ModelPhase,
    pub fraction: f32,
    pub detail: String,
    pub backend: Option<String>,
    pub version: Option<String>,
}

impl ModelUpdate {
    pub fn new(kind: ModelKind, phase: ModelPhase, detail: impl Into<String>) -> Self {
        Self {
            kind,
            phase,
            fraction: 0.0,
            detail: detail.into(),
            backend: None,
            version: None,
        }
    }

    pub fn with_fraction(mut self, fraction: f32) -> Self {
        self.fraction = fraction.clamp(0.0, 1.0);
        self
    }

    pub fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = Some(backend.into());
        self
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn initial(paths: &AppPaths, kind: ModelKind) -> Self {
        match kind {
            ModelKind::Speech => {
                let spec = speech::production_spec();
                let path = speech::model_path(paths);
                if speech::cheap_verify(&path, &spec) {
                    ModelUpdate::new(
                        kind,
                        ModelPhase::Installed,
                        format!(
                            "{} installed · load it to warm the runtime",
                            models::human_bytes(spec.bytes)
                        ),
                    )
                } else {
                    ModelUpdate::new(kind, ModelPhase::Missing, "Not installed · 1.6 GB download")
                }
            }
            ModelKind::Intent => {
                let status = models::intent_status(paths);
                if status.is_installed() {
                    let mut update = ModelUpdate::new(
                        kind,
                        ModelPhase::Installed,
                        format!("{} · load it to warm the runtime", status.detail),
                    );
                    if let Some(version) = status.version {
                        update.version = Some(version);
                    }
                    update
                } else {
                    ModelUpdate::new(
                        kind,
                        ModelPhase::Missing,
                        format!("{} · about 206 MB download", status.detail),
                    )
                }
            }
        }
    }
}

pub struct ModelManager {
    paths: AppPaths,
    sender: Sender<ModelUpdate>,
    notifier: Arc<Mutex<Option<Notifier>>>,
    speech_cancel: Arc<AtomicBool>,
    intent_cancel: Arc<AtomicBool>,
    speech_running: Arc<AtomicBool>,
    intent_running: Arc<AtomicBool>,
}

impl ModelManager {
    pub fn new(paths: AppPaths, sender: Sender<ModelUpdate>) -> Self {
        Self {
            paths,
            sender,
            notifier: Arc::new(Mutex::new(None)),
            speech_cancel: Arc::new(AtomicBool::new(false)),
            intent_cancel: Arc::new(AtomicBool::new(false)),
            speech_running: Arc::new(AtomicBool::new(false)),
            intent_running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_notifier(&self, notifier: Notifier) {
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(notifier);
        }
    }

    pub fn install(&self, kind: ModelKind) {
        if !self.claim(kind) {
            return;
        }
        self.reset_cancel(kind);
        self.send(ModelUpdate::new(
            kind,
            ModelPhase::Verifying,
            match kind {
                ModelKind::Speech => "Preparing the speech model download...",
                ModelKind::Intent => "Checking the latest intent model...",
            },
        ));

        let paths = self.paths.clone();
        let sender = self.sender.clone();
        let notifier = self.notifier.clone();
        let cancel = match kind {
            ModelKind::Speech => self.speech_cancel.clone(),
            ModelKind::Intent => self.intent_cancel.clone(),
        };
        let running = match kind {
            ModelKind::Speech => self.speech_running.clone(),
            ModelKind::Intent => self.intent_running.clone(),
        };

        thread::spawn(move || {
            logging::info(
                &paths.log_file,
                &format!("{}: install started", kind.label()),
            );
            let result = download(kind, &paths, &cancel, &sender, &notifier);
            match result {
                Ok((version, detail)) => {
                    send_update(
                        &sender,
                        &notifier,
                        ModelUpdate::new(kind, ModelPhase::Loading, "Verifying and loading...")
                            .with_fraction(1.0)
                            .with_version(version.clone().unwrap_or_default()),
                    );
                    match load_engine(kind, &paths) {
                        Ok(warmup) => {
                            logging::info(
                                &paths.log_file,
                                &format!("{}: {}", kind.label(), warmup.detail),
                            );
                            send_update(
                                &sender,
                                &notifier,
                                ModelUpdate {
                                    kind,
                                    phase: ModelPhase::Ready,
                                    fraction: 1.0,
                                    detail: format!("{} {}", detail, warmup.detail),
                                    backend: Some(warmup.backend),
                                    version,
                                },
                            );
                        }
                        Err(error) => {
                            logging::warn(
                                &paths.log_file,
                                &format!("{}: warm-up failed: {error}", kind.label()),
                            );
                            send_update(
                                &sender,
                                &notifier,
                                ModelUpdate::new(
                                    kind,
                                    ModelPhase::Error,
                                    format!("Downloaded and verified, but loading failed: {error}"),
                                )
                                .with_fraction(1.0)
                                .with_version(version.clone().unwrap_or_default()),
                            );
                        }
                    }
                }
                Err(error) if error.is_cancelled() => {
                    send_update(
                        &sender,
                        &notifier,
                        ModelUpdate::new(
                            kind,
                            ModelPhase::Cancelled,
                            "Download cancelled · partial file kept for resume",
                        ),
                    );
                }
                Err(error) => {
                    logging::error(
                        &paths.log_file,
                        &format!("{}: install failed: {error}", kind.label()),
                    );
                    send_update(
                        &sender,
                        &notifier,
                        ModelUpdate::new(kind, ModelPhase::Error, error.to_string()),
                    );
                }
            }
            running.store(false, Ordering::SeqCst);
        });
    }

    pub fn load(&self, kind: ModelKind) {
        if !self.claim(kind) {
            return;
        }
        self.reset_cancel(kind);
        self.send(ModelUpdate::new(kind, ModelPhase::Loading, "Loading..."));

        let paths = self.paths.clone();
        let sender = self.sender.clone();
        let notifier = self.notifier.clone();
        let running = match kind {
            ModelKind::Speech => self.speech_running.clone(),
            ModelKind::Intent => self.intent_running.clone(),
        };

        thread::spawn(move || {
            match load_engine(kind, &paths) {
                Ok(warmup) => {
                    logging::info(
                        &paths.log_file,
                        &format!("{}: {}", kind.label(), warmup.detail),
                    );
                    let version = match kind {
                        ModelKind::Speech => None,
                        ModelKind::Intent => intent_store::current_info(&paths.intent_dir)
                            .map(|receipt| receipt.version),
                    };
                    send_update(
                        &sender,
                        &notifier,
                        ModelUpdate {
                            kind,
                            phase: ModelPhase::Ready,
                            fraction: 1.0,
                            detail: warmup.detail,
                            backend: Some(warmup.backend),
                            version,
                        },
                    );
                }
                Err(error) => {
                    logging::warn(
                        &paths.log_file,
                        &format!("{}: load failed: {error}", kind.label()),
                    );
                    send_update(
                        &sender,
                        &notifier,
                        ModelUpdate::new(kind, ModelPhase::Error, error.to_string()),
                    );
                }
            }
            running.store(false, Ordering::SeqCst);
        });
    }

    pub fn cancel(&self, kind: ModelKind) {
        match kind {
            ModelKind::Speech => self.speech_cancel.store(true, Ordering::SeqCst),
            ModelKind::Intent => self.intent_cancel.store(true, Ordering::SeqCst),
        }
    }

    pub fn is_running(&self, kind: ModelKind) -> bool {
        match kind {
            ModelKind::Speech => self.speech_running.load(Ordering::SeqCst),
            ModelKind::Intent => self.intent_running.load(Ordering::SeqCst),
        }
    }

    fn claim(&self, kind: ModelKind) -> bool {
        let running = match kind {
            ModelKind::Speech => &self.speech_running,
            ModelKind::Intent => &self.intent_running,
        };
        !running.swap(true, Ordering::SeqCst)
    }

    fn reset_cancel(&self, kind: ModelKind) {
        match kind {
            ModelKind::Speech => self.speech_cancel.store(false, Ordering::SeqCst),
            ModelKind::Intent => self.intent_cancel.store(false, Ordering::SeqCst),
        }
    }

    fn send(&self, update: ModelUpdate) {
        send_update(&self.sender, &self.notifier, update);
    }
}

fn download(
    kind: ModelKind,
    paths: &AppPaths,
    cancel: &AtomicBool,
    sender: &Sender<ModelUpdate>,
    notifier: &Arc<Mutex<Option<Notifier>>>,
) -> Result<(Option<String>, String)> {
    match kind {
        ModelKind::Speech => {
            let spec = speech::production_spec();
            speech::install(paths, &spec, cancel, &mut |progress| {
                send_update(
                    sender,
                    notifier,
                    ModelUpdate::new(kind, ModelPhase::Downloading, progress_detail(progress))
                        .with_fraction(progress.fraction),
                );
            })?;
            send_update(
                sender,
                notifier,
                ModelUpdate::new(
                    kind,
                    ModelPhase::Verifying,
                    "Speech model verified · preparing the runtime",
                )
                .with_fraction(1.0),
            );
            Ok((None, "Speech model downloaded and verified.".to_string()))
        }
        ModelKind::Intent => {
            let remote = IntentRemote::default();
            let outcome = remote.install(&paths.intent_dir, cancel, &mut |progress| {
                send_update(
                    sender,
                    notifier,
                    ModelUpdate::new(kind, ModelPhase::Downloading, progress_detail(progress))
                        .with_fraction(progress.fraction),
                );
            })?;
            let version = outcome.receipt().version.clone();
            let detail = match &outcome {
                InstallOutcome::Installed(_) => {
                    format!("Intent model v{version} installed and verified.")
                }
                InstallOutcome::UpToDate(_) => {
                    format!("Intent model v{version} is already installed.")
                }
                InstallOutcome::KeptNewer(_) => {
                    format!("A newer intent model v{version} is already installed.")
                }
            };
            Ok((Some(version), detail))
        }
    }
}

fn load_engine(kind: ModelKind, paths: &AppPaths) -> Result<engines::Warmup> {
    match kind {
        ModelKind::Speech => engines::warm_speech(paths),
        ModelKind::Intent => engines::warm_intent(paths),
    }
}

fn send_update(
    sender: &Sender<ModelUpdate>,
    notifier: &Arc<Mutex<Option<Notifier>>>,
    update: ModelUpdate,
) {
    let _ = sender.send(update);
    let notifier = notifier.lock().ok().and_then(|slot| slot.as_ref().cloned());
    if let Some(notifier) = notifier {
        notifier();
    }
}

fn progress_detail(progress: Progress) -> String {
    if progress.fraction >= 1.0 {
        return "Download complete · verifying".into();
    }
    if progress.speed_bps > 1.0 {
        format!(
            "{} of {} · {} · {} left",
            models::human_bytes(progress.downloaded),
            models::human_bytes(progress.total),
            format_speed(progress.speed_bps),
            format_duration(progress.eta_secs)
        )
    } else {
        format!(
            "{} of {}",
            models::human_bytes(progress.downloaded),
            models::human_bytes(progress.total)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn initial_states_reflect_the_files_on_disk() {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(directory.path());
        assert_eq!(
            ModelUpdate::initial(&paths, ModelKind::Speech).phase,
            ModelPhase::Missing
        );
        assert_eq!(
            ModelUpdate::initial(&paths, ModelKind::Intent).phase,
            ModelPhase::Missing
        );
    }

    #[test]
    fn loading_a_missing_model_reports_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(directory.path());
        let (sender, receiver) = mpsc::channel();
        let manager = ModelManager::new(paths, sender);
        manager.load(ModelKind::Speech);

        let mut saw_error = false;
        for _ in 0..4 {
            if let Ok(update) = receiver.recv_timeout(Duration::from_secs(2)) {
                if update.phase == ModelPhase::Error {
                    saw_error = true;
                    break;
                }
            } else {
                break;
            }
        }
        assert!(saw_error);
    }

    #[test]
    fn progress_details_are_human_readable() {
        let detail = progress_detail(Progress {
            downloaded: 1_000_000,
            total: 2_000_000,
            fraction: 0.5,
            speed_bps: 1_500_000.0,
            eta_secs: 1,
        });
        assert!(detail.contains("MB"));
        assert!(detail.contains("MB/s"));
        assert!(detail.contains("1s"));
    }

    #[test]
    fn phases_map_to_user_labels() {
        assert_eq!(ModelPhase::Downloading.label(), "Downloading");
        assert!(!ModelPhase::Ready.is_busy());
        assert!(ModelPhase::Loading.is_busy());
    }
}
