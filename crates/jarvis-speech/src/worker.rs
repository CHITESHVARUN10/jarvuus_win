use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use jarvis_core::{logging, AppPaths};
use jarvis_models::WhisperEngine;

use crate::stitch::{dedupe_merge, last_words, stitch};
use crate::vad;
use crate::{Notifier, SpeechEvent};

pub(crate) enum WhisperCommand {
    StartSession {
        id: u64,
    },
    Chunk {
        id: u64,
        samples: Vec<f32>,
        is_final: bool,
        final_start: usize,
    },
    Cancel {
        id: u64,
    },
    UnloadIfIdle {
        idle_secs: u64,
    },
    Shutdown,
}

pub(crate) fn spawn(
    paths: AppPaths,
    events: Sender<SpeechEvent>,
    notifier: Notifier,
    chunk_failed: Arc<AtomicBool>,
) -> Sender<WhisperCommand> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || run(paths, receiver, events, notifier, chunk_failed));
    sender
}

fn run(
    paths: AppPaths,
    receiver: Receiver<WhisperCommand>,
    events: Sender<SpeechEvent>,
    notifier: Notifier,
    chunk_failed: Arc<AtomicBool>,
) {
    let mut engine: Option<WhisperEngine> = None;
    let mut backend = String::new();
    let mut current_session = 0_u64;
    let mut session_active = false;
    let mut session_text = String::new();
    let mut last_tail: Option<String> = None;
    let mut chunks_done = 0_u64;
    let mut last_activity = Instant::now();

    while let Ok(command) = receiver.recv() {
        match command {
            WhisperCommand::StartSession { id } => {
                current_session = id;
                session_active = true;
                session_text.clear();
                last_tail = None;
                chunks_done = 0;
                chunk_failed.store(false, Ordering::SeqCst);
                last_activity = Instant::now();
                if !ensure_engine(&paths, &mut engine, &mut backend, &events, &notifier, id) {
                    session_active = false;
                }
            }
            WhisperCommand::Chunk {
                id,
                samples,
                is_final,
                final_start,
            } => {
                if id != current_session || !session_active {
                    continue;
                }
                last_activity = Instant::now();
                if !ensure_engine(&paths, &mut engine, &mut backend, &events, &notifier, id) {
                    session_active = false;
                    continue;
                }

                let start = if is_final {
                    let start = if chunk_failed.load(Ordering::SeqCst) || chunks_done == 0 {
                        0
                    } else {
                        final_start.min(samples.len())
                    };
                    if start == 0 {
                        session_text.clear();
                        last_tail = None;
                    }
                    start
                } else {
                    0
                };
                let Some(window) = vad::speech_window(&samples[start..]) else {
                    if is_final {
                        finish(
                            &events,
                            &notifier,
                            id,
                            &backend,
                            &mut session_text,
                            &mut session_active,
                        );
                        last_tail = None;
                        chunks_done = 0;
                        chunk_failed.store(false, Ordering::SeqCst);
                    } else {
                        chunk_failed.store(true, Ordering::SeqCst);
                    }
                    continue;
                };

                let raw = match engine.as_mut() {
                    Some(engine) => match engine.transcribe(window) {
                        Ok(text) => text,
                        Err(error) => {
                            logging::warn(
                                &paths.log_file,
                                &format!("speech chunk failed: {error}"),
                            );
                            if is_final {
                                send(
                                    &events,
                                    &notifier,
                                    SpeechEvent::Error {
                                        session: id,
                                        message: format!(
                                            "Speech processing failed: {error}. Check the model in Setup and try again."
                                        ),
                                    },
                                );
                                session_active = false;
                                session_text.clear();
                                last_tail = None;
                                chunks_done = 0;
                                chunk_failed.store(false, Ordering::SeqCst);
                            } else {
                                chunk_failed.store(true, Ordering::SeqCst);
                            }
                            continue;
                        }
                    },
                    None => continue,
                };

                let text = vad::remove_sound_captions(&raw);
                if text.is_empty() {
                    if is_final {
                        finish(
                            &events,
                            &notifier,
                            id,
                            &backend,
                            &mut session_text,
                            &mut session_active,
                        );
                        last_tail = None;
                        chunks_done = 0;
                        chunk_failed.store(false, Ordering::SeqCst);
                    } else {
                        chunk_failed.store(true, Ordering::SeqCst);
                    }
                    continue;
                }

                let merged = dedupe_merge(last_tail.as_deref(), &text);
                stitch(&mut session_text, &merged);
                last_tail = Some(last_words(&text, 12));

                if is_final {
                    finish(
                        &events,
                        &notifier,
                        id,
                        &backend,
                        &mut session_text,
                        &mut session_active,
                    );
                    last_tail = None;
                    chunks_done = 0;
                    chunk_failed.store(false, Ordering::SeqCst);
                } else {
                    chunks_done += 1;
                    let preview = session_text.trim().to_string();
                    if !preview.is_empty() {
                        send(
                            &events,
                            &notifier,
                            SpeechEvent::Partial {
                                session: id,
                                text: preview,
                            },
                        );
                    }
                }
            }
            WhisperCommand::Cancel { id } => {
                if id == current_session {
                    current_session = 0;
                    session_active = false;
                    session_text.clear();
                    last_tail = None;
                    chunks_done = 0;
                    chunk_failed.store(false, Ordering::SeqCst);
                }
            }
            WhisperCommand::UnloadIfIdle { idle_secs } => {
                if !session_active
                    && engine.is_some()
                    && last_activity.elapsed() >= Duration::from_secs(idle_secs)
                {
                    engine = None;
                    backend.clear();
                    logging::info(&paths.log_file, "speech model unloaded after idle");
                    send(
                        &events,
                        &notifier,
                        SpeechEvent::Model {
                            session: 0,
                            loaded: false,
                            backend: None,
                            detail: "Speech model unloaded after idle.".into(),
                        },
                    );
                }
            }
            WhisperCommand::Shutdown => break,
        }
    }
}

