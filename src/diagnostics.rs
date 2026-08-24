use crate::{
    audio::AudioState,
    instance_paths::InstanceRecord,
    model::{ClientRole, Detection, LaunchBackend, ProtectionState, ResourceMode, RobloxClient},
    powershell::PowerShellInfo,
    updater::UpdateSnapshot,
};
use regex::Regex;
use std::{collections::HashMap, sync::OnceLock};

pub fn redact(text: &str) -> String {
    static SECURITY: OnceLock<Regex> = OnceLock::new();
    static QUERY: OnceLock<Regex> = OnceLock::new();
    static BEARER: OnceLock<Regex> = OnceLock::new();
    let value = SECURITY
        .get_or_init(|| Regex::new(r"(?i)(\.ROBLOSECURITY\s*[:=]\s*)[^\s;]+").unwrap())
        .replace_all(text, "$1[REDACTED]");
    let value = QUERY
        .get_or_init(|| {
            Regex::new(r"(?i)(ticket|token|code|auth|privateServerLinkCode)=([^&\s]+)").unwrap()
        })
        .replace_all(&value, "$1=[REDACTED]");
    BEARER
        .get_or_init(|| Regex::new(r"(?i)(Bearer\s+)[A-Za-z0-9._~-]+").unwrap())
        .replace_all(&value, "$1[REDACTED]")
        .into_owned()
}

pub struct ReportContext<'a> {
    pub detection: &'a Detection,
    pub selected: LaunchBackend,
    pub clients: &'a [RobloxClient],
    pub protection: ProtectionState,
    pub singleton_mutex_held: bool,
    pub singleton_event_held: bool,
    pub cookie_locked: bool,
    pub isolated_instances: &'a [InstanceRecord],
    pub path_isolation_error: Option<&'a str>,
    pub powershell: &'a PowerShellInfo,
    pub recent_error: Option<&'a str>,
    pub updater: &'a UpdateSnapshot,
    pub roles: &'a HashMap<u32, ClientRole>,
    pub resources: &'a HashMap<u32, ResourceMode>,
    pub audio: &'a HashMap<u32, AudioState>,
}

