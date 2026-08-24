use crate::{
    diagnostics,
    instance_paths::InstancePathManager,
    launch::{LaunchEvent, LaunchManager},
    local_state::{LocalStateTracer, TraceEvent},
    model::{
        Activity, Detection, LaunchBackend, LayoutMode, ProtectionState, RobloxClient, Settings,
    },
    platform::{self, AppInstanceGuard, ProtectionController, ProtectionEvent},
    powershell::{self, PowerShellInfo, PowerShellKind},
    settings::{self, Logger, SettingsStore},
};
use eframe::egui::{self, Color32, RichText, Stroke};
use std::{
    collections::HashSet,
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
        let instance_paths = InstancePathManager::new(root.join("Instances"), !clients.is_empty());
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
            settings,
            settings_store,
            logger,
            powershell,
            page: Page::Home,
            launch_text: "Launch Roblox".into(),
            status: "Ready. Multi-account mode is off.".into(),
            recent_error: None,
            activities: vec![Activity {
                timestamp: SystemTime::now(),
                message: "Launcher started in normal mode.".into(),
                error: false,
            }],
            last_refresh: Instant::now(),
            last_health: Instant::now(),
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
            }
        }
        for pid in self.previous_pids.clone() {
            if !current.iter().any(|client| client.pid == pid) {
                self.add_activity(format!("Roblox client PID {pid} exited."), false);
            }
        }
        self.previous_pids = current.iter().map(|client| client.pid).collect();
        self.clients = current;
        if !self.launch.busy() {
            let running: HashSet<u32> = self.clients.iter().map(|client| client.pid).collect();
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

    fn start_force_close(&mut self, pids: Vec<u32>, after_enable: bool, exit_after: bool) {
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
        let isolated_launch = if required_protection.is_some() {
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
        match self.launch.begin(
            self.detection.clone(),
            self.settings.backend,
            Duration::from_secs(self.settings.launch_timeout_seconds),
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
                    if let Some(isolation) = isolation {
                        self.instance_paths.bind_pid(isolation.client_id, pid);
                    }
                    self.launch_text = "Roblox launched".into();
                    self.status = format!(
                        "New Roblox client confirmed through {backend} (PID {pid}). {detail}"
                    );
                    self.add_activity(self.status.clone(), warning);
                }
                LaunchEvent::Failed { message, isolation } => {
                    if let Some(isolation) = isolation {
                        self.instance_paths.mark_unconfirmed(isolation.client_id);
                    }
                    self.launch_text = "Launch failed".into();
                    self.status = format!("Launch failed: {message}");
                    self.recent_error = Some(message.clone());
                    self.add_activity(self.status.clone(), true);
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
                if let Some(client) = self.clients.first() {
                    platform::focus_window(client.window);
                }
            } else if event.id == ids.4 {
                if let Some(client) = self.clients.get(1) {
                    platform::focus_window(client.window);
                }
            } else if event.id == ids.5 {
                let pids = self.clients.iter().map(|client| client.pid).collect();
                self.start_graceful_close(pids, false, false);
            } else if event.id == ids.6 {
                if self.launch.busy() {
                    self.status = "Wait for the active Roblox bootstrap to finish before disabling multi-account protection.".into();
                } else if self.clients.is_empty() {
                    self.disable();
                } else {
                    self.dialog = Some(Dialog::DisableWithClients);
                }
            } else if event.id == ids.7 {
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
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(
                    RichText::new("Roblox Multi-Account Launcher")
                        .size(24.0)
                        .color(Color32::from_rgb(244, 246, 250)),
                );
                ui.label(
                    RichText::new(
                        "Rust core • native Windows APIs • optional PowerShell assistance",
                    )
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
            for (page, label) in [
                (Page::Home, "Home"),
                (Page::Clients, "Clients"),
                (Page::Diagnostics, "Diagnostics"),
                (Page::PowerShell, "PowerShell Assist"),
                (Page::Settings, "Settings"),
            ] {
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
            });
            card(&mut columns[1], "Launch Backend", |ui| {
                let resolved = self.detection.resolve(self.settings.backend);
                state_row(ui, "Selected", self.settings.backend.label(), ProtectionState::Disabled);
                state_row(ui, "Resolved", resolved.label(), if self.detection.get(resolved).installed() { ProtectionState::Protected } else { ProtectionState::Warning });
                state_row(ui, "Fishstrap", if self.detection.fishstrap.installed() { "Detected" } else { "Not detected" }, if self.detection.fishstrap.installed() { ProtectionState::Protected } else { ProtectionState::Disabled });
                state_row(ui, "Bloxstrap", if self.detection.bloxstrap.installed() { "Detected" } else { "Not detected" }, if self.detection.bloxstrap.installed() { ProtectionState::Protected } else { ProtectionState::Disabled });
                state_row(ui, "Stock Roblox", if self.detection.stock.installed() { "Detected" } else { "Not detected" }, if self.detection.stock.installed() { ProtectionState::Protected } else { ProtectionState::Warning });
                if ui.button("Refresh Detection").clicked() { self.refresh_detection(); }
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
                self.start_graceful_close(
                    self.clients.iter().map(|client| client.pid).collect(),
                    false,
                    false,
                );
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
            card(
                ui,
                &format!("Client {}  •  PID {}", client.number, client.pid),
                |ui| {
                    ui.label(format!(
                        "Uptime: {}  •  RAM: {:.0} MB  •  Window: {}",
                        format_duration(client.uptime),
                        client.working_set as f64 / 1_048_576.0,
                        client.title
                    ));
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
            });
        }
    }

    fn tile(&mut self) {
        match platform::tile_clients(
            &self.clients,
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
            });
        if ui.button("Save Settings").clicked() {
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
        ui.add_space(12.0);
        ui.label(
            RichText::new("Multi-account mode is intentionally never persisted.")
                .color(Color32::from_rgb(155, 159, 172)),
        );
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
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.launch.cancel();
        self.stop_local_state_trace();
        self.ps_cancel
            .store(true, std::sync::atomic::Ordering::Release);
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
        close: close.id().clone(),
        disable: disable.id().clone(),
        exit: exit.id().clone(),
    };
    (tray, Some(ids))
}
