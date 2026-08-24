use crate::{model::UpdateChannel, settings};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Storage::FileSystem::SYNCHRONIZE,
    System::Threading::{OpenProcess, WaitForSingleObject},
};

const EXECUTABLE_ASSET: &str = "RobloxMultiAccountLauncher.exe";
const MANIFEST_ASSET: &str = "update-manifest.json";
const USER_AGENT: &str = concat!(
    "RobloxMultiAccountLauncher/",
    env!("CARGO_PKG_VERSION"),
    " portable-updater"
);
const API_VERSION: &str = "2022-11-28";
const MAX_METADATA_BYTES: usize = 2 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 128 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone)]
pub struct UpdateConfig {
    pub owner: String,
    pub repository: String,
    pub public_verification_key: Option<String>,
}

impl UpdateConfig {
    pub fn compiled() -> Option<Self> {
        let owner = option_env!("RMAL_GITHUB_OWNER")?.trim();
        let repository = option_env!("RMAL_GITHUB_REPOSITORY")?.trim();
        if owner.is_empty() || repository.is_empty() {
            return None;
        }
        Some(Self {
            owner: owner.to_owned(),
            repository: repository.to_owned(),
            public_verification_key: option_env!("RMAL_UPDATE_PUBLIC_KEY")
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(str::to_owned),
        })
    }

    fn releases_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/releases?per_page=20",
            self.owner, self.repository
        )
    }

    fn manifest_authentication(&self) -> ManifestAuthentication {
        self.public_verification_key
            .as_ref()
            .map(|_| ManifestAuthentication::SignedManifestRequired)
            .unwrap_or(ManifestAuthentication::Sha256Only)
    }
}

#[derive(Debug, Clone)]
enum ManifestAuthentication {
    Sha256Only,
    SignedManifestRequired,
}

