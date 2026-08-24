use crate::model::{Detection, Installation, LayoutMode, ProtocolInfo, RobloxClient};
use std::{
    collections::HashMap,
    ffi::OsStr,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, FILETIME, GetHandleInformation, GetLastError, HANDLE, HWND,
        INVALID_HANDLE_VALUE, LPARAM, RECT, WAIT_ABANDONED, WAIT_OBJECT_0,
    },
    Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW},
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ,
        GetFileInformationByHandle, OPEN_EXISTING, SYNCHRONIZE,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Registry::{
            HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ,
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
        },
        Threading::{
            CreateMutexW, GetProcessId, GetProcessTimes, OpenProcess,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, ReleaseMutex, TerminateProcess,
            WaitForSingleObject,
        },
    },
    UI::{
        Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::{
            EnumWindows, FindWindowW, GetWindowTextLengthW, GetWindowTextW,
            GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SW_RESTORE, SWP_NOZORDER,
            SetForegroundWindow, SetWindowPos, ShowWindow, WM_CLOSE,
        },
    },
};

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

pub struct AppInstanceGuard {
    handle: HANDLE,
    owned: bool,
}

impl AppInstanceGuard {
    pub fn acquire(name: &str) -> Option<Self> {
        let name = wide(name);
        let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return None;
        }
        let result = unsafe { WaitForSingleObject(handle, 0) };
        if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED {
            unsafe {
                CloseHandle(handle);
            }
            return None;
        }
        Some(Self {
            handle,
            owned: true,
        })
    }
}

