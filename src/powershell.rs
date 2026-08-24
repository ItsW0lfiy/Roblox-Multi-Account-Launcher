use serde_json::Value;
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

pub const DIAGNOSE_ROBLOX: &str = include_str!("../scripts/diagnose_roblox.ps1");
pub const DIAGNOSE_FISHSTRAP: &str = include_str!("../scripts/diagnose_fishstrap.ps1");
pub const DIAGNOSE_BLOXSTRAP: &str = include_str!("../scripts/diagnose_bloxstrap.ps1");
pub const INSPECT_PROTOCOLS: &str = include_str!("../scripts/inspect_protocols.ps1");
pub const INSPECT_PROCESSES: &str = include_str!("../scripts/inspect_processes.ps1");
pub const REPAIR_LAUNCH_STATE: &str = include_str!("../scripts/repair_launch_state.ps1");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerShellKind {
    PowerShell7,
    WindowsPowerShell,
    None,
}

#[derive(Debug, Clone)]
pub struct PowerShellInfo {
    pub kind: PowerShellKind,
    pub executable: Option<PathBuf>,
    pub version: Option<String>,
}

impl PowerShellInfo {
    pub fn engine_label(&self) -> &'static str {
        match self.kind {
            PowerShellKind::PowerShell7 => "PowerShell 7",
            PowerShellKind::WindowsPowerShell => "Windows PowerShell 5.1",
            PowerShellKind::None => "Not available",
        }
    }
    pub fn capability(&self) -> &'static str {
        match self.kind {
            PowerShellKind::PowerShell7 => "Full",
            PowerShellKind::WindowsPowerShell => "Compatibility",
            PowerShellKind::None => "Rust-only mode",
        }
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(';')
        .map(Path::new)
        .map(|path| path.join(name))
        .find(|path| path.is_file())
}

pub fn detect() -> PowerShellInfo {
    let pwsh = find_on_path("pwsh.exe").or_else(|| {
        std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .map(|path| path.join("PowerShell").join("7").join("pwsh.exe"))
            .filter(|path| path.is_file())
    });
    let (kind, executable) = if pwsh.is_some() {
        (PowerShellKind::PowerShell7, pwsh)
    } else {
        let legacy = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .map(|path| {
                path.join("System32")
                    .join("WindowsPowerShell")
                    .join("v1.0")
                    .join("powershell.exe")
            })
            .filter(|path| path.is_file());
        if legacy.is_some() {
            (PowerShellKind::WindowsPowerShell, legacy)
        } else {
            (PowerShellKind::None, None)
        }
    };
    let version = executable
        .as_ref()
        .and_then(|exe| {
            run_command(
                exe,
                "$PSVersionTable.PSVersion.ToString()",
                Duration::from_secs(3),
                Arc::new(AtomicBool::new(false)),
            )
            .ok()
        })
        .map(|value| value.trim().to_string());
    PowerShellInfo {
        kind,
        executable,
        version,
    }
}

fn run_command(
    executable: &Path,
    command: &str,
    timeout: Duration,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    let mut child = Command::new(executable)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            command,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if cancel.load(Ordering::Acquire) {
                "PowerShell Assist command was cancelled.".into()
            } else {
                "PowerShell Assist command timed out.".into()
            });
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|error| error.to_string())?;
                if !output.status.success() {
                    return Err(crate::diagnostics::redact(&String::from_utf8_lossy(
                        &output.stderr,
                    )));
                }
                return Ok(crate::diagnostics::redact(&String::from_utf8_lossy(
                    &output.stdout,
                )));
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                let _ = child.kill();
                return Err(error.to_string());
            }
        }
    }
}

pub fn run_script(
    info: &PowerShellInfo,
    script: &'static str,
    timeout: Duration,
    cancel: Arc<AtomicBool>,
) -> Result<Value, String> {
    let executable = info.executable.as_ref().ok_or(
        "PowerShell is not available. Core launcher features remain available in Rust-only mode.",
    )?;
    let output = run_command(executable, script, timeout, cancel)?;
    serde_json::from_str(output.trim())
        .map_err(|error| format!("PowerShell returned invalid structured output: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_engine_reports_rust_only_mode() {
        let info = PowerShellInfo {
            kind: PowerShellKind::None,
            executable: None,
            version: None,
        };
        assert_eq!(info.capability(), "Rust-only mode");
        assert!(
            run_script(
                &info,
                "'{}'",
                Duration::from_millis(1),
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
    }
    #[test]
    fn structured_output_parsing_and_bounded_timeout() {
        let info = detect();
        if info.executable.is_none() {
            return;
        }
        let value = run_script(
            &info,
            "'{\"status\":\"ok\"}'",
            Duration::from_secs(3),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(value["status"], "ok");
        let timed = run_script(
            &info,
            "Start-Sleep -Seconds 2; '{}'",
            Duration::from_millis(50),
            Arc::new(AtomicBool::new(false)),
        );
        assert!(timed.is_err());
    }
}