impl ManifestAuthentication {
    fn verify_metadata(&self, _manifest: &[u8]) -> Result<(), OperationError> {
        match self {
            Self::Sha256Only => Ok(()),
            Self::SignedManifestRequired => Err(OperationError::Message(
                "Signed-manifest verification is required by this build, but the verifier is not enabled. Update installation was refused."
                    .into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateState {
    NotConfigured,
    Idle,
    Checking,
    UpToDate,
    UpdateAvailable,
    Downloading,
    Verifying,
    ReadyToInstall,
    WaitingForSafeRestart,
    Installing,
    Failed,
}

impl UpdateState {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotConfigured => "Not configured",
            Self::Idle => "Idle",
            Self::Checking => "Checking",
            Self::UpToDate => "Up to date",
            Self::UpdateAvailable => "Update available",
            Self::Downloading => "Downloading",
            Self::Verifying => "Verifying",
            Self::ReadyToInstall => "Ready to install",
            Self::WaitingForSafeRestart => "Waiting for safe restart",
            Self::Installing => "Installing",
            Self::Failed => "Failed",
        }
    }

    pub fn busy(self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Downloading | Self::Verifying | Self::Installing
        )
    }
}

#[derive(Debug, Clone)]
pub struct ReleaseCandidate {
    pub version: Version,
    pub name: String,
    pub notes: String,
    pub release_url: String,
    executable_url: String,
    manifest_url: String,
}

#[derive(Debug, Clone)]
pub struct PreparedUpdate {
    pub version: Version,
    pub staged_path: PathBuf,
    pub expected_sha256: String,
}

#[derive(Debug, Clone)]
pub struct UpdateSnapshot {
    pub configured: bool,
    pub channel: UpdateChannel,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub latest_name: Option<String>,
    pub release_notes: Option<String>,
    pub release_url: Option<String>,
    pub last_checked: Option<SystemTime>,
    pub state: UpdateState,
    pub error: Option<String>,
    pub signing: &'static str,
}

#[derive(Debug)]
pub enum UpdateNotice {
    BackgroundFailure(String),
    ManualFailure(String),
    Checked { available: bool, manual: bool },
    DownloadReady(Version),
    Cancelled,
}

enum WorkerEvent {
    CheckFinished {
        manual: bool,
        result: Result<Option<ReleaseCandidate>, OperationError>,
    },
    Phase(UpdateState),
    DownloadFinished(Result<PreparedUpdate, OperationError>),
}

#[derive(Debug)]
enum OperationError {
    Cancelled,
    Message(String),
}

impl From<String> for OperationError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

pub struct UpdateManager {
    config: Option<UpdateConfig>,
    channel: UpdateChannel,
    state: UpdateState,
    candidate: Option<ReleaseCandidate>,
    prepared: Option<PreparedUpdate>,
    last_checked: Option<SystemTime>,
    error: Option<String>,
    tx: mpsc::Sender<WorkerEvent>,
    rx: mpsc::Receiver<WorkerEvent>,
    busy: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    update_root: PathBuf,
}

impl UpdateManager {
    pub fn new(update_root: PathBuf, channel: UpdateChannel) -> Self {
        let config = UpdateConfig::compiled();
        let (tx, rx) = mpsc::channel();
        let _ = fs::create_dir_all(&update_root);
        cleanup_stale_files(&update_root, Duration::from_secs(7 * 24 * 60 * 60));
        Self {
            state: if config.is_some() {
                UpdateState::Idle
            } else {
                UpdateState::NotConfigured
            },
            config,
            channel,
            candidate: None,
            prepared: None,
            last_checked: None,
            error: take_helper_error(&update_root),
            tx,
            rx,
            busy: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            update_root,
        }
    }

    pub fn set_channel(&mut self, channel: UpdateChannel) {
        if self.channel != channel {
            self.channel = channel;
            self.candidate = None;
            self.prepared = None;
            self.error = None;
            self.state = if self.config.is_some() {
                UpdateState::Idle
            } else {
                UpdateState::NotConfigured
            };
        }
    }

    pub fn snapshot(&self) -> UpdateSnapshot {
        UpdateSnapshot {
            configured: self.config.is_some(),
            channel: self.channel,
            current_version: env!("CARGO_PKG_VERSION").into(),
            latest_version: self.candidate.as_ref().map(|item| item.version.to_string()),
            latest_name: self.candidate.as_ref().map(|item| item.name.clone()),
            release_notes: self.candidate.as_ref().map(|item| item.notes.clone()),
            release_url: self.candidate.as_ref().map(|item| item.release_url.clone()),
            last_checked: self.last_checked,
            state: self.state,
            error: self.error.clone(),
            signing: if self
                .config
                .as_ref()
                .and_then(|config| config.public_verification_key.as_ref())
                .is_some()
            {
                "Public key configured; signed-manifest verifier not enabled"
            } else {
                "SHA-256 manifest required; signed manifests deferred"
            },
        }
    }

    pub fn start_check(&mut self, manual: bool) -> bool {
        let Some(config) = self.config.clone() else {
            self.state = UpdateState::NotConfigured;
            return false;
        };
        if self.busy.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.cancel.store(false, Ordering::Release);
        self.state = UpdateState::Checking;
        self.error = None;
        let sender = self.tx.clone();
        let cancel = self.cancel.clone();
        let channel = self.channel;
        thread::spawn(move || {
            let source = GithubSource::new();
            let result = check_for_update(
                &source,
                &config,
                channel,
                env!("CARGO_PKG_VERSION"),
                &cancel,
            );
            let _ = sender.send(WorkerEvent::CheckFinished { manual, result });
        });
        true
    }

    pub fn start_download(&mut self) -> bool {
        let Some(candidate) = self.candidate.clone() else {
            return false;
        };
        if self.busy.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.cancel.store(false, Ordering::Release);
        self.state = UpdateState::Downloading;
        self.error = None;
        let sender = self.tx.clone();
        let cancel = self.cancel.clone();
        let update_root = self.update_root.clone();
        let authentication = self
            .config
            .as_ref()
            .map(UpdateConfig::manifest_authentication)
            .unwrap_or(ManifestAuthentication::Sha256Only);
        thread::spawn(move || {
            let source = GithubSource::new();
            let result = download_and_verify(
                &source,
                &candidate,
                &update_root,
                &cancel,
                &authentication,
                || {
                    let _ = sender.send(WorkerEvent::Phase(UpdateState::Verifying));
                },
            );
            let _ = sender.send(WorkerEvent::DownloadFinished(result));
        });
        true
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub fn mark_waiting_for_safe_restart(&mut self) {
        if self.prepared.is_some() {
            self.state = UpdateState::WaitingForSafeRestart;
        }
    }

    pub fn mark_ready_if_safe(&mut self) {
        if self.state == UpdateState::WaitingForSafeRestart && self.prepared.is_some() {
            self.state = UpdateState::ReadyToInstall;
        }
    }

    pub fn poll(&mut self) -> Vec<UpdateNotice> {
        let mut notices = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            match event {
                WorkerEvent::Phase(state) => self.state = state,
                WorkerEvent::CheckFinished { manual, result } => {
                    self.busy.store(false, Ordering::Release);
                    self.last_checked = Some(SystemTime::now());
                    match result {
                        Ok(Some(candidate)) => {
                            self.state = UpdateState::UpdateAvailable;
                            self.candidate = Some(candidate);
                            self.error = None;
                            notices.push(UpdateNotice::Checked {
                                available: true,
                                manual,
                            });
                        }
                        Ok(None) => {
                            self.state = UpdateState::UpToDate;
                            self.candidate = None;
                            self.error = None;
                            notices.push(UpdateNotice::Checked {
                                available: false,
                                manual,
                            });
                        }
                        Err(OperationError::Cancelled) => {
                            self.state = UpdateState::Idle;
                            notices.push(UpdateNotice::Cancelled);
                        }
                        Err(OperationError::Message(message)) => {
                            self.state = UpdateState::Failed;
                            self.error = Some(message.clone());
                            notices.push(if manual {
                                UpdateNotice::ManualFailure(message)
                            } else {
                                UpdateNotice::BackgroundFailure(message)
                            });
                        }
                    }
                }
                WorkerEvent::DownloadFinished(result) => {
                    self.busy.store(false, Ordering::Release);
                    match result {
                        Ok(prepared) => {
                            self.state = UpdateState::ReadyToInstall;
                            self.error = None;
                            notices.push(UpdateNotice::DownloadReady(prepared.version.clone()));
                            self.prepared = Some(prepared);
                        }
                        Err(OperationError::Cancelled) => {
                            self.state = UpdateState::UpdateAvailable;
                            notices.push(UpdateNotice::Cancelled);
                        }
                        Err(OperationError::Message(message)) => {
                            self.state = UpdateState::Failed;
                            self.error = Some(message.clone());
                            notices.push(UpdateNotice::ManualFailure(message));
                        }
                    }
                }
            }
        }
        notices
    }

    pub fn start_install_helper(&mut self) -> Result<(), String> {
        let prepared = self
            .prepared
            .as_ref()
            .ok_or("No verified update is ready to install.")?;
        spawn_update_helper(prepared, &self.update_root)?;
        self.state = UpdateState::Installing;
        Ok(())
    }
}

trait ReleaseSource {
    fn get_bytes(
        &self,
        url: &str,
        limit: usize,
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>, OperationError>;

    fn download(
        &self,
        url: &str,
        destination: &Path,
        limit: u64,
        cancel: &AtomicBool,
    ) -> Result<(), OperationError>;
}

struct GithubSource {
    agent: ureq::Agent,
}

impl GithubSource {
    fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .https_only(true)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    fn request(&self, url: &str) -> Result<ureq::http::Response<ureq::Body>, OperationError> {
        self.agent
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|error| OperationError::Message(format!("HTTPS request failed: {error}")))
    }
}

impl ReleaseSource for GithubSource {
    fn get_bytes(
        &self,
        url: &str,
        limit: usize,
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>, OperationError> {
        if cancel.load(Ordering::Acquire) {
            return Err(OperationError::Cancelled);
        }
        let response = self.request(url)?;
        let mut reader = response
            .into_body()
            .into_with_config()
            .limit(limit as u64)
            .reader();
        let mut bytes = Vec::new();
        read_with_cancellation(&mut reader, &mut bytes, limit as u64, cancel)?;
        Ok(bytes)
    }

    fn download(
        &self,
        url: &str,
        destination: &Path,
        limit: u64,
        cancel: &AtomicBool,
    ) -> Result<(), OperationError> {
        let response = self.request(url)?;
        let mut reader = response.into_body().into_reader();
        let mut file = fs::File::create(destination)
            .map_err(|error| OperationError::Message(format!("Could not stage update: {error}")))?;
        let result = copy_with_cancellation(&mut reader, &mut file, limit, cancel);
        if result.is_ok() {
            file.sync_all().map_err(|error| {
                OperationError::Message(format!("Could not flush staged update: {error}"))
            })?;
        } else {
            let _ = fs::remove_file(destination);
        }
        result
    }
}

fn read_with_cancellation(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    limit: u64,
    cancel: &AtomicBool,
) -> Result<(), OperationError> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(OperationError::Cancelled);
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|error| OperationError::Message(format!("Download failed: {error}")))?;
        if count == 0 {
            return Ok(());
        }
        total += count as u64;
        if total > limit {
            return Err(OperationError::Message(
                "Downloaded data exceeded the safety size limit.".into(),
            ));
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

fn copy_with_cancellation(
    reader: &mut impl Read,
    writer: &mut impl Write,
    limit: u64,
    cancel: &AtomicBool,
) -> Result<(), OperationError> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(OperationError::Cancelled);
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|error| OperationError::Message(format!("Download failed: {error}")))?;
        if count == 0 {
            return Ok(());
        }
        total += count as u64;
        if total > limit {
            return Err(OperationError::Message(
                "Downloaded executable exceeded the safety size limit.".into(),
            ));
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|error| OperationError::Message(format!("Could not stage update: {error}")))?;
    }
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct UpdateManifest {
    version: String,
    filename: String,
    sha256: String,
}

fn check_for_update(
    source: &impl ReleaseSource,
    config: &UpdateConfig,
    channel: UpdateChannel,
    current: &str,
    cancel: &AtomicBool,
) -> Result<Option<ReleaseCandidate>, OperationError> {
    let bytes = source.get_bytes(&config.releases_url(), MAX_METADATA_BYTES, cancel)?;
    let releases: Vec<GithubRelease> = serde_json::from_slice(&bytes).map_err(|error| {
        OperationError::Message(format!("GitHub release metadata was malformed: {error}"))
    })?;
    select_release(releases, channel, current).map_err(OperationError::Message)
}

fn select_release(
    releases: Vec<GithubRelease>,
    channel: UpdateChannel,
    current: &str,
) -> Result<Option<ReleaseCandidate>, String> {
    let current = Version::parse(current)
        .map_err(|error| format!("Installed application version is invalid: {error}"))?;
    let mut eligible = releases
        .into_iter()
        .filter_map(|release| {
            let version = Version::parse(
                release
                    .tag_name
                    .strip_prefix(['v', 'V'])
                    .unwrap_or(&release.tag_name),
            )
            .ok()?;
            let allowed = !release.draft
                && match channel {
                    UpdateChannel::Stable => !release.prerelease && version.pre.is_empty(),
                    UpdateChannel::Prerelease => true,
                };
            allowed.then_some((version, release))
        })
        .collect::<Vec<_>>();
    eligible.sort_by(|left, right| right.0.cmp(&left.0));
    let Some((version, release)) = eligible.into_iter().next() else {
        return Ok(None);
    };
    if version <= current {
        return Ok(None);
    }
    let executable_url = release
        .assets
        .iter()
        .find(|asset| asset.name == EXECUTABLE_ASSET)
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| format!("Release v{version} does not contain {EXECUTABLE_ASSET}."))?;
    let manifest_url = release
        .assets
        .iter()
        .find(|asset| asset.name == MANIFEST_ASSET)
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| format!("Release v{version} does not contain {MANIFEST_ASSET}."))?;
    Ok(Some(ReleaseCandidate {
        version,
        name: release.name.unwrap_or_else(|| release.tag_name.clone()),
        notes: concise_notes(release.body.as_deref().unwrap_or("")),
        release_url: release.html_url,
        executable_url,
        manifest_url,
    }))
}

