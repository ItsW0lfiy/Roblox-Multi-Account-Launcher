use crate::{
    instance_paths::IsolatedLaunch,
    model::{Detection, LaunchBackend},
    platform::{self, ProtectionHealth},
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const STABLE_CLIENT_WINDOW: Duration = Duration::from_secs(2);

pub fn should_use_isolated_path(protection_active: bool, existing_clients: usize) -> bool {
    protection_active && existing_clients > 0
}

#[derive(Debug, Clone)]
pub enum LaunchEvent {
    Starting(String),
    Waiting(Option<u32>),
    Progress(String),
    Complete {
        pid: u32,
        backend: String,
        detail: String,
        warning: bool,
        isolation: Option<IsolatedLaunch>,
    },
    Failed {
        message: String,
        isolation: Option<IsolatedLaunch>,
    },
}

#[derive(Default)]
struct LaunchTracker {
    before: HashSet<u32>,
    first_seen: HashMap<u32, Duration>,
    exited_new: HashSet<u32>,
    closed_existing: HashSet<u32>,
    current_new: HashSet<u32>,
}

impl LaunchTracker {
    fn new(before: HashSet<u32>) -> Self {
        Self {
            before,
            ..Default::default()
        }
    }

    fn observe(&mut self, current: &HashSet<u32>, elapsed: Duration) -> Option<u32> {
        for pid in &self.before {
            if !current.contains(pid) {
                self.closed_existing.insert(*pid);
            }
        }
        for pid in self.first_seen.keys() {
            if !current.contains(pid) {
                self.exited_new.insert(*pid);
            }
        }
        for pid in current.difference(&self.before) {
            self.first_seen.entry(*pid).or_insert(elapsed);
        }
        self.current_new = current.difference(&self.before).copied().collect();
        current
            .difference(&self.before)
            .filter(|pid| {
                self.first_seen
                    .get(pid)
                    .is_some_and(|seen| elapsed.saturating_sub(*seen) >= STABLE_CLIENT_WINDOW)
            })
            .copied()
            .min()
    }

    fn success_detail(&self, backend_pid: Option<u32>, bootstrap_seen: &HashSet<String>) -> String {
        let mut details = Vec::new();
        if let Some(pid) = backend_pid {
            details.push(format!("Backend process PID {pid} started."));
        } else {
            details.push("Windows accepted the backend launch request.".into());
        }
        if !self.exited_new.is_empty() {
            details.push(format!(
                "Roblox bootstrap replacement observed after transient PID(s): {}.",
                sorted_pids(&self.exited_new)
            ));
        }
        if !bootstrap_seen.is_empty() {
            let mut names: Vec<_> = bootstrap_seen.iter().cloned().collect();
            names.sort();
            details.push(format!(
                "Installer/update process observed: {}.",
                names.join(", ")
            ));
        }
        if !self.closed_existing.is_empty() {
            details.push(format!(
                "WARNING: existing Roblox PID(s) closed during launch: {}.",
                sorted_pids(&self.closed_existing)
            ));
        }
        details.join(" ")
    }

    fn failure_detail(
        &self,
        backend: &str,
        timeout: Duration,
        bootstrap_seen: &HashSet<String>,
    ) -> String {
        let mut message = if self.first_seen.is_empty() {
            format!(
                "{backend} started, but no new RobloxPlayerBeta.exe PID appeared within {} seconds.",
                timeout.as_secs()
            )
        } else if self.current_new.is_empty() {
            format!(
                "{backend} started, but new Roblox PID(s) {} appeared and exited before remaining stable for {} seconds.",
                sorted_pids(&self.exited_new),
                STABLE_CLIENT_WINDOW.as_secs()
            )
        } else {
            format!(
                "{backend} started and new Roblox PID(s) {} appeared, but none remained alive for the {}-second stability window before the {}-second timeout.",
                sorted_pids(&self.current_new),
                STABLE_CLIENT_WINDOW.as_secs(),
                timeout.as_secs()
            )
        };
        if !bootstrap_seen.is_empty() {
            let mut names: Vec<_> = bootstrap_seen.iter().cloned().collect();
            names.sort();
            message.push_str(&format!(
                " A Roblox installer/update flow was observed ({}).",
                names.join(", ")
            ));
        } else if !self.exited_new.is_empty() {
            message.push_str(" The Roblox bootstrap may have replaced or rejected the process.");
        }
        if !self.closed_existing.is_empty() {
            message.push_str(&format!(
                " Existing Roblox PID(s) closed during this launch: {}.",
                sorted_pids(&self.closed_existing)
            ));
        }
        message
    }
}

fn sorted_pids(values: &HashSet<u32>) -> String {
    let mut values: Vec<_> = values.iter().copied().collect();
    values.sort_unstable();
    values
        .into_iter()
        .map(|pid| pid.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn verify_protection(health: &ProtectionHealth) -> Result<(), String> {
    let snapshot = health.snapshot();
    if snapshot.singleton_mutex && snapshot.singleton_event {
        Ok(())
    } else {
        Err(format!(
            "Multi-instance protection is incomplete (singletonMutex: {}, singletonEvent: {}).",
            if snapshot.singleton_mutex {
                "HELD"
            } else {
                "NOT HELD"
            },
            if snapshot.singleton_event {
                "HELD"
            } else {
                "NOT HELD"
            }
        ))
    }
}

pub struct LaunchManager {
    busy: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    pub events: mpsc::Receiver<LaunchEvent>,
    event_tx: mpsc::Sender<LaunchEvent>,
}

impl LaunchManager {
    pub fn new() -> Self {
        let (event_tx, events) = mpsc::channel();
        Self {
            busy: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            events,
            event_tx,
        }
    }
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn begin(
        &self,
        detection: Detection,
        selected: LaunchBackend,
        timeout: Duration,
        launch_uri: String,
        required_protection: Option<ProtectionHealth>,
        isolated_launch: Option<IsolatedLaunch>,
    ) -> Result<(), String> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("A launcher bootstrap operation is already active.".into());
        }
        if let Some(health) = &required_protection {
            if let Err(message) = verify_protection(health) {
                self.busy.store(false, Ordering::Release);
                return Err(format!("{message} The backend was not launched."));
            }
        } else if isolated_launch.is_some() {
            self.busy.store(false, Ordering::Release);
            return Err(
                "An isolated launch path cannot be used while multi-account protection is off."
                    .into(),
            );
        }
        self.cancel.store(false, Ordering::Release);
        let busy = Arc::clone(&self.busy);
        let cancel = Arc::clone(&self.cancel);
        let events = self.event_tx.clone();
        thread::Builder::new()
            .name("roblox-launch-monitor".into())
            .spawn(move || {
                let backend = isolated_launch
                    .as_ref()
                    .map(|launch| launch.backend)
                    .unwrap_or_else(|| detection.resolve(selected));
                let backend_label = backend.label().replace(" (recommended)", "");
                let installation = detection.get(backend);
                let result = (|| -> Result<(u32, String, String, bool), String> {
                    if isolated_launch.is_none() && !installation.installed() {
                        return Err(format!(
                            "{} is not available. Refresh detection or choose another backend.",
                            backend.label()
                        ));
                    }
                    let before: HashSet<u32> = platform::enumerate_clients()
                        .into_iter()
                        .map(|client| client.pid)
                        .collect();
                    let mut tracker = LaunchTracker::new(before);
                    let launch_label = isolated_launch.as_ref().map_or_else(
                        || backend_label.clone(),
                        |isolated| {
                            format!(
                                "{} through isolated Client-{:04}",
                                backend_label, isolated.client_id
                            )
                        },
                    );
                    let _ = events.send(LaunchEvent::Starting(launch_label));
                    if let Some(health) = &required_protection {
                        verify_protection(health)?;
                    }
                    let backend_pid = if let Some(isolated) = &isolated_launch {
                        if isolated.backend != backend {
                            return Err(format!(
                                "The isolated path was prepared for {}, but the resolved backend changed to {}. Refresh detection and retry.",
                                isolated.backend.label(),
                                backend.label()
                            ));
                        }
                        platform::shell_launch(&isolated.executable, Some(&launch_uri))
                    } else {
                        match backend {
                        LaunchBackend::Fishstrap | LaunchBackend::Bloxstrap => {
                            let arguments = format!("-player {launch_uri}");
                            platform::shell_launch(
                                installation.executable.as_ref().unwrap(),
                                Some(&arguments),
                            )
                        }
                        LaunchBackend::DefaultRoblox
                            if detection.roblox_protocol.healthy
                                && detection.roblox_protocol.owner == "Default Roblox"
                                && launch_uri.to_ascii_lowercase().starts_with("roblox:") =>
                        {
                            platform::shell_open_uri(&launch_uri)
                        }
                        LaunchBackend::DefaultRoblox => platform::shell_launch(
                            installation.executable.as_ref().unwrap(),
                            Some(&launch_uri),
                        ),
                        LaunchBackend::Auto => unreachable!(),
                        }
                    }
                    .map_err(|message| format!("{backend_label} failed to start: {message}"))?;
                    let _ = events.send(LaunchEvent::Waiting(backend_pid));
                    let began = Instant::now();
                    let mut bootstrap_seen = HashSet::new();
                    let mut progress_reported = false;
                    loop {
                        let elapsed = began.elapsed();
                        if elapsed >= timeout {
                            return Err(tracker.failure_detail(
                                &backend_label,
                                timeout,
                                &bootstrap_seen,
                            ));
                        }
                        if cancel.load(Ordering::Acquire) {
                            return Err("Launch was cancelled.".into());
                        }
                        if let Some(health) = &required_protection {
                            verify_protection(health).map_err(|message| {
                                format!(
                                    "Multi-instance protection was lost while waiting for a new Roblox client. {message}"
                                )
                            })?;
                        }
                        for (_, name) in platform::roblox_bootstrap_processes() {
                            bootstrap_seen.insert(name);
                        }
                        let current: HashSet<u32> = platform::enumerate_clients()
                            .into_iter()
                            .map(|client| client.pid)
                            .collect();
                        let stable = tracker.observe(&current, elapsed);
                        if !progress_reported
                            && (!tracker.exited_new.is_empty() || !bootstrap_seen.is_empty())
                        {
                            let _ = events.send(LaunchEvent::Progress(
                                "Roblox bootstrap/update transition detected; waiting for a stable new client PID."
                                    .into(),
                            ));
                            progress_reported = true;
                        }
                        if let Some(pid) = stable {
                            let warning = !tracker.closed_existing.is_empty();
                            let mut detail = tracker.success_detail(backend_pid, &bootstrap_seen);
                            if let Some(isolated) = &isolated_launch {
                                detail.push_str(&format!(
                                    " Per-instance path Client-{:04} ({}) targets {} ({}) via {}.",
                                    isolated.client_id,
                                    isolated.alias_path.display(),
                                    isolated.version,
                                    isolated.target_version.display(),
                                    isolated.backend.label()
                                ));
                            }
                            return Ok((
                                pid,
                                backend_label.clone(),
                                detail,
                                warning,
                            ));
                        }
                        thread::sleep(Duration::from_millis(250));
                    }
                })();
                match result {
                    Ok((pid, backend, detail, warning)) => {
                        let _ = events.send(LaunchEvent::Complete {
                            pid,
                            backend,
                            detail,
                            warning,
                            isolation: isolated_launch.clone(),
                        });
                    }
                    Err(message) => {
                        let _ = events.send(LaunchEvent::Failed {
                            message,
                            isolation: isolated_launch.clone(),
                        });
                    }
                }
                busy.store(false, Ordering::Release);
            })
            .map_err(|error| {
                self.busy.store(false, Ordering::Release);
                error.to_string()
            })?;
        Ok(())
    }
}

impl Drop for LaunchManager {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_queue_rejects_overlap_without_launching_anything() {
        let manager = LaunchManager::new();
        manager.busy.store(true, Ordering::Release);
        let empty =
            crate::platform::detect_launchers(Some(std::path::Path::new(".tmp/tests/absent")));
        assert!(
            manager
                .begin(
                    empty,
                    LaunchBackend::Auto,
                    Duration::from_secs(1),
                    "roblox:".into(),
                    None,
                    None,
                )
                .is_err()
        );
    }

    #[test]
    fn launch_tracker_requires_a_new_stable_pid_and_records_replacements() {
        let mut tracker = LaunchTracker::new(HashSet::from([10, 20]));
        assert_eq!(
            tracker.observe(&HashSet::from([10, 20, 30]), Duration::ZERO),
            None
        );
        assert_eq!(
            tracker.observe(&HashSet::from([10, 20]), Duration::from_secs(1)),
            None
        );
        assert_eq!(
            tracker.observe(&HashSet::from([10, 20, 40]), Duration::from_secs(2)),
            None
        );
        assert_eq!(
            tracker.observe(&HashSet::from([10, 40]), Duration::from_secs(4)),
            Some(40)
        );
        assert!(tracker.exited_new.contains(&30));
        assert!(tracker.closed_existing.contains(&20));
    }

    #[test]
    fn protected_first_client_is_normal_and_additional_clients_are_isolated() {
        assert!(!should_use_isolated_path(true, 0));
        assert!(should_use_isolated_path(true, 1));
        assert!(should_use_isolated_path(true, 2));
        assert!(!should_use_isolated_path(false, 2));
    }
}
