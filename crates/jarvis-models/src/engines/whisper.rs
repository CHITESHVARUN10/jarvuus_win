use std::path::Path;

use jarvis_core::AppPaths;

use crate::speech::{self, production_spec};
use crate::Result;

#[cfg(feature = "gpu-vulkan")]
const USE_GPU: bool = true;
#[cfg(not(feature = "gpu-vulkan"))]
const USE_GPU: bool = false;

pub fn warm(paths: &AppPaths) -> Result<super::Warmup> {
    let spec = production_spec();
    let model = speech::model_path(paths);
    if !speech::cheap_verify(&model, &spec) {
        return Err(crate::ModelError::message(
            "the speech model is not installed or is incomplete",
        ));
    }

    let backend = if USE_GPU {
        match load_and_warm(&model, true) {
            Ok(()) => super::whisper_backend_name(),
            Err(gpu_error) => match load_and_warm(&model, false) {
                Ok(()) => "cpu",
                Err(cpu_error) => {
                    return Err(crate::ModelError::message(format!(
                        "Whisper could not load on GPU ({gpu_error}) or CPU ({cpu_error})"
                    )))
                }
            },
        }
    } else {
        load_and_warm(&model, false).map_err(crate::ModelError::message)?;
        "cpu"
    };

    Ok(super::Warmup {
        backend: backend.into(),
        detail: format!("Whisper loaded on {backend} and passed a silent warm-up."),
    })
}

fn load_and_warm(path: &Path, use_gpu: bool) -> std::result::Result<(), String> {
    let context = whisper_rs::WhisperContext::new_with_params(
        &path.to_string_lossy(),
        whisper_rs::WhisperContextParameters {
            use_gpu,
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())?;

    let mut params =
        whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(4);
    params.set_language(Some("en"));
    params.set_no_context(true);
    params.set_suppress_blank(true);
    params.set_suppress_non_speech_tokens(true);
    params.set_no_timestamps(true);
    params.set_single_segment(true);

    let mut state = context.create_state().map_err(|error| error.to_string())?;
    state
        .full(params, &vec![0.0_f32; 16_000])
        .map_err(|error| error.to_string())?;
    Ok(())
}