fn concise_notes(notes: &str) -> String {
    let mut value = notes.trim().chars().take(1_500).collect::<String>();
    if notes.chars().count() > 1_500 {
        value.push('…');
    }
    value
}

fn download_and_verify(
    source: &impl ReleaseSource,
    candidate: &ReleaseCandidate,
    update_root: &Path,
    cancel: &AtomicBool,
    authentication: &ManifestAuthentication,
    verifying: impl FnOnce(),
) -> Result<PreparedUpdate, OperationError> {
    fs::create_dir_all(update_root).map_err(|error| {
        OperationError::Message(format!("Could not create update storage: {error}"))
    })?;
    let manifest_bytes = source.get_bytes(&candidate.manifest_url, MAX_MANIFEST_BYTES, cancel)?;
    let manifest: UpdateManifest = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        OperationError::Message(format!("Update manifest was malformed: {error}"))
    })?;
    authentication.verify_metadata(&manifest_bytes)?;
    let manifest_version = Version::parse(manifest.version.trim_start_matches(['v', 'V']))
        .map_err(|error| {
            OperationError::Message(format!("Manifest version is invalid: {error}"))
        })?;
    if manifest_version != candidate.version {
        return Err(OperationError::Message(format!(
            "Manifest version {} does not match release version {}.",
            manifest_version, candidate.version
        )));
    }
    if manifest.filename != EXECUTABLE_ASSET {
        return Err(OperationError::Message(
            "Update manifest names an unexpected executable.".into(),
        ));
    }
    let expected = normalize_sha256(&manifest.sha256)?;
    let part = update_root.join(format!("{}-{}.part", EXECUTABLE_ASSET, candidate.version));
    let staged = update_root.join(format!(
        "{}-{}.verified.exe",
        EXECUTABLE_ASSET, candidate.version
    ));
    let manifest_path = update_root.join(format!("manifest-{}.json", candidate.version));
    let _ = fs::remove_file(&part);
    source.download(
        &candidate.executable_url,
        &part,
        MAX_EXECUTABLE_BYTES,
        cancel,
    )?;
    verifying();
    if cancel.load(Ordering::Acquire) {
        let _ = fs::remove_file(&part);
        return Err(OperationError::Cancelled);
    }
    let actual = sha256_file(&part).map_err(OperationError::Message)?;
    if actual != expected {
        let _ = fs::remove_file(&part);
        return Err(OperationError::Message(format!(
            "SECURITY ERROR: SHA-256 mismatch. Expected {expected}, calculated {actual}. The existing launcher was not changed."
        )));
    }
    let _ = fs::remove_file(&staged);
    fs::rename(&part, &staged).map_err(|error| {
        OperationError::Message(format!("Could not finalize verified update: {error}"))
    })?;
    fs::write(&manifest_path, manifest_bytes).map_err(|error| {
        OperationError::Message(format!("Could not retain verified manifest: {error}"))
    })?;
    Ok(PreparedUpdate {
        version: candidate.version.clone(),
        staged_path: staged,
        expected_sha256: expected,
    })
}

