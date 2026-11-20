use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use jarvis_core::AppPaths;
use jarvis_models::speech;

use crate::audio::{CaptureConfig, CaptureHandle, SAMPLE_RATE};
use crate::stitch::final_chunk_start;
use crate::vad;
use crate::worker::{self, WhisperCommand};
use crate::{DictationState, Notifier, SpeechEvent};

const CHUNK_SECONDS: u64 = 6;
const OVERLAP_SECONDS: u64 = 1;
const MAX_SECONDS: u64 = 300;
const IDLE_UNLOAD_SECONDS: u64 = 300;

pub struct DictationController {
    paths: AppPaths,
    events: Sender<SpeechEvent>,
    notifier: Arc<Mutex<Option<Notifier>>>,
    session: u64,
    capture: Option<CaptureHandle>,
    chunker: Option<ChunkerHandle>,
    worker: Option<Sender<WhisperCommand>>,
    idle_stop: Option<Arc<AtomicBool>>,
    chunks_sent: Arc<AtomicU64>,
    last_chunk_end: Arc<AtomicU64>,
    chunk_failed: Arc<AtomicBool>,
}

impl DictationController {
    pub fn new(paths: AppPaths, events: Sender<SpeechEvent>) -> Self {
        Self {
            paths,
            events,
            notifier: Arc::new(Mutex::new(None)),
            session: 0,
            capture: None,
            chunker: None,
            worker: None,
            idle_stop: None,
            chunks_sent: Arc::new(AtomicU64::new(0)),
            last_chunk_end: Arc::new(AtomicU64::new(0)),
            chunk_failed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_notifier(&self, notifier: Notifier) {
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(notifier);
        }
    }

    pub fn is_active(&self) -> bool {
        self.capture.is_some()
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.capture.is_some() {
            return Ok(());
        }
        let spec = speech::production_spec();
        if !speech::cheap_verify(&speech::model_path(&self.paths), &spec) {
            return Err("The speech model is not installed. Download it in Setup first.".into());
        }

        let session = self.session + 1;
        self.session = session;
        self.chunks_sent.store(0, Ordering::SeqCst);
        self.last_chunk_end.store(0, Ordering::SeqCst);
        self.chunk_failed.store(false, Ordering::SeqCst);

        let buffer = Arc::new(Mutex::new(Vec::new()));
        let notifier = self.current_notifier();
        let (capture, device) = CaptureHandle::start(CaptureConfig {
            session,
            max_seconds: MAX_SECONDS,
            buffer: buffer.clone(),
            events: self.events.clone(),
            notifier,
        })?;
        self.capture = Some(capture);

        self.send(SpeechEvent::State {
            session,
            state: DictationState::Listening,
            detail: "Listening...".into(),
        });
        self.send(SpeechEvent::Device {
            session,
            name: device,
        });

        self.ensure_worker();
        self.start_chunker(session, buffer);
        if let Some(worker) = &self.worker {
            let _ = worker.send(WhisperCommand::StartSession { id: session });
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        let Some(capture) = self.capture.take() else {
            return;
        };
        let session = self.session;
        self.send(SpeechEvent::State {
            session,
            state: DictationState::Processing,
            detail: "Transcribing...".into(),
        });
        if let Some(chunker) = self.chunker.take() {
            drop(chunker);
        }
        let samples = capture.stop();
        let chunks = self.chunks_sent.load(Ordering::SeqCst);
        let last_end = self.last_chunk_end.load(Ordering::SeqCst) as usize;
        let failed = self.chunk_failed.load(Ordering::SeqCst);
        let start = final_chunk_start(
            samples.len(),
            failed,
            chunks,
            last_end,
            SAMPLE_RATE as usize * OVERLAP_SECONDS as usize,
        );
        match &self.worker {
            Some(worker) => {
                if worker
                    .send(WhisperCommand::Chunk {
                        id: session,
                        samples,
                        is_final: true,
                        final_start: start,
                    })
                    .is_err()
                {
                    self.send(SpeechEvent::Error {
                        session,
                        message: "The speech worker stopped unexpectedly. Restart Jarvis.".into(),
                    });
                }
            }
            None => self.send(SpeechEvent::Error {
                session,
                message: "The speech worker is unavailable. Restart Jarvis.".into(),
            }),
        }
    }

    pub fn abort(&mut self) {
        self.stop_active();
    }

    pub fn cancel(&mut self) {
        self.stop_active();
        self.send(SpeechEvent::State {
            session: 0,
            state: DictationState::Idle,
            detail: "Ready. Press the dictation hotkey or Start to speak.".into(),
        });
    }

    fn stop_active(&mut self) {
        let old = self.session;
        self.session = old + 1;
        self.chunks_sent.store(0, Ordering::SeqCst);
        self.last_chunk_end.store(0, Ordering::SeqCst);
        self.chunk_failed.store(false, Ordering::SeqCst);
        if let Some(chunker) = self.chunker.take() {
            drop(chunker);
        }
        if let Some(capture) = self.capture.take() {
            let _ = capture.stop();
        }
        if let Some(worker) = &self.worker {
            let _ = worker.send(WhisperCommand::Cancel { id: old });
        }
    }

    fn ensure_worker(&mut self) {
        if self.worker.is_some() {
            return;
        }
        let notifier = self.current_notifier();
        let sender = worker::spawn(
            self.paths.clone(),
            self.events.clone(),
            notifier,
            self.chunk_failed.clone(),
        );
        self.worker = Some(sender.clone());
        let stop = Arc::new(AtomicBool::new(false));
        self.idle_stop = Some(stop.clone());
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_secs(10));
                if sender
                    .send(WhisperCommand::UnloadIfIdle {
                        idle_secs: IDLE_UNLOAD_SECONDS,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
    }

    fn start_chunker(&mut self, session: u64, buffer: Arc<Mutex<Vec<f32>>>) {
        let Some(worker) = self.worker.clone() else {
            return;
        };
        let chunks_sent = self.chunks_sent.clone();
        let last_end = self.last_chunk_end.clone();
        let (stop, stop_rx) = mpsc::channel();
        let join = thread::spawn(move || {
            let window = (CHUNK_SECONDS + OVERLAP_SECONDS) as usize * SAMPLE_RATE as usize;
            loop {
                match stop_rx.recv_timeout(Duration::from_secs(CHUNK_SECONDS)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                let (chunk, length) = {
                    let buffer = buffer
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let start = buffer.len().saturating_sub(window);
                    (buffer[start..].to_vec(), buffer.len())
                };
                if chunk.is_empty() || vad::speech_window(&chunk).is_none() {
                    continue;
                }
                chunks_sent.fetch_add(1, Ordering::SeqCst);
                last_end.store(length as u64, Ordering::SeqCst);
                if worker
                    .send(WhisperCommand::Chunk {
                        id: session,
                        samples: chunk,
                        is_final: false,
                        final_start: 0,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.chunker = Some(ChunkerHandle {
            stop: Some(stop),
            join: Some(join),
        });
    }

    fn current_notifier(&self) -> Notifier {
        self.notifier
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .unwrap_or_else(|| Arc::new(|| {}))
    }

    fn send(&self, event: SpeechEvent) {
        let _ = self.events.send(event);
        self.current_notifier()();
    }
}

impl Drop for DictationController {
    fn drop(&mut self) {
        if let Some(stop) = self.idle_stop.take() {
            stop.store(true, Ordering::SeqCst);
        }
        if let Some(chunker) = self.chunker.take() {
            drop(chunker);
        }
        if let Some(capture) = self.capture.take() {
            let _ = capture.stop();
        }
        if let Some(worker) = &self.worker {
            let _ = worker.send(WhisperCommand::Shutdown);
        }
    }
}

struct ChunkerHandle {
    stop: Option<Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl Drop for ChunkerHandle {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_without_a_speech_model_is_an_error_and_does_not_capture() {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(directory.path());
        let (sender, _receiver) = mpsc::channel();
        let mut controller = DictationController::new(paths, sender);
        assert!(controller.start().is_err());
        assert!(!controller.is_active());
    }
}
