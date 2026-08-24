use crate::model::{Detection, LaunchBackend};
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    time::SystemTime,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GENERIC_WRITE, GetLastError, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, MAXIMUM_REPARSE_DATA_BUFFER_SIZE, OPEN_EXISTING,
    },
    System::{
        IO::DeviceIoControl,
        Ioctl::{FSCTL_GET_REPARSE_POINT, FSCTL_SET_REPARSE_POINT},
        SystemServices::IO_REPARSE_TAG_MOUNT_POINT,
    },
};

const CLIENT_PREFIX: &str = "Client-";
const ROBLOX_EXECUTABLE: &str = "RobloxPlayerBeta.exe";

#[derive(Debug, Clone)]
pub struct VersionTarget {
    pub backend: LaunchBackend,
    pub directory: PathBuf,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct IsolatedLaunch {
    pub client_id: u32,
    pub alias_path: PathBuf,
    pub executable: PathBuf,
    pub target_version: PathBuf,
    pub version: String,
    pub backend: LaunchBackend,
}

#[derive(Debug, Clone)]
pub struct InstanceRecord {
    pub client_id: u32,
    pub alias_path: PathBuf,
    pub target_version: PathBuf,
    pub version: String,
    pub backend: LaunchBackend,
    pub pid: Option<u32>,
    pub launch_confirmed: bool,
}

pub struct InstancePathManager {
    root: PathBuf,
    records: Vec<InstanceRecord>,
    next_id: u32,
}

impl InstancePathManager {
    pub fn new(root: PathBuf, clients_running: bool) -> Self {
        let mut manager = Self {
            root,
            records: Vec::new(),
            next_id: 1,
        };
        manager.next_id = manager.next_available_id();
        if !clients_running {
            let _ = manager.cleanup_stale_aliases();
            manager.next_id = manager.next_available_id();
        }
        manager
    }

    pub fn records(&self) -> &[InstanceRecord] {
        &self.records
    }

    pub fn record_for_pid(&self, pid: u32) -> Option<&InstanceRecord> {
        self.records.iter().find(|record| record.pid == Some(pid))
    }

    pub fn allocate(
        &mut self,
        detection: &Detection,
        selected: LaunchBackend,
    ) -> Result<IsolatedLaunch, String> {
        let target = resolve_for_selection(detection, selected)?;
        fs::create_dir_all(&self.root).map_err(|error| {
            format!(
                "Could not create the per-instance path root {}: {error}",
                self.root.display()
            )
        })?;

        let client_id = self.reserve_id()?;
        let alias_path = self.root.join(format!("{CLIENT_PREFIX}{client_id:04}"));
        create_junction(&alias_path, &target.directory)?;
        let executable = alias_path.join(ROBLOX_EXECUTABLE);
        if !executable.is_file() {
            let _ = remove_junction(&alias_path);
            return Err(format!(
                "The isolated path did not resolve to {ROBLOX_EXECUTABLE}: {}",
                executable.display()
            ));
        }

        let launch = IsolatedLaunch {
            client_id,
            alias_path: alias_path.clone(),
            executable,
            target_version: target.directory.clone(),
            version: target.version.clone(),
            backend: target.backend,
        };
        self.records.push(InstanceRecord {
            client_id,
            alias_path,
            target_version: target.directory,
            version: target.version,
            backend: target.backend,
            pid: None,
            launch_confirmed: false,
        });
        Ok(launch)
    }

    pub fn bind_pid(&mut self, client_id: u32, pid: u32) {
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.client_id == client_id)
        {
            record.pid = Some(pid);
            record.launch_confirmed = true;
        }
    }

