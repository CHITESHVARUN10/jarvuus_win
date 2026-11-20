use std::sync::atomic::AtomicBool;

use jarvis_core::AppPaths;
use jarvis_models::engines;
use jarvis_models::intent_remote::IntentRemote;
use jarvis_models::intent_store;

#[test]
#[ignore = "downloads the published intent release and needs ONNX Runtime"]
fn published_intent_release_installs_and_warms() {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::with_data_dir(directory.path());
    paths.ensure_dirs().unwrap();

    let mut last_milestone = 0_u32;
    let outcome = IntentRemote::default()
        .install(
            &paths.intent_dir,
            &AtomicBool::new(false),
            &mut |progress| {
                let milestone = (progress.fraction * 10.0) as u32;
                if milestone > last_milestone {
                    last_milestone = milestone;
                    eprintln!(
                        "intent download: {}% ({} of {})",
                        milestone * 10,
                        progress.downloaded,
                        progress.total
                    );
                }
            },
        )
        .unwrap();

    eprintln!(
        "install outcome: {} v{}",
        outcome.label(),
        outcome.receipt().version
    );
    let receipt = intent_store::receipt(&paths.intent_dir).unwrap();
    intent_store::verify(&paths.intent_dir, &receipt).unwrap();

    let warmup = engines::warm_intent(&paths).unwrap();
    eprintln!(
        "warm-up: backend={} detail={}",
        warmup.backend, warmup.detail
    );
    assert!(!warmup.backend.is_empty());

    let mut engine = jarvis_models::IntentEngine::load(&paths).unwrap();
    let generation = engine.generate("open chrome").unwrap();
    eprintln!(
        "intent generation: backend={} confidence={:.3} raw={}",
        engine.backend(),
        generation.confidence,
        generation.raw
    );
    assert!(generation.raw.contains("app.open"));
}
