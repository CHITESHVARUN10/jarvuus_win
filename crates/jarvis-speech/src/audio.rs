use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;

use crate::{Notifier, SpeechEvent};

pub const SAMPLE_RATE: u32 = 16_000;
const FRAME_SAMPLES: usize = 320;
const LEVEL_INTERVAL: Duration = Duration::from_millis(50);
const NO_AUDIO_TIMEOUT: Duration = Duration::from_secs(3);

pub struct CaptureConfig {
    pub session: u64,
    pub max_seconds: u64,
    pub buffer: Arc<Mutex<Vec<f32>>>,
    pub events: Sender<SpeechEvent>,
    pub notifier: Notifier,
}

pub struct CaptureHandle {
    command: Sender<()>,
    join: Option<JoinHandle<()>>,
    buffer: Arc<Mutex<Vec<f32>>>,
}

impl CaptureHandle {
    pub fn start(config: CaptureConfig) -> Result<(Self, String), String> {
        let CaptureConfig {
            session,
            max_seconds,
            buffer,
            events,
            notifier,
        } = config;
        let (command, commands) = mpsc::channel();
        let (ready, readiness) = mpsc::channel();
        let thread_buffer = buffer.clone();
        let max_samples = SAMPLE_RATE as usize * max_seconds.max(1) as usize;

        let join = thread::spawn(move || {
            let host = cpal::default_host();
            let Some(device) = host.default_input_device() else {
                let _ = ready.send(Err("No microphone input device was found.".to_string()));
                return;
            };
            let device_name = device
                .name()
                .unwrap_or_else(|_| "Default microphone".into());
            let supported = match device.default_input_config() {
                Ok(supported) => supported,
                Err(error) => {
                    let _ = ready.send(Err(format!("Could not open the microphone: {error}")));
                    return;
                }
            };
            let sample_format = supported.sample_format();
            let stream_config: cpal::StreamConfig = supported.into();
            if stream_config.channels == 0 || stream_config.sample_rate.0 == 0 {
                let _ = ready.send(Err(
                    "The microphone reported no usable input format.".to_string()
                ));
                return;
            }

            let level = Arc::new(Mutex::new(Instant::now()));
            let last_audio = Arc::new(Mutex::new(Instant::now()));
            let failure_sent = Arc::new(AtomicBool::new(false));
            let limit_sent = Arc::new(AtomicBool::new(false));
            let stream = match build_stream(
                &device,
                &stream_config,
                sample_format,
                thread_buffer,
                max_samples,
                session,
                events.clone(),
                notifier.clone(),
                level.clone(),
                last_audio.clone(),
                failure_sent.clone(),
                limit_sent,
            ) {
                Ok(stream) => stream,
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            if let Err(error) = stream.play() {
                let _ = ready.send(Err(format!("Could not start microphone capture: {error}")));
                return;
            }

            let _ = ready.send(Ok(device_name));

            loop {
                match commands.recv_timeout(Duration::from_secs(1)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let silent_for = last_audio
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .elapsed();
                        if silent_for >= NO_AUDIO_TIMEOUT
                            && !failure_sent.swap(true, Ordering::SeqCst)
                        {
                            send(
                                &events,
                                &notifier,
                                SpeechEvent::Error {
                                    session,
                                    message:
                                        "Microphone input did not deliver audio. Check the input device and try again."
                                            .into(),
                                },
                            );
                            break;
                        }
                    }
                }
            }
            drop(stream);
        });

        match readiness.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(device)) => Ok((
                Self {
                    command,
                    join: Some(join),
                    buffer,
                },
                device,
            )),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(_) => {
                let _ = command.send(());
                let _ = join.join();
                Err("The microphone did not start in time.".into())
            }
        }
    }

    pub fn stop(mut self) -> Vec<f32> {
        self.shutdown();
        let mut buffer = self
            .buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *buffer)
    }

    fn shutdown(&mut self) {
        let _ = self.command.send(());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn default_input_name() -> Result<String, String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No microphone input device was found.".to_string())?;
    device.name().map_err(|error| error.to_string())
}

