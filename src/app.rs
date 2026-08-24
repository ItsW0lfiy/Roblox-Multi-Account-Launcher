use crate::{
    audio::{self, AudioState},
    diagnostics,
    history::{self, HistoryStore, LaunchHistoryEntry},
    hotkeys::{HotkeyEvent, HotkeyManager},
    instance_paths::InstancePathManager,
    launch::{LaunchEvent, LaunchManager},
    local_state::{LocalStateTracer, TraceEvent},
    model::{
        Activity, ClientRole, Detection, LaunchBackend, LayoutMode, ProtectionState, ResourceMode,
        RobloxClient, Settings, UpdateChannel,
    },
    platform::{self, AppInstanceGuard, ProtectionController, ProtectionEvent},
    powershell::{self, PowerShellInfo, PowerShellKind},
    settings::{self, Logger, SettingsStore},
    updater::{self, UpdateManager, UpdateNotice, UpdateState},
};
use eframe::egui::{self, Color32, RichText, Stroke};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
    time::{Duration, Instant, SystemTime},
};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Clients,
    Diagnostics,
    PowerShell,
    Settings,
}

enum Dialog {
    EnableWithClients(Vec<u32>),
    DisableWithClients,
    ExitProtected,
    ForceClose {
        pids: Vec<u32>,
        after_enable: bool,
        exit_after: bool,
    },
    ForceClient(u32),
    CloseMultiple(Vec<u32>),
    UpdateBlocked(String),
    ResetSettings,
}

enum CloseEvent {
    Graceful {
        remaining: Vec<u32>,
        after_enable: bool,
        exit_after: bool,
    },
    Forced {
        failures: Vec<u32>,
        after_enable: bool,
        exit_after: bool,
    },
}

struct TrayIds {
    open: MenuId,
    launch: MenuId,
    tile: MenuId,
    focus1: MenuId,
    focus2: MenuId,
    mute_alt: MenuId,
    resource_alt: MenuId,
    close: MenuId,
    disable: MenuId,
    exit: MenuId,
}

pub struct LauncherApp {
    _instance_guard: AppInstanceGuard,
    protection: ProtectionController,
    protection_state: ProtectionState,
    singleton_mutex_held: bool,
    singleton_event_held: bool,
    cookie_locked: bool,
    instance_paths: InstancePathManager,
    path_isolation_error: Option<String>,
    launch: LaunchManager,
    detection: Detection,
    clients: Vec<RobloxClient>,
    client_roles: HashMap<u32, ClientRole>,
    resource_modes: HashMap<u32, ResourceMode>,
    audio_states: HashMap<u32, AudioState>,
    audio_errors: HashMap<u32, String>,
    previous_pids: Vec<u32>,
    settings: Settings,
    settings_store: SettingsStore,
    logger: Logger,
    powershell: PowerShellInfo,
    page: Page,
    launch_text: String,
    status: String,
    recent_error: Option<String>,
    activities: Vec<Activity>,
    last_refresh: Instant,
    last_health: Instant,
    last_audio_refresh: Instant,
    dialog: Option<Dialog>,
    close_tx: mpsc::Sender<CloseEvent>,
    close_rx: mpsc::Receiver<CloseEvent>,
    ps_tx: mpsc::Sender<Result<String, String>>,
    ps_rx: mpsc::Receiver<Result<String, String>>,
    ps_cancel: Arc<AtomicBool>,
    ps_busy: bool,
    ps_output: String,
    local_state_tracer: Option<LocalStateTracer>,
    trace_status: String,
    trace_events: Vec<String>,
    data_root: std::path::PathBuf,
    swapped: bool,
    tray: Option<TrayIcon>,
    tray_ids: Option<TrayIds>,
    hidden_to_tray: bool,
    exit_confirmed: bool,
    exit_after_disable: bool,
    smoke_test: bool,
    started: Instant,
    updater: UpdateManager,
    automatic_update_started: bool,
    dismissed_update: Option<String>,
    pending_target: Option<usize>,
    next_queued_launch: Option<Instant>,
    hotkeys: Option<HotkeyManager>,
    launch_link_input: String,
    history: HistoryStore,
    expected_closes: HashSet<u32>,
    forced_closes: HashSet<u32>,
}

impl LauncherApp {
    pub fn new(cc: &eframe::CreationContext<'_>, instance_guard: AppInstanceGuard) -> Self {
        configure_theme(&cc.egui_ctx);
        let root = settings::data_root();
        let settings_store = SettingsStore::new(&root);
        let settings = settings_store.load();
        let logger = Logger::new(&root);
        logger.write("INFO", "Launcher started. Multi-account mode is OFF.");
        let (close_tx, close_rx) = mpsc::channel();
        let (ps_tx, ps_rx) = mpsc::channel();
        let powershell = if std::env::var_os("RMAL_DISABLE_POWERSHELL").is_some() {
            PowerShellInfo {
                kind: PowerShellKind::None,
                executable: None,
                version: None,
            }
        } else {
            powershell::detect()
        };
        let (tray, tray_ids) = build_tray();
        let clients = platform::enumerate_clients();
        let client_roles = clients
            .iter()
            .enumerate()
            .map(|(index, client)| {
                (
                    client.pid,
                    if index == 0 {
                        ClientRole::Primary
                    } else {
                        ClientRole::Secondary
                    },
                )
            })
            .collect();
        let resource_modes = clients
            .iter()
            .map(|client| (client.pid, ResourceMode::Normal))
            .collect();
        let (hotkeys, hotkey_error) = if settings.global_hotkeys {
            match HotkeyManager::start() {
                Ok(manager) => (Some(manager), None),
                Err(message) => (None, Some(message)),
            }
        } else {
            (None, None)
        };
        let instance_paths = InstancePathManager::new(root.join("Instances"), !clients.is_empty());
        let history = HistoryStore::new(&root);
        let updater = UpdateManager::new(root.join("Updates"), settings.update_channel);
        let updater_error = updater.snapshot().error;
        let initial_status = hotkey_error
            .as_ref()
            .map(|message| format!("Global hotkeys are unavailable: {message}"))
            .or_else(|| {
                updater_error
                    .as_ref()
                    .map(|message| format!("The previous update attempt failed: {message}"))
            })
            .unwrap_or_else(|| "Ready. Multi-account mode is off.".into());
        let initial_activities = vec![Activity {
            timestamp: SystemTime::now(),
            message: updater_error
                .as_ref()
                .map(|message| format!("Previous update attempt failed: {message}"))
                .unwrap_or_else(|| "Launcher started in normal mode.".into()),
            error: updater_error.is_some(),
        }];
        Self {
            _instance_guard: instance_guard,
            protection: ProtectionController::new("ROBLOX_singletonMutex", "ROBLOX_singletonEvent"),
            protection_state: ProtectionState::Disabled,
            singleton_mutex_held: false,
            singleton_event_held: false,
            cookie_locked: false,
            instance_paths,
            path_isolation_error: None,
            launch: LaunchManager::new(),
            detection: platform::detect_launchers(None),
            previous_pids: clients.iter().map(|client| client.pid).collect(),
            clients,
            client_roles,
            resource_modes,
            audio_states: HashMap::new(),
            audio_errors: HashMap::new(),
            settings,
            settings_store,
            logger,
            powershell,
            page: Page::Home,
            launch_text: "Launch Roblox".into(),
            status: initial_status,
            recent_error: updater_error.clone(),
            activities: initial_activities,
            last_refresh: Instant::now(),
            last_health: Instant::now(),
            last_audio_refresh: Instant::now() - Duration::from_secs(10),
            dialog: None,
            close_tx,
            close_rx,
            ps_tx,
            ps_rx,
            ps_cancel: Arc::new(AtomicBool::new(false)),
            ps_busy: false,
            ps_output: "PowerShell Assist results will appear here.".into(),
            local_state_tracer: None,
            trace_status: "Stopped. No file contents are read.".into(),
            trace_events: Vec::new(),
            data_root: root,
            swapped: false,
            tray,
            tray_ids,
            hidden_to_tray: false,
            exit_confirmed: false,
            exit_after_disable: false,
            smoke_test: std::env::args().any(|arg| arg.eq_ignore_ascii_case("--smoke-test")),
            started: Instant::now(),
            updater,
            automatic_update_started: false,
            dismissed_update: None,
            pending_target: None,
            next_queued_launch: None,
            hotkeys,
            launch_link_input: String::new(),
            history,
            expected_closes: HashSet::new(),
            forced_closes: HashSet::new(),
        }
    }

    fn add_activity(&mut self, message: impl Into<String>, error: bool) {
        let message = message.into();
        self.logger
            .write(if error { "WARN" } else { "INFO" }, &message);
        self.activities.insert(
            0,
            Activity {
                timestamp: SystemTime::now(),
                message,
                error,
            },
        );
        self.activities.truncate(16);
    }

    fn refresh_clients(&mut self) {
        let current = platform::enumerate_clients();
        for client in &current {
            if !self.previous_pids.contains(&client.pid) {
                self.add_activity(
                    format!("Client {} detected (PID {}).", client.number, client.pid),
                    false,
                );
                let role = if current.iter().position(|value| value.pid == client.pid) == Some(0) {
                    ClientRole::Primary
                } else {
                    ClientRole::Secondary
                };
                self.client_roles.insert(client.pid, role);
                let resource = if role == ClientRole::Secondary {
                    self.settings.secondary_resource_mode
                } else {
                    ResourceMode::Normal
                };
                self.resource_modes.insert(client.pid, resource);
                if let Err(message) = platform::set_resource_mode(client.pid, resource) {
                    self.add_activity(message, true);
                }
                if role == ClientRole::Secondary {
                    match audio::set(
                        client.pid,
                        self.settings.secondary_volume_percent,
                        self.settings.secondary_muted,
                    ) {
                        Ok(sessions) => {
                            self.audio_states.insert(
                                client.pid,
                                AudioState {
                                    volume_percent: self.settings.secondary_volume_percent,
                                    muted: self.settings.secondary_muted,
                                    sessions,
                                },
                            );
                        }
                        Err(message) => {
                            self.audio_errors.insert(client.pid, message);
                        }
                    }
                }
            }
        }
        for pid in self.previous_pids.clone() {
            if !current.iter().any(|client| client.pid == pid) {
                let classification = if self.forced_closes.remove(&pid) {
                    "Force closed by launcher"
                } else if self.expected_closes.remove(&pid) {
                    "Closed normally"
                } else {
                    "Unexpected exit"
                };
                self.history.classify_exit(pid, classification);
                self.add_activity(
                    format!("Roblox client PID {pid} exited ({classification})."),
                    classification == "Unexpected exit",
                );
            }
        }
        self.previous_pids = current.iter().map(|client| client.pid).collect();
        self.clients = current;
        let running: HashSet<u32> = self.clients.iter().map(|client| client.pid).collect();
        self.client_roles.retain(|pid, _| running.contains(pid));
        self.resource_modes.retain(|pid, _| running.contains(pid));
        self.audio_states.retain(|pid, _| running.contains(pid));
        self.audio_errors.retain(|pid, _| running.contains(pid));
        if let Some(tray) = &self.tray {
            let _ = tray.set_tooltip(Some(format!(
                "Roblox Multi-Account Launcher • {} • {} client(s)",
                self.protection_state.label(),
                self.clients.len()
            )));
        }
        if self.last_audio_refresh.elapsed() >= Duration::from_secs(3) {
            for pid in running.iter().copied() {
                match audio::state(pid) {
                    Ok(state) => {
                        self.audio_states.insert(pid, state);
                        self.audio_errors.remove(&pid);
                    }
                    Err(message) => {
                        self.audio_errors.insert(pid, message);
                    }
                }
            }
            self.last_audio_refresh = Instant::now();
        }
        if !self.launch.busy() {
            for message in self.instance_paths.cleanup_exited(&running) {
                let error = message.starts_with("Could not");
                if error {
                    self.path_isolation_error = Some(message.clone());
                }
                self.add_activity(message, error);
            }
        }
    }

