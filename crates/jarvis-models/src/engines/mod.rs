use jarvis_core::AppPaths;

use crate::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warmup {
    pub backend: String,
    pub detail: String,
}

#[cfg(feature = "inference")]
mod intent;
#[cfg(feature = "inference")]
mod runtime;
#[cfg(feature = "inference")]
mod whisper;

pub fn warm_speech(paths: &AppPaths) -> Result<Warmup> {
    #[cfg(feature = "inference")]
    {
        whisper::warm(paths)
    }
    #[cfg(not(feature = "inference"))]
    {
        let _ = paths;
        Err(crate::ModelError::message(
            "this build does not include the speech runtime",
        ))
    }
}

pub fn warm_intent(paths: &AppPaths) -> Result<Warmup> {
    #[cfg(feature = "inference")]
    {
        intent::warm(paths)
    }
    #[cfg(not(feature = "inference"))]
    {
        let _ = paths;
        Err(crate::ModelError::message(
            "this build does not include the intent runtime",
        ))
    }
}

#[cfg(feature = "inference")]
pub fn whisper_backend_name() -> &'static str {
    if cfg!(feature = "gpu-vulkan") {
        "vulkan"
    } else {
        "cpu"
    }
}