#[allow(clippy::too_many_arguments)]
fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    buffer: Arc<Mutex<Vec<f32>>>,
    max_samples: usize,
    session: u64,
    events: Sender<SpeechEvent>,
    notifier: Notifier,
    level: Arc<Mutex<Instant>>,
    last_audio: Arc<Mutex<Instant>>,
    failure_sent: Arc<AtomicBool>,
    limit_sent: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    macro_rules! build {
        ($ty:ty) => {
            build_typed_stream::<$ty>(
                device,
                config,
                buffer,
                max_samples,
                session,
                events,
                notifier,
                level,
                last_audio,
                failure_sent,
                limit_sent,
            )
        };
    }
    match format {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::F64 => build!(f64),
        cpal::SampleFormat::I8 => build!(i8),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::I32 => build!(i32),
        cpal::SampleFormat::I64 => build!(i64),
        cpal::SampleFormat::U8 => build!(u8),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::U32 => build!(u32),
        cpal::SampleFormat::U64 => build!(u64),
        other => Err(format!("Unsupported microphone sample format: {other:?}")),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_typed_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<Mutex<Vec<f32>>>,
    max_samples: usize,
    session: u64,
    events: Sender<SpeechEvent>,
    notifier: Notifier,
    level: Arc<Mutex<Instant>>,
    last_audio: Arc<Mutex<Instant>>,
    failure_sent: Arc<AtomicBool>,
    limit_sent: Arc<AtomicBool>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let mut resampler = InputResampler::new(config.sample_rate.0, config.channels as usize);
    let error_events = events.clone();
    let error_notifier = notifier.clone();
    let error_session = session;
    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let samples = resampler.convert(data);
                if samples.is_empty() {
                    return;
                }
                *last_audio
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Instant::now();

                let mut last_level = level
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if last_level.elapsed() >= LEVEL_INTERVAL {
                    let normalized = (rms(&samples) / 0.12).min(1.0);
                    send(
                        &events,
                        &notifier,
                        SpeechEvent::Level {
                            session,
                            level: normalized,
                        },
                    );
                    *last_level = Instant::now();
                }
                drop(last_level);

                let full = {
                    let mut buffer = buffer
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    append_bounded(&mut buffer, &samples, max_samples);
                    buffer.len() >= max_samples
                };
                if full && !limit_sent.swap(true, Ordering::SeqCst) {
                    send(
                        &events,
                        &notifier,
                        SpeechEvent::AudioLimit { session },
                    );
                }
            },
            move |error| {
                if !failure_sent.swap(true, Ordering::SeqCst) {
                    send(
                        &error_events,
                        &error_notifier,
                        SpeechEvent::Error {
                            session: error_session,
                            message: format!(
                                "Microphone input was interrupted: {error}. Check the input device and try again."
                            ),
                        },
                    );
                }
            },
            None,
        )
        .map_err(|error| error.to_string())?;
    Ok(stream)
}

fn send(events: &Sender<SpeechEvent>, notifier: &Notifier, event: SpeechEvent) {
    let _ = events.send(event);
    notifier();
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|sample| sample * sample).sum();
    (sum / samples.len() as f32).sqrt()
}

pub fn voiced_fraction(samples: &[f32], floor: f32) -> f32 {
    if samples.len() < FRAME_SAMPLES {
        return if rms(samples) >= floor { 1.0 } else { 0.0 };
    }
    let frames = samples.len() / FRAME_SAMPLES;
    let mut hot = 0;
    for index in 0..frames {
        let frame = &samples[index * FRAME_SAMPLES..(index + 1) * FRAME_SAMPLES];
        if rms(frame) >= floor {
            hot += 1;
        }
    }
    hot as f32 / frames as f32
}

pub(crate) fn append_bounded(buffer: &mut Vec<f32>, data: &[f32], max: usize) {
    let remaining = max.saturating_sub(buffer.len());
    buffer.extend_from_slice(&data[..data.len().min(remaining)]);
}

