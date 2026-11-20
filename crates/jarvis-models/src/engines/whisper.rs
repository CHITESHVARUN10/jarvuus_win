use std::path::Path;

use jarvis_core::AppPaths;

use crate::speech::{self, production_spec};
use crate::{ModelError, Result};

#[cfg(feature = "gpu-vulkan")]
const USE_GPU: bool = true;
#[cfg(not(feature = "gpu-vulkan"))]
const USE_GPU: bool = false;

pub struct WhisperEngine {
    context: whisper_rs::WhisperContext,
    backend: &'static str,
}

impl WhisperEngine {
    pub fn load(paths: &AppPaths) -> Result<Self> {
        let spec = production_spec();
        let model = speech::model_path(paths);
        if !speech::cheap_verify(&model, &spec) {
            return Err(ModelError::message(
                "the speech model is not installed or is incomplete",
            ));
        }

        let (context, backend) = if USE_GPU {
            match create_context(&model, true) {
                Ok(context) => (context, super::whisper_backend_name()),
                Err(gpu_error) => match create_context(&model, false) {
                    Ok(context) => (context, "cpu"),
                    Err(cpu_error) => {
                        return Err(ModelError::message(format!(
                            "Whisper could not load on GPU ({gpu_error}) or CPU ({cpu_error})"
                        )))
                    }
                },
            }
        } else {
            (create_context(&model, false)?, "cpu")
        };

        Ok(Self { context, backend })
    }

    pub fn backend(&self) -> &'static str {
        self.backend
    }

    pub fn transcribe(&mut self, samples: &[f32]) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let mut params =
            whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        configure_decode(&mut params);

        let mut state = self
            .context
            .create_state()
            .map_err(|error| ModelError::message(error.to_string()))?;
        state
            .full(params, samples)
            .map_err(|error| ModelError::message(error.to_string()))?;

        let segments = state
            .full_n_segments()
            .map_err(|error| ModelError::message(error.to_string()))?;
        let mut text = String::new();
        for index in 0..segments {
            if let Ok(segment) = state.full_get_segment_text(index) {
                text.push_str(&segment);
            }
        }
        Ok(text)
    }

    pub fn warmup(&mut self) -> Result<()> {
        let silence = vec![0.0_f32; 16_000];
        let _ = self.transcribe(&silence)?;
        Ok(())
    }
}

pub fn warm(paths: &AppPaths) -> Result<super::Warmup> {
    let mut engine = WhisperEngine::load(paths)?;
    engine.warmup()?;
    let backend = engine.backend();
    Ok(super::Warmup {
        backend: backend.into(),
        detail: format!("Whisper loaded on {backend} and passed a silent warm-up."),
    })
}

fn create_context(path: &Path, use_gpu: bool) -> Result<whisper_rs::WhisperContext> {
    whisper_rs::WhisperContext::new_with_params(
        &path.to_string_lossy(),
        whisper_rs::WhisperContextParameters {
            use_gpu,
            ..Default::default()
        },
    )
    .map_err(|error| ModelError::message(error.to_string()))
}

fn configure_decode(params: &mut whisper_rs::FullParams<'_, '_>) {
    params.set_n_threads(4);
    params.set_language(Some("en"));
    params.set_no_context(true);
    params.set_suppress_blank(true);
    params.set_suppress_non_speech_tokens(true);
    params.set_no_speech_thold(0.6);
    params.set_temperature(0.0);
    params.set_temperature_inc(0.0);
    params.set_entropy_thold(2.4);
    params.set_logprob_thold(-1.0);
    params.set_no_timestamps(true);
    params.set_single_segment(true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_model_is_rejected_before_loading() {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(directory.path());
        assert!(WhisperEngine::load(&paths).is_err());
    }
}