impl Drop for AppInstanceGuard {
    fn drop(&mut self) {
        if self.owned {
            unsafe {
                ReleaseMutex(self.handle);
            }
        }
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

pub fn activate_existing_window(title: &str) {
    let title = wide(title);
    unsafe {
        let window = FindWindowW(null(), title.as_ptr());
        if !window.is_null() {
            ShowWindow(window, SW_RESTORE);
            SetForegroundWindow(window);
        }
    }
}

struct MutexGuard {
    handle: HANDLE,
    owned: bool,
}

impl MutexGuard {
    fn acquire(name: &str, timeout: Duration) -> Result<Self, String> {
        let name = wide(name);
        let started = Instant::now();
        loop {
            let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
            if handle.is_null() {
                let error = unsafe { GetLastError() };
                if started.elapsed() >= timeout {
                    return Err(format!(
                        "The named mutex could not be created within {} seconds (Windows error {error}).",
                        timeout.as_secs()
                    ));
                }
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            let result = unsafe { WaitForSingleObject(handle, 0) };
            if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED {
                return Ok(Self {
                    handle,
                    owned: true,
                });
            }
            if started.elapsed() >= timeout {
                unsafe {
                    CloseHandle(handle);
                }
                return Err(format!(
                    "The named mutex is still owned by another process after {} seconds.",
                    timeout.as_secs()
                ));
            }
            unsafe {
                CloseHandle(handle);
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    fn healthy(&self) -> bool {
        let mut flags = 0u32;
        self.owned && unsafe { GetHandleInformation(self.handle, &mut flags) != 0 }
    }
}

impl Drop for MutexGuard {
    fn drop(&mut self) {
        if self.owned {
            unsafe {
                ReleaseMutex(self.handle);
            }
        }
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

struct CookieGuard {
    handle: HANDLE,
}

impl CookieGuard {
    fn acquire(path: &Path) -> Result<Self, String> {
        if !path.is_file() {
            return Err("RobloxCookies.dat was not found. Multiple clients may launch, but multi-client teleports may fail.".into());
        }
        let path = wide(path.as_os_str());
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_GENERIC_READ,
                0,
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!(
                "RobloxCookies.dat could not be opened read-only with exclusive sharing (Windows error {}). Multiple clients may launch, but multi-client teleports may fail.",
                unsafe { GetLastError() }
            ));
        }
        Ok(Self { handle })
    }

    fn healthy(&self) -> bool {
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
        unsafe { GetFileInformationByHandle(self.handle, &mut info) != 0 }
    }
}

impl Drop for CookieGuard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProtectionEvent {
    Enabled {
        singleton_mutex: bool,
        singleton_event: bool,
        cookie: bool,
        message: String,
    },
    Disabled,
    Failed(String),
    Lost(String),
}

enum ProtectionCommand {
    Enable {
        timeout: Duration,
        cookie_path: PathBuf,
    },
    Disable,
    Health,
    Shutdown,
}

pub struct ProtectionController {
    commands: mpsc::Sender<ProtectionCommand>,
    pub events: mpsc::Receiver<ProtectionEvent>,
    thread: Option<thread::JoinHandle<()>>,
    health: ProtectionHealth,
}

const HEALTH_SINGLETON_MUTEX: u8 = 0b001;
const HEALTH_SINGLETON_EVENT: u8 = 0b010;
const HEALTH_COOKIE: u8 = 0b100;

#[derive(Clone)]
pub struct ProtectionHealth {
    bits: Arc<AtomicU8>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProtectionSnapshot {
    pub singleton_mutex: bool,
    pub singleton_event: bool,
    pub cookie: bool,
}

impl ProtectionHealth {
    pub fn snapshot(&self) -> ProtectionSnapshot {
        let bits = self.bits.load(Ordering::Acquire);
        ProtectionSnapshot {
            singleton_mutex: bits & HEALTH_SINGLETON_MUTEX != 0,
            singleton_event: bits & HEALTH_SINGLETON_EVENT != 0,
            cookie: bits & HEALTH_COOKIE != 0,
        }
    }
}

impl ProtectionController {
    pub fn new(mutex_name: &str, event_name: &str) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let mutex_name = mutex_name.to_owned();
        let event_name = event_name.to_owned();
        let health = ProtectionHealth {
            bits: Arc::new(AtomicU8::new(0)),
        };
        let owner_health = health.clone();
        let owner = thread::Builder::new()
            .name("multi-account-resource-owner".into())
            .spawn(move || {
                let mut singleton_mutex: Option<MutexGuard> = None;
                // This is deliberately a mutex occupying the event name. Mutexes and events share
                // the BaseNamedObjects namespace, so Roblox cannot create its singleton event.
                let mut singleton_event: Option<MutexGuard> = None;
                let mut cookie: Option<CookieGuard> = None;
                while let Ok(command) = command_rx.recv() {
                    match command {
                        ProtectionCommand::Enable {
                            timeout,
                            cookie_path,
                        } => {
                            if singleton_mutex.is_some() && singleton_event.is_some() {
                                let _ = event_tx.send(ProtectionEvent::Enabled {
                                    singleton_mutex: true,
                                    singleton_event: true,
                                    cookie: cookie.is_some(),
                                    message: "Multi-account protection is already prepared.".into(),
                                });
                                continue;
                            }
                            let started = Instant::now();
                            match MutexGuard::acquire(&mutex_name, timeout) {
                                Ok(mutex_guard) => {
                                    let remaining = timeout.saturating_sub(started.elapsed());
                                    match MutexGuard::acquire(&event_name, remaining) {
                                        Ok(event_guard) => {
                                            singleton_mutex = Some(mutex_guard);
                                            singleton_event = Some(event_guard);
                                            owner_health.bits.store(
                                                HEALTH_SINGLETON_MUTEX | HEALTH_SINGLETON_EVENT,
                                                Ordering::Release,
                                            );
                                        }
                                        Err(message) => {
                                            drop(mutex_guard);
                                            owner_health.bits.store(0, Ordering::Release);
                                            let _ = event_tx.send(ProtectionEvent::Failed(format!(
                                                "ROBLOX_singletonEvent could not be held as a compatibility mutex: {message}"
                                            )));
                                            continue;
                                        }
                                    }
                                    match CookieGuard::acquire(&cookie_path) {
                                        Ok(handle) => {
                                            cookie = Some(handle);
                                            owner_health.bits.fetch_or(
                                                HEALTH_COOKIE,
                                                Ordering::AcqRel,
                                            );
                                            let _ = event_tx.send(ProtectionEvent::Enabled {
                                                singleton_mutex: true,
                                                singleton_event: true,
                                                cookie: true,
                                                message: "Multi-account mode is ready.".into(),
                                            });
                                        }
                                        Err(message) => {
                                            let _ = event_tx.send(ProtectionEvent::Enabled {
                                                singleton_mutex: true,
                                                singleton_event: true,
                                                cookie: false,
                                                message,
                                            });
                                        }
                                    }
                                }
                                Err(message) => {
                                    owner_health.bits.store(0, Ordering::Release);
                                    let _ = event_tx.send(ProtectionEvent::Failed(format!(
                                        "ROBLOX_singletonMutex could not be acquired: {message}"
                                    )));
                                }
                            }
                        }
                        ProtectionCommand::Disable => {
                            cookie = None;
                            singleton_event = None;
                            singleton_mutex = None;
                            owner_health.bits.store(0, Ordering::Release);
                            let _ = event_tx.send(ProtectionEvent::Disabled);
                        }
                        ProtectionCommand::Health => {
                            let mutex_invalid = singleton_mutex
                                .as_ref()
                                .is_some_and(|guard| !guard.healthy());
                            let event_invalid = singleton_event
                                .as_ref()
                                .is_some_and(|guard| !guard.healthy());
                            if mutex_invalid || event_invalid {
                                cookie = None;
                                singleton_event = None;
                                singleton_mutex = None;
                                owner_health.bits.store(0, Ordering::Release);
                                let _ = event_tx.send(ProtectionEvent::Lost(
                                    "A Roblox singleton-object handle is no longer valid.".into(),
                                ));
                            } else if cookie.as_ref().is_some_and(|guard| !guard.healthy()) {
                                cookie = None;
                                owner_health
                                    .bits
                                    .fetch_and(!HEALTH_COOKIE, Ordering::AcqRel);
                                let _ = event_tx.send(ProtectionEvent::Enabled {
                                    singleton_mutex: true,
                                    singleton_event: true,
                                    cookie: false,
                                    message: "The exclusive cookie-file handle is no longer valid. Multi-instance protection remains active, but multi-client teleports may fail.".into(),
                                });
                            }
                        }
                        ProtectionCommand::Shutdown => break,
                    }
                }
                drop(cookie);
                drop(singleton_event);
                drop(singleton_mutex);
                owner_health.bits.store(0, Ordering::Release);
            })
            .expect("resource-owner thread");
        Self {
            commands: command_tx,
            events: event_rx,
            thread: Some(owner),
            health,
        }
    }

    pub fn enable(&self, timeout: Duration, cookie_path: PathBuf) -> Result<(), String> {
        self.commands
            .send(ProtectionCommand::Enable {
                timeout,
                cookie_path,
            })
            .map_err(|_| "Protection owner thread is unavailable.".into())
    }
    pub fn disable(&self) {
        let _ = self.commands.send(ProtectionCommand::Disable);
    }
    pub fn health_check(&self) {
        let _ = self.commands.send(ProtectionCommand::Health);
    }
    pub fn health(&self) -> ProtectionHealth {
        self.health.clone()
    }
}

impl Drop for ProtectionController {
    fn drop(&mut self) {
        let _ = self.commands.send(ProtectionCommand::Shutdown);
        if let Some(owner) = self.thread.take() {
            let _ = owner.join();
        }
    }
}

pub fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

fn empty_installation() -> Installation {
    Installation {
        executable: None,
        version: None,
        base_dir: None,
        logs_dir: None,
    }
}

fn find_named_executable(base: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .map(|name| base.join(name))
        .find(|path| path.is_file())
        .or_else(|| {
            std::fs::read_dir(base)
                .ok()?
                .flatten()
                .map(|entry| entry.path())
                .find(|path| {
                    path.extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                        && path.file_stem().is_some_and(|stem| {
                            names.iter().any(|name| {
                                stem.to_string_lossy().to_ascii_lowercase().starts_with(
                                    &name.trim_end_matches(".exe").to_ascii_lowercase(),
                                )
                            })
                        })
                })
        })
}

pub fn detect_launchers(root: Option<&Path>) -> Detection {
    let use_registered_protocols = root.is_none();
    let local = root
        .map(PathBuf::from)
        .or_else(local_app_data)
        .unwrap_or_default();
    let roblox_protocol = read_protocol("roblox");
    let player_protocol = read_protocol("roblox-player");
    let fish_dir = local.join("Fishstrap");
    let blox_dir = local.join("Bloxstrap");
    let fish_exe = find_named_executable(&fish_dir, &["Fishstrap.exe"]).or_else(|| {
        (use_registered_protocols && roblox_protocol.owner == "Fishstrap")
            .then(|| roblox_protocol.executable.clone())
            .flatten()
            .filter(|path| path.is_file())
    });
    let blox_exe = find_named_executable(&blox_dir, &["Bloxstrap.exe", "Bloxstrap-v2.0.0.exe"])
        .or_else(|| {
            (use_registered_protocols && roblox_protocol.owner == "Bloxstrap")
                .then(|| roblox_protocol.executable.clone())
                .flatten()
                .filter(|path| path.is_file())
        });
    let mut stock = empty_installation();
    if use_registered_protocols
        && roblox_protocol.owner == "Default Roblox"
        && roblox_protocol.healthy
    {
        stock.executable = roblox_protocol.executable.clone();
    }
    let versions = local.join("Roblox").join("Versions");
    if let Ok(entries) = std::fs::read_dir(&versions) {
        stock.executable = entries
            .flatten()
            .map(|entry| entry.path().join("RobloxPlayerBeta.exe"))
            .filter(|path| path.is_file())
            .max_by_key(|path| {
                std::fs::metadata(path)
                    .and_then(|meta| meta.modified())
                    .ok()
            });
    }
    stock.base_dir = versions.is_dir().then_some(versions);
    let logs = local.join("Roblox").join("logs");
    stock.logs_dir = logs.is_dir().then_some(logs);
    let fishstrap = Installation {
        executable: fish_exe,
        version: None,
        base_dir: fish_dir.is_dir().then_some(fish_dir.clone()),
        logs_dir: [fish_dir.join("Logs"), fish_dir.join("logs")]
            .into_iter()
            .find(|p| p.is_dir()),
    };
    let bloxstrap = Installation {
        executable: blox_exe,
        version: None,
        base_dir: blox_dir.is_dir().then_some(blox_dir.clone()),
        logs_dir: [blox_dir.join("Logs"), blox_dir.join("logs")]
            .into_iter()
            .find(|p| p.is_dir()),
    };
    Detection {
        fishstrap,
        bloxstrap,
        stock,
        roblox_protocol,
        player_protocol,
    }
}

pub fn parse_protocol_command(scheme: &str, command: Option<String>) -> ProtocolInfo {
    let executable = command.as_deref().and_then(|value| {
        let trimmed = value.trim();
        let extracted = if let Some(rest) = trimmed.strip_prefix('"') {
            rest.split('"').next()
        } else {
            trimmed.split_whitespace().next()
        };
        extracted
            .filter(|value| value.to_ascii_lowercase().ends_with(".exe"))
            .map(PathBuf::from)
    });
    let owner = executable
        .as_ref()
        .map(|path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase()
        })
        .map(|name| {
            if name.contains("fishstrap") {
                "Fishstrap"
            } else if name.contains("bloxstrap") {
                "Bloxstrap"
            } else if name.contains("roblox") {
                "Default Roblox"
            } else {
                "Unknown"
            }
        })
        .unwrap_or("Unregistered")
        .to_string();
    let healthy = executable.as_ref().is_some_and(|path| path.is_file());
    ProtocolInfo {
        scheme: scheme.into(),
        command,
        executable,
        owner,
        healthy,
    }
}

fn query_registry_string(root: HKEY, subkey: &str) -> Option<String> {
    let key_name = wide(subkey);
    let mut key: HKEY = null_mut();
    if unsafe { RegOpenKeyExW(root, key_name.as_ptr(), 0, KEY_READ, &mut key) } != 0 {
        return None;
    }
    let mut size = 0u32;
    let mut kind = 0u32;
    let first =
        unsafe { RegQueryValueExW(key, null(), null_mut(), &mut kind, null_mut(), &mut size) };
    if first != 0 || kind != REG_SZ || size == 0 {
        unsafe {
            RegCloseKey(key);
        }
        return None;
    }
    let mut buffer = vec![0u16; size as usize / 2 + 1];
    let second = unsafe {
        RegQueryValueExW(
            key,
            null(),
            null_mut(),
            &mut kind,
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    unsafe {
        RegCloseKey(key);
    }
    if second != 0 {
        return None;
    }
    let end = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

pub fn read_protocol(scheme: &str) -> ProtocolInfo {
    let subkey = format!(r"Software\Classes\{}\shell\open\command", scheme);
    let command = query_registry_string(HKEY_CURRENT_USER, &subkey)
        .or_else(|| {
            query_registry_string(
                HKEY_CLASSES_ROOT,
                &format!(r"{}\shell\open\command", scheme),
            )
        })
        .or_else(|| query_registry_string(HKEY_LOCAL_MACHINE, &subkey));
    parse_protocol_command(scheme, command)
}

#[derive(Default)]
struct WindowMap(HashMap<u32, (isize, String)>);

unsafe extern "system" fn enum_window(window: HWND, param: LPARAM) -> i32 {
    if unsafe { IsWindowVisible(window) } == 0 {
        return 1;
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut pid);
    }
    if pid == 0 {
        return 1;
    }
    let length = unsafe { GetWindowTextLengthW(window) };
    let mut buffer = vec![0u16; length.max(0) as usize + 1];
    unsafe {
        GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32);
    }
    let title = String::from_utf16_lossy(&buffer[..length.max(0) as usize]);
    let map = unsafe { &mut *(param as *mut WindowMap) };
    map.0.entry(pid).or_insert((
        window as isize,
        if title.is_empty() {
            "Roblox".into()
        } else {
            title
        },
    ));
    1
}

fn filetime_to_u64(value: FILETIME) -> u64 {
    ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
}

pub fn enumerate_clients() -> Vec<RobloxClient> {
    let mut windows = WindowMap::default();
    unsafe {
        EnumWindows(Some(enum_window), &mut windows as *mut _ as LPARAM);
    }
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return vec![];
    }
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut result = Vec::new();
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) };
    while ok != 0 {
        let end = entry
            .szExeFile
            .iter()
            .position(|ch| *ch == 0)
            .unwrap_or(entry.szExeFile.len());
        if String::from_utf16_lossy(&entry.szExeFile[..end])
            .eq_ignore_ascii_case("RobloxPlayerBeta.exe")
        {
            let handle = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
                    0,
                    entry.th32ProcessID,
                )
            };
            let mut uptime = Duration::ZERO;
            let mut memory = 0;
            if !handle.is_null() {
                let mut created: FILETIME = unsafe { zeroed() };
                let mut exited: FILETIME = unsafe { zeroed() };
                let mut kernel: FILETIME = unsafe { zeroed() };
                let mut user: FILETIME = unsafe { zeroed() };
                if unsafe {
                    GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user)
                } != 0
                {
                    let now_ticks = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                        / 100
                        + 116444736000000000u128;
                    uptime = Duration::from_micros(
                        ((now_ticks as u64).saturating_sub(filetime_to_u64(created))) / 10,
                    );
                }
                let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { zeroed() };
                counters.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
                if unsafe { K32GetProcessMemoryInfo(handle, &mut counters, counters.cb) } != 0 {
                    memory = counters.WorkingSetSize as u64;
                }
                unsafe {
                    CloseHandle(handle);
                }
            }
            let (window, title) = windows
                .0
                .get(&entry.th32ProcessID)
                .cloned()
                .unwrap_or((0, "Roblox".into()));
            result.push(RobloxClient {
                number: result.len() + 1,
                pid: entry.th32ProcessID,
                uptime,
                working_set: memory,
                window,
                title,
            });
        }
        ok = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    unsafe {
        CloseHandle(snapshot);
    }
    result.sort_by_key(|client| client.pid);
    for (index, client) in result.iter_mut().enumerate() {
        client.number = index + 1;
    }
    result
}

pub fn roblox_bootstrap_processes() -> Vec<(u32, String)> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return vec![];
    }
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut result = Vec::new();
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) };
    while ok != 0 {
        let end = entry
            .szExeFile
            .iter()
            .position(|ch| *ch == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        if [
            "RobloxPlayerLauncher.exe",
            "RobloxPlayerInstaller.exe",
            "RobloxInstaller.exe",
        ]
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
        {
            result.push((entry.th32ProcessID, name));
        }
        ok = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    unsafe {
        CloseHandle(snapshot);
    }
    result
}

pub fn process_exists(pid: u32) -> bool {
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let running = unsafe { WaitForSingleObject(handle, 0) != WAIT_OBJECT_0 };
    unsafe {
        CloseHandle(handle);
    }
    running
}

pub fn request_close(pids: &[u32]) {
    let targets: std::collections::HashSet<u32> = pids.iter().copied().collect();
    unsafe extern "system" fn close_window(window: HWND, param: LPARAM) -> i32 {
        let targets = unsafe { &*(param as *const std::collections::HashSet<u32>) };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(window, &mut pid);
        }
        if targets.contains(&pid) {
            unsafe {
                PostMessageW(window, WM_CLOSE, 0, 0);
            }
        }
        1
    }
    unsafe {
        EnumWindows(Some(close_window), &targets as *const _ as LPARAM);
    }
}

