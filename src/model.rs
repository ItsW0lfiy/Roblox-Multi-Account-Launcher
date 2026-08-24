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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateChannel {
    Stable,
    Prerelease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientRole {
    Primary,
    Secondary,
    Custom,
}

impl ClientRole {
    pub const ALL: [Self; 3] = [Self::Primary, Self::Secondary, Self::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Self::Primary => "Primary",
            Self::Secondary => "Secondary / Alt",
            Self::Custom => "Custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceMode {
    Normal,
    Balanced,
    AltSaver,
}

impl ResourceMode {
    pub const ALL: [Self; 3] = [Self::Normal, Self::Balanced, Self::AltSaver];

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Balanced => "Balanced",
            Self::AltSaver => "Alt Saver",
        }
    }
}

impl UpdateChannel {
    pub const ALL: [Self; 2] = [Self::Stable, Self::Prerelease];

    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Prerelease => "Prerelease",
        }
    }
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
    pub cpu_time: Duration,
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
    pub automatic_update_checks: bool,
    pub update_channel: UpdateChannel,
    pub auto_arrange_after_launch: bool,
    pub client_limit: usize,
    pub desired_client_count: usize,
    pub launch_to_desired_count: bool,
    pub secondary_resource_mode: ResourceMode,
    pub secondary_volume_percent: u8,
    pub secondary_muted: bool,
    pub global_hotkeys: bool,
    pub advanced_mode: bool,
    pub first_run_completed: bool,
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
            automatic_update_checks: true,
            update_channel: UpdateChannel::Stable,
            auto_arrange_after_launch: false,
            client_limit: 2,
            desired_client_count: 1,
            launch_to_desired_count: false,
            secondary_resource_mode: ResourceMode::Balanced,
            secondary_volume_percent: 35,
            secondary_muted: false,
            global_hotkeys: false,
            advanced_mode: false,
            first_run_completed: false,
        }
    }
}

impl Settings {
    pub fn normalize(&mut self) {
        self.launch_timeout_seconds = self.launch_timeout_seconds.clamp(10, 180);
        self.graceful_close_seconds = self.graceful_close_seconds.clamp(2, 30);
        self.client_limit = self.client_limit.clamp(1, 8);
        self.desired_client_count = self.desired_client_count.clamp(1, self.client_limit);
        self.secondary_volume_percent = self.secondary_volume_percent.min(100);
    }

    pub fn requested_launch_target(&self, current_clients: usize) -> Result<usize, String> {
        if current_clients >= self.client_limit {
            return Err(format!("Client limit reached ({}).", self.client_limit));
        }
        Ok(if self.launch_to_desired_count {
            self.desired_client_count
                .max(current_clients.saturating_add(1))
                .min(self.client_limit)
        } else {
            current_clients.saturating_add(1)
        })
    }
}

#[derive(Debug, Clone)]
pub struct Activity {
    pub timestamp: std::time::SystemTime,
    pub message: String,
    pub error: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desired_client_count_respects_current_count_and_soft_limit() {
        let mut settings = Settings {
            client_limit: 4,
            desired_client_count: 3,
            launch_to_desired_count: true,
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.requested_launch_target(0).unwrap(), 3);
        assert_eq!(settings.requested_launch_target(3).unwrap(), 4);
        assert!(settings.requested_launch_target(4).is_err());
    }
}