    pub fn mark_unconfirmed(&mut self, client_id: u32) {
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.client_id == client_id)
        {
            record.launch_confirmed = false;
        }
    }

    pub fn discard_unlaunched(&mut self, client_id: u32) -> Result<(), String> {
        let Some(index) = self
            .records
            .iter()
            .position(|record| record.client_id == client_id && record.pid.is_none())
        else {
            return Ok(());
        };
        let record = self.records.remove(index);
        remove_junction(&record.alias_path)
    }

    pub fn cleanup_exited(&mut self, running_pids: &HashSet<u32>) -> Vec<String> {
        let all_clients_gone = running_pids.is_empty();
        let mut messages = Vec::new();
        let mut retained = Vec::new();
        for record in self.records.drain(..) {
            let safe_to_remove = match record.pid {
                Some(pid) => !running_pids.contains(&pid),
                None => all_clients_gone,
            };
            if !safe_to_remove {
                retained.push(record);
                continue;
            }
            match remove_junction(&record.alias_path) {
                Ok(()) => messages.push(format!(
                    "Released isolated Client-{:04} path.",
                    record.client_id
                )),
                Err(message) => {
                    messages.push(format!(
                        "Could not release isolated Client-{:04} path: {message}",
                        record.client_id
                    ));
                    retained.push(record);
                }
            }
        }
        self.records = retained;
        messages
    }

    fn reserve_id(&mut self) -> Result<u32, String> {
        for _ in 0..u32::MAX {
            let id = self.next_id.max(1);
            self.next_id = id.saturating_add(1);
            if fs::symlink_metadata(self.root.join(format!("{CLIENT_PREFIX}{id:04}"))).is_err() {
                return Ok(id);
            }
        }
        Err("No per-instance client ID is available.".into())
    }

    fn next_available_id(&self) -> u32 {
        fs::read_dir(&self.root)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| parse_client_id(&entry.file_name().to_string_lossy()))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1)
    }

    fn cleanup_stale_aliases(&self) -> Vec<String> {
        let mut messages = Vec::new();
        let Ok(entries) = fs::read_dir(&self.root) else {
            return messages;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if parse_client_id(&name.to_string_lossy()).is_none() {
                continue;
            }
            let path = entry.path();
            if !is_junction(&path) {
                continue;
            }
            match remove_junction(&path) {
                Ok(()) => messages.push(format!("Removed stale alias {}.", path.display())),
                Err(message) => messages.push(message),
            }
        }
        messages
    }
}

pub fn resolve_active_version(
    detection: &Detection,
    backend: LaunchBackend,
) -> Result<VersionTarget, String> {
    let installation = detection.get(backend);
    let mut candidates = Vec::new();
    if let Some(base) = installation.base_dir.as_ref() {
        push_versions_root(&mut candidates, base);
    }
    if let Some(executable) = installation.executable.as_ref() {
        if executable
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(ROBLOX_EXECUTABLE))
        {
            if let Some(parent) = executable.parent() {
                candidates.push(parent.to_path_buf());
            }
        } else if let Some(parent) = executable.parent() {
            push_versions_root(&mut candidates, parent);
        }
    }
    deduplicate_paths(&mut candidates);

    for versions in candidates {
        if let Some(directory) = newest_version_directory(&versions) {
            let version = directory
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown-version".into());
            return Ok(VersionTarget {
                backend,
                directory,
                version,
            });
        }
    }
    Err(format!(
        "{} is installed, but no active version directory containing {ROBLOX_EXECUTABLE} was found. Refresh the backend once so it can finish installing/updating Roblox.",
        backend.label()
    ))
}

fn resolve_for_selection(
    detection: &Detection,
    selected: LaunchBackend,
) -> Result<VersionTarget, String> {
    if selected != LaunchBackend::Auto {
        return resolve_active_version(detection, selected);
    }
    let mut failures = Vec::new();
    for backend in [
        LaunchBackend::Fishstrap,
        LaunchBackend::Bloxstrap,
        LaunchBackend::DefaultRoblox,
    ] {
        if !detection.get(backend).installed() {
            continue;
        }
        match resolve_active_version(detection, backend) {
            Ok(target) => return Ok(target),
            Err(message) => failures.push(message),
        }
    }
    if failures.is_empty() {
        Err("No installed Roblox launch backend was detected.".into())
    } else {
        Err(format!(
            "No installed backend has a usable active Roblox version directory. {}",
            failures.join(" ")
        ))
    }
}

fn push_versions_root(candidates: &mut Vec<PathBuf>, base: &Path) {
    if base
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("Versions"))
    {
        candidates.push(base.to_path_buf());
    } else {
        candidates.push(base.join("Versions"));
    }
}

fn deduplicate_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.to_string_lossy().to_ascii_lowercase()));
}

fn newest_version_directory(versions: &Path) -> Option<PathBuf> {
    fs::read_dir(versions)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let directory = entry.path();
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir() || !directory.join(ROBLOX_EXECUTABLE).is_file() {
                return None;
            }
            let modified = fs::metadata(directory.join(ROBLOX_EXECUTABLE))
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((modified, directory))
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)))
        .map(|(_, directory)| directory)
}

