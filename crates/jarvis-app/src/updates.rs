use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use jarvis_core::logging;
use jarvis_core::paths::AppPaths;
use jarvis_core::settings::{AppSettings, UpdateChannel};
use jarvis_core::update::{self as update, Release, ReleaseAsset, UpdateClient};
use jarvis_platform::SignatureStatus;
use jarvis_platform::Wake;
use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdatePhase {
    Idle,
    Checking,
    Available,
    UpToDate,
    Downloading,
    Verifying,
    ReadyToInstall,
    Installing,
    Error,
}

impl UpdatePhase {
    pub fn is_busy(self) -> bool {
        matches!(
            self,
            UpdatePhase::Checking
                | UpdatePhase::Downloading
                | UpdatePhase::Verifying
                | UpdatePhase::Installing
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            UpdatePhase::Idle => "Not checked yet",
            UpdatePhase::Checking => "Checking for updates",
            UpdatePhase::Available => "Update available",
            UpdatePhase::UpToDate => "Up to date",
            UpdatePhase::Downloading => "Downloading",
            UpdatePhase::Verifying => "Verifying signature",
            UpdatePhase::ReadyToInstall => "Ready to install",
            UpdatePhase::Installing => "Installing",
            UpdatePhase::Error => "Update failed",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DownloadState {
    pub downloaded: u64,
    pub total: u64,
}

impl DownloadState {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.downloaded as f32 / self.total as f32).clamp(0.0, 1.0)
    }

    pub fn label(&self) -> String {
        if self.total == 0 {
            return human_bytes(self.downloaded);
        }
        format!(
            "{} of {}",
            human_bytes(self.downloaded),
            human_bytes(self.total)
        )
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let bytes = bytes as f64;
    if bytes < KIB {
        format!("{bytes:.0} B")
    } else if bytes < KIB * KIB {
        format!("{:.1} KB", bytes / KIB)
    } else {
        format!("{:.1} MB", bytes / (KIB * KIB))
    }
}

#[derive(Clone, Debug)]
pub enum UpdateEvent {
    Checking,
    Latest(Option<Release>),
    Downloaded(DownloadState),
    Verifying,
    Verified {
        path: PathBuf,
        signature: SignatureStatus,
    },
    InstallStarted,
    Failed(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateReceipt {
    pub version: String,
    pub checked_at: String,
}

pub struct UpdatesUi {
    pub phase: UpdatePhase,
    pub current: Version,
    pub latest: Option<Release>,
    pub download: DownloadState,
    pub verified: Option<SignatureStatus>,
    pub installer: Option<PathBuf>,
    pub error: Option<String>,
    pub last_checked: Option<String>,
    pub signature_required: bool,
}

impl UpdatesUi {
    pub fn new(current: Version) -> Self {
        Self {
            phase: UpdatePhase::Idle,
            current,
            latest: None,
            download: DownloadState::default(),
            verified: None,
            installer: None,
            error: None,
            last_checked: None,
            signature_required: !update::allow_unsigned(),
        }
    }

    pub fn latest_version(&self) -> Option<&Version> {
        self.latest.as_ref().map(|release| &release.version)
    }

    pub fn asset(&self) -> Option<&ReleaseAsset> {
        self.latest.as_ref().and_then(Release::preferred_asset)
    }

    pub fn is_newer(&self) -> bool {
        self.latest_version()
            .is_some_and(|version| update::is_newer(version, &self.current))
    }

    pub fn can_download(&self) -> bool {
        matches!(self.phase, UpdatePhase::Available) && self.asset().is_some()
    }

    pub fn can_install(&self) -> bool {
        matches!(self.phase, UpdatePhase::ReadyToInstall) && self.installer.is_some()
    }

    pub fn trusted(&self) -> bool {
        self.verified
            .as_ref()
            .is_some_and(|signature| signature.trusted)
    }

    fn reset_download(&mut self) {
        self.download = DownloadState::default();
        self.verified = None;
        self.installer = None;
    }

    pub fn apply(&mut self, event: UpdateEvent, channel: UpdateChannel, current: &Version) -> bool {
        match event {
            UpdateEvent::Checking => {
                if self.phase.is_busy() {
                    return false;
                }
                self.phase = UpdatePhase::Checking;
                self.error = None;
                true
            }
            UpdateEvent::Latest(release) => {
                self.reset_download();
                self.error = None;
                let eligible = release
                    .as_ref()
                    .filter(|release| channel.accepts(&release.version))
                    .filter(|release| update::is_newer(&release.version, current))
                    .cloned();
                self.phase = if eligible.is_some() {
                    UpdatePhase::Available
                } else {
                    UpdatePhase::UpToDate
                };
                self.latest = eligible;
                true
            }
            UpdateEvent::Downloaded(download) => {
                if matches!(
                    self.phase,
                    UpdatePhase::Available | UpdatePhase::Downloading | UpdatePhase::Verifying
                ) {
                    self.phase = UpdatePhase::Downloading;
                    self.download = download;
                    true
                } else {
                    false
                }
            }
            UpdateEvent::Verifying => {
                self.phase = UpdatePhase::Verifying;
                true
            }
            UpdateEvent::Verified { path, signature } => {
                self.installer = Some(path);
                self.verified = Some(signature);
                self.download.downloaded = self.download.total;
                self.phase = UpdatePhase::ReadyToInstall;
                true
            }
            UpdateEvent::InstallStarted => {
                self.phase = UpdatePhase::Installing;
                true
            }
            UpdateEvent::Failed(error) => {
                self.error = Some(error);
                self.phase = UpdatePhase::Error;
                self.reset_download();
                true
            }
        }
    }
}

/// Background check and download worker. Progress events reuse the app's waker so
/// the UI redraws from the same event loop as the model and speech workers.
pub struct UpdateService {
    paths: AppPaths,
    tx: Sender<UpdateEvent>,
    rx: Receiver<UpdateEvent>,
}

impl UpdateService {
    pub fn new(paths: AppPaths) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { paths, tx, rx }
    }

    pub fn check(&self, waker: Wake) {
        let tx = self.tx.clone();
        let log_file = self.paths.log_file.clone();
        thread::spawn(move || {
            let _ = tx.send(UpdateEvent::Checking);
            waker();
            let result = UpdateClient::new().and_then(|client| client.latest());
            logging::info(
                &log_file,
                &format!(
                    "update check · {}",
                    match &result {
                        Ok(release) => format!("latest {}", release.tag),
                        Err(error) => format!("failed: {error}"),
                    }
                ),
            );
            let event = match result {
                Ok(release) => UpdateEvent::Latest(Some(release)),
                Err(error) => UpdateEvent::Failed(error.to_string()),
            };
            let _ = tx.send(event);
            waker();
        });
    }

    /// Download, checksum and signature verification all run on this thread. The
    /// installer is only handed to the UI after its Authenticode status is known.
    pub fn download(&self, release: Release, asset: ReleaseAsset, waker: Wake) {
        let tx = self.tx.clone();
        let directory = updates_dir(&self.paths);
        let log_file = self.paths.log_file.clone();
        let pin = update::publisher_pin();
        let allow_unsigned = update::allow_unsigned();
        thread::spawn(move || {
            let outcome = UpdateClient::new().and_then(|client| {
                let path = client.download(&asset, &directory, |downloaded, total| {
                    let _ = tx.send(UpdateEvent::Downloaded(DownloadState { downloaded, total }));
                })?;
                let _ = tx.send(UpdateEvent::Verifying);
                let signature = jarvis_platform::verify_signature(&path);
                logging::info(
                    &log_file,
                    &format!(
                        "update download · {} {} · {}",
                        release.tag, asset.name, signature.detail
                    ),
                );

                if !signature.trusted && !allow_unsigned {
                    return Err(update::UpdateError::Invalid(format!(
                        "{}. Unsigned update artifacts are refused.",
                        signature.detail
                    )));
                }
                if let Some(pin) = &pin {
                    if !update::publisher_matches(signature.publisher.as_deref(), pin) {
                        return Err(update::UpdateError::Invalid(format!(
                            "the signer is not the pinned publisher ({pin})."
                        )));
                    }
                }
                let _ = tx.send(UpdateEvent::Verified { path, signature });
                Ok(())
            });
            if let Err(error) = outcome {
                let _ = tx.send(UpdateEvent::Failed(error.to_string()));
            }
            waker();
        });
    }

    pub fn drain(&self) -> Vec<UpdateEvent> {
        self.rx.try_iter().collect()
    }
}

pub fn updates_dir(paths: &AppPaths) -> PathBuf {
    paths.data_dir.join("updates")
}

pub fn receipt_path(paths: &AppPaths) -> PathBuf {
    paths.data_dir.join("update-check.json")
}

pub fn load_receipt(paths: &AppPaths) -> Option<UpdateReceipt> {
    std::fs::read(receipt_path(paths))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

pub fn save_receipt(paths: &AppPaths, receipt: &UpdateReceipt) {
    let path = receipt_path(paths);
    if let Ok(bytes) = serde_json::to_vec_pretty(receipt) {
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(&temporary, &path);
        }
    }
}

pub fn should_check(settings: &AppSettings, paths: &AppPaths) -> bool {
    if !settings.check_for_updates {
        return false;
    }
    let Some(receipt) = load_receipt(paths) else {
        return true;
    };
    let Ok(checked) = chrono::DateTime::parse_from_rfc3339(&receipt.checked_at) else {
        return true;
    };
    chrono::Utc::now()
        .signed_duration_since(checked.with_timezone(&chrono::Utc))
        .num_hours()
        >= 24
}

#[cfg(test)]
fn preferred_kind(release: &Release) -> Option<jarvis_core::ArtifactKind> {
    release.preferred_asset().and_then(ReleaseAsset::kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jarvis_core::ArtifactKind;

    fn release(tag: &str, name: &str) -> Release {
        Release {
            version: update::parse_version(tag).unwrap(),
            tag: tag.into(),
            notes: String::new(),
            notes_url: String::new(),
            published_at: String::new(),
            assets: vec![ReleaseAsset {
                name: name.into(),
                url: "https://example.test/asset".into(),
                bytes: 1024,
                sha256: Some("a".repeat(64)),
            }],
        }
    }

    fn apply(ui: &mut UpdatesUi, event: UpdateEvent, channel: UpdateChannel) -> bool {
        let current = ui.current.clone();
        ui.apply(event, channel, &current)
    }

    #[test]
    fn a_newer_stable_release_becomes_available() {
        let mut ui = UpdatesUi::new(Version::new(0, 1, 0));
        assert!(apply(
            &mut ui,
            UpdateEvent::Latest(Some(release("v0.2.0", "Jarvis-0.2.0-x64-setup.exe"))),
            UpdateChannel::Stable
        ));
        assert_eq!(ui.phase, UpdatePhase::Available);
        assert!(ui.is_newer());
        assert!(ui.can_download());
        assert!(!ui.can_install());
    }

    #[test]
    fn an_older_or_equal_release_is_up_to_date() {
        let mut ui = UpdatesUi::new(Version::new(0, 2, 0));
        apply(
            &mut ui,
            UpdateEvent::Latest(Some(release("v0.2.0", "Jarvis-0.2.0-x64-setup.exe"))),
            UpdateChannel::Stable,
        );
        assert_eq!(ui.phase, UpdatePhase::UpToDate);
        assert!(!ui.is_newer());

        apply(
            &mut ui,
            UpdateEvent::Latest(Some(release("v0.1.9", "Jarvis-0.1.9-x64-setup.exe"))),
            UpdateChannel::Stable,
        );
        assert_eq!(ui.phase, UpdatePhase::UpToDate);
    }

    #[test]
    fn the_stable_channel_ignores_prereleases() {
        let mut ui = UpdatesUi::new(Version::new(0, 1, 0));
        apply(
            &mut ui,
            UpdateEvent::Latest(Some(release("v0.2.0-beta.1", "Jarvis-0.2.0-x64-setup.exe"))),
            UpdateChannel::Stable,
        );
        assert_eq!(ui.phase, UpdatePhase::UpToDate);
        assert!(ui.latest.is_none());

        apply(
            &mut ui,
            UpdateEvent::Latest(Some(release("v0.2.0-beta.1", "Jarvis-0.2.0-x64-setup.exe"))),
            UpdateChannel::Prerelease,
        );
        assert_eq!(ui.phase, UpdatePhase::Available);
        assert!(ui.is_newer());
    }

    #[test]
    fn a_verified_download_becomes_installable() {
        let mut ui = UpdatesUi::new(Version::new(0, 1, 0));
        apply(
            &mut ui,
            UpdateEvent::Latest(Some(release("v0.2.0", "Jarvis-0.2.0-x64-setup.exe"))),
            UpdateChannel::Stable,
        );
        apply(
            &mut ui,
            UpdateEvent::Downloaded(DownloadState {
                downloaded: 512,
                total: 1024,
            }),
            UpdateChannel::Stable,
        );
        assert_eq!(ui.phase, UpdatePhase::Downloading);
        assert_eq!(ui.download.fraction(), 0.5);

        apply(
            &mut ui,
            UpdateEvent::Verified {
                path: PathBuf::from("/tmp/Jarvis-0.2.0-x64-setup.exe"),
                signature: SignatureStatus {
                    trusted: true,
                    publisher: Some("Jarvis contributors".into()),
                    detail: "Signed by Jarvis contributors.".into(),
                },
            },
            UpdateChannel::Stable,
        );
        assert_eq!(ui.phase, UpdatePhase::ReadyToInstall);
        assert!(ui.can_install());
        assert!(ui.trusted());
        assert_eq!(ui.download.downloaded, 1024);
    }

    #[test]
    fn a_failure_clears_partial_progress() {
        let mut ui = UpdatesUi::new(Version::new(0, 1, 0));
        apply(
            &mut ui,
            UpdateEvent::Latest(Some(release("v0.2.0", "Jarvis-0.2.0-x64-setup.exe"))),
            UpdateChannel::Stable,
        );
        apply(
            &mut ui,
            UpdateEvent::Downloaded(DownloadState {
                downloaded: 1,
                total: 2,
            }),
            UpdateChannel::Stable,
        );
        apply(
            &mut ui,
            UpdateEvent::Failed("checksum verification failed".into()),
            UpdateChannel::Stable,
        );
        assert_eq!(ui.phase, UpdatePhase::Error);
        assert_eq!(ui.download.downloaded, 0);
        assert!(ui.error.is_some());
        assert!(!ui.can_install());
    }

    #[test]
    fn a_check_is_refused_while_busy() {
        let mut ui = UpdatesUi::new(Version::new(0, 1, 0));
        assert!(apply(&mut ui, UpdateEvent::Checking, UpdateChannel::Stable));
        assert!(!apply(
            &mut ui,
            UpdateEvent::Checking,
            UpdateChannel::Stable
        ));
    }

    #[test]
    fn stale_download_events_are_ignored_after_a_refresh() {
        let mut ui = UpdatesUi::new(Version::new(0, 1, 0));
        apply(
            &mut ui,
            UpdateEvent::Latest(Some(release("v0.2.0", "Jarvis-0.2.0-x64-setup.exe"))),
            UpdateChannel::Stable,
        );
        apply(&mut ui, UpdateEvent::Checking, UpdateChannel::Stable);
        assert!(!apply(
            &mut ui,
            UpdateEvent::Downloaded(DownloadState {
                downloaded: 5,
                total: 10,
            }),
            UpdateChannel::Stable,
        ));
    }

    #[test]
    fn the_preferred_asset_prefers_the_installer() {
        let mut release = release("v0.2.0", "Jarvis-0.2.0-x64-portable.zip");
        release.assets.push(ReleaseAsset {
            name: "Jarvis-0.2.0-x64.msix".into(),
            url: "https://example.test/msix".into(),
            bytes: 1,
            sha256: Some("b".repeat(64)),
        });
        release.assets.push(ReleaseAsset {
            name: "Jarvis-0.2.0-x64-setup.exe".into(),
            url: "https://example.test/setup".into(),
            bytes: 1,
            sha256: Some("c".repeat(64)),
        });
        assert_eq!(preferred_kind(&release), Some(ArtifactKind::Installer));
    }

    #[test]
    fn a_fresh_install_checks_and_the_receipt_gates_later_checks() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_data_dir(dir.path());
        let settings = AppSettings::default();
        assert!(should_check(&settings, &paths));

        save_receipt(
            &paths,
            &UpdateReceipt {
                version: "0.1.0".into(),
                checked_at: chrono::Utc::now().to_rfc3339(),
            },
        );
        assert!(!should_check(&settings, &paths));

        let mut off = settings;
        off.check_for_updates = false;
        assert!(!should_check(&off, &paths));
    }
}