fn normalize_sha256(value: &str) -> Result<String, OperationError> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(OperationError::Message(
            "Update manifest contains an invalid SHA-256 value.".into(),
        ));
    }
    Ok(normalized)
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| format!("Could not hash file: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not hash file: {error}"))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn update_block_reason(protection_active: bool, client_count: usize) -> Option<String> {
    if protection_active || client_count > 0 {
        Some(format!(
            "Install after Roblox sessions are finished. Multi-account protection active: {}; Roblox clients: {}.",
            if protection_active { "yes" } else { "no" },
            client_count
        ))
    } else {
        None
    }
}

fn spawn_update_helper(prepared: &PreparedUpdate, update_root: &Path) -> Result<(), String> {
    let target = std::env::current_exe()
        .map_err(|error| format!("Could not determine the portable executable path: {error}"))?;
    fs::create_dir_all(update_root)
        .map_err(|error| format!("Could not create update storage: {error}"))?;
    let helper = update_root.join(format!("update-helper-{}.exe", std::process::id()));
    fs::copy(&target, &helper)
        .map_err(|error| format!("Could not create the temporary updater helper: {error}"))?;
    let mut command = Command::new(&helper);
    command
        .arg("--self-update-helper")
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--target")
        .arg(&target)
        .arg("--staged")
        .arg(&prepared.staged_path)
        .arg("--sha256")
        .arg(&prepared.expected_sha256)
        .arg("--update-root")
        .arg(update_root);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
        .spawn()
        .map_err(|error| format!("Could not start the temporary updater helper: {error}"))?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InternalMode {
    Helper {
        parent_pid: u32,
        target: PathBuf,
        staged: PathBuf,
        sha256: String,
        update_root: PathBuf,
    },
    Cleanup {
        helper_pid: u32,
        helper: PathBuf,
        staged: PathBuf,
        backup: PathBuf,
    },
}

pub fn parse_internal_mode(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Option<InternalMode>, String> {
    let args = arguments.into_iter().skip(1).collect::<Vec<_>>();
    let helper = args.iter().any(|arg| arg == "--self-update-helper");
    let cleanup = args.iter().any(|arg| arg == "--self-update-cleanup");
    if !helper && !cleanup {
        return Ok(None);
    }
    if helper && cleanup {
        return Err("Conflicting internal updater modes.".into());
    }
    let value = |name: &str| -> Result<OsString, String> {
        let index = args
            .iter()
            .position(|arg| arg == name)
            .ok_or_else(|| format!("Internal updater argument {name} is missing."))?;
        args.get(index + 1)
            .cloned()
            .ok_or_else(|| format!("Internal updater argument {name} has no value."))
    };
    let number = |name: &str| -> Result<u32, String> {
        value(name)?
            .to_string_lossy()
            .parse()
            .map_err(|_| format!("Internal updater argument {name} is invalid."))
    };
    if helper {
        let sha256 = value("--sha256")?.to_string_lossy().to_ascii_lowercase();
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Internal updater SHA-256 is invalid.".into());
        }
        Ok(Some(InternalMode::Helper {
            parent_pid: number("--parent-pid")?,
            target: PathBuf::from(value("--target")?),
            staged: PathBuf::from(value("--staged")?),
            sha256,
            update_root: PathBuf::from(value("--update-root")?),
        }))
    } else {
        Ok(Some(InternalMode::Cleanup {
            helper_pid: number("--helper-pid")?,
            helper: PathBuf::from(value("--helper")?),
            staged: PathBuf::from(value("--staged")?),
            backup: PathBuf::from(value("--backup")?),
        }))
    }
}

pub fn run_internal_mode(mode: InternalMode) -> bool {
    match mode {
        InternalMode::Helper {
            parent_pid,
            target,
            staged,
            sha256,
            update_root,
        } => {
            if let Err(message) = run_helper(parent_pid, &target, &staged, &sha256, &update_root) {
                write_helper_error(&update_root, &message);
                if target.is_file() {
                    let _ = hidden_command(&target).spawn();
                }
            }
            true
        }
        InternalMode::Cleanup {
            helper_pid,
            helper,
            staged,
            backup,
        } => {
            let _ = wait_for_process_exit(helper_pid, Duration::from_secs(15));
            for path in [&helper, &staged, &backup] {
                remove_file_retry(path, 30, Duration::from_millis(100));
            }
            false
        }
    }
}

fn run_helper(
    parent_pid: u32,
    target: &Path,
    staged: &Path,
    expected_sha256: &str,
    update_root: &Path,
) -> Result<(), String> {
    if !wait_for_process_exit(parent_pid, Duration::from_secs(120))? {
        return Err(
            "Timed out waiting for the original launcher to exit; update was not installed.".into(),
        );
    }
    let actual = sha256_file(staged)?;
    if actual != expected_sha256 {
        return Err("SECURITY ERROR: staged update failed SHA-256 re-verification; update was not installed.".into());
    }
    let backup = replace_portable_executable(target, staged)?;
    let helper = std::env::current_exe()
        .map_err(|error| format!("Could not determine updater helper path: {error}"))?;
    let mut command = hidden_command(target);
    command
        .arg("--self-update-cleanup")
        .arg("--helper-pid")
        .arg(std::process::id().to_string())
        .arg("--helper")
        .arg(&helper)
        .arg("--staged")
        .arg(staged)
        .arg("--backup")
        .arg(&backup);
    command.spawn().map_err(|error| {
        format!("Update installed, but the updated launcher could not restart: {error}")
    })?;
    let _ = fs::remove_file(update_root.join("helper-error.txt"));
    Ok(())
}

fn hidden_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn replace_portable_executable(target: &Path, staged: &Path) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or("Portable executable has no parent directory.")?;
    let suffix = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let replacement = parent.join(format!(".rmal-update-{suffix}.exe"));
    let backup = parent.join(format!(".rmal-backup-{suffix}.exe"));
    fs::copy(staged, &replacement).map_err(|error| {
        format!("Could not stage the update beside the portable executable: {error}")
    })?;
    fs::OpenOptions::new()
        .write(true)
        .open(&replacement)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("Could not flush the replacement executable: {error}"))?;
    rename_retry(target, &backup, 30, Duration::from_millis(100))?;
    if let Err(message) = rename_retry(&replacement, target, 30, Duration::from_millis(100)) {
        let rollback = rename_retry(&backup, target, 30, Duration::from_millis(100));
        let _ = fs::remove_file(&replacement);
        return match rollback {
            Ok(()) => Err(format!("{message} The original executable was restored.")),
            Err(rollback_error) => Err(format!(
                "{message} Automatic rollback also failed: {rollback_error}. Backup: {}",
                backup.display()
            )),
        };
    }
    Ok(backup)
}

