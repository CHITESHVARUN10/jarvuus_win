use std::collections::HashMap;
use std::path::Path;

use jarvis_core::AppPaths;
use ort::environment::Environment;
use ort::ep::ExecutionProviderDispatch;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::{DynValue, Tensor};
use sentencepiece::SentencePieceProcessor;
use serde::Serialize;
use serde_json::Value;

use super::runtime;
use crate::intent_store;
use crate::{ModelError, Result};

type EngineResult<T> = std::result::Result<T, String>;

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[derive(Debug, Serialize)]
pub struct Generation {
    pub raw: String,
    pub confidence: f64,
    pub input_ids: Vec<i64>,
    pub generated_ids: Vec<i64>,
}

pub struct IntentEngine {
    engine: Engine,
    backend: &'static str,
    version: String,
}

impl IntentEngine {
    pub fn load(paths: &AppPaths) -> Result<Self> {
        let root = paths.intent_dir.clone();
        let (folder, receipt) = intent_store::load_selection(&root)?;
        let environment = runtime::environment(paths)?;

        if cfg!(windows) {
            if let Ok(engine) = Engine::load(&folder, environment, &providers(true), threads()) {
                return Ok(Self {
                    engine,
                    backend: "directml",
                    version: receipt.version,
                });
            }
        }

        let engine =
            Engine::load(&folder, environment, &providers(false), threads()).map_err(|error| {
                ModelError::message(format!("the intent model could not be loaded: {error}"))
            })?;
        Ok(Self {
            engine,
            backend: "cpu",
            version: receipt.version,
        })
    }

    pub fn generate(&mut self, text: &str) -> Result<Generation> {
        self.engine.generate(text).map_err(ModelError::message)
    }

    pub fn backend(&self) -> &'static str {
        self.backend
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

struct Engine {
    enc: Session,
    dec: Session,
    sp: SentencePieceProcessor,
    added: HashMap<char, i64>,
    reverse: HashMap<i64, String>,
    layers: usize,
    heads: i64,
    d_kv: i64,
    start: i64,
}

impl Engine {
    fn load(
        folder: &Path,
        environment: &Environment,
        providers: &[ExecutionProviderDispatch],
        threads: usize,
    ) -> EngineResult<Self> {
        let config: Value =
            serde_json::from_slice(&std::fs::read(folder.join("config.json")).map_err(err)?)
                .map_err(err)?;
        let integer = |key: &str| {
            config[key]
                .as_i64()
                .filter(|value| *value > 0 && *value <= 4096)
                .ok_or_else(|| format!("invalid {key}"))
        };
        let added: HashMap<String, i64> =
            serde_json::from_slice(&std::fs::read(folder.join("added_tokens.json")).map_err(err)?)
                .map_err(err)?;
        let reverse = added
            .iter()
            .map(|(key, value)| (*value, key.clone()))
            .collect();
        let mut chars = HashMap::new();
        for (key, value) in added {
            let mut iter = key.chars();
            let character = iter.next().ok_or("empty added token")?;
            if iter.next().is_some() || value < 0 {
                return Err("unsupported added token".into());
            }
            chars.insert(character, value);
        }

        let session = || {
            Session::builder(environment)
                .map_err(err)?
                .with_execution_providers(providers)
                .map_err(err)?
                .with_optimization_level(GraphOptimizationLevel::All)
                .map_err(err)?
                .with_intra_threads(threads)
                .map_err(err)
        };

        Ok(Self {
            enc: session()?
                .commit_from_file(folder.join("encoder_model_quantized.onnx"))
                .map_err(err)?,
            dec: session()?
                .commit_from_file(folder.join("decoder_model_merged_quantized.onnx"))
                .map_err(err)?,
            sp: SentencePieceProcessor::open(folder.join("spiece.model")).map_err(err)?,
            added: chars,
            reverse,
            layers: integer("num_decoder_layers")? as usize,
            heads: integer("num_heads")?,
            d_kv: integer("d_kv")?,
            start: config["decoder_start_token_id"].as_i64().unwrap_or(0),
        })
    }

    fn encode(&self, text: &str) -> EngineResult<Vec<i64>> {
        let mut ids = Vec::new();
        let mut buffer = String::new();
        for character in text.chars() {
            if let Some(id) = self.added.get(&character) {
                flush_encode(&self.sp, &mut buffer, &mut ids)?;
                ids.push(*id);
            } else {
                buffer.push(character);
            }
        }
        flush_encode(&self.sp, &mut buffer, &mut ids)?;
        if ids.len() >= 128 {
            return Err("input_too_long".into());
        }
        ids.push(1);
        Ok(ids)
    }

