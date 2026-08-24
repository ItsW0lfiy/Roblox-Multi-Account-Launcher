use crate::model::Settings;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub fn data_root() -> PathBuf {
    if let Some(root) = std::env::var_os("RMAL_DATA_DIR") {
        return PathBuf::from(root);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("RobloxMultiAccountLauncher");
    }
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("data")))
        .unwrap_or_else(|| PathBuf::from("data"))
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join("settings.json"),
        }
    }
    pub fn load(&self) -> Settings {
        let mut settings: Settings = fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        settings.normalize();
        settings
    }
    pub fn save(&self, settings: &Settings) -> Result<(), String> {
        let mut clean = settings.clone();
        clean.normalize();
        let parent = self.path.parent().ok_or("Settings path has no parent.")?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(&clean).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        fs::rename(&temporary, &self.path).map_err(|error| error.to_string())
    }
}

pub struct Logger {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Logger {
    pub fn new(root: &Path) -> Self {
        let directory = root.join("logs");
        let _ = fs::create_dir_all(&directory);
        Self {
            path: directory.join("launcher.log"),
            lock: Mutex::new(()),
        }
    }
    pub fn write(&self, level: &str, message: &str) {
        let _guard = self.lock.lock().ok();
        let safe = crate::diagnostics::redact(&message.replace(['\r', '\n'], " "));
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let seconds = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let _ = writeln!(file, "{seconds} [{level}] {safe}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_round_trip_never_contains_multi_account_state() {
        let root = PathBuf::from(".tmp").join("tests").join("settings");
        let _ = fs::remove_dir_all(&root);
        let store = SettingsStore::new(&root);
        let settings = Settings {
            launch_timeout_seconds: 4,
            ..Default::default()
        };
        store.save(&settings).unwrap();
        let loaded = store.load();
        assert_eq!(loaded.launch_timeout_seconds, 10);
        let text = fs::read_to_string(root.join("settings.json")).unwrap();
        assert!(!text.to_ascii_lowercase().contains("multi_account"));
        let _ = fs::remove_dir_all(&root);
    }
}