    fn refresh_detection(&mut self) {
        self.detection = platform::detect_launchers(None);
        self.add_activity("Launcher and protocol detection refreshed.", false);
    }

    fn run_protection_self_test(&mut self) {
        if self.protection_state == ProtectionState::Disabled {
            self.status = "Protection self-test: Multi-Account Mode is disabled.".into();
            return;
        }
        let health = self.protection.health().snapshot();
        let resolved = self.detection.resolve(self.settings.backend);
        let path_check = crate::instance_paths::resolve_active_version(&self.detection, resolved)
            .map(|target| target.directory.join("RobloxPlayerBeta.exe").is_file())
            .unwrap_or(false);
        let owner_healthy = matches!(
            self.protection_state,
            ProtectionState::Protected | ProtectionState::Warning
        );
        let all = health.singleton_mutex
            && health.singleton_event
            && self.cookie_locked
            && path_check
            && owner_healthy;
        self.status = format!(
            "Protection self-test: mutex={}, event={}, cookie={}, path subsystem={}, owner state={}.",
            held_label(health.singleton_mutex),
            held_label(health.singleton_event),
            held_label(self.cookie_locked),
            if path_check { "READY" } else { "FAILED" },
            if owner_healthy { "HEALTHY" } else { "FAILED" }
        );
        self.add_activity(self.status.clone(), !all);
    }

    fn begin_enable(&mut self) {
        self.protection_state = ProtectionState::Preparing;
        self.status = "Preparing multi-account protection…".into();
        if let Err(message) = self
            .protection
            .enable(Duration::from_secs(6), platform::cookie_path())
        {
            self.protection_state = ProtectionState::Disabled;
            self.recent_error = Some(message.clone());
            self.status = format!("Multi-account setup failed: {message}");
        }
    }

    fn disable(&mut self) {
        self.status = "Releasing multi-account protection…".into();
        self.protection.disable();
    }

    fn start_graceful_close(&mut self, pids: Vec<u32>, after_enable: bool, exit_after: bool) {
        self.expected_closes.extend(pids.iter().copied());
        let sender = self.close_tx.clone();
        let timeout = Duration::from_secs(self.settings.graceful_close_seconds);
        self.status = "Requesting graceful Roblox shutdown…".into();
        thread::spawn(move || {
            platform::request_close(&pids);
            let started = Instant::now();
            while started.elapsed() < timeout {
                let remaining: Vec<u32> = pids
                    .iter()
                    .copied()
                    .filter(|pid| platform::process_exists(*pid))
                    .collect();
                if remaining.is_empty() {
                    let _ = sender.send(CloseEvent::Graceful {
                        remaining,
                        after_enable,
                        exit_after,
                    });
                    return;
                }
                thread::sleep(Duration::from_millis(200));
            }
            let remaining = pids
                .into_iter()
                .filter(|pid| platform::process_exists(*pid))
                .collect();
            let _ = sender.send(CloseEvent::Graceful {
                remaining,
                after_enable,
                exit_after,
            });
        });
    }

    fn request_close_roblox(&mut self) {
        let pids: Vec<u32> = self.clients.iter().map(|client| client.pid).collect();
        if pids.len() > 1 {
            self.dialog = Some(Dialog::CloseMultiple(pids));
        } else if !pids.is_empty() {
            self.start_graceful_close(pids, false, false);
        }
    }

    fn start_force_close(&mut self, pids: Vec<u32>, after_enable: bool, exit_after: bool) {
        for pid in &pids {
            self.expected_closes.remove(pid);
        }
        self.forced_closes.extend(pids.iter().copied());
        let sender = self.close_tx.clone();
        thread::spawn(move || {
            let failures = platform::force_close(&pids);
            let _ = sender.send(CloseEvent::Forced {
                failures,
                after_enable,
                exit_after,
            });
        });
    }

    fn begin_launch(&mut self) {
        let target = match self.settings.requested_launch_target(self.clients.len()) {
            Ok(target) => target,
            Err(message) => {
                self.status =
                    format!("{message} Raise the soft limit in Settings to launch another client.");
                self.add_activity(self.status.clone(), true);
                return;
            }
        };
        if target > 1 && self.protection_state == ProtectionState::Disabled {
            self.status =
                "Enable Multi-Account Mode before requesting more than one Roblox client.".into();
            self.add_activity(self.status.clone(), true);
            return;
        }
        self.pending_target = Some(target);
        self.begin_launch_once();
    }

    fn begin_launch_once(&mut self) {
        if self.protection_state == ProtectionState::Preparing {
            self.status = "Multi-account protection is still being prepared. Wait for READY before launching.".into();
            return;
        }
        if self.protection_state == ProtectionState::Lost {
            self.status =
                "PROTECTION LOST. Repair multi-account mode before launching another client."
                    .into();
            return;
        }
        let required_protection = matches!(
            self.protection_state,
            ProtectionState::Protected | ProtectionState::Warning
        )
        .then(|| self.protection.health());
        let isolated_launch = if crate::launch::should_use_isolated_path(
            required_protection.is_some(),
            self.clients.len(),
        ) {
            match self
                .instance_paths
                .allocate(&self.detection, self.settings.backend)
            {
                Ok(launch) => {
                    self.path_isolation_error = None;
                    Some(launch)
                }
                Err(message) => {
                    self.path_isolation_error = Some(message.clone());
                    self.status = format!("Per-instance path setup failed: {message}");
                    self.recent_error = Some(message.clone());
                    self.add_activity(self.status.clone(), true);
                    return;
                }
            }
        } else {
            None
        };
        let allocated_id = isolated_launch.as_ref().map(|launch| launch.client_id);
        let launch_uri = match crate::links::validate(&self.launch_link_input) {
            Ok(value) => value,
            Err(message) => {
                self.status = message.clone();
                self.recent_error = Some(message);
                return;
            }
        };
        match self.launch.begin(
            self.detection.clone(),
            self.settings.backend,
            Duration::from_secs(self.settings.launch_timeout_seconds),
            launch_uri,
            required_protection,
            isolated_launch,
        ) {
            Ok(()) => {
                self.launch_text = "Preparing launch…".into();
                self.status = "Preparing Roblox launch…".into();
            }
            Err(message) => {
                if let Some(client_id) = allocated_id
                    && let Err(cleanup) = self.instance_paths.discard_unlaunched(client_id)
                {
                    self.path_isolation_error = Some(cleanup.clone());
                    self.add_activity(cleanup, true);
                }
                self.status = message.clone();
                self.recent_error = Some(message);
            }
        }
    }

    fn run_powershell(&mut self, label: &'static str, script: &'static str) {
        if self.ps_busy {
            return;
        }
        self.ps_busy = true;
        self.ps_output = format!("Running {label}…");
        self.ps_cancel
            .store(false, std::sync::atomic::Ordering::Release);
        let info = self.powershell.clone();
        let sender = self.ps_tx.clone();
        let cancel = Arc::clone(&self.ps_cancel);
        thread::spawn(move || {
            let result = powershell::run_script(&info, script, Duration::from_secs(8), cancel).map(
                |value| serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
            );
            let _ = sender.send(result);
        });
    }

    fn start_local_state_trace(&mut self) {
        if self.local_state_tracer.is_some() {
            return;
        }
        let seconds = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let log_path = self
            .data_root
            .join("traces")
            .join(format!("local-state-trace-{seconds}.log"));
        match LocalStateTracer::start(platform::roblox_local_storage_path(), log_path) {
            Ok(tracer) => {
                self.trace_events.clear();
                self.trace_status = "Starting metadata-only trace…".into();
                self.local_state_tracer = Some(tracer);
            }
            Err(message) => {
                self.trace_status = format!("Trace could not start: {message}");
                self.recent_error = Some(message.clone());
                self.add_activity(
                    format!("Local-state trace could not start: {message}"),
                    true,
                );
            }
        }
    }

    fn stop_local_state_trace(&mut self) {
        if let Some(tracer) = &self.local_state_tracer {
            tracer.stop();
            self.trace_status = "Stopping metadata-only trace…".into();
        }
    }