pub fn force_close(pids: &[u32]) -> Vec<u32> {
    pids.iter()
        .copied()
        .filter(|pid| {
            let handle = unsafe { OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE, 0, *pid) };
            if handle.is_null() {
                return process_exists(*pid);
            }
            let failed = unsafe { TerminateProcess(handle, 1) == 0 };
            unsafe {
                CloseHandle(handle);
            }
            failed
        })
        .collect()
}

pub fn focus_window(window: isize) -> bool {
    if window == 0 {
        return false;
    }
    unsafe {
        ShowWindow(window as HWND, SW_RESTORE);
        SetForegroundWindow(window as HWND) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub fn calculate_layout(area: Area, mode: LayoutMode, swapped: bool) -> (Area, Area) {
    let (mut first, mut second) = match mode {
        LayoutMode::PrimarySecondary => {
            let width = ((area.width as f32) * 0.70).round() as i32;
            (
                Area { width, ..area },
                Area {
                    x: area.x + width,
                    width: area.width - width,
                    ..area
                },
            )
        }
        LayoutMode::Vertical => {
            let height = area.height / 2;
            (
                Area { height, ..area },
                Area {
                    y: area.y + height,
                    height: area.height - height,
                    ..area
                },
            )
        }
        LayoutMode::FiftyFifty => {
            let width = area.width / 2;
            (
                Area { width, ..area },
                Area {
                    x: area.x + width,
                    width: area.width - width,
                    ..area
                },
            )
        }
    };
    if swapped {
        std::mem::swap(&mut first, &mut second);
    }
    (first, second)
}

#[derive(Debug, Clone)]
pub struct Monitor {
    pub name: String,
    pub work: Area,
}

unsafe extern "system" fn enum_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    param: LPARAM,
) -> i32 {
    let mut info: MONITORINFOEXW = unsafe { zeroed() };
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) } != 0 {
        let end = info
            .szDevice
            .iter()
            .position(|ch| *ch == 0)
            .unwrap_or(info.szDevice.len());
        let rect = info.monitorInfo.rcWork;
        let monitors = unsafe { &mut *(param as *mut Vec<Monitor>) };
        monitors.push(Monitor {
            name: String::from_utf16_lossy(&info.szDevice[..end]),
            work: Area {
                x: rect.left,
                y: rect.top,
                width: rect.right - rect.left,
                height: rect.bottom - rect.top,
            },
        });
    }
    1
}