struct InputResampler {
    rate: u32,
    channels: usize,
    index: u64,
    next_output: f64,
    previous: f32,
}

impl InputResampler {
    fn new(rate: u32, channels: usize) -> Self {
        Self {
            rate,
            channels,
            index: 0,
            next_output: 0.0,
            previous: 0.0,
        }
    }

    fn convert<T: Sample>(&mut self, data: &[T]) -> Vec<f32>
    where
        f32: cpal::FromSample<T>,
    {
        let mut output = Vec::new();
        if self.channels == 0 {
            return output;
        }
        for frame in data.chunks_exact(self.channels) {
            let mono = frame
                .iter()
                .map(|sample| f32::from_sample(*sample))
                .sum::<f32>()
                / self.channels as f32;
            let mono = if mono.is_finite() { mono } else { 0.0 };
            while self.next_output <= self.index as f64 {
                let value = if self.index == 0 {
                    mono
                } else {
                    let fraction = (self.next_output - (self.index - 1) as f64) as f32;
                    self.previous + (mono - self.previous) * fraction
                };
                output.push(value);
                self.next_output += self.rate as f64 / SAMPLE_RATE as f64;
            }
            self.previous = mono;
            self.index += 1;
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs a microphone and OS permission"]
    fn microphone_capture_smoke() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let (events, _receiver) = mpsc::channel();
        let notifier: Notifier = Arc::new(|| {});
        let (capture, device) = CaptureHandle::start(CaptureConfig {
            session: 1,
            max_seconds: 2,
            buffer: buffer.clone(),
            events,
            notifier,
        })
        .unwrap();
        thread::sleep(Duration::from_millis(1200));
        let samples = capture.stop();
        assert!(!device.is_empty());
        assert!(
            !samples.is_empty(),
            "no samples arrived from the microphone"
        );
    }

    #[test]
    fn native_stereo_48khz_is_downmixed_and_resampled_for_whisper() {
        let input: Vec<f32> = (0..48_000).flat_map(|_| [0.2, 0.6]).collect();
        let mut converter = InputResampler::new(48_000, 2);
        let output = converter.convert(&input);
        assert_eq!(output.len(), 16_000);
        assert!(output.iter().all(|value| (*value - 0.4).abs() < 0.0001));
    }

    #[test]
    fn resampling_44100_keeps_time_across_callback_boundaries() {
        let input: Vec<f32> = (0..44_100)
            .map(|index| (index as f32 * 0.01).sin())
            .collect();
        let expected = InputResampler::new(44_100, 1).convert(&input);
        let mut converter = InputResampler::new(44_100, 1);
        let actual: Vec<f32> = input
            .chunks(257)
            .flat_map(|chunk| converter.convert(chunk))
            .collect();
        assert_eq!(actual.len(), 16_000);
        assert_eq!(actual, expected);
    }

    #[test]
    fn integer_microphone_samples_are_normalized() {
        let output = InputResampler::new(16_000, 1).convert(&[i16::MIN, 0, i16::MAX]);
        assert_eq!(output[0], -1.0);
        assert_eq!(output[1], 0.0);
        assert!(output[2] > 0.99 && output[2] <= 1.0);
    }

    #[test]
    fn oversized_callback_cannot_overflow_the_recording_buffer() {
        let mut buffer = vec![1.0, 2.0];
        append_bounded(&mut buffer, &[3.0, 4.0, 5.0, 6.0], 3);
        assert_eq!(buffer, vec![1.0, 2.0, 3.0]);
        append_bounded(&mut buffer, &[7.0], 3);
        assert_eq!(buffer, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn silence_and_speech_levels_differ() {
        assert_eq!(rms(&vec![0.0; 1600]), 0.0);
        assert!(rms(&vec![0.03; 1600]) > 0.02);
        assert_eq!(voiced_fraction(&vec![0.0; 1600], 0.004), 0.0);
        assert!(voiced_fraction(&vec![0.03; 1600], 0.004) > 0.9);
    }
}