    fn decode(&self, ids: &[i64]) -> EngineResult<String> {
        let mut parts = String::new();
        let mut buffer = Vec::new();
        for id in ids {
            if *id == 1 {
                break;
            }
            if let Some(text) = self.reverse.get(id) {
                flush_decode(&self.sp, &mut buffer, &mut parts)?;
                parts.push_str(text);
            } else if *id != 0 && *id != 2 {
                buffer.push(u32::try_from(*id).map_err(err)?);
            }
        }
        flush_decode(&self.sp, &mut buffer, &mut parts)?;
        Ok(parts.trim_matches(python_whitespace).to_owned())
    }

    fn generate(&mut self, text: &str) -> EngineResult<Generation> {
        let ids = self.encode(&format!("parse: {text}"))?;
        let length = ids.len();
        let mask = Tensor::from_array(([1, length], vec![1_i64; length]))
            .map_err(err)?
            .into_dyn();
        let mut outputs = self
            .enc
            .run(ort::inputs![
                "input_ids" => Tensor::from_array(([1, length], ids.clone())).map_err(err)?,
                "attention_mask" => &mask
            ])
            .map_err(err)?;
        let first = outputs
            .keys()
            .next()
            .ok_or("missing encoder output")?
            .to_string();
        let hidden = outputs.remove(&first).ok_or("missing encoder output")?;
        drop(outputs);

        let mut feed: HashMap<String, DynValue> = HashMap::new();
        feed.insert("encoder_attention_mask".into(), mask);
        feed.insert("encoder_hidden_states".into(), hidden);
        for layer in 0..self.layers {
            for kind in ["decoder", "encoder"] {
                for slot in ["key", "value"] {
                    feed.insert(
                        format!("past_key_values.{layer}.{kind}.{slot}"),
                        Tensor::from_array(([1, self.heads, 0, self.d_kv], Vec::<f32>::new()))
                            .map_err(err)?
                            .into_dyn(),
                    );
                }
            }
        }

        let mut generated = Vec::new();
        let mut logprobs = Vec::new();
        let mut current = self.start;
        for step in 0..448 {
            feed.insert(
                "input_ids".into(),
                Tensor::from_array(([1, 1], vec![current]))
                    .map_err(err)?
                    .into_dyn(),
            );
            feed.insert(
                "use_cache_branch".into(),
                Tensor::from_array(([1], vec![step > 0]))
                    .map_err(err)?
                    .into_dyn(),
            );
            let inputs: Vec<_> = feed
                .iter()
                .map(|(key, value)| (key.as_str(), value))
                .collect();
            let mut output = self.dec.run(inputs).map_err(err)?;
            let logits = output.get("logits").ok_or("missing decoder logits")?;
            let (shape, values) = logits.try_extract_tensor::<f32>().map_err(err)?;
            let count = *shape.last().ok_or("invalid logits shape")? as usize;
            if count == 0 || values.len() < count {
                return Err("invalid decoder logits".into());
            }
            let scores = &values[values.len() - count..];
            let (next, logprob) = argmax_logprob(scores)?;
            if next == 1 {
                drop(output);
                return Ok(Generation {
                    raw: self.decode(&generated)?,
                    confidence: if logprobs.is_empty() {
                        0.0
                    } else {
                        (logprobs.iter().sum::<f64>() / logprobs.len() as f64).exp()
                    },
                    input_ids: ids,
                    generated_ids: generated,
                });
            }
            generated.push(next);
            logprobs.push(logprob);
            current = next;
            for layer in 0..self.layers {
                for kind in ["decoder", "encoder"] {
                    if step > 0 && kind == "encoder" {
                        continue;
                    }
                    for slot in ["key", "value"] {
                        let key = format!("present.{layer}.{kind}.{slot}");
                        feed.insert(
                            format!("past_key_values.{layer}.{kind}.{slot}"),
                            output
                                .remove(&key)
                                .ok_or_else(|| format!("missing {key}"))?,
                        );
                    }
                }
            }
        }

        Ok(Generation {
            raw: String::new(),
            confidence: 0.0,
            input_ids: ids,
            generated_ids: generated,
        })
    }
}

fn flush_encode(
    processor: &SentencePieceProcessor,
    buffer: &mut String,
    ids: &mut Vec<i64>,
) -> EngineResult<()> {
    if !buffer.is_empty() {
        ids.extend(
            processor
                .encode(buffer)
                .map_err(err)?
                .into_iter()
                .map(|piece| i64::from(piece.id)),
        );
        buffer.clear();
    }
    Ok(())
}

fn flush_decode(
    processor: &SentencePieceProcessor,
    buffer: &mut Vec<u32>,
    parts: &mut String,
) -> EngineResult<()> {
    if !buffer.is_empty() {
        parts.push_str(&processor.decode_piece_ids(buffer).map_err(err)?);
        buffer.clear();
    }
    Ok(())
}

fn argmax_logprob(scores: &[f32]) -> EngineResult<(i64, f64)> {
    if scores.is_empty() || scores.iter().any(|value| !value.is_finite()) {
        return Err("invalid decoder scores".into());
    }
    let mut best = 0;
    for index in 1..scores.len() {
        if scores[index] > scores[best] {
            best = index;
        }
    }
    let peak = f64::from(scores[best]);
    let sum = scores
        .iter()
        .map(|value| (f64::from(*value) - peak).exp())
        .sum::<f64>();
    Ok((best as i64, peak - (peak + sum.ln())))
}

fn python_whitespace(character: char) -> bool {
    character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
}

fn providers(use_gpu: bool) -> Vec<ExecutionProviderDispatch> {
    let mut providers = Vec::new();
    #[cfg(windows)]
    if use_gpu {
        providers.push(
            ort::ep::DirectML::default()
                .with_device_id(0)
                .build()
                .error_on_failure(),
        );
    }
    #[cfg(not(windows))]
    let _ = use_gpu;
    providers.push(ort::ep::CPU::default().with_arena_allocator(false).build());
    providers
}

fn threads() -> usize {
    std::env::var("JARVIS_INTENT_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=64).contains(value))
        .unwrap_or_else(|| {
            (std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(4)
                / 2)
            .max(1)
        })
}

fn load_and_warm(folder: &Path, environment: &Environment, use_gpu: bool) -> EngineResult<()> {
    let mut engine = Engine::load(folder, environment, &providers(use_gpu), threads())?;
    let warm = engine.generate("pause")?;
    let value: Value = serde_json::from_str(&warm.raw).map_err(err)?;
    if !value.as_array().is_some_and(|actions| {
        !actions.is_empty()
            && actions
                .iter()
                .all(|action| action.is_object() && action["name"].is_string())
    }) {
        return Err("model warm-up returned an unexpected plan".into());
    }
    Ok(())
}

pub fn warm(paths: &AppPaths) -> Result<super::Warmup> {
    let root = paths.intent_dir.clone();
    let (folder, receipt) = intent_store::load_selection(&root)?;
    let environment = runtime::environment(paths)?;

    let mut gpu_error = None;

    if cfg!(windows) {
        match load_and_warm(&folder, environment, true) {
            Ok(()) => {
                let backend = "directml";
                let _ = intent_store::remember_good(&root, &receipt);
                return Ok(super::Warmup {
                    backend: backend.into(),
                    detail: format!(
                        "Intent model v{} loaded on {backend} and passed its warm-up.",
                        receipt.version
                    ),
                });
            }
            Err(error) => gpu_error = Some(error),
        }
    }

    match load_and_warm(&folder, environment, false) {
        Ok(()) => {
            let _ = intent_store::remember_good(&root, &receipt);
            Ok(super::Warmup {
                backend: "cpu".into(),
                detail: if let Some(error) = gpu_error {
                    format!(
                        "Intent model v{} loaded on CPU after the GPU path failed ({error}).",
                        receipt.version
                    )
                } else {
                    format!(
                        "Intent model v{} loaded on CPU and passed its warm-up.",
                        receipt.version
                    )
                },
            })
        }
        Err(error) => Err(ModelError::message(format!(
            "the intent model could not be loaded: {error}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argmax_ties_and_confidence() {
        let (index, probability) = argmax_logprob(&[1.0, 1.0, 0.0]).unwrap();
        assert_eq!(index, 0);
        assert!((probability.exp() - 1.0 / (2.0 + (-1.0f64).exp())).abs() < 1e-12);
        assert!(argmax_logprob(&[f32::NAN]).is_err());
    }

    #[test]
    fn missing_models_fail_before_touching_the_runtime() {
        let paths = AppPaths::with_data_dir(tempfile::tempdir().unwrap().path());
        let error = warm(&paths).unwrap_err().to_string();
        assert!(error.contains("intent model"));
        assert!(IntentEngine::load(&paths).is_err());
    }
}