pub fn monitors() -> Vec<Monitor> {
    let mut result = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(enum_monitor),
            &mut result as *mut _ as LPARAM,
        );
    }
    result
}

pub fn tile_clients(
    clients: &[RobloxClient],
    mode: LayoutMode,
    monitor_name: Option<&str>,
    swapped: bool,
) -> Result<(), String> {
    if clients.len() != 2 || clients.iter().any(|client| client.window == 0) {
        return Err("Exactly two Roblox clients with visible windows are required.".into());
    }
    let monitors = monitors();
    let monitor = monitor_name
        .and_then(|name| {
            monitors
                .iter()
                .find(|monitor| monitor.name.eq_ignore_ascii_case(name))
        })
        .or(monitors.first())
        .ok_or("No monitor work area was available.")?;
    let layout = calculate_layout(monitor.work, mode, swapped);
    for (client, area) in clients.iter().zip([layout.0, layout.1]) {
        unsafe {
            ShowWindow(client.window as HWND, SW_RESTORE);
            if SetWindowPos(
                client.window as HWND,
                null_mut(),
                area.x,
                area.y,
                area.width,
                area.height,
                SWP_NOZORDER,
            ) == 0
            {
                return Err(format!("Windows could not move Client {}.", client.number));
            }
        }
    }
    Ok(())
}