    fn poll_events(&mut self, ctx: &egui::Context) {
        let hotkey_events = self
            .hotkeys
            .as_ref()
            .map(|manager| manager.events.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for event in hotkey_events {
            let role = match event {
                HotkeyEvent::FocusPrimary => ClientRole::Primary,
                HotkeyEvent::FocusSecondary => ClientRole::Secondary,
            };
            if let Some(client) = self
                .clients
                .iter()
                .find(|client| self.client_roles.get(&client.pid) == Some(&role))
            {
                platform::focus_window(client.window);
            }
        }
        while let Ok(event) = self.protection.events.try_recv() {
            match event {
                ProtectionEvent::Enabled {
                    singleton_mutex,
                    singleton_event,
                    cookie,
                    message,
                } => {
                    self.singleton_mutex_held = singleton_mutex;
                    self.singleton_event_held = singleton_event;
                    self.cookie_locked = cookie;
                    self.protection_state = if singleton_mutex && singleton_event && cookie {
                        ProtectionState::Protected
                    } else if singleton_mutex && singleton_event {
                        ProtectionState::Warning
                    } else {
                        ProtectionState::Lost
                    };
                    self.status = message.clone();
                    self.recent_error = (!cookie).then_some(message.clone());
                    self.add_activity(
                        if cookie {
                            "Multi-account mode enabled.".into()
                        } else {
                            format!(
                                "Multi-instance protected; teleport protection warning: {message}"
                            )
                        },
                        !cookie,
                    );
                }
                ProtectionEvent::Disabled => {
                    self.singleton_mutex_held = false;
                    self.singleton_event_held = false;
                    self.cookie_locked = false;
                    self.protection_state = ProtectionState::Disabled;
                    self.status = "Normal mode. Multi-account protection is off.".into();
                    self.add_activity("Multi-account mode disabled.", false);
                    if self.exit_after_disable {
                        self.exit_confirmed = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                ProtectionEvent::Failed(message) => {
                    self.singleton_mutex_held = false;
                    self.singleton_event_held = false;
                    self.cookie_locked = false;
                    self.protection_state = ProtectionState::Disabled;
                    self.status = format!("Multi-account setup failed: {message}");
                    self.recent_error = Some(message.clone());
                    self.add_activity(format!("Multi-account setup failed: {message}"), true);
                }
                ProtectionEvent::Lost(message) => {
                    let health = self.protection.health().snapshot();
                    self.singleton_mutex_held = health.singleton_mutex;
                    self.singleton_event_held = health.singleton_event;
                    self.cookie_locked = false;
                    self.protection_state = ProtectionState::Lost;
                    self.status = format!("PROTECTION LOST: {message}");
                    self.recent_error = Some(message.clone());
                    self.add_activity(format!("PROTECTION LOST: {message}"), true);
                }
            }
        }
        while let Ok(event) = self.launch.events.try_recv() {
            match event {
                LaunchEvent::Starting(backend) => {
                    self.launch_text = format!("Starting {backend}…");
                    self.status = self.launch_text.clone();
                }
                LaunchEvent::Waiting(backend_pid) => {
                    self.launch_text = "Waiting for a new Roblox PID…".into();
                    self.status = self.launch_text.clone();
                    if let Some(pid) = backend_pid {
                        self.status = format!(
                            "Backend PID {pid} started. Waiting for a genuinely new stable RobloxPlayerBeta.exe PID…"
                        );
                    }
                }
                LaunchEvent::Progress(message) => {
                    self.launch_text = "Tracking Roblox bootstrap…".into();
                    self.status = message;
                }
                LaunchEvent::Complete {
                    pid,
                    backend,
                    detail,
                    warning,
                    isolation,
                } => {
                    let (client_name, version, launch_type) = if let Some(isolation) = isolation {
                        let values = (
                            Some(format!("Client-{:04}", isolation.client_id)),
                            isolation.version.clone(),
                            "Isolated junction path".to_string(),
                        );
                        self.instance_paths.bind_pid(isolation.client_id, pid);
                        values
                    } else {
                        let resolved = self.detection.resolve(self.settings.backend);
                        (
                            Some("Client 1".into()),
                            self.detection
                                .get(resolved)
                                .version
                                .clone()
                                .unwrap_or_else(|| "Unknown".into()),
                            "Normal backend".into(),
                        )
                    };
                    self.launch_text = "Roblox launched".into();
                    self.status = format!(
                        "New Roblox client confirmed through {backend} (PID {pid}). {detail}"
                    );
                    self.add_activity(self.status.clone(), warning);
                    self.history.push(LaunchHistoryEntry {
                        timestamp_unix: history::now_unix(),
                        pid: Some(pid),
                        client: client_name,
                        role: if self.clients.is_empty() {
                            ClientRole::Primary.label().into()
                        } else {
                            ClientRole::Secondary.label().into()
                        },
                        backend: backend.clone(),
                        version,
                        launch_type,
                        result: if warning {
                            "Success with warning"
                        } else {
                            "Success"
                        }
                        .into(),
                        exit_classification: None,
                    });
                    self.refresh_clients();
                    if self.settings.auto_arrange_after_launch && self.clients.len() >= 2 {
                        self.tile();
                    }
                    if self
                        .pending_target
                        .is_some_and(|target| self.clients.len() >= target)
                    {
                        self.pending_target = None;
                        self.next_queued_launch = None;
                    } else if self.pending_target.is_some() {
                        self.next_queued_launch = Some(Instant::now() + Duration::from_secs(1));
                    }
                }
                LaunchEvent::Failed { message, isolation } => {
                    let client_name = isolation
                        .as_ref()
                        .map(|value| format!("Client-{:04}", value.client_id));
                    let version = isolation
                        .as_ref()
                        .map(|value| value.version.clone())
                        .unwrap_or_else(|| "Unknown".into());
                    let launch_type = if isolation.is_some() {
                        "Isolated junction path"
                    } else {
                        "Normal backend"
                    };
                    if let Some(isolation) = isolation {
                        self.instance_paths.mark_unconfirmed(isolation.client_id);
                    }
                    self.launch_text = "Launch failed".into();
                    self.status = format!("Launch failed: {message}");
                    self.recent_error = Some(message.clone());
                    self.add_activity(self.status.clone(), true);
                    self.history.push(LaunchHistoryEntry {
                        timestamp_unix: history::now_unix(),
                        pid: None,
                        client: client_name,
                        role: "Unassigned".into(),
                        backend: self.detection.resolve(self.settings.backend).label().into(),
                        version,
                        launch_type: launch_type.into(),
                        result: format!("Failed: {}", diagnostics::redact(&message)),
                        exit_classification: Some("Exited during bootstrap / no stable PID".into()),
                    });
                    self.pending_target = None;
                    self.next_queued_launch = None;
                }
            }
        }
        let trace_events = self
            .local_state_tracer
            .as_ref()
            .map(|tracer| tracer.events.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for event in trace_events {
            match event {
                TraceEvent::Started(path) => {
                    self.trace_status = format!("Tracing metadata only. Log: {}", path.display());
                    self.add_activity("Local-state metadata trace started.", false);
                }
                TraceEvent::Change(line) => {
                    self.trace_events.insert(0, line);
                    self.trace_events.truncate(40);
                }
                TraceEvent::Stopped => {
                    self.trace_status =
                        "Stopped. Trace log retained; no file contents were read.".into();
                    self.local_state_tracer = None;
                    self.add_activity("Local-state metadata trace stopped.", false);
                }
                TraceEvent::Failed(message) => {
                    self.trace_status = format!("Trace failed: {message}");
                    self.recent_error = Some(message.clone());
                    self.local_state_tracer = None;
                    self.add_activity(format!("Local-state trace failed: {message}"), true);
                }
            }
        }
        if !self.launch.busy()
            && self.launch_text != "Launch Roblox"
            && !self.launch_text.ends_with('…')
            && self.last_refresh.elapsed() > Duration::from_secs(1)
        {
            self.launch_text = "Launch Roblox".into();
        }
        while let Ok(event) = self.close_rx.try_recv() {
            match event {
                CloseEvent::Graceful {
                    remaining,
                    after_enable,
                    exit_after,
                } if remaining.is_empty() => {
                    self.refresh_clients();
                    self.add_activity("Roblox closed gracefully.", false);
                    if after_enable {
                        self.begin_enable();
                    } else if exit_after {
                        self.exit_after_disable = true;
                        self.disable();
                    }
                }
                CloseEvent::Graceful {
                    remaining,
                    after_enable,
                    exit_after,
                } => {
                    self.dialog = Some(Dialog::ForceClose {
                        pids: remaining,
                        after_enable,
                        exit_after,
                    });
                }
                CloseEvent::Forced {
                    failures,
                    after_enable,
                    exit_after,
                } if failures.is_empty() => {
                    self.refresh_clients();
                    self.add_activity(
                        "Roblox force-closed after graceful shutdown timed out.",
                        false,
                    );
                    if after_enable {
                        self.begin_enable();
                    } else if exit_after {
                        self.exit_after_disable = true;
                        self.disable();
                    }
                }
                CloseEvent::Forced { failures, .. } => {
                    let message = format!(
                        "Could not force-close PID(s): {}.",
                        failures
                            .iter()
                            .map(u32::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    self.recent_error = Some(message.clone());
                    self.status = message.clone();
                    self.add_activity(message, true);
                }
            }
        }
        while let Ok(result) = self.ps_rx.try_recv() {
            self.ps_busy = false;
            match result {
                Ok(output) => {
                    self.ps_output = output;
                    self.add_activity("PowerShell Assist completed.", false);
                }
                Err(message) => {
                    self.ps_output = format!("PowerShell Assist failed:\n{message}");
                    self.add_activity(self.ps_output.clone(), true);
                }
            }
        }
        for notice in self.updater.poll() {
            match notice {
                UpdateNotice::BackgroundFailure(message) => {
                    self.logger.write(
                        "INFO",
                        &format!("Background update check failed; launcher unaffected: {message}"),
                    );
                }
                UpdateNotice::ManualFailure(message) => {
                    self.status = format!("Updater failed: {message}");
                    self.recent_error = Some(message.clone());
                    self.add_activity(format!("Updater failed: {message}"), true);
                }
                UpdateNotice::Checked {
                    available: true, ..
                } => {
                    self.dismissed_update = None;
                    let version = self
                        .updater
                        .snapshot()
                        .latest_version
                        .unwrap_or_else(|| "unknown".into());
                    self.add_activity(format!("Update v{version} is available."), false);
                }
                UpdateNotice::Checked {
                    available: false,
                    manual,
                } => {
                    if manual {
                        self.status = "The launcher is up to date.".into();
                        self.add_activity("Manual update check found no newer release.", false);
                    }
                }
                UpdateNotice::DownloadReady(version) => {
                    self.status = format!(
                        "Update v{version} downloaded and verified. It is ready to install."
                    );
                    self.add_activity(
                        format!("Update v{version} passed SHA-256 verification."),
                        false,
                    );
                }
                UpdateNotice::Cancelled => {
                    self.status =
                        "Update operation cancelled; the installed executable was unchanged."
                            .into();
                    self.add_activity("Update operation cancelled safely.", false);
                }
            }
        }
        if let (Some(target), Some(ready_at)) = (self.pending_target, self.next_queued_launch)
            && !self.launch.busy()
            && Instant::now() >= ready_at
        {
            self.refresh_clients();
            if self.clients.len() >= target {
                self.pending_target = None;
                self.next_queued_launch = None;
            } else if self.clients.len() >= self.settings.client_limit {
                self.pending_target = None;
                self.next_queued_launch = None;
                self.status = "Queued launching stopped at the configured client limit.".into();
            } else {
                self.next_queued_launch = None;
                self.begin_launch_once();
            }
        }
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        let Some(ids) = &self.tray_ids else {
            return;
        };
        let ids = (
            ids.open.clone(),
            ids.launch.clone(),
            ids.tile.clone(),
            ids.focus1.clone(),
            ids.focus2.clone(),
            ids.mute_alt.clone(),
            ids.resource_alt.clone(),
            ids.close.clone(),
            ids.disable.clone(),
            ids.exit.clone(),
        );
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == ids.0 {
                self.hidden_to_tray = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            } else if event.id == ids.1 {
                self.begin_launch();
            } else if event.id == ids.2 {
                let _ = platform::tile_clients(
                    &self.clients,
                    self.settings.layout,
                    self.settings.preferred_monitor.as_deref(),
                    self.swapped,
                );
            } else if event.id == ids.3 {
                if let Some(client) = self
                    .clients
                    .iter()
                    .find(|client| self.client_roles.get(&client.pid) == Some(&ClientRole::Primary))
                {
                    platform::focus_window(client.window);
                }
            } else if event.id == ids.4 {
                if let Some(client) = self.clients.iter().find(|client| {
                    self.client_roles.get(&client.pid) == Some(&ClientRole::Secondary)
                }) {
                    platform::focus_window(client.window);
                }
            } else if event.id == ids.5 {
                if let Some(client) = self.clients.iter().find(|client| {
                    self.client_roles.get(&client.pid) == Some(&ClientRole::Secondary)
                }) {
                    let volume = self
                        .audio_states
                        .get(&client.pid)
                        .map_or(self.settings.secondary_volume_percent, |state| {
                            state.volume_percent
                        });
                    let muted = !self
                        .audio_states
                        .get(&client.pid)
                        .is_some_and(|state| state.muted);
                    match audio::set(client.pid, volume, muted) {
                        Ok(sessions) => {
                            self.audio_states.insert(
                                client.pid,
                                AudioState {
                                    volume_percent: volume,
                                    muted,
                                    sessions,
                                },
                            );
                        }
                        Err(message) => self.status = message,
                    }
                }
            } else if event.id == ids.6 {
                if let Some(client) = self.clients.iter().find(|client| {
                    self.client_roles.get(&client.pid) == Some(&ClientRole::Secondary)
                }) {
                    let current = self
                        .resource_modes
                        .get(&client.pid)
                        .copied()
                        .unwrap_or(ResourceMode::Normal);
                    let next = if current == ResourceMode::AltSaver {
                        ResourceMode::Normal
                    } else {
                        ResourceMode::AltSaver
                    };
                    match platform::set_resource_mode(client.pid, next) {
                        Ok(()) => {
                            self.resource_modes.insert(client.pid, next);
                        }
                        Err(message) => self.status = message,
                    }
                }
            } else if event.id == ids.7 {
                self.request_close_roblox();
            } else if event.id == ids.8 {
                if self.launch.busy() {
                    self.status = "Wait for the active Roblox bootstrap to finish before disabling multi-account protection.".into();
                } else if self.clients.is_empty() {
                    self.disable();
                } else {
                    self.dialog = Some(Dialog::DisableWithClients);
                }
            } else if event.id == ids.9 {
                self.request_exit(ctx);
            }
        }
    }

    fn request_exit(&mut self, ctx: &egui::Context) {
        if self.protection_state != ProtectionState::Disabled {
            self.dialog = Some(Dialog::ExitProtected);
        } else {
            self.exit_confirmed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let update = self.updater.snapshot();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(
                    RichText::new("Roblox Multi-Account Launcher")
                        .size(24.0)
                        .color(Color32::from_rgb(244, 246, 250)),
                );
                ui.label(
                    RichText::new(concat!(
                        "v",
                        env!("CARGO_PKG_VERSION"),
                        " • Experimental pre-release • Rust core • native Windows APIs"
                    ))
                    .color(Color32::from_rgb(150, 155, 168)),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (label, color) = match self.protection_state {
                    ProtectionState::Protected => {
                        ("MULTI-ACCOUNT ACTIVE", Color32::from_rgb(77, 198, 133))
                    }
                    ProtectionState::Warning => {
                        ("MULTI-ACCOUNT WARNING", Color32::from_rgb(230, 176, 70))
                    }
                    ProtectionState::Lost => ("PROTECTION LOST", Color32::from_rgb(238, 78, 91)),
                    ProtectionState::Preparing => ("PREPARING", Color32::from_rgb(230, 176, 70)),
                    ProtectionState::Disabled => ("NORMAL MODE", Color32::from_rgb(165, 169, 181)),
                };
                ui.label(RichText::new(label).strong().color(color));
                if matches!(
                    update.state,
                    UpdateState::UpdateAvailable
                        | UpdateState::ReadyToInstall
                        | UpdateState::WaitingForSafeRestart
                ) && update.latest_version.as_ref() != self.dismissed_update.as_ref()
                {
                    let text = format!(
                        "Update available — v{}",
                        update.latest_version.as_deref().unwrap_or("?")
                    );
                    if ui
                        .button(RichText::new(text).color(Color32::from_rgb(230, 176, 70)))
                        .clicked()
                    {
                        self.page = Page::Settings;
                    }
                }
            });
        });
        ui.add_space(14.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(30, 30, 36))
            .stroke(Stroke::new(1.0, Color32::from_rgb(56, 57, 66)))
            .corner_radius(10.0)
            .inner_margin(14.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Quick Launch").strong());
                        let resolved = self.detection.resolve(self.settings.backend);
                        ui.label(
                            RichText::new(format!(
                                "{} -> {}  |  {} Roblox client(s)",
                                self.settings.backend.label(),
                                resolved.label(),
                                self.clients.len()
                            ))
                            .color(Color32::from_rgb(155, 159, 172)),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.launch_link_input)
                                .hint_text("Optional Roblox game/share/protocol link")
                                .desired_width(430.0),
                        )
                        .on_hover_text("Transient input only. Credentials and foreign hosts are refused; the link is not saved in settings or history.");
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let enabled = !self.launch.busy()
                            && !matches!(
                                self.protection_state,
                                ProtectionState::Lost | ProtectionState::Preparing
                            );
                        let button = egui::Button::new(RichText::new(&self.launch_text).strong())
                            .fill(Color32::from_rgb(191, 46, 66))
                            .min_size(egui::vec2(210.0, 42.0));
                        if ui.add_enabled(enabled, button).clicked() {
                            self.begin_launch();
                        }
                    });
                });
            });
    }

    fn navigation(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut pages = vec![
                (Page::Home, "Home"),
                (Page::Clients, "Clients"),
                (Page::Diagnostics, "Diagnostics"),
                (Page::Settings, "Settings"),
            ];
            if self.settings.advanced_mode {
                pages.insert(3, (Page::PowerShell, "PowerShell Assist"));
            }
            for (page, label) in pages {
                if ui.selectable_label(self.page == page, label).clicked() {
                    self.page = page;
                }
            }
        });
        ui.separator();
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |columns| {
            card(&mut columns[0], "Multi-Account Mode", |ui| {
                ui.label("Always starts OFF. Enabling owns both Roblox singleton names and optional cookie-file protection.");
                let mut enabled = self.protection_state != ProtectionState::Disabled;
                let response = ui.add_enabled(
                    self.protection_state != ProtectionState::Preparing && !self.launch.busy(),
                    egui::Checkbox::new(&mut enabled, "Enable multi-account protection"),
                );
                if response.changed() {
                    if enabled {
                        if self.clients.is_empty() { self.begin_enable(); }
                        else { self.dialog = Some(Dialog::EnableWithClients(self.clients.iter().map(|client| client.pid).collect())); }
                    } else if self.clients.is_empty() { self.disable(); } else { self.dialog = Some(Dialog::DisableWithClients); }
                }
                ui.add_space(8.0);
                state_row(ui, "Multi-instance", if self.protection_state == ProtectionState::Disabled { "Disabled" } else { self.protection_state.label() }, self.protection_state);
                state_row(ui, "singletonMutex", if self.singleton_mutex_held { "HELD" } else { "Not held" }, if self.singleton_mutex_held { ProtectionState::Protected } else { ProtectionState::Disabled });
                state_row(ui, "singletonEvent", if self.singleton_event_held { "HELD" } else { "Not held" }, if self.singleton_event_held { ProtectionState::Protected } else { ProtectionState::Disabled });
                state_row(ui, "Teleport protection", if self.cookie_locked { "Protected" } else if self.protection_state == ProtectionState::Warning { "Warning" } else { "Disabled" }, if self.cookie_locked { ProtectionState::Protected } else { self.protection_state });
                let path_state = if self.path_isolation_error.is_some() { ProtectionState::Warning } else if self.protection_state == ProtectionState::Disabled { ProtectionState::Disabled } else { ProtectionState::Protected };
                let path_label = if self.path_isolation_error.is_some() { "Warning" } else if self.protection_state == ProtectionState::Disabled { "Disabled" } else if self.instance_paths.records().is_empty() { "Ready" } else { "ACTIVE" };
                state_row(ui, "Per-instance path isolation", path_label, path_state);
                state_row(ui, "Login-state behavior", "Experimental / manually validated", ProtectionState::Warning);
                if self.protection_state == ProtectionState::Warning { ui.colored_label(Color32::from_rgb(230, 176, 70), "Multiple clients may launch, but multi-client teleports may fail."); }
                if matches!(self.protection_state, ProtectionState::Lost | ProtectionState::Disabled) && self.recent_error.is_some()
                    && ui.button("Retry Multi-Account Setup").clicked() {
                    if self.clients.is_empty() { self.begin_enable(); } else { self.dialog = Some(Dialog::EnableWithClients(self.clients.iter().map(|client| client.pid).collect())); }
                }
                if ui
                    .add_enabled(
                        self.protection_state != ProtectionState::Disabled,
                        egui::Button::new("Run Protection Self-Test"),
                    )
                    .clicked()
                {
                    self.run_protection_self_test();
                }
            });
            card(&mut columns[1], "Launch Backend", |ui| {
                let resolved = self.detection.resolve(self.settings.backend);
                state_row(ui, "Selected", self.settings.backend.label(), ProtectionState::Disabled);
                state_row(ui, "Resolved", resolved.label(), if self.detection.get(resolved).installed() { ProtectionState::Protected } else { ProtectionState::Warning });
                state_row(ui, "Fishstrap", if self.detection.fishstrap.installed() { "Detected" } else { "Not detected" }, if self.detection.fishstrap.installed() { ProtectionState::Protected } else { ProtectionState::Disabled });
                state_row(ui, "Bloxstrap", if self.detection.bloxstrap.installed() { "Detected" } else { "Not detected" }, if self.detection.bloxstrap.installed() { ProtectionState::Protected } else { ProtectionState::Disabled });
                state_row(ui, "Stock Roblox", if self.detection.stock.installed() { "Detected" } else { "Not detected" }, if self.detection.stock.installed() { ProtectionState::Protected } else { ProtectionState::Warning });
                if ui.button("Refresh Detection").clicked() { self.refresh_detection(); }
                let installation = self.detection.get(resolved).clone();
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(installation.installed(), egui::Button::new(format!("Open {}", resolved.label())))
                        .clicked()
                        && let Some(executable) = installation.executable
                    {
                        let _ = platform::shell_launch(&executable, None);
                    }
                    if ui
                        .add_enabled(installation.base_dir.is_some(), egui::Button::new("Open Folder"))
                        .clicked()
                        && let Some(folder) = installation.base_dir
                    {
                        let _ = platform::shell_launch(&folder, None);
                    }
                    if ui
                        .add_enabled(installation.logs_dir.is_some(), egui::Button::new("Open Logs"))
                        .clicked()
                        && let Some(logs) = installation.logs_dir
                    {
                        let _ = platform::shell_launch(&logs, None);
                    }
                });
                let busy = platform::launcher_related_processes();
                if !busy.is_empty() {
                    ui.colored_label(
                        Color32::from_rgb(230, 176, 70),
                        format!(
                            "Bootstrap/update activity: {}",
                            busy.iter()
                                .map(|(pid, name)| format!("{name} ({pid})"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                }
            });
        });
        ui.add_space(12.0);
        card(ui, "Status", |ui| {
            let color = if self
                .recent_error
                .as_deref()
                .is_some_and(|error| self.status.contains(error))
            {
                Color32::from_rgb(238, 105, 113)
            } else {
                Color32::from_rgb(224, 226, 232)
            };
            ui.label(RichText::new(&self.status).color(color));
            ui.separator();
            for activity in self.activities.iter().take(7) {
                let seconds = activity
                    .timestamp
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    % 86_400;
                let time = format!(
                    "{:02}:{:02}:{:02}",
                    seconds / 3600,
                    (seconds / 60) % 60,
                    seconds % 60
                );
                ui.label(
                    RichText::new(format!("{time}  {}", activity.message)).color(
                        if activity.error {
                            Color32::from_rgb(238, 105, 113)
                        } else {
                            Color32::from_rgb(169, 173, 185)
                        },
                    ),
                );
            }
        });
    }

    fn clients_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(format!("Roblox clients: {}", self.clients.len()));
            if ui.button("Refresh").clicked() {
                self.refresh_clients();
            }
            if ui.button("Close Roblox").clicked() && !self.clients.is_empty() {
                self.request_close_roblox();
            }
            if ui.button("Restore All").clicked() {
                platform::restore_clients(&self.clients);
            }
            if ui.button("Center Primary").clicked()
                && let Some(primary) = self
                    .clients
                    .iter()
                    .find(|client| self.client_roles.get(&client.pid) == Some(&ClientRole::Primary))
                && let Err(message) =
                    platform::center_client(primary, self.settings.preferred_monitor.as_deref())
            {
                self.status = message;
            }
        });
        if self.clients.is_empty() {
            ui.add_space(25.0);
            ui.label(
                RichText::new("No Roblox clients detected. Use Launch Roblox above to start one.")
                    .color(Color32::from_rgb(160, 164, 176)),
            );
        }
        for client in self.clients.clone() {
            let isolated = self.instance_paths.record_for_pid(client.pid).cloned();
            let mut role = self
                .client_roles
                .get(&client.pid)
                .copied()
                .unwrap_or(ClientRole::Custom);
            let original_role = role;
            let mut resource = self
                .resource_modes
                .get(&client.pid)
                .copied()
                .unwrap_or(ResourceMode::Normal);
            let original_resource = resource;
            let audio_state = self.audio_states.get(&client.pid).copied();
            let mut volume = audio_state.map_or(100, |state| state.volume_percent);
            let mut muted = audio_state.is_some_and(|state| state.muted);
            let mut apply_audio = false;
            card(
                ui,
                &format!("Client {}  •  PID {}", client.number, client.pid),
                |ui| {
                    ui.label(format!(
                        "Uptime: {}  •  CPU time: {}  •  RAM: {:.0} MB  •  Window: {}",
                        format_duration(client.uptime),
                        format_duration(client.cpu_time),
                        client.working_set as f64 / 1_048_576.0,
                        client.title
                    ));
                    ui.horizontal(|ui| {
                        ui.label("Role");
                        egui::ComboBox::from_id_salt(("client-role", client.pid))
                            .selected_text(role.label())
                            .show_ui(ui, |ui| {
                                for value in ClientRole::ALL {
                                    ui.selectable_value(&mut role, value, value.label());
                                }
                            });
                        ui.label("Resource mode");
                        egui::ComboBox::from_id_salt(("resource-mode", client.pid))
                            .selected_text(resource.label())
                            .show_ui(ui, |ui| {
                                for value in ResourceMode::ALL {
                                    ui.selectable_value(&mut resource, value, value.label());
                                }
                            });
                    });
                    if let Some(record) = &isolated {
                        ui.label(format!(
                            "Launch path: isolated Client-{:04}  •  Backend: {}  •  Version: {}",
                            record.client_id,
                            record.backend.label(),
                            record.version
                        ));
                        ui.label(
                            RichText::new(format!("Alias: {}", record.alias_path.display()))
                                .monospace()
                                .color(Color32::from_rgb(155, 159, 172)),
                        );
                    } else {
                        ui.label("Launch path: external or normal backend path");
                    }
                    ui.horizontal(|ui| {
                        ui.label("Audio");
                        if ui
                            .add(egui::Slider::new(&mut volume, 0..=100).suffix("%"))
                            .changed()
                        {
                            apply_audio = true;
                        }
                        if ui.checkbox(&mut muted, "Mute").changed() {
                            apply_audio = true;
                        }
                        if let Some(state) = audio_state {
                            ui.label(format!("{} session(s)", state.sessions));
                        } else {
                            ui.label("No active audio session");
                        }
                    });
                    if let Some(message) = self.audio_errors.get(&client.pid) {
                        ui.label(
                            RichText::new(message)
                                .small()
                                .color(Color32::from_rgb(155, 159, 172)),
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Focus Client").clicked() {
                            platform::focus_window(client.window);
                        }
                        if ui.button("Close Client").clicked() {
                            self.start_graceful_close(vec![client.pid], false, false);
                        }
                        if ui.button("Force Close Client").clicked() {
                            self.dialog = Some(Dialog::ForceClient(client.pid));
                        }
                    });
                },
            );
            if role != original_role {
                if matches!(role, ClientRole::Primary | ClientRole::Secondary)
                    && let Some(previous) = self
                        .client_roles
                        .iter()
                        .find(|(pid, value)| **pid != client.pid && **value == role)
                        .map(|(pid, _)| *pid)
                {
                    self.client_roles.insert(previous, ClientRole::Custom);
                }
                self.client_roles.insert(client.pid, role);
                self.add_activity(
                    format!("PID {} role changed to {}.", client.pid, role.label()),
                    false,
                );
            }
            if resource != original_resource {
                match platform::set_resource_mode(client.pid, resource) {
                    Ok(()) => {
                        self.resource_modes.insert(client.pid, resource);
                        self.add_activity(
                            format!(
                                "PID {} resource mode set to {}.",
                                client.pid,
                                resource.label()
                            ),
                            false,
                        );
                    }
                    Err(message) => {
                        self.status = message.clone();
                        self.add_activity(message, true);
                    }
                }
            }
            if apply_audio {
                match audio::set(client.pid, volume, muted) {
                    Ok(sessions) => {
                        self.audio_states.insert(
                            client.pid,
                            AudioState {
                                volume_percent: volume,
                                muted,
                                sessions,
                            },
                        );
                    }
                    Err(message) => {
                        self.audio_errors.insert(client.pid, message.clone());
                        self.status = message;
                    }
                }
            }
        }
        if self.clients.len() == 2 {
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Tile 50/50").clicked() {
                    self.settings.layout = LayoutMode::FiftyFifty;
                    self.tile();
                }
                if ui.button("Primary / Secondary 70/30").clicked() {
                    self.settings.layout = LayoutMode::PrimarySecondary;
                    self.tile();
                }
                if ui.button("Vertical").clicked() {
                    self.settings.layout = LayoutMode::Vertical;
                    self.tile();
                }
                if ui.button("Swap Clients").clicked() {
                    self.swapped = !self.swapped;
                    self.tile();
                }
                if ui.button("Swap Roles").clicked() {
                    let primary = self
                        .clients
                        .iter()
                        .find(|client| {
                            self.client_roles.get(&client.pid) == Some(&ClientRole::Primary)
                        })
                        .map(|client| client.pid);
                    let secondary = self
                        .clients
                        .iter()
                        .find(|client| {
                            self.client_roles.get(&client.pid) == Some(&ClientRole::Secondary)
                        })
                        .map(|client| client.pid);
                    if let (Some(primary), Some(secondary)) = (primary, secondary) {
                        self.client_roles.insert(primary, ClientRole::Secondary);
                        self.client_roles.insert(secondary, ClientRole::Primary);
                    }
                }
            });
        }
    }

    fn tile(&mut self) {
        let mut clients = self.clients.clone();
        clients.sort_by_key(|client| match self.client_roles.get(&client.pid) {
            Some(ClientRole::Primary) => 0,
            Some(ClientRole::Secondary) => 1,
            _ => 2,
        });
        match platform::tile_clients(
            &clients,
            self.settings.layout,
            self.settings.preferred_monitor.as_deref(),
            self.swapped,
        ) {
            Ok(()) => self.add_activity(
                format!("Clients tiled using {}.", self.settings.layout.label()),
                false,
            ),
            Err(message) => {
                self.status = message.clone();
                self.recent_error = Some(message);
            }
        }
    }

    fn diagnostics_page(&mut self, ui: &mut egui::Ui) {
        let update_snapshot = self.updater.snapshot();
        let report = diagnostics::report(diagnostics::ReportContext {
            detection: &self.detection,
            selected: self.settings.backend,
            clients: &self.clients,
            protection: self.protection_state,
            singleton_mutex_held: self.singleton_mutex_held,
            singleton_event_held: self.singleton_event_held,
            cookie_locked: self.cookie_locked,
            isolated_instances: self.instance_paths.records(),
            path_isolation_error: self.path_isolation_error.as_deref(),
            powershell: &self.powershell,
            recent_error: self.recent_error.as_deref(),
            updater: &update_snapshot,
            roles: &self.client_roles,
            resources: &self.resource_modes,
            audio: &self.audio_states,
        });
        ui.horizontal(|ui| {
            ui.heading("Diagnostics");
            if ui.button("Refresh").clicked() {
                self.refresh_detection();
                self.refresh_clients();
            }
            if ui.button("Copy Diagnostics").clicked() {
                ui.ctx().copy_text(report.clone());
                self.add_activity("Sanitized diagnostics copied.", false);
            }
            if ui.button("Create Support Bundle").clicked() {
                match crate::support::create_bundle(
                    &self.data_root,
                    &report,
                    &self.settings,
                    self.history.entries(),
                ) {
                    Ok(path) => {
                        self.status =
                            format!("Sanitized support bundle created at {}.", path.display());
                        self.add_activity(self.status.clone(), false);
                        let _ = platform::shell_launch(&path, None);
                    }
                    Err(message) => {
                        self.status = format!("Support bundle failed: {message}");
                        self.add_activity(self.status.clone(), true);
                    }
                }
            }
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut report.clone())
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(27)
                    .interactive(false),
            );
            ui.add_space(12.0);
            ui.heading("Launch recovery & instance cleanup");
            let related = platform::launcher_related_processes();
            let windowless: Vec<_> = self
                .clients
                .iter()
                .filter(|client| client.window == 0)
                .map(|client| client.pid)
                .collect();
            let stale = self.instance_paths.stale_aliases();
            ui.label(format!(
                "Active clients: {} • Windowless Roblox PIDs: {} • Related bootstrap/crash processes: {} • Stale aliases: {}",
                self.clients.len(),
                if windowless.is_empty() { "None".into() } else { windowless.iter().map(u32::to_string).collect::<Vec<_>>().join(", ") },
                related.len(),
                stale.len()
            ));
            for (pid, name) in &related {
                ui.label(format!("{name} • PID {pid}"));
            }
            for alias in &stale {
                ui.label(format!("Stale alias: {}", alias.display()));
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button("Run Protection Self-Test").clicked() {
                    self.run_protection_self_test();
                }
                if ui.button("Open Instance Folder").clicked() {
                    let _ = platform::shell_launch(self.instance_paths.root(), None);
                }
                if ui
                    .add_enabled(self.clients.is_empty() && !stale.is_empty(), egui::Button::new("Clean Stale Aliases"))
                    .clicked()
                {
                    match self.instance_paths.cleanup_stale_now(false) {
                        Ok(messages) => {
                            self.status = if messages.is_empty() {
                                "No stale aliases required cleanup.".into()
                            } else {
                                messages.join(" ")
                            };
                        }
                        Err(message) => self.status = message,
                    }
                }
            });
            ui.label("Recovery actions are intentionally conservative: force-close remains a separately confirmed action, and aliases are never deleted while Roblox is running.");
            ui.separator();
            ui.heading("Recent launch history");
            for entry in self.history.entries().iter().take(12) {
                ui.label(format!(
                    "{} • {} • {} • {} • {}",
                    entry.timestamp_unix,
                    entry.client.as_deref().unwrap_or("Unconfirmed client"),
                    entry.role,
                    entry.backend,
                    entry.result
                ));
                if let Some(exit) = &entry.exit_classification {
                    ui.label(RichText::new(format!("  Exit: {exit}")).small());
                }
            }
            ui.separator();
            ui.heading("Shared login-state investigation");
            state_row(
                ui,
                "Login-state behavior",
                "Experimental / manually validated",
                ProtectionState::Warning,
            );
            ui.label("Bidirectional logout isolation was manually validated three times with the external protections active. This remains experimental until it is validated across Roblox updates and other Windows profiles.");
            ui.label("No credential, cookie, ticket, or file-content handling is implemented.");
            ui.label("The tracer below records only relative path, time, and create/write/delete/rename metadata under Roblox\\LocalStorage. Windows directory notifications do not identify the responsible process, so process is recorded as unavailable.");
            if self.settings.advanced_mode {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.local_state_tracer.is_none(),
                        egui::Button::new("Start Local-State Trace"),
                    )
                    .clicked()
                {
                    self.start_local_state_trace();
                }
                if ui
                    .add_enabled(
                        self.local_state_tracer.is_some(),
                        egui::Button::new("Stop Trace"),
                    )
                    .clicked()
                {
                    self.stop_local_state_trace();
                }
            });
            ui.label(RichText::new(&self.trace_status).color(Color32::from_rgb(155, 159, 172)));
            for event in self.trace_events.iter().take(12) {
                ui.label(RichText::new(event).monospace().color(Color32::from_rgb(169, 173, 185)));
            }
            } else {
                ui.label("Enable Advanced Mode in Settings to use the metadata-only local-state tracer.");
            }
        });
    }

    fn powershell_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("PowerShell Assist");
        ui.label("Optional, hidden, bounded diagnostic assistance. Core launcher functionality remains implemented in Rust.");
        state_row(
            ui,
            "Engine",
            self.powershell.engine_label(),
            if self.powershell.kind == PowerShellKind::None {
                ProtectionState::Warning
            } else {
                ProtectionState::Protected
            },
        );
        state_row(
            ui,
            "Version",
            self.powershell
                .version
                .as_deref()
                .unwrap_or("Not available"),
            ProtectionState::Disabled,
        );
        state_row(
            ui,
            "Status",
            self.powershell.capability(),
            ProtectionState::Disabled,
        );
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let actions = [
                ("Inspect Roblox State", powershell::DIAGNOSE_ROBLOX),
                ("Inspect Fishstrap", powershell::DIAGNOSE_FISHSTRAP),
                ("Inspect Bloxstrap", powershell::DIAGNOSE_BLOXSTRAP),
                ("Check Roblox Protocols", powershell::INSPECT_PROTOCOLS),
                ("Inspect Processes", powershell::INSPECT_PROCESSES),
                ("Repair Launch State", powershell::REPAIR_LAUNCH_STATE),
            ];
            for (label, script) in actions {
                if ui
                    .add_enabled(
                        !self.ps_busy && self.settings.powershell_assist,
                        egui::Button::new(label),
                    )
                    .clicked()
                {
                    self.run_powershell(label, script);
                }
            }
            if self.ps_busy && ui.button("Cancel").clicked() {
                self.ps_cancel
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        });
        ui.separator();
        if !self.settings.powershell_assist {
            ui.colored_label(
                Color32::from_rgb(230, 176, 70),
                "PowerShell Assist is disabled in Settings. Core Rust functionality is unaffected.",
            );
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.ps_output)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(22)
                    .interactive(false),
            );
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        let previous_channel = self.settings.update_channel;
        egui::Grid::new("settings-grid")
            .num_columns(2)
            .spacing([24.0, 14.0])
            .show(ui, |ui| {
                ui.label("Launch backend");
                egui::ComboBox::from_id_salt("backend")
                    .selected_text(self.settings.backend.label())
                    .show_ui(ui, |ui| {
                        for backend in LaunchBackend::ALL {
                            ui.selectable_value(
                                &mut self.settings.backend,
                                backend,
                                backend.label(),
                            );
                        }
                    });
                ui.end_row();
                ui.label("Preferred monitor");
                let monitors = platform::monitors();
                egui::ComboBox::from_id_salt("monitor")
                    .selected_text(
                        self.settings
                            .preferred_monitor
                            .as_deref()
                            .unwrap_or("Primary / automatic"),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.settings.preferred_monitor,
                            None,
                            "Primary / automatic",
                        );
                        for monitor in monitors {
                            ui.selectable_value(
                                &mut self.settings.preferred_monitor,
                                Some(monitor.name.clone()),
                                monitor.name,
                            );
                        }
                    });
                ui.end_row();
                ui.label("Default tile layout");
                egui::ComboBox::from_id_salt("layout")
                    .selected_text(self.settings.layout.label())
                    .show_ui(ui, |ui| {
                        for layout in LayoutMode::ALL {
                            ui.selectable_value(&mut self.settings.layout, layout, layout.label());
                        }
                    });
                ui.end_row();
                ui.label("Auto-arrange");
                ui.checkbox(
                    &mut self.settings.auto_arrange_after_launch,
                    "Apply the selected layout after Client 2+ launches",
                );
                ui.end_row();
                ui.label("Soft client limit");
                ui.add(egui::Slider::new(&mut self.settings.client_limit, 1..=8));
                ui.end_row();
                ui.label("One-click target");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.settings.launch_to_desired_count, "Launch until");
                    ui.add(
                        egui::Slider::new(
                            &mut self.settings.desired_client_count,
                            1..=self.settings.client_limit,
                        )
                        .suffix(" clients"),
                    );
                });
                ui.end_row();
                ui.label("Secondary resource preset");
                egui::ComboBox::from_id_salt("secondary-resource-mode")
                    .selected_text(self.settings.secondary_resource_mode.label())
                    .show_ui(ui, |ui| {
                        for mode in ResourceMode::ALL {
                            ui.selectable_value(
                                &mut self.settings.secondary_resource_mode,
                                mode,
                                mode.label(),
                            );
                        }
                    });
                ui.end_row();
                ui.label("Secondary audio preset");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(&mut self.settings.secondary_volume_percent, 0..=100)
                            .suffix("%"),
                    );
                    ui.checkbox(&mut self.settings.secondary_muted, "Mute");
                });
                ui.end_row();
                ui.label("Global hotkeys");
                ui.checkbox(
                    &mut self.settings.global_hotkeys,
                    "Ctrl+Alt+1 / Ctrl+Alt+2 (off by default)",
                );
                ui.end_row();
                ui.label("Interface");
                ui.checkbox(&mut self.settings.advanced_mode, "Show advanced controls");
                ui.end_row();
                ui.label("Launch timeout");
                ui.add(
                    egui::Slider::new(&mut self.settings.launch_timeout_seconds, 10..=180)
                        .suffix(" seconds"),
                );
                ui.end_row();
                ui.label("Graceful close timeout");
                ui.add(
                    egui::Slider::new(&mut self.settings.graceful_close_seconds, 2..=30)
                        .suffix(" seconds"),
                );
                ui.end_row();
                ui.label("Tray behavior");
                ui.checkbox(
                    &mut self.settings.minimize_to_tray,
                    "Minimize to tray when closing normally",
                );
                ui.end_row();
                ui.label("PowerShell Assist");
                ui.checkbox(
                    &mut self.settings.powershell_assist,
                    "Enable optional assistance",
                );
                ui.end_row();
                ui.label("Automatic update checks");
                ui.checkbox(
                    &mut self.settings.automatic_update_checks,
                    "Check quietly after startup",
                );
                ui.end_row();
                ui.label("Update channel");
                egui::ComboBox::from_id_salt("update-channel")
                    .selected_text(self.settings.update_channel.label())
                    .show_ui(ui, |ui| {
                        for channel in UpdateChannel::ALL {
                            ui.selectable_value(
                                &mut self.settings.update_channel,
                                channel,
                                channel.label(),
                            );
                        }
                    });
                ui.end_row();
            });
        if self.settings.update_channel != previous_channel {
            self.updater.set_channel(self.settings.update_channel);
        }
        if ui.button("Save Settings").clicked() {
            self.settings.normalize();
            if self.settings.global_hotkeys && self.hotkeys.is_none() {
                match HotkeyManager::start() {
                    Ok(manager) => self.hotkeys = Some(manager),
                    Err(message) => {
                        self.settings.global_hotkeys = false;
                        self.status = format!("Global hotkeys could not be enabled: {message}");
                        self.add_activity(self.status.clone(), true);
                    }
                }
            } else if !self.settings.global_hotkeys {
                self.hotkeys = None;
            }
            match self.settings_store.save(&self.settings) {
                Ok(()) => {
                    self.status = "Settings saved.".into();
                    self.add_activity("Settings saved.", false);
                }
                Err(message) => {
                    self.status = format!("Settings could not be saved: {message}");
                    self.recent_error = Some(message);
                }
            }
        }
        ui.horizontal_wrapped(|ui| {
            let transfer = self.data_root.join("settings-transfer.json");
            if ui.button("Export Settings").clicked() {
                match self.settings_store.export_to(&self.settings, &transfer) {
                    Ok(()) => self.status = format!("Settings exported to {}.", transfer.display()),
                    Err(message) => self.status = format!("Settings export failed: {message}"),
                }
            }
            if ui.button("Import Settings").clicked() {
                match self.settings_store.import_from(&transfer) {
                    Ok(settings) => {
                        self.settings = settings;
                        self.updater.set_channel(self.settings.update_channel);
                        self.status = format!(
                            "Settings imported from {}. Save to keep them.",
                            transfer.display()
                        );
                    }
                    Err(message) => self.status = format!("Settings import failed: {message}"),
                }
            }
            if ui.button("Open Application Data").clicked() {
                let _ = platform::shell_launch(&self.data_root, None);
            }
            if ui.button("Reset Application Settings").clicked() {
                self.dialog = Some(Dialog::ResetSettings);
            }
        });
        ui.label(
            RichText::new(format!(
                "Data mode: {} • {}",
                if std::env::current_exe()
                    .ok()
                    .and_then(|path| path.parent().map(|parent| parent.join("portable.flag")))
                    .is_some_and(|path| path.is_file())
                {
                    "Portable"
                } else {
                    "LocalAppData"
                },
                self.data_root.display()
            ))
            .small()
            .color(Color32::from_rgb(155, 159, 172)),
        );
        ui.add_space(12.0);
        ui.label(
            RichText::new("Multi-account mode is intentionally never persisted.")
                .color(Color32::from_rgb(155, 159, 172)),
        );
        ui.add_space(16.0);
        self.update_settings(ui);
    }

    fn update_settings(&mut self, ui: &mut egui::Ui) {
        let blocked = if self.launch.busy() {
            Some("Wait for the active Roblox launch attempt to finish before updating.".into())
        } else {
            updater::update_block_reason(
                self.protection_state != ProtectionState::Disabled,
                self.clients.len(),
            )
        };
        if blocked.is_none() {
            self.updater.mark_ready_if_safe();
        }
        let snapshot = self.updater.snapshot();
        card(ui, "About & Updates", |ui| {
            egui::Grid::new("updater-status")
                .num_columns(2)
                .spacing([24.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Application");
                    ui.label("Roblox Multi-Account Launcher");
                    ui.end_row();
                    ui.label("Current version");
                    ui.label(format!("v{}", snapshot.current_version));
                    ui.end_row();
                    ui.label("Updater configured");
                    ui.label(if snapshot.configured { "Yes" } else { "No" });
                    ui.end_row();
                    ui.label("Channel");
                    ui.label(snapshot.channel.label());
                    ui.end_row();
                    ui.label("Latest known version");
                    ui.label(
                        snapshot
                            .latest_version
                            .as_deref()
                            .map(|version| format!("v{version}"))
                            .unwrap_or_else(|| "Unknown".into()),
                    );
                    ui.end_row();
                    ui.label("Last checked");
                    ui.label(format_last_checked(snapshot.last_checked));
                    ui.end_row();
                    ui.label("Update status");
                    ui.label(snapshot.state.label());
                    ui.end_row();
                    ui.label("Integrity");
                    ui.label("Required SHA-256 manifest");
                    ui.end_row();
                });

            ui.horizontal_wrapped(|ui| {
                for (label, url) in [
                    ("Open GitHub Repository", updater::GITHUB_URL),
                    ("Open Releases", updater::RELEASES_URL),
                    ("Report a Bug", updater::ISSUES_URL),
                ] {
                    if ui.button(label).clicked() {
                        let _ = platform::shell_open_uri(url);
                    }
                }
            });

            if !snapshot.configured {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Updater not configured for a public repository yet.")
                        .color(Color32::from_rgb(155, 159, 172)),
                );
            }
            if let Some(error) = &snapshot.error {
                ui.colored_label(Color32::from_rgb(238, 105, 113), error);
            }
            if let Some(blocked) = &blocked
                && matches!(
                    snapshot.state,
                    UpdateState::UpdateAvailable
                        | UpdateState::ReadyToInstall
                        | UpdateState::WaitingForSafeRestart
                )
            {
                ui.colored_label(Color32::from_rgb(230, 176, 70), blocked);
            }

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        snapshot.configured && !snapshot.state.busy(),
                        egui::Button::new("Check Now"),
                    )
                    .clicked()
                {
                    self.updater.start_check(true);
                }
                if matches!(
                    snapshot.state,
                    UpdateState::Checking | UpdateState::Downloading | UpdateState::Verifying
                ) && ui.button("Cancel").clicked()
                {
                    self.updater.cancel();
                }
                if snapshot.rollback_available && ui.button("Prepare Rollback").clicked() {
                    let live_block = updater::update_block_reason(
                        self.protection_state != ProtectionState::Disabled,
                        self.clients.len(),
                    );
                    if let Some(reason) = live_block {
                        self.dialog = Some(Dialog::UpdateBlocked(reason));
                    } else if let Err(message) = self.updater.prepare_rollback() {
                        self.status = format!("Rollback could not be prepared: {message}");
                    } else {
                        self.status = "Verified previous executable prepared. Use Update & Restart to roll back.".into();
                    }
                }
                if snapshot.state == UpdateState::UpdateAvailable {
                    if ui.button("Update").clicked() {
                        if let Some(reason) = blocked.clone() {
                            self.dialog = Some(Dialog::UpdateBlocked(reason));
                        } else {
                            self.updater.start_download();
                        }
                    }
                    if ui.button("Later").clicked() {
                        self.dismissed_update = snapshot.latest_version.clone();
                        self.status = "Update postponed for this launcher session.".into();
                    }
                }
                if matches!(
                    snapshot.state,
                    UpdateState::ReadyToInstall | UpdateState::WaitingForSafeRestart
                ) && ui.button("Update & Restart").clicked()
                {
                    let live_client_count = platform::enumerate_clients().len();
                    let live_block = if self.launch.busy() {
                        Some(
                            "Wait for the active Roblox launch attempt to finish before updating."
                                .into(),
                        )
                    } else {
                        updater::update_block_reason(
                            self.protection_state != ProtectionState::Disabled,
                            live_client_count,
                        )
                    };
                    if let Some(reason) = live_block {
                        self.updater.mark_waiting_for_safe_restart();
                        self.dialog = Some(Dialog::UpdateBlocked(reason));
                    } else {
                        match self.updater.start_install_helper() {
                            Ok(()) => {
                                self.status = "Installing verified update and restarting…".into();
                                self.exit_confirmed = true;
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            Err(message) => {
                                self.status = format!("Updater failed: {message}");
                                self.recent_error = Some(message.clone());
                                self.add_activity(format!("Updater failed: {message}"), true);
                            }
                        }
                    }
                }
                if let Some(url) = &snapshot.release_url
                    && ui.button("View Release").clicked()
                    && let Err(message) = platform::shell_open_uri(url)
                {
                    self.status = format!("Could not open release page: {message}");
                }
            });

            if let Some(name) = &snapshot.latest_name {
                ui.add_space(8.0);
                ui.label(RichText::new(name).strong());
            }
            if let Some(notes) = &snapshot.release_notes
                && !notes.is_empty()
            {
                egui::ScrollArea::vertical()
                    .max_height(120.0)
                    .show(ui, |ui| {
                        ui.label(notes);
                    });
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        match dialog {
            Dialog::EnableWithClients(pids) => {
                egui::Window::new("Enable Multi-Account Mode?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("{} Roblox client(s) are currently running.", pids.len()));
                    ui.colored_label(Color32::from_rgb(230, 176, 70), "Enabling multi-account mode requires closing them. Active game progress/session will be interrupted.");
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() { keep = false; }
                        if ui.add(egui::Button::new("Close Roblox and Enable").fill(Color32::from_rgb(191, 46, 66))).clicked() { self.start_graceful_close(pids.clone(), true, false); keep = false; }
                    });
                });
                if keep {
                    self.dialog = Some(Dialog::EnableWithClients(pids));
                }
            }
            Dialog::DisableWithClients => {
                egui::Window::new("Disable Multi-Account Mode?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label("Roblox clients are still running. Disabling releases multi-instance and teleport protection.");
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() { keep = false; }
                        if ui.button("Disable Anyway").clicked() { self.disable(); keep = false; }
                        if ui.button("Close Roblox and Disable").clicked() { self.start_graceful_close(self.clients.iter().map(|c| c.pid).collect(), false, false); keep = false; }
                    });
                });
                if keep {
                    self.dialog = Some(Dialog::DisableWithClients);
                }
            }
            Dialog::ExitProtected => {
                egui::Window::new("Exit Launcher?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.colored_label(Color32::from_rgb(230, 176, 70), format!("Multi-account protection is active and {} Roblox client(s) are running.", self.clients.len()));
                    ui.label("Exiting releases multi-instance and teleport protection.");
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Cancel").clicked() { keep = false; }
                        if ui.button("Minimize to Tray").clicked() { self.hidden_to_tray = true; ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false)); keep = false; }
                        if ui.button("Exit Anyway").clicked() { self.exit_after_disable = true; self.disable(); keep = false; }
                        if !self.clients.is_empty() && ui.button("Close Roblox and Exit").clicked() { self.start_graceful_close(self.clients.iter().map(|c| c.pid).collect(), false, true); keep = false; }
                    });
                });
                if keep {
                    self.dialog = Some(Dialog::ExitProtected);
                }
            }
            Dialog::ForceClose {
                pids,
                after_enable,
                exit_after,
            } => {
                egui::Window::new("Roblox is still running").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("Roblox did not close within the configured timeout. Remaining PID(s): {}.", pids.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")));
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() { keep = false; }
                        if ui.add(egui::Button::new("Force Close").fill(Color32::from_rgb(191, 46, 66))).clicked() { self.start_force_close(pids.clone(), after_enable, exit_after); keep = false; }
                    });
                });
                if keep {
                    self.dialog = Some(Dialog::ForceClose {
                        pids,
                        after_enable,
                        exit_after,
                    });
                }
            }
            Dialog::ForceClient(pid) => {
                egui::Window::new("Force Close Roblox")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!(
                            "Force-close Roblox PID {pid}? Unsaved game progress may be lost."
                        ));
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                keep = false;
                            }
                            if ui
                                .add(
                                    egui::Button::new("Force Close")
                                        .fill(Color32::from_rgb(191, 46, 66)),
                                )
                                .clicked()
                            {
                                self.start_force_close(vec![pid], false, false);
                                keep = false;
                            }
                        });
                    });
                if keep {
                    self.dialog = Some(Dialog::ForceClient(pid));
                }
            }
            Dialog::CloseMultiple(pids) => {
                egui::Window::new("Close all Roblox clients?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!(
                            "Close {} Roblox clients? Active game progress may be interrupted.",
                            pids.len()
                        ));
                        ui.label(format!(
                            "PIDs: {}",
                            pids.iter()
                                .map(u32::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                keep = false;
                            }
                            if ui.button("Close Roblox").clicked() {
                                self.start_graceful_close(pids.clone(), false, false);
                                keep = false;
                            }
                        });
                    });
                if keep {
                    self.dialog = Some(Dialog::CloseMultiple(pids));
                }
            }
            Dialog::UpdateBlocked(reason) => {
                egui::Window::new("Update postponed")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.colored_label(Color32::from_rgb(230, 176, 70), reason.clone());
                        ui.label("The launcher will not release protection or terminate Roblox to install an update.");
                        ui.label("Finish Roblox sessions and disable Multi-Account Mode, then try again.");
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                if keep {
                    self.dialog = Some(Dialog::UpdateBlocked(reason));
                }
            }
            Dialog::ResetSettings => {
                egui::Window::new("Reset Application Settings?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label("This resets launcher preferences only. Roblox, Fishstrap, Bloxstrap, browser, and Windows account data are not touched.");
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                keep = false;
                            }
                            if ui.button("Reset Launcher Settings").clicked() {
                                self.settings = Settings::default();
                                self.hotkeys = None;
                                match self.settings_store.save(&self.settings) {
                                    Ok(()) => self.status = "Application settings reset.".into(),
                                    Err(message) => self.status = format!("Settings reset could not be saved: {message}"),
                                }
                                keep = false;
                            }
                        });
                    });
                if keep {
                    self.dialog = Some(Dialog::ResetSettings);
                }
            }
        }
    }
}

