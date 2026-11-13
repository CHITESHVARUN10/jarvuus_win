use std::time::Duration;

use chrono::Local;

use crate::models::{self, ModelState};
use crate::ollama::{self, OllamaInfo};
use crate::paths::AppPaths;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consent {
    Allowed,
    Denied,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicrophoneAccess {
    pub consent: Consent,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuAdapter {
    pub name: String,
    pub vram_mb: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GpuInfo {
    pub adapters: Vec<GpuAdapter>,
    pub note: Option<String>,
}

pub trait SystemProbe {
    fn microphone(&self) -> MicrophoneAccess;
    fn gpu(&self) -> GpuInfo;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupKind {
    Microphone,
    SpeechModel,
    IntentModel,
    Gpu,
    Ollama,
}

impl SetupKind {
    pub const ALL: [SetupKind; 5] = [
        SetupKind::Microphone,
        SetupKind::SpeechModel,
        SetupKind::IntentModel,
        SetupKind::Gpu,
        SetupKind::Ollama,
    ];

    pub fn key(self) -> &'static str {
        match self {
            SetupKind::Microphone => "microphone",
            SetupKind::SpeechModel => "speech-model",
            SetupKind::IntentModel => "intent-model",
            SetupKind::Gpu => "gpu",
            SetupKind::Ollama => "ollama",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            SetupKind::Microphone => "Microphone access",
            SetupKind::SpeechModel => "Speech model",
            SetupKind::IntentModel => "Intent model",
            SetupKind::Gpu => "GPU acceleration",
            SetupKind::Ollama => "Ollama",
        }
    }

    pub fn is_essential(self) -> bool {
        matches!(
            self,
            SetupKind::Microphone | SetupKind::SpeechModel | SetupKind::IntentModel
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupStatus {
    Ready,
    Missing,
    Incomplete,
    Denied,
    Warning,
    Checking,
}

impl SetupStatus {
    pub fn label(self) -> &'static str {
        match self {
            SetupStatus::Ready => "Ready",
            SetupStatus::Missing => "Not installed",
            SetupStatus::Incomplete => "Incomplete",
            SetupStatus::Denied => "Blocked",
            SetupStatus::Warning => "Attention",
            SetupStatus::Checking => "Checking",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupAction {
    OpenMicrophoneSettings,
    OpenOllamaWebsite,
    OpenModelFolder,
    None,
}

impl SetupAction {
    pub fn label(self) -> Option<&'static str> {
        match self {
            SetupAction::OpenMicrophoneSettings => Some("Open Windows settings"),
            SetupAction::OpenOllamaWebsite => Some("Get Ollama"),
            SetupAction::OpenModelFolder => Some("Model folder"),
            SetupAction::None => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupItem {
    pub kind: SetupKind,
    pub detail: String,
    pub status: SetupStatus,
    pub action: SetupAction,
}

impl SetupItem {
    pub fn title(&self) -> &'static str {
        self.kind.title()
    }

    pub fn key(&self) -> &'static str {
        self.kind.key()
    }

    pub fn is_ready(&self) -> bool {
        self.status == SetupStatus::Ready
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupReport {
    pub items: Vec<SetupItem>,
    pub essentials_ready: usize,
    pub essentials_total: usize,
    pub checked_at: Option<String>,
}

impl SetupReport {
    pub fn new(items: Vec<SetupItem>) -> Self {
        let essentials_total = items.iter().filter(|item| item.kind.is_essential()).count();
        let essentials_ready = items
            .iter()
            .filter(|item| item.kind.is_essential() && item.is_ready())
            .count();
        Self {
            items,
            essentials_ready,
            essentials_total,
            checked_at: Some(Local::now().format("%H:%M:%S").to_string()),
        }
    }

    pub fn checking() -> Self {
        Self {
            items: SetupKind::ALL
                .into_iter()
                .map(|kind| SetupItem {
                    kind,
                    detail: "Checking...".into(),
                    status: SetupStatus::Checking,
                    action: SetupAction::None,
                })
                .collect(),
            essentials_ready: 0,
            essentials_total: SetupKind::ALL
                .iter()
                .filter(|kind| kind.is_essential())
                .count(),
            checked_at: None,
        }
    }

    pub fn essentials_complete(&self) -> bool {
        self.essentials_ready == self.essentials_total
    }

    pub fn item(&self, kind: SetupKind) -> Option<&SetupItem> {
        self.items.iter().find(|item| item.kind == kind)
    }

    pub fn update_item(&mut self, kind: SetupKind, status: SetupStatus, detail: impl Into<String>) {
        if let Some(item) = self.items.iter_mut().find(|item| item.kind == kind) {
            item.status = status;
            item.detail = detail.into();
        }
        self.recompute_essentials();
    }

    fn recompute_essentials(&mut self) {
        self.essentials_ready = self
            .items
            .iter()
            .filter(|item| item.kind.is_essential() && item.is_ready())
            .count();
    }

    pub fn run(paths: &AppPaths, probe: &impl SystemProbe) -> Self {
        let mic = probe.microphone();
        let gpu = probe.gpu();
        let whisper = models::whisper_status(paths);
        let intent = models::intent_status(paths);
        let ollama = ollama::probe(Duration::from_millis(900));

        let microphone = SetupItem {
            kind: SetupKind::Microphone,
            status: match mic.consent {
                Consent::Allowed => SetupStatus::Ready,
                Consent::Denied => SetupStatus::Denied,
                Consent::Unknown => SetupStatus::Warning,
            },
            detail: mic.detail,
            action: if mic.consent == Consent::Allowed {
                SetupAction::None
            } else {
                SetupAction::OpenMicrophoneSettings
            },
        };

        let speech = SetupItem {
            kind: SetupKind::SpeechModel,
            status: model_setup_status(whisper.state),
            detail: whisper.detail,
            action: if whisper.state == ModelState::Installed {
                SetupAction::OpenModelFolder
            } else {
                SetupAction::None
            },
        };

        let intent = SetupItem {
            kind: SetupKind::IntentModel,
            status: model_setup_status(intent.state),
            detail: intent.detail,
            action: if intent.state == ModelState::Installed {
                SetupAction::OpenModelFolder
            } else {
                SetupAction::None
            },
        };

        let (gpu_status, gpu_detail) = gpu_reading(&gpu);
        let gpu_item = SetupItem {
            kind: SetupKind::Gpu,
            status: gpu_status,
            detail: gpu_detail,
            action: SetupAction::None,
        };

        let ollama_item = ollama_item(ollama);

        Self::new(vec![microphone, speech, intent, gpu_item, ollama_item])
    }
}

fn model_setup_status(state: ModelState) -> SetupStatus {
    match state {
        ModelState::Installed => SetupStatus::Ready,
        ModelState::Missing => SetupStatus::Missing,
        ModelState::Incomplete => SetupStatus::Incomplete,
    }
}

fn gpu_reading(gpu: &GpuInfo) -> (SetupStatus, String) {
    if gpu.adapters.is_empty() {
        let detail = gpu
            .note
            .clone()
            .unwrap_or_else(|| "No hardware GPU detected. Whisper will use the CPU.".into());
        return (SetupStatus::Warning, detail);
    }

    let names: Vec<String> = gpu
        .adapters
        .iter()
        .take(2)
        .map(|adapter| match adapter.vram_mb {
            Some(mb) if mb > 0 => {
                format!(
                    "{} ({} GB)",
                    adapter.name,
                    (mb as f64 / 1024.0).round() as u64
                )
            }
            _ => adapter.name.clone(),
        })
        .collect();
    let suffix = if gpu.adapters.len() > 2 {
        format!(" and {} more", gpu.adapters.len() - 2)
    } else {
        String::new()
    };
    let mut detail = format!(
        "{}{} · Vulkan for speech, DirectML for intent, CPU fallback",
        names.join(" + "),
        suffix
    );
    if let Some(note) = &gpu.note {
        detail.push_str(" · ");
        detail.push_str(note);
    }
    (SetupStatus::Ready, detail)
}

fn ollama_item(info: OllamaInfo) -> SetupItem {
    let detail = if info.reachable {
        let model = if info.models.is_empty() {
            "no models pulled".to_string()
        } else {
            format!(
                "{} model{}",
                info.models.len(),
                if info.models.len() == 1 { "" } else { "s" }
            )
        };
        format!("Running · {model} · optional local answers")
    } else {
        "Not running · optional local LLM answers".to_string()
    };

    SetupItem {
        kind: SetupKind::Ollama,
        status: if info.reachable {
            SetupStatus::Ready
        } else {
            SetupStatus::Missing
        },
        detail,
        action: if info.reachable {
            SetupAction::None
        } else {
            SetupAction::OpenOllamaWebsite
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct FakeProbe {
        mic: MicrophoneAccess,
        gpu: GpuInfo,
    }

    impl SystemProbe for FakeProbe {
        fn microphone(&self) -> MicrophoneAccess {
            self.mic.clone()
        }

        fn gpu(&self) -> GpuInfo {
            self.gpu.clone()
        }
    }

    fn probe(consent: Consent, gpu: GpuInfo) -> FakeProbe {
        FakeProbe {
            mic: MicrophoneAccess {
                consent,
                detail: "test".into(),
            },
            gpu,
        }
    }

    #[test]
    fn essentials_survive_missing_models() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(dir.path());
        let report = SetupReport::run(&paths, &probe(Consent::Allowed, GpuInfo::default()));

        assert_eq!(report.essentials_total, 3);
        assert_eq!(report.essentials_ready, 1);
        assert!(!report.essentials_complete());
        assert_eq!(
            report.item(SetupKind::Microphone).unwrap().status,
            SetupStatus::Ready
        );
        assert_eq!(
            report.item(SetupKind::SpeechModel).unwrap().status,
            SetupStatus::Missing
        );
        assert_eq!(
            report.item(SetupKind::Gpu).unwrap().status,
            SetupStatus::Warning
        );
    }

    #[test]
    fn a_complete_install_reports_ready_essentials() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(dir.path());
        let release = "0123456789abcdef0123456789abcdef01234567-0123456789abcdef0123456789abcdef";
        let folder = paths.intent_dir.join("versions").join(release);
        fs::create_dir_all(&folder).unwrap();
        for file in models::INTENT_REQUIRED_FILES {
            fs::write(folder.join(file), b"x").unwrap();
        }
        let receipt = models::IntentReceipt {
            commit: release[..40].to_string(),
            version: "1.0.0".into(),
            directory: release.into(),
            manifest_sha256: "0".repeat(64),
            generation: "test".into(),
        };
        fs::write(
            paths.intent_dir.join("current.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        fs::write(models::whisper_model_path(&paths), vec![0_u8; 16]).unwrap();

        let report = SetupReport::run(
            &paths,
            &probe(
                Consent::Allowed,
                GpuInfo {
                    adapters: vec![GpuAdapter {
                        name: "RTX 4060".into(),
                        vram_mb: Some(8192),
                    }],
                    note: None,
                },
            ),
        );

        assert_eq!(
            report.item(SetupKind::IntentModel).unwrap().status,
            SetupStatus::Ready
        );
        assert_eq!(
            report.item(SetupKind::SpeechModel).unwrap().status,
            SetupStatus::Incomplete
        );
        assert_eq!(
            report.item(SetupKind::Gpu).unwrap().status,
            SetupStatus::Ready
        );
        assert!(report
            .item(SetupKind::Gpu)
            .unwrap()
            .detail
            .contains("Vulkan"));
    }

    #[test]
    fn updating_items_recomputes_the_essentials_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut report = SetupReport::run(
            &AppPaths::with_data_dir(dir.path()),
            &probe(Consent::Allowed, GpuInfo::default()),
        );
        assert_eq!(report.essentials_ready, 1);
        report.update_item(SetupKind::SpeechModel, SetupStatus::Ready, "installed");
        assert_eq!(report.essentials_ready, 2);
        report.update_item(SetupKind::SpeechModel, SetupStatus::Missing, "removed");
        assert_eq!(report.essentials_ready, 1);
    }

    #[test]
    fn denied_microphone_offers_the_settings_action() {
        let dir = tempfile::tempdir().unwrap();
        let report = SetupReport::run(
            &AppPaths::with_data_dir(dir.path()),
            &probe(Consent::Denied, GpuInfo::default()),
        );
        let mic = report.item(SetupKind::Microphone).unwrap();
        assert_eq!(mic.status, SetupStatus::Denied);
        assert_eq!(mic.action, SetupAction::OpenMicrophoneSettings);
    }
}
