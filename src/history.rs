use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchHistoryEntry {
    pub timestamp_unix: u64,
    pub pid: Option<u32>,
    pub client: Option<String>,
    pub role: String,
    pub backend: String,
    pub version: String,
    pub launch_type: String,
    pub result: String,
    pub exit_classification: Option<String>,
}

pub struct HistoryStore {
    path: PathBuf,
    entries: Vec<LaunchHistoryEntry>,
}

impl HistoryStore {
    pub fn new(root: &Path) -> Self {
        let path = root.join("launch-history.json");
        let entries = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<LaunchHistoryEntry>>(&bytes).ok())
            .unwrap_or_default()
            .into_iter()
            .take(100)
            .collect();
        Self { path, entries }
    }

    pub fn entries(&self) -> &[LaunchHistoryEntry] {
        &self.entries
    }

    pub fn push(&mut self, entry: LaunchHistoryEntry) {
        self.entries.insert(0, entry);
        self.entries.truncate(100);
        let _ = self.save();
    }

    pub fn classify_exit(&mut self, pid: u32, classification: &str) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.pid == Some(pid) && entry.exit_classification.is_none())
        {
            entry.exit_classification = Some(classification.into());
            let _ = self.save();
        }
    }

    fn save(&self) -> Result<(), String> {
        let parent = self.path.parent().ok_or("History path has no parent.")?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(&self.entries).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        fs::rename(temporary, &self.path).map_err(|error| error.to_string())
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_and_exit_classification_is_updated() {
        let root = PathBuf::from(".tmp/tests/history");
        let _ = fs::remove_dir_all(&root);
        let mut store = HistoryStore::new(&root);
        for pid in 1..=105 {
            store.push(LaunchHistoryEntry {
                timestamp_unix: now_unix(),
                pid: Some(pid),
                client: None,
                role: "Custom".into(),
                backend: "Fixture".into(),
                version: "Fixture".into(),
                launch_type: "Fixture".into(),
                result: "Success".into(),
                exit_classification: None,
            });
        }
        assert_eq!(store.entries().len(), 100);
        store.classify_exit(105, "Unexpected exit");
        assert_eq!(
            store.entries()[0].exit_classification.as_deref(),
            Some("Unexpected exit")
        );
        let _ = fs::remove_dir_all(root);
    }
}