fn parse_client_id(name: &str) -> Option<u32> {
    name.strip_prefix(CLIENT_PREFIX)?.parse().ok()
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn display_target(target: &Path) -> Result<String, String> {
    let canonical = fs::canonicalize(target).map_err(|error| {
        format!(
            "Could not resolve target version directory {}: {error}",
            target.display()
        )
    })?;
    let value = canonical.to_string_lossy();
    let value = value.strip_prefix(r"\\?\").unwrap_or(&value);
    if !Path::new(value).is_absolute() {
        return Err(format!(
            "The target version directory is not absolute: {}",
            target.display()
        ));
    }
    Ok(value.into())
}

fn create_junction(alias: &Path, target: &Path) -> Result<(), String> {
    if fs::symlink_metadata(alias).is_ok() {
        return Err(format!(
            "The per-instance alias already exists and was not replaced: {}",
            alias.display()
        ));
    }
    if !target.join(ROBLOX_EXECUTABLE).is_file() {
        return Err(format!(
            "The target version does not contain {ROBLOX_EXECUTABLE}: {}",
            target.display()
        ));
    }
    let print_name = display_target(target)?;
    fs::create_dir(alias).map_err(|error| {
        format!(
            "Could not create per-instance alias directory {}: {error}",
            alias.display()
        )
    })?;

    let substitute_name = format!(r"\??\{print_name}");
    let substitute: Vec<u16> = OsStr::new(&substitute_name).encode_wide().collect();
    let print: Vec<u16> = OsStr::new(&print_name).encode_wide().collect();
    let path_units = substitute.len() + 1 + print.len() + 1;
    let path_bytes = path_units
        .checked_mul(2)
        .ok_or("The junction target path is too long.")?;
    let reparse_data_length = 8usize
        .checked_add(path_bytes)
        .ok_or("The junction reparse buffer is too large.")?;
    let total_length = 8usize
        .checked_add(reparse_data_length)
        .ok_or("The junction reparse buffer is too large.")?;
    if total_length > MAXIMUM_REPARSE_DATA_BUFFER_SIZE as usize
        || reparse_data_length > u16::MAX as usize
    {
        let _ = fs::remove_dir(alias);
        return Err("The junction target path exceeds the Windows reparse-point limit.".into());
    }

    let mut buffer = vec![0u8; total_length];
    write_u32(&mut buffer, 0, IO_REPARSE_TAG_MOUNT_POINT);
    write_u16(&mut buffer, 4, reparse_data_length as u16);
    write_u16(&mut buffer, 8, 0);
    write_u16(&mut buffer, 10, (substitute.len() * 2) as u16);
    write_u16(&mut buffer, 12, ((substitute.len() + 1) * 2) as u16);
    write_u16(&mut buffer, 14, (print.len() * 2) as u16);
    let mut offset = 16;
    for unit in substitute
        .iter()
        .chain(Some(&0))
        .chain(print.iter())
        .chain(Some(&0))
    {
        write_u16(&mut buffer, offset, *unit);
        offset += 2;
    }

    let alias_wide = wide(alias.as_os_str());
    let handle = unsafe {
        CreateFileW(
            alias_wide.as_ptr(),
            GENERIC_WRITE,
            0,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = unsafe { GetLastError() };
        let _ = fs::remove_dir(alias);
        return Err(format!(
            "Windows could not open the isolated alias for junction setup (error {error})."
        ));
    }
    let mut returned = 0u32;
    let set = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr().cast(),
            buffer.len() as u32,
            null_mut(),
            0,
            &mut returned,
            null_mut(),
        )
    };
    let error = if set == 0 {
        Some(unsafe { GetLastError() })
    } else {
        None
    };
    unsafe {
        CloseHandle(handle);
    }
    if let Some(error) = error {
        let _ = fs::remove_dir(alias);
        return Err(format!(
            "Windows could not create the per-instance junction (error {error})."
        ));
    }
    Ok(())
}

fn is_junction(path: &Path) -> bool {
    let path_wide = wide(path.as_os_str());
    let handle = unsafe {
        CreateFileW(
            path_wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return false;
    }
    let mut buffer = vec![0u8; MAXIMUM_REPARSE_DATA_BUFFER_SIZE as usize];
    let mut returned = 0u32;
    let result = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_GET_REPARSE_POINT,
            null(),
            0,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
            &mut returned,
            null_mut(),
        )
    };
    unsafe {
        CloseHandle(handle);
    }
    result != 0 && returned >= 4 && read_u32(&buffer, 0) == IO_REPARSE_TAG_MOUNT_POINT
}

fn remove_junction(path: &Path) -> Result<(), String> {
    if !path.exists() && fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    if !is_junction(path) {
        return Err(format!(
            "Refusing to remove a non-junction instance path: {}",
            path.display()
        ));
    }
    fs::remove_dir(path).map_err(|error| {
        format!(
            "Could not remove per-instance junction {}: {error}",
            path.display()
        )
    })
}