fn ensure_engine(
    paths: &AppPaths,
    engine: &mut Option<WhisperEngine>,
    backend: &mut String,
    events: &Sender<SpeechEvent>,
    notifier: &Notifier,
    session: u64,
) -> bool {
    if engine.is_some() {
        return true;
    }
    send(
        events,
        notifier,
        SpeechEvent::Model {
            session,
            loaded: false,
            backend: None,
            detail: "Loading the speech model...".into(),
        },
    );
    match WhisperEngine::load(paths) {
        Ok(loaded) => {
            *backend = loaded.backend().to_string();
            *engine = Some(loaded);
            send(
                events,
                notifier,
                SpeechEvent::Model {
                    session,
                    loaded: true,
                    backend: Some(backend.clone()),
                    detail: format!("Whisper loaded on {backend}."),
                },
            );
            true
        }
        Err(error) => {
            send(
                events,
                notifier,
                SpeechEvent::Error {
                    session,
                    message: format!("Could not load the speech model: {error}"),
                },
            );
            false
        }
    }
}

fn finish(
    events: &Sender<SpeechEvent>,
    notifier: &Notifier,
    session: u64,
    backend: &str,
    session_text: &mut String,
    active: &mut bool,
) {
    let text = session_text.trim().to_string();
    if text.is_empty() {
        no_speech(events, notifier, session, active, session_text);
        return;
    }
    send(
        events,
        notifier,
        SpeechEvent::Final {
            session,
            text,
            backend: backend.to_string(),
        },
    );
    *active = false;
    session_text.clear();
}

fn no_speech(
    events: &Sender<SpeechEvent>,
    notifier: &Notifier,
    session: u64,
    active: &mut bool,
    session_text: &mut String,
) {
    *active = false;
    session_text.clear();
    send(
        events,
        notifier,
        SpeechEvent::Error {
            session,
            message: "No speech detected. Try again and speak a little longer.".into(),
        },
    );
}

fn send(events: &Sender<SpeechEvent>, notifier: &Notifier, event: SpeechEvent) {
    let _ = events.send(event);
    notifier();
}
