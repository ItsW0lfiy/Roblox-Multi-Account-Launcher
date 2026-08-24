use crate::{
    model::{Detection, LaunchBackend, ProtectionState, RobloxClient},
    powershell::PowerShellInfo,
};
use regex::Regex;
use std::sync::OnceLock;

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

pub fn report(
    detection: &Detection,
    selected: LaunchBackend,
    clients: &[RobloxClient],
    protection: ProtectionState,
    singleton_mutex_held: bool,
    singleton_event_held: bool,
    cookie_locked: bool,
    powershell: &PowerShellInfo,
    recent_error: Option<&str>,
) -> String {
    let resolved = detection.resolve(selected);
    redact(&format!(
        "Roblox Multi-Account Launcher\nVersion: {}\nArchitecture: Rust + egui/eframe + native Windows APIs\nMode: {}\n\nROBLOX\nInstalled: {}\nProcesses: {}\n{}: owner: {} ({}, registered: {})\n{}: owner: {} ({}, registered: {})\n\nFISHSTRAP\nDetected: {}\nVersion: {}\nExecutable: {}\n\nBLOXSTRAP\nDetected: {}\nVersion: {}\nExecutable: {}\n\nLAUNCH BACKEND\nSelected: {}\nResolved: {}\n\nMULTI-ACCOUNT\nApplication mutex: Owned\nsingletonMutex: {}\nsingletonEvent compatibility mutex: {}\nCookie file: {}\nTeleport cookie lock: {}\nMulti-instance protection: {}\nLogin-state isolation: Unsupported / not enabled\nLogin-state note: Desktop clients under one Windows profile may share local login state.\n\nPOWERSHELL ASSIST\nEngine: {}\nVersion: {}\nStatus: {}\n\nRecent error: {}",
        env!("CARGO_PKG_VERSION"),
        if protection == ProtectionState::Disabled {
            "Normal"
        } else {
            "Multi-account"
        },
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
        if cookie_locked { "Owned" } else { "Not owned" },
        protection.label(),
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