impl eframe::App for LauncherApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(250));
        self.poll_events(&ctx);
        self.poll_tray(&ctx);
        if self.last_refresh.elapsed() >= Duration::from_secs(1) {
            self.refresh_clients();
            self.last_refresh = Instant::now();
        }
        if self.last_health.elapsed() >= Duration::from_secs(2) {
            if self.protection_state != ProtectionState::Disabled {
                self.protection.health_check();
            }
            self.last_health = Instant::now();
        }
        if !self.automatic_update_started && self.started.elapsed() >= Duration::from_secs(2) {
            self.automatic_update_started = true;
            if self.settings.automatic_update_checks {
                self.updater.start_check(false);
            }
        }
        if self.smoke_test && self.started.elapsed() > Duration::from_millis(800) {
            self.exit_confirmed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if close_requested && !self.exit_confirmed {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.protection_state != ProtectionState::Disabled {
                self.dialog = Some(Dialog::ExitProtected);
            } else if self.settings.minimize_to_tray && self.tray.is_some() {
                self.hidden_to_tray = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            } else {
                self.exit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        egui::Frame::new()
            .fill(Color32::from_rgb(18, 18, 22))
            .inner_margin(22.0)
            .show(ui, |ui| {
                self.top_bar(ui);
                ui.add_space(10.0);
                self.navigation(ui);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Home => self.home(ui),
                        Page::Clients => self.clients_page(ui),
                        Page::Diagnostics => self.diagnostics_page(ui),
                        Page::PowerShell => self.powershell_page(ui),
                        Page::Settings => self.settings_page(ui),
                    });
            });
        self.dialogs(&ctx);
        if !self.settings.first_run_completed {
            egui::Window::new("Welcome")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    ui.heading("Roblox Multi-Account Launcher");
                    ui.label("Multi-Account Mode always starts off. Enable it before launching concurrent clients.");
                    ui.label("Client 1 uses the selected backend; Client 2+ use isolated junction launch paths.");
                    ui.label("The launcher does not store credentials, read cookie contents, inject into Roblox, or send telemetry.");
                    ui.label("PowerShell 7 assistance is optional; all core protection and launching remain native Rust.");
                    if ui.button("Continue").clicked() {
                        self.settings.first_run_completed = true;
                        let _ = self.settings_store.save(&self.settings);
                    }
                });
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.launch.cancel();
        self.updater.cancel();
        self.stop_local_state_trace();
        self.ps_cancel
            .store(true, std::sync::atomic::Ordering::Release);
        for (pid, mode) in self.resource_modes.clone() {
            if mode != ResourceMode::Normal && platform::process_exists(pid) {
                let _ = platform::set_resource_mode(pid, ResourceMode::Normal);
            }
        }
        let _ = self.settings_store.save(&self.settings);
        let running: HashSet<u32> = platform::enumerate_clients()
            .into_iter()
            .map(|client| client.pid)
            .collect();
        let _ = self.instance_paths.cleanup_exited(&running);
        self.logger.write(
            "INFO",
            "Application shutdown cleanup started; Rust resource owners will now drop.",
        );
    }
}