fn write_u16(buffer: &mut [u8], offset: usize, value: u16) {
    buffer[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_u32(buffer: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(buffer[offset..offset + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Installation, ProtocolInfo};
    use std::{thread, time::Duration};

    fn unique_root(name: &str) -> PathBuf {
        PathBuf::from(".tmp").join("tests").join(format!(
            "{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn fixture_detection(root: &Path) -> Detection {
        let fishstrap = root.join("Fishstrap");
        let stock = root.join("Roblox").join("Versions");
        fs::create_dir_all(fishstrap.join("Versions").join("version-old")).unwrap();
        fs::write(
            fishstrap
                .join("Versions")
                .join("version-old")
                .join(ROBLOX_EXECUTABLE),
            b"MZ-old",
        )
        .unwrap();
        thread::sleep(Duration::from_millis(25));
        fs::create_dir_all(fishstrap.join("Versions").join("version-new")).unwrap();
        fs::write(
            fishstrap
                .join("Versions")
                .join("version-new")
                .join(ROBLOX_EXECUTABLE),
            b"MZ-new",
        )
        .unwrap();
        fs::write(fishstrap.join("Fishstrap.exe"), b"MZ").unwrap();
        fs::create_dir_all(stock.join("version-stock")).unwrap();
        fs::write(
            stock.join("version-stock").join(ROBLOX_EXECUTABLE),
            b"MZ-stock",
        )
        .unwrap();
        Detection {
            fishstrap: Installation {
                executable: Some(fishstrap.join("Fishstrap.exe")),
                version: None,
                base_dir: Some(fishstrap),
                logs_dir: None,
            },
            bloxstrap: Installation {
                executable: None,
                version: None,
                base_dir: None,
                logs_dir: None,
            },
            stock: Installation {
                executable: Some(stock.join("version-stock").join(ROBLOX_EXECUTABLE)),
                version: Some("version-stock".into()),
                base_dir: Some(stock),
                logs_dir: None,
            },
            roblox_protocol: ProtocolInfo::default(),
            player_protocol: ProtocolInfo::default(),
        }
    }

    #[test]
    fn resolver_prefers_selected_backend_active_version() {
        let root = unique_root("instance-resolver");
        let detection = fixture_detection(&root);
        let target = resolve_active_version(&detection, LaunchBackend::Fishstrap).unwrap();
        assert_eq!(target.backend, LaunchBackend::Fishstrap);
        assert_eq!(target.version, "version-new");
        assert!(target.directory.ends_with("Fishstrap/Versions/version-new"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn auto_falls_back_only_when_a_backend_has_no_usable_version() {
        let root = unique_root("instance-auto-fallback");
        let detection = fixture_detection(&root);
        fs::remove_dir_all(root.join("Fishstrap").join("Versions")).unwrap();
        let target = resolve_for_selection(&detection, LaunchBackend::Auto).unwrap();
        assert_eq!(target.backend, LaunchBackend::DefaultRoblox);
        assert_eq!(target.version, "version-stock");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn junction_lifecycle_never_copies_or_removes_target() {
        let root = unique_root("instance-junction");
        let detection = fixture_detection(&root);
        let instances = root.join("Instances");
        let mut manager = InstancePathManager::new(instances.clone(), false);
        let launch = manager
            .allocate(&detection, LaunchBackend::Fishstrap)
            .unwrap();
        assert!(is_junction(&launch.alias_path));
        assert_eq!(fs::read(&launch.executable).unwrap(), b"MZ-new");
        manager.bind_pid(launch.client_id, 4242);
        assert!(manager.cleanup_exited(&HashSet::from([4242])).is_empty());
        assert!(launch.alias_path.exists());
        let messages = manager.cleanup_exited(&HashSet::new());
        assert_eq!(messages.len(), 1);
        assert!(!launch.alias_path.exists());
        assert!(launch.target_version.join(ROBLOX_EXECUTABLE).is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_cleanup_preserves_aliases_when_any_client_is_running() {
        let root = unique_root("instance-stale");
        let detection = fixture_detection(&root);
        let instances = root.join("Instances");
        let mut manager = InstancePathManager::new(instances.clone(), false);
        let launch = manager
            .allocate(&detection, LaunchBackend::Fishstrap)
            .unwrap();
        drop(manager);
        let manager = InstancePathManager::new(instances.clone(), true);
        assert!(launch.alias_path.exists());
        drop(manager);
        let _manager = InstancePathManager::new(instances, false);
        assert!(!launch.alias_path.exists());
        assert!(launch.target_version.join(ROBLOX_EXECUTABLE).is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
