use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchBackend {
    Auto,
    Fishstrap,
    Bloxstrap,
    DefaultRoblox,
}

impl LaunchBackend {
    pub const ALL: [Self; 4] = [
        Self::Auto,
        Self::Fishstrap,
        Self::Bloxstrap,
        Self::DefaultRoblox,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto (recommended)",
            Self::Fishstrap => "Fishstrap",
            Self::Bloxstrap => "Bloxstrap",
            Self::DefaultRoblox => "Default Roblox",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutMode {
    FiftyFifty,
    PrimarySecondary,
    Vertical,
}

impl LayoutMode {
    pub const ALL: [Self; 3] = [Self::FiftyFifty, Self::PrimarySecondary, Self::Vertical];
    pub fn label(self) -> &'static str {
        match self {
            Self::FiftyFifty => "Tile 50/50",
            Self::PrimarySecondary => "Primary / Secondary 70/30",
            Self::Vertical => "Vertical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionState {
    Disabled,
    Preparing,
    Protected,
    Warning,
    Lost,
}

impl ProtectionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Preparing => "Preparing",
            Self::Protected => "Protected",
            Self::Warning => "Warning",
            Self::Lost => "Lost",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Installation {
    pub executable: Option<PathBuf>,
    pub version: Option<String>,
    pub base_dir: Option<PathBuf>,
    pub logs_dir: Option<PathBuf>,
}

impl Installation {
    pub fn installed(&self) -> bool {
        self.executable.as_ref().is_some_and(|path| path.is_file())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProtocolInfo {
    pub scheme: String,
    pub command: Option<String>,
    pub executable: Option<PathBuf>,
    pub owner: String,
    pub healthy: bool,
}

#[derive(Debug, Clone)]
pub struct Detection {
    pub fishstrap: Installation,
    pub bloxstrap: Installation,
    pub stock: Installation,
    pub roblox_protocol: ProtocolInfo,
    pub player_protocol: ProtocolInfo,
}

impl Detection {
    pub fn resolve(&self, selected: LaunchBackend) -> LaunchBackend {
        match selected {
            LaunchBackend::Auto if self.fishstrap.installed() => LaunchBackend::Fishstrap,
            LaunchBackend::Auto if self.bloxstrap.installed() => LaunchBackend::Bloxstrap,
            LaunchBackend::Auto => LaunchBackend::DefaultRoblox,
            value => value,
        }
    }
    pub fn get(&self, backend: LaunchBackend) -> &Installation {
        match backend {
            LaunchBackend::Fishstrap => &self.fishstrap,
            LaunchBackend::Bloxstrap => &self.bloxstrap,
            _ => &self.stock,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RobloxClient {
    pub number: usize,
    pub pid: u32,
    pub uptime: Duration,
    pub working_set: u64,
    pub window: isize,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub backend: LaunchBackend,
    pub layout: LayoutMode,
    pub preferred_monitor: Option<String>,
    pub minimize_to_tray: bool,
    pub launch_timeout_seconds: u64,
    pub graceful_close_seconds: u64,
    pub powershell_assist: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            backend: LaunchBackend::Auto,
            layout: LayoutMode::FiftyFifty,
            preferred_monitor: None,
            minimize_to_tray: true,
            launch_timeout_seconds: 45,
            graceful_close_seconds: 8,
            powershell_assist: true,
        }
    }
}

impl Settings {
    pub fn normalize(&mut self) {
        self.launch_timeout_seconds = self.launch_timeout_seconds.clamp(10, 180);
        self.graceful_close_seconds = self.graceful_close_seconds.clamp(2, 30);
    }
}

#[derive(Debug, Clone)]
pub struct Activity {
    pub timestamp: std::time::SystemTime,
    pub message: String,
    pub error: bool,
}