fn configure_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(18, 18, 22);
    visuals.window_fill = Color32::from_rgb(28, 28, 34);
    visuals.extreme_bg_color = Color32::from_rgb(23, 23, 28);
    visuals.faint_bg_color = Color32::from_rgb(32, 32, 38);
    visuals.selection.bg_fill = Color32::from_rgb(155, 36, 55);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(38, 38, 45);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(55, 48, 55);
    visuals.widgets.active.bg_fill = Color32::from_rgb(191, 46, 66);
    visuals.widgets.noninteractive.fg_stroke.color = Color32::from_rgb(225, 227, 234);
    visuals.widgets.inactive.fg_stroke.color = Color32::from_rgb(220, 222, 230);
    visuals.widgets.hovered.fg_stroke.color = Color32::WHITE;
    visuals.window_corner_radius = 10.0.into();
    ctx.set_visuals(visuals);
}

fn card(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::from_rgb(28, 28, 34))
        .stroke(Stroke::new(1.0, Color32::from_rgb(55, 56, 65)))
        .corner_radius(10.0)
        .inner_margin(14.0)
        .show(ui, |ui| {
            ui.label(
                RichText::new(title)
                    .size(16.0)
                    .strong()
                    .color(Color32::from_rgb(242, 243, 247)),
            );
            ui.add_space(8.0);
            content(ui);
        });
    ui.add_space(8.0);
}

