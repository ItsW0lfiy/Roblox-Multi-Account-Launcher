use crate::model::Settings;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub fn data_root() -> PathBuf {
    data_root_for(
        std::env::var_os("RMAL_DATA_DIR").map(PathBuf::from),
        std::env::current_exe().ok(),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    )
}

fn data_root_for(
    override_root: Option<PathBuf>,
    executable: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
) -> PathBuf {
    if let Some(root) = override_root {
        return root;
    }
    if let Some(directory) = executable.as_ref().and_then(|path| path.parent())
        && directory.join("portable.flag").is_file()
    {
        return directory.join("data");
    }
    if let Some(local) = local_app_data {
        return local.join("RobloxMultiAccountLauncher");
    }
    executable
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

    pub fn export_to(&self, settings: &Settings, destination: &Path) -> Result<(), String> {
        let mut clean = settings.clone();
        clean.normalize();
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(
            destination,
            serde_json::to_vec_pretty(&clean).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    }

    pub fn import_from(&self, source: &Path) -> Result<Settings, String> {
        let metadata = fs::metadata(source).map_err(|error| error.to_string())?;
        if metadata.len() > 256 * 1024 {
            return Err("Settings import is larger than the 256 KiB safety limit.".into());
        }
        let mut settings: Settings =
            serde_json::from_slice(&fs::read(source).map_err(|error| error.to_string())?)
                .map_err(|error| format!("Settings file is not valid: {error}"))?;
        settings.normalize();
        Ok(settings)
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

    #[test]
    fn settings_import_export_is_bounded_and_normalized() {
        let root = PathBuf::from(".tmp").join("tests").join("settings-import");
        let _ = fs::remove_dir_all(&root);
        let store = SettingsStore::new(&root);
        let path = root.join("export.json");
        let settings = Settings {
            client_limit: 99,
            desired_client_count: 77,
            ..Default::default()
        };
        store.export_to(&settings, &path).unwrap();
        let imported = store.import_from(&path).unwrap();
        assert_eq!(imported.client_limit, 8);
        assert_eq!(imported.desired_client_count, 8);
        assert!(!fs::read_to_string(path).unwrap().contains("ROBLOSECURITY"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_flag_and_override_have_intentional_precedence() {
        let root = PathBuf::from(".tmp/tests/portable-data");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("portable.flag"), []).unwrap();
        let executable = root.join("Launcher.exe");
        assert_eq!(
            data_root_for(None, Some(executable.clone()), Some(PathBuf::from("Local"))),
            root.join("data")
        );
        assert_eq!(
            data_root_for(Some(PathBuf::from("Override")), Some(executable), None),
            PathBuf::from("Override")
        );
        let _ = fs::remove_dir_all(root);
    }
}