pub fn report(context: ReportContext<'_>) -> String {
    let ReportContext {
        detection,
        selected,
        clients,
        protection,
        singleton_mutex_held,
        singleton_event_held,
        cookie_locked,
        isolated_instances,
        path_isolation_error,
        powershell,
        recent_error,
        updater,
        roles,
        resources,
        audio,
    } = context;
    let resolved = detection.resolve(selected);
    let path_state = if protection == ProtectionState::Disabled {
        "DISABLED"
    } else if path_isolation_error.is_some() {
        "WARNING"
    } else if isolated_instances.is_empty() {
        "READY"
    } else {
        "ACTIVE"
    };
    let instance_details = if isolated_instances.is_empty() {
        "No isolated client paths allocated in this launcher session.".into()
    } else {
        isolated_instances
            .iter()
            .map(|instance| {
                format!(
                    "Client-{:04}\n  PID: {}\n  Launch path: isolated\n  Alias: {}\n  Backend: {}\n  Version: {}\n  Target: {}",
                    instance.client_id,
                    instance
                        .pid
                        .map_or_else(|| "Pending / unconfirmed".into(), |pid| pid.to_string()),
                    instance.alias_path.display(),
                    instance.backend.label(),
                    instance.version,
                    instance.target_version.display()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let client_details = if clients.is_empty() {
        "No Roblox clients detected.".into()
    } else {
        clients
            .iter()
            .map(|client| {
                let role = roles
                    .get(&client.pid)
                    .copied()
                    .unwrap_or(ClientRole::Custom);
                let resource = resources
                    .get(&client.pid)
                    .copied()
                    .unwrap_or(ResourceMode::Normal);
                let audio = audio.get(&client.pid).map_or_else(
                    || "Unavailable".into(),
                    |state| {
                        format!(
                            "{}%, {}, {} session(s)",
                            state.volume_percent,
                            if state.muted { "muted" } else { "unmuted" },
                            state.sessions
                        )
                    },
                );
                format!(
                    "Client {}\n  Role: {}\n  PID: {}\n  Uptime: {} seconds\n  CPU time: {} seconds\n  RAM: {} MiB\n  Window: {}\n  Resource mode: {}\n  Audio: {}",
                    client.number,
                    role.label(),
                    client.pid,
                    client.uptime.as_secs(),
                    client.cpu_time.as_secs(),
                    client.working_set / 1_048_576,
                    if client.window == 0 { "Windowless" } else { "Visible/known" },
                    resource.label(),
                    audio
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    redact(&format!(
        "Roblox Multi-Account Launcher\nVersion: {}\nArchitecture: Rust + egui/eframe + native Windows APIs\nMode: {}\nData root: {}\nRepository: {}\n\nROBLOX\nInstalled: {}\nProcesses: {}\n{}: owner: {} ({}, registered: {})\n{}: owner: {} ({}, registered: {})\n\nFISHSTRAP\nDetected: {}\nVersion: {}\nExecutable: {}\n\nBLOXSTRAP\nDetected: {}\nVersion: {}\nExecutable: {}\n\nLAUNCH BACKEND\nSelected: {}\nResolved: {}\n\nCLIENT DASHBOARD\n{}\n\nMULTI-ACCOUNT\nApplication mutex: Owned\nShared singleton mutex: {}\nShared singleton event: {}\nCookie file: {}\nTeleport/login protection: {}\nMulti-instance protection: {}\nPer-instance path isolation: {}\nPath-isolation root: {}\nPath-isolation error: {}\nLogin-state behavior: Experimental / manually validated on the current configuration\n\nISOLATED CLIENT PATHS\n{}\n\nUPDATER\nConfigured: {}\nChannel: {}\nCurrent version: {}\nLatest known version: {}\nLast checked (Unix seconds): {}\nState: {}\nIntegrity: {}\nRollback available: {}\n\nPOWERSHELL ASSIST\nEngine: {}\nVersion: {}\nStatus: {}\n\nRecent error: {}",
        env!("CARGO_PKG_VERSION"),
        if protection == ProtectionState::Disabled {
            "Normal"
        } else {
            "Multi-account"
        },
        crate::settings::data_root().display(),
        crate::updater::GITHUB_URL,
        detection.stock.installed(),
        clients.len(),
        detection.roblox_protocol.scheme,
        detection.roblox_protocol.owner,
        if detection.roblox_protocol.healthy {
            "Healthy"
        } else {
            "Unavailable"
        },
        detection.roblox_protocol.command.is_some(),
        detection.player_protocol.scheme,
        detection.player_protocol.owner,
        if detection.player_protocol.healthy {
            "Healthy"
        } else {
            "Unavailable"
        },
        detection.player_protocol.command.is_some(),
        detection.fishstrap.installed(),
        detection.fishstrap.version.as_deref().unwrap_or("Unknown"),
        detection
            .fishstrap
            .executable
            .as_ref()
            .map_or("Not found".into(), |p| p.display().to_string()),
        detection.bloxstrap.installed(),
        detection.bloxstrap.version.as_deref().unwrap_or("Unknown"),
        detection
            .bloxstrap
            .executable
            .as_ref()
            .map_or("Not found".into(), |p| p.display().to_string()),
        selected.label(),
        resolved.label(),
        client_details,
        if singleton_mutex_held {
            "HELD"
        } else {
            "NOT HELD"
        },
        if singleton_event_held {
            "HELD"
        } else {
            "NOT HELD"
        },
        crate::platform::cookie_path().is_file(),
        if cookie_locked { "HELD" } else { "NOT HELD" },
        protection.label(),
        path_state,
        crate::settings::data_root().join("Instances").display(),
        path_isolation_error.unwrap_or("None"),
        instance_details,
        if updater.configured { "Yes" } else { "No" },
        updater.channel.label(),
        updater.current_version,
        updater.latest_version.as_deref().unwrap_or("Unknown"),
        updater
            .last_checked
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or_else(|| "Never".into(), |duration| duration.as_secs().to_string()),
        updater.state.label(),
        updater.signing,
        updater.rollback_available,
        powershell.engine_label(),
        powershell.version.as_deref().unwrap_or("Unknown"),
        powershell.capability(),
        recent_error.unwrap_or("None"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_credentials_and_sensitive_uri_fields() {
        let input = ".ROBLOSECURITY=secret ticket=abc token=def Bearer xyz.private privateServerLinkCode=123";
        let output = redact(input);
        assert!(!output.contains("secret"));
        assert!(!output.contains("ticket=abc"));
        assert!(!output.contains("Bearer xyz"));
        assert!(!output.contains("privateServerLinkCode=123"));
    }
}