fn format_last_checked(checked: Option<SystemTime>) -> String {
    let Some(checked) = checked else {
        return "Never".into();
    };
    let elapsed = SystemTime::now()
        .duration_since(checked)
        .unwrap_or_default();
    if elapsed < Duration::from_secs(60) {
        "Just now".into()
    } else if elapsed < Duration::from_secs(60 * 60) {
        format!("{} minute(s) ago", elapsed.as_secs() / 60)
    } else {
        format!("{} hour(s) ago", elapsed.as_secs() / 3_600)
    }
}

fn state_row(ui: &mut egui::Ui, label: &str, value: &str, state: ProtectionState) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let color = match state {
                ProtectionState::Protected => Color32::from_rgb(77, 198, 133),
                ProtectionState::Warning | ProtectionState::Preparing => {
                    Color32::from_rgb(230, 176, 70)
                }
                ProtectionState::Lost => Color32::from_rgb(238, 78, 91),
                ProtectionState::Disabled => Color32::from_rgb(165, 169, 181),
            };
            ui.label(RichText::new(value).strong().color(color));
        });
    });
}

fn held_label(value: bool) -> &'static str {
    if value { "HELD" } else { "NOT HELD" }
}

fn format_duration(value: Duration) -> String {
    let seconds = value.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    )
}

