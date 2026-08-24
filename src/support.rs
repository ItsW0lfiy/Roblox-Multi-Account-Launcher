use crate::{diagnostics, history::LaunchHistoryEntry, model::Settings};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn create_bundle(
    root: &Path,
    report: &str,
    settings: &Settings,
    history: &[LaunchHistoryEntry],
) -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let directory = root
        .join("SupportBundles")
        .join(format!("support-{timestamp}"));
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    fs::write(
        directory.join("diagnostics.txt"),
        diagnostics::redact(report),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        directory.join("settings.json"),
        diagnostics::redact(
            &serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?,
        ),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        directory.join("launch-history.json"),
        diagnostics::redact(
            &serde_json::to_string_pretty(history).map_err(|error| error.to_string())?,
        ),
    )
    .map_err(|error| error.to_string())?;
    let log = root.join("logs").join("launcher.log");
    if let Ok(text) = fs::read_to_string(log) {
        fs::write(directory.join("launcher.log"), diagnostics::redact(&text))
            .map_err(|error| error.to_string())?;
    }
    fs::write(
        directory.join("README.txt"),
        "Local sanitized support bundle. Review every file before sharing. This bundle intentionally excludes RobloxCookies.dat, credentials, authentication tickets, private URLs, browser data, and application runtime binaries.\n",
    )
    .map_err(|error| error.to_string())?;
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_bundle_redacts_and_excludes_credentials() {
        let root = PathBuf::from(".tmp/tests/support-bundle");
        let _ = fs::remove_dir_all(&root);
        let bundle = create_bundle(
            &root,
            ".ROBLOSECURITY=secret token=private",
            &Settings::default(),
            &[],
        )
        .unwrap();
        let report = fs::read_to_string(bundle.join("diagnostics.txt")).unwrap();
        assert!(!report.contains("secret"));
        assert!(!report.contains("private"));
        assert!(!bundle.join("RobloxCookies.dat").exists());
        let _ = fs::remove_dir_all(root);
    }
}
