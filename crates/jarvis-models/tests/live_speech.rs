use std::sync::atomic::AtomicBool;

use jarvis_core::AppPaths;
use jarvis_models::engines;
use jarvis_models::speech;

#[test]
#[ignore = "downloads the 1.6 GB speech model"]
fn published_speech_model_installs_and_warms() {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::with_data_dir(directory.path());
    paths.ensure_dirs().unwrap();
    let spec = speech::production_spec();

    let mut last_milestone = 0_u32;
    speech::install(&paths, &spec, &AtomicBool::new(false), &mut |progress| {
        let milestone = (progress.fraction * 10.0) as u32;
        if milestone > last_milestone {
            last_milestone = milestone;
            eprintln!(
                "speech download: {}% ({} of {}) · {}",
                milestone * 10,
                progress.downloaded,
                progress.total,
                jarvis_models::download::format_speed(progress.speed_bps)
            );
        }
    })
    .unwrap();

    speech::verify(&speech::model_path(&paths), &spec).unwrap();
    let warmup = engines::warm_speech(&paths).unwrap();
    eprintln!(
        "warm-up: backend={} detail={}",
        warmup.backend, warmup.detail
    );
    assert!(warmup.detail.contains("Whisper"));
}