fn rename_retry(from: &Path, to: &Path, attempts: usize, delay: Duration) -> Result<(), String> {
    let mut last_error = None;
    for _ in 0..attempts {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(delay);
            }
        }
    }
    Err(format!(
        "Could not replace {} with {}: {}",
        from.display(),
        to.display(),
        last_error.map_or_else(|| "unknown error".into(), |error| error.to_string())
    ))
}

#[cfg(windows)]
fn wait_for_process_exit(pid: u32, timeout: Duration) -> Result<bool, String> {
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return Ok(true);
    }
    let milliseconds = timeout.as_millis().min(u32::MAX as u128) as u32;
    let result = unsafe { WaitForSingleObject(handle, milliseconds) };
    unsafe { CloseHandle(handle) };
    match result {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        value => Err(format!(
            "Could not wait for process {pid} (Windows result {value})."
        )),
    }
}

#[cfg(not(windows))]
fn wait_for_process_exit(_pid: u32, _timeout: Duration) -> Result<bool, String> {
    Ok(true)
}

fn remove_file_retry(path: &Path, attempts: usize, delay: Duration) {
    for _ in 0..attempts {
        if !path.exists() || fs::remove_file(path).is_ok() {
            return;
        }
        thread::sleep(delay);
    }
}

fn cleanup_stale_files(root: &Path, minimum_age: Duration) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let owned = name.starts_with("RobloxMultiAccountLauncher.exe-")
            || name.starts_with("manifest-")
            || name.starts_with("update-helper-");
        let old_enough = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= minimum_age);
        if owned && old_enough && path.is_file() {
            let _ = fs::remove_file(path);
        }
    }
}