pub fn shell_launch(file: &Path, arguments: Option<&str>) -> Result<Option<u32>, String> {
    let file = wide(file.as_os_str());
    let verb = wide("open");
    let parameters = arguments.map(wide);
    let mut info: SHELLEXECUTEINFOW = unsafe { zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ref().map_or(null(), |value| value.as_ptr());
    info.nShow = 1;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(format!(
            "Windows could not start the launcher (error {}).",
            unsafe { GetLastError() }
        ));
    }
    let pid = if info.hProcess.is_null() {
        None
    } else {
        let pid = unsafe { GetProcessId(info.hProcess) };
        unsafe {
            CloseHandle(info.hProcess);
        }
        (pid != 0).then_some(pid)
    };
    Ok(pid)
}

pub fn shell_open_uri(uri: &str) -> Result<Option<u32>, String> {
    let value = Path::new(uri);
    shell_launch(value, None)
}

pub fn roblox_local_storage_path() -> PathBuf {
    local_app_data()
        .unwrap_or_default()
        .join("Roblox")
        .join("LocalStorage")
}

pub fn cookie_path() -> PathBuf {
    roblox_local_storage_path().join("RobloxCookies.dat")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LaunchBackend;

    fn unique(name: &str) -> String {
        format!(
            "{}_{}_{}",
            name,
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[test]
    fn layout_calculations_cover_all_modes_and_swap() {
        let area = Area {
            x: 10,
            y: 20,
            width: 1000,
            height: 800,
        };
        assert_eq!(
            calculate_layout(area, LayoutMode::FiftyFifty, false)
                .0
                .width,
            500
        );
        assert_eq!(
            calculate_layout(area, LayoutMode::PrimarySecondary, false)
                .0
                .width,
            700
        );
        assert_eq!(calculate_layout(area, LayoutMode::Vertical, false).1.y, 420);
        assert_eq!(
            calculate_layout(area, LayoutMode::FiftyFifty, true).0.x,
            510
        );
    }

    #[test]
    fn protocol_parser_is_read_only_and_classifies_owner() {
        let info = parse_protocol_command("roblox", Some(r#""C:\Apps\Fishstrap.exe" "%1""#.into()));
        assert_eq!(info.owner, "Fishstrap");
        assert_eq!(
            info.executable.unwrap(),
            PathBuf::from(r"C:\Apps\Fishstrap.exe")
        );
    }

    #[test]
    fn process_enumeration_is_safe() {
        let _ = enumerate_clients();
    }

    #[test]
    fn real_protocol_inspection_is_read_only() {
        let _ = read_protocol("roblox");
        let _ = read_protocol("roblox-player");
    }

    #[test]
    fn application_mutex_lifecycle_is_reacquirable() {
        let name = unique("Wolfy_RMAL_App_Test");
        let first = AppInstanceGuard::acquire(&name).expect("first app mutex");
        let contender_name = name.clone();
        assert!(
            thread::spawn(move || AppInstanceGuard::acquire(&contender_name).is_none())
                .join()
                .unwrap()
        );
        drop(first);
        assert!(AppInstanceGuard::acquire(&name).is_some());
    }

    #[test]
    fn isolated_mutex_and_cookie_handles_release_and_reacquire() {
        let root = PathBuf::from(".tmp")
            .join("tests")
            .join(unique("protection"));
        std::fs::create_dir_all(&root).unwrap();
        let fixture = root.join("RobloxCookies.dat");
        std::fs::write(&fixture, b"fixture-only-not-a-cookie").unwrap();
        let name = unique("ROBLOX_singletonMutex_test");
        let event_name = unique("ROBLOX_singletonEvent_test");
        let mutex = MutexGuard::acquire(&name, Duration::from_millis(50)).unwrap();
        let event = MutexGuard::acquire(&event_name, Duration::from_millis(50)).unwrap();
        let contender_name = name.clone();
        let contender_event_name = event_name.clone();
        assert!(
            thread::spawn(
                move || MutexGuard::acquire(&contender_name, Duration::from_millis(25)).is_err()
            )
            .join()
            .unwrap()
        );
        assert!(
            thread::spawn(move || {
                MutexGuard::acquire(&contender_event_name, Duration::from_millis(25)).is_err()
            })
            .join()
            .unwrap()
        );
        let cookie = CookieGuard::acquire(&fixture).unwrap();
        assert!(CookieGuard::acquire(&fixture).is_err());
        drop(cookie);
        drop(event);
        drop(mutex);
        assert!(MutexGuard::acquire(&name, Duration::from_millis(50)).is_ok());
        assert!(MutexGuard::acquire(&event_name, Duration::from_millis(50)).is_ok());
        assert!(CookieGuard::acquire(&fixture).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn protection_controller_partial_failure_and_cleanup_remain_alive() {
        let name = unique("ROBLOX_singletonMutex_controller_test");
        let event_name = unique("ROBLOX_singletonEvent_controller_test");
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let holder_name = name.clone();
        let holder = thread::spawn(move || {
            let _guard = MutexGuard::acquire(&holder_name, Duration::from_millis(100)).unwrap();
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();

        let controller = ProtectionController::new(&name, &event_name);
        controller
            .enable(
                Duration::from_millis(100),
                PathBuf::from(".tmp/tests/missing-cookie"),
            )
            .unwrap();
        let event = controller
            .events
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(matches!(event, ProtectionEvent::Failed(_)));

        release_tx.send(()).unwrap();
        holder.join().unwrap();
        controller
            .enable(
                Duration::from_millis(100),
                PathBuf::from(".tmp/tests/missing-cookie"),
            )
            .unwrap();
        let event = controller
            .events
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(matches!(
            event,
            ProtectionEvent::Enabled {
                singleton_mutex: true,
                singleton_event: true,
                cookie: false,
                ..
            }
        ));
        assert_eq!(
            controller.health().snapshot(),
            ProtectionSnapshot {
                singleton_mutex: true,
                singleton_event: true,
                cookie: false,
            }
        );
        controller.disable();
        assert!(matches!(
            controller
                .events
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            ProtectionEvent::Disabled
        ));
        assert_eq!(
            controller.health().snapshot(),
            ProtectionSnapshot::default()
        );
        controller
            .enable(
                Duration::from_millis(100),
                PathBuf::from(".tmp/tests/missing-cookie"),
            )
            .unwrap();
        assert!(matches!(
            controller
                .events
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            ProtectionEvent::Enabled { .. }
        ));
    }

    #[test]
    fn backend_detection_and_auto_resolution_use_fixture_executables() {
        let root = PathBuf::from(".tmp")
            .join("tests")
            .join(unique("detection"));
        std::fs::create_dir_all(root.join("Fishstrap")).unwrap();
        std::fs::create_dir_all(root.join("Bloxstrap")).unwrap();
        std::fs::create_dir_all(root.join("Roblox/Versions/version-test")).unwrap();
        std::fs::write(root.join("Fishstrap/Fishstrap.exe"), b"MZ").unwrap();
        std::fs::write(root.join("Bloxstrap/Bloxstrap.exe"), b"MZ").unwrap();
        std::fs::write(
            root.join("Roblox/Versions/version-test/RobloxPlayerBeta.exe"),
            b"MZ",
        )
        .unwrap();
        let detection = detect_launchers(Some(&root));
        assert!(detection.fishstrap.installed());
        assert!(detection.bloxstrap.installed());
        assert!(detection.stock.installed());
        assert_eq!(
            detection.resolve(LaunchBackend::Auto),
            LaunchBackend::Fishstrap
        );
        std::fs::remove_file(root.join("Fishstrap/Fishstrap.exe")).unwrap();
        let detection = detect_launchers(Some(&root));
        assert_eq!(
            detection.resolve(LaunchBackend::Auto),
            LaunchBackend::Bloxstrap
        );
        std::fs::remove_file(root.join("Bloxstrap/Bloxstrap.exe")).unwrap();
        let detection = detect_launchers(Some(&root));
        assert_eq!(
            detection.resolve(LaunchBackend::Auto),
            LaunchBackend::DefaultRoblox
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