fn build_tray() -> (Option<TrayIcon>, Option<TrayIds>) {
    let menu = Menu::new();
    let open = MenuItem::new("Open Launcher", true, None);
    let launch = MenuItem::new("Launch Roblox", true, None);
    let tile = MenuItem::new("Tile Clients", true, None);
    let focus1 = MenuItem::new("Focus Client 1", true, None);
    let focus2 = MenuItem::new("Focus Client 2", true, None);
    let mute_alt = MenuItem::new("Toggle Alt Mute", true, None);
    let resource_alt = MenuItem::new("Toggle Alt Resource Saver", true, None);
    let close = MenuItem::new("Close Roblox", true, None);
    let disable = MenuItem::new("Disable Multi-Account", true, None);
    let exit = MenuItem::new("Exit", true, None);
    if menu
        .append_items(&[
            &open,
            &launch,
            &tile,
            &focus1,
            &focus2,
            &mute_alt,
            &resource_alt,
            &close,
            &disable,
            &PredefinedMenuItem::separator(),
            &exit,
        ])
        .is_err()
    {
        return (None, None);
    }
    let mut rgba = vec![0u8; 32 * 32 * 4];
    for y in 0..32usize {
        for x in 0..32usize {
            let index = (y * 32 + x) * 4;
            let inside = (4..28).contains(&x) && (4..28).contains(&y);
            rgba[index..index + 4].copy_from_slice(if inside {
                &[191, 46, 66, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    let icon = Icon::from_rgba(rgba, 32, 32).ok();
    let tray = icon.and_then(|icon| {
        TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Roblox Multi-Account Launcher")
            .with_icon(icon)
            .build()
            .ok()
    });
    let ids = TrayIds {
        open: open.id().clone(),
        launch: launch.id().clone(),
        tile: tile.id().clone(),
        focus1: focus1.id().clone(),
        focus2: focus2.id().clone(),
        mute_alt: mute_alt.id().clone(),
        resource_alt: resource_alt.id().clone(),
        close: close.id().clone(),
        disable: disable.id().clone(),
        exit: exit.id().clone(),
    };
    (tray, Some(ids))
}