fn helper_error_path(root: &Path) -> PathBuf {
    root.join("helper-error.txt")
}

fn write_helper_error(root: &Path, message: &str) {
    let _ = fs::create_dir_all(root);
    let _ = fs::write(helper_error_path(root), message);
}

fn take_helper_error(root: &Path) -> Option<String> {
    let path = helper_error_path(root);
    let value = fs::read_to_string(&path).ok();
    let _ = fs::remove_file(path);
    value
}

pub fn record_helper_error(message: &str) {
    let root = settings::data_root().join("Updates");
    write_helper_error(&root, message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MockSource {
        metadata: Result<Vec<u8>, String>,
        manifest: Option<Vec<u8>>,
        executable: Option<Vec<u8>>,
        calls: Mutex<Vec<String>>,
    }

    impl MockSource {
        fn releases(json: &str) -> Self {
            Self {
                metadata: Ok(json.as_bytes().to_vec()),
                manifest: None,
                executable: None,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl ReleaseSource for MockSource {
        fn get_bytes(
            &self,
            url: &str,
            _limit: usize,
            cancel: &AtomicBool,
        ) -> Result<Vec<u8>, OperationError> {
            if cancel.load(Ordering::Acquire) {
                return Err(OperationError::Cancelled);
            }
            self.calls.lock().unwrap().push(url.into());
            if url.ends_with("manifest") {
                self.manifest
                    .clone()
                    .ok_or_else(|| OperationError::Message("fixture manifest missing".into()))
            } else {
                self.metadata.clone().map_err(OperationError::Message)
            }
        }

        fn download(
            &self,
            url: &str,
            destination: &Path,
            _limit: u64,
            cancel: &AtomicBool,
        ) -> Result<(), OperationError> {
            if cancel.load(Ordering::Acquire) {
                return Err(OperationError::Cancelled);
            }
            self.calls.lock().unwrap().push(url.into());
            fs::write(destination, self.executable.as_deref().unwrap_or_default())
                .map_err(|error| OperationError::Message(error.to_string()))
        }
    }

    fn config() -> UpdateConfig {
        UpdateConfig {
            owner: "fixture-owner".into(),
            repository: "fixture-repository".into(),
            public_verification_key: None,
        }
    }

    fn release(version: &str, prerelease: bool, assets: bool) -> String {
        let assets = if assets {
            format!(
                r#"[{{"name":"{EXECUTABLE_ASSET}","browser_download_url":"https://fixture/exe"}},{{"name":"{MANIFEST_ASSET}","browser_download_url":"https://fixture/manifest"}}]"#
            )
        } else {
            "[]".into()
        };
        format!(
            r#"{{"tag_name":"v{version}","name":"Release {version}","body":"Notes","html_url":"https://fixture/release","draft":false,"prerelease":{prerelease},"assets":{assets}}}"#
        )
    }

    #[test]
    fn stable_and_prerelease_filtering_use_semver() {
        let json = format!(
            "[{},{}]",
            release("4.0.0-beta.1", true, true),
            release("3.1.0", false, true)
        );
        let releases: Vec<GithubRelease> = serde_json::from_str(&json).unwrap();
        let stable = select_release(releases, UpdateChannel::Stable, "3.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(stable.version, Version::parse("3.1.0").unwrap());
        let releases: Vec<GithubRelease> = serde_json::from_str(&json).unwrap();
        let prerelease = select_release(releases, UpdateChannel::Prerelease, "3.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(prerelease.version, Version::parse("4.0.0-beta.1").unwrap());
    }

    #[test]
    fn detects_update_and_no_update() {
        let source = MockSource::releases(&format!("[{}]", release("3.1.0", false, true)));
        let cancel = AtomicBool::new(false);
        assert!(
            check_for_update(&source, &config(), UpdateChannel::Stable, "3.0.0", &cancel)
                .unwrap()
                .is_some()
        );
        assert!(
            check_for_update(&source, &config(), UpdateChannel::Stable, "3.1.0", &cancel)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn malformed_metadata_network_failure_and_missing_asset_fail_safely() {
        let cancel = AtomicBool::new(false);
        let malformed = MockSource::releases("not-json");
        assert!(
            check_for_update(
                &malformed,
                &config(),
                UpdateChannel::Stable,
                "3.0.0",
                &cancel
            )
            .is_err()
        );
        let network = MockSource {
            metadata: Err("offline".into()),
            manifest: None,
            executable: None,
            calls: Mutex::new(Vec::new()),
        };
        assert!(
            check_for_update(&network, &config(), UpdateChannel::Stable, "3.0.0", &cancel).is_err()
        );
        let missing = MockSource::releases(&format!("[{}]", release("3.1.0", false, false)));
        assert!(
            check_for_update(&missing, &config(), UpdateChannel::Stable, "3.0.0", &cancel).is_err()
        );
    }

    #[test]
    fn sha256_verification_accepts_match_and_rejects_mismatch() {
        let root = PathBuf::from(".tmp/tests/updater-hash");
        let _ = fs::remove_dir_all(&root);
        let bytes = b"fixture executable".to_vec();
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let source = MockSource {
            metadata: Ok(Vec::new()),
            manifest: Some(
                format!(
                    r#"{{"version":"3.1.0","filename":"{EXECUTABLE_ASSET}","sha256":"{digest}"}}"#
                )
                .into_bytes(),
            ),
            executable: Some(bytes.clone()),
            calls: Mutex::new(Vec::new()),
        };
        let releases: Vec<GithubRelease> =
            serde_json::from_str(&format!("[{}]", release("3.1.0", false, true))).unwrap();
        let candidate = select_release(releases, UpdateChannel::Stable, "3.0.0")
            .unwrap()
            .unwrap();
        let prepared = download_and_verify(
            &source,
            &candidate,
            &root,
            &AtomicBool::new(false),
            &ManifestAuthentication::Sha256Only,
            || {},
        )
        .unwrap();
        assert!(prepared.staged_path.is_file());
        let mismatch = MockSource {
            metadata: Ok(Vec::new()),
            manifest: Some(
                format!(
                    r#"{{"version":"3.1.0","filename":"{EXECUTABLE_ASSET}","sha256":"{}"}}"#,
                    "0".repeat(64)
                )
                .into_bytes(),
            ),
            executable: Some(bytes),
            calls: Mutex::new(Vec::new()),
        };
        let error = download_and_verify(
            &mismatch,
            &candidate,
            &root,
            &AtomicBool::new(false),
            &ManifestAuthentication::Sha256Only,
            || {},
        )
        .unwrap_err();
        assert!(
            matches!(error, OperationError::Message(message) if message.contains("SECURITY ERROR"))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn configured_signing_policy_fails_closed_until_verifier_exists() {
        let policy = ManifestAuthentication::SignedManifestRequired;
        assert!(matches!(
            policy.verify_metadata(br#"{"version":"3.1.0"}"#),
            Err(OperationError::Message(message)) if message.contains("installation was refused")
        ));
    }

    #[test]
    fn cancellation_and_protection_blocking_are_explicit() {
        let source = MockSource::releases("[]");
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            check_for_update(&source, &config(), UpdateChannel::Stable, "3.0.0", &cancel),
            Err(OperationError::Cancelled)
        ));
        assert!(update_block_reason(true, 0).is_some());
        assert!(update_block_reason(false, 2).is_some());
        assert!(update_block_reason(false, 0).is_none());
    }

    #[test]
    fn helper_arguments_parse_without_lossy_paths() {
        let hash = "a".repeat(64);
        let args = vec![
            OsString::from("app.exe"),
            OsString::from("--self-update-helper"),
            OsString::from("--parent-pid"),
            OsString::from("123"),
            OsString::from("--target"),
            OsString::from(r"E:\Random Tools\Launcher.exe"),
            OsString::from("--staged"),
            OsString::from(r"C:\Updates\new.exe"),
            OsString::from("--sha256"),
            OsString::from(hash.clone()),
            OsString::from("--update-root"),
            OsString::from(r"C:\Updates"),
        ];
        let mode = parse_internal_mode(args).unwrap().unwrap();
        assert!(
            matches!(mode, InternalMode::Helper { parent_pid: 123, sha256, .. } if sha256 == hash)
        );
    }

    #[test]
    fn replacement_logic_uses_dummy_files_and_cleanup_is_scoped() {
        let root = PathBuf::from(".tmp/tests/updater-replace");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let target = root.join("portable.exe");
        let staged = root.join("verified.exe");
        let unrelated = root.join("keep.txt");
        fs::write(&target, b"old").unwrap();
        fs::write(&staged, b"new").unwrap();
        fs::write(&unrelated, b"keep").unwrap();
        let backup = replace_portable_executable(&target, &staged).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(fs::read(&backup).unwrap(), b"old");
        cleanup_stale_files(&root, Duration::ZERO);
        assert!(unrelated.is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn updater_is_not_configured_without_real_coordinates() {
        if option_env!("RMAL_GITHUB_OWNER").is_none()
            || option_env!("RMAL_GITHUB_REPOSITORY").is_none()
        {
            assert!(UpdateConfig::compiled().is_none());
            let manager = UpdateManager::new(
                PathBuf::from(".tmp/tests/updater-not-configured"),
                UpdateChannel::Stable,
            );
            assert_eq!(manager.snapshot().state, UpdateState::NotConfigured);
        }
    }

    #[test]
    fn update_state_transitions_do_not_overlap_or_skip_verification() {
        let root = PathBuf::from(".tmp/tests/updater-state");
        let _ = fs::remove_dir_all(&root);
        let mut manager = UpdateManager::new(root.clone(), UpdateChannel::Stable);
        manager.config = Some(config());
        manager.state = UpdateState::Checking;
        manager.busy.store(true, Ordering::Release);
        let releases: Vec<GithubRelease> =
            serde_json::from_str(&format!("[{}]", release("3.1.0", false, true))).unwrap();
        let candidate = select_release(releases, UpdateChannel::Stable, "3.0.0")
            .unwrap()
            .unwrap();
        manager
            .tx
            .send(WorkerEvent::CheckFinished {
                manual: false,
                result: Ok(Some(candidate)),
            })
            .unwrap();
        manager.poll();
        assert_eq!(manager.state, UpdateState::UpdateAvailable);
        assert!(!manager.busy.load(Ordering::Acquire));

        manager.busy.store(true, Ordering::Release);
        manager.state = UpdateState::Downloading;
        manager
            .tx
            .send(WorkerEvent::Phase(UpdateState::Verifying))
            .unwrap();
        manager.poll();
        assert_eq!(manager.state, UpdateState::Verifying);
        manager
            .tx
            .send(WorkerEvent::DownloadFinished(Err(
                OperationError::Cancelled,
            )))
            .unwrap();
        manager.poll();
        assert_eq!(manager.state, UpdateState::UpdateAvailable);
        assert!(!manager.busy.load(Ordering::Acquire));
        let _ = fs::remove_dir_all(root);
    }
}
