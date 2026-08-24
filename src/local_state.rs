use std::{
    fs::{self, OpenOptions},
    io::Write,
    mem::zeroed,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::SystemTime,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_IO_PENDING, GetLastError, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_ACTION_ADDED, FILE_ACTION_MODIFIED, FILE_ACTION_REMOVED,
        FILE_ACTION_RENAMED_NEW_NAME, FILE_ACTION_RENAMED_OLD_NAME, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_CREATION,
        FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
        FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW,
    },
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Threading::{CreateEventW, WaitForSingleObject},
    },
};

#[derive(Debug, Clone)]
pub enum TraceEvent {
    Started(PathBuf),
    Change(String),
    Stopped,
    Failed(String),
}

pub struct LocalStateTracer {
    cancel: Arc<AtomicBool>,
    pub events: mpsc::Receiver<TraceEvent>,
    thread: Option<thread::JoinHandle<()>>,
}

impl LocalStateTracer {
    pub fn start(directory: PathBuf, log_path: PathBuf) -> Result<Self, String> {
        if !directory.is_dir() {
            return Err(format!(
                "Roblox LocalStorage was not found at {}.",
                directory.display()
            ));
        }
        if let Some(parent) = log_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, events) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("roblox-local-state-tracer".into())
            .spawn(move || {
                let result = watch_directory(&directory, &log_path, &worker_cancel, &sender);
                match result {
                    Ok(()) => {
                        let _ = sender.send(TraceEvent::Stopped);
                    }
                    Err(message) => {
                        let _ = sender.send(TraceEvent::Failed(message));
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            cancel,
            events,
            thread: Some(worker),
        })
    }

    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Drop for LocalStateTracer {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}

fn wide(value: &Path) -> Vec<u16> {
    value.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn watch_directory(
    directory: &Path,
    log_path: &Path,
    cancel: &AtomicBool,
    sender: &mpsc::Sender<TraceEvent>,
) -> Result<(), String> {
    let path = wide(directory);
    let directory_handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            null_mut(),
        )
    };
    if directory_handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "Roblox LocalStorage tracing could not open the directory (Windows error {}).",
            unsafe { GetLastError() }
        ));
    }
    let event_handle = unsafe { CreateEventW(null(), 0, 0, null()) };
    if event_handle.is_null() {
        unsafe {
            CloseHandle(directory_handle);
        }
        return Err(format!(
            "Local-state trace event creation failed (Windows error {}).",
            unsafe { GetLastError() }
        ));
    }

    let started_line = format_trace_line("TRACE_STARTED", &directory.display().to_string());
    append_line(log_path, &started_line)?;
    let _ = sender.send(TraceEvent::Started(log_path.to_path_buf()));
    let mut buffer = vec![0u32; 16 * 1024];
    let filters = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_LAST_WRITE
        | FILE_NOTIFY_CHANGE_CREATION;
    let result = (|| {
        while !cancel.load(Ordering::Acquire) {
            let mut overlapped: OVERLAPPED = unsafe { zeroed() };
            overlapped.hEvent = event_handle;
            let queued = unsafe {
                ReadDirectoryChangesW(
                    directory_handle,
                    buffer.as_mut_ptr().cast(),
                    (buffer.len() * size_of::<u32>()) as u32,
                    1,
                    filters,
                    null_mut(),
                    &mut overlapped,
                    None,
                )
            };
            if queued == 0 && unsafe { GetLastError() } != ERROR_IO_PENDING {
                return Err(format!(
                    "ReadDirectoryChangesW failed with Windows error {}.",
                    unsafe { GetLastError() }
                ));
            }
            loop {
                match unsafe { WaitForSingleObject(event_handle, 250) } {
                    WAIT_OBJECT_0 => break,
                    WAIT_TIMEOUT if !cancel.load(Ordering::Acquire) => continue,
                    WAIT_TIMEOUT => {
                        unsafe {
                            CancelIoEx(directory_handle, &overlapped);
                        }
                        return Ok(());
                    }
                    _ => {
                        unsafe {
                            CancelIoEx(directory_handle, &overlapped);
                        }
                        return Err(format!(
                            "Waiting for local-state changes failed with Windows error {}.",
                            unsafe { GetLastError() }
                        ));
                    }
                }
            }
            let mut transferred = 0u32;
            if unsafe { GetOverlappedResult(directory_handle, &overlapped, &mut transferred, 0) }
                == 0
            {
                if cancel.load(Ordering::Acquire) {
                    return Ok(());
                }
                return Err(format!(
                    "Reading local-state trace results failed with Windows error {}.",
                    unsafe { GetLastError() }
                ));
            }
            if transferred == 0 {
                let line = format_trace_line("OVERFLOW", "change buffer overflowed");
                append_line(log_path, &line)?;
                let _ = sender.send(TraceEvent::Change(line));
                continue;
            }
            parse_changes(&buffer, transferred as usize, |action, relative| {
                let line = format_trace_line(action, relative);
                append_line(log_path, &line)?;
                let _ = sender.send(TraceEvent::Change(line));
                Ok(())
            })?;
        }
        Ok(())
    })();
    unsafe {
        CloseHandle(event_handle);
        CloseHandle(directory_handle);
    }
    result
}

fn parse_changes(
    buffer: &[u32],
    transferred: usize,
    mut record: impl FnMut(&str, &str) -> Result<(), String>,
) -> Result<(), String> {
    let bytes = buffer.as_ptr().cast::<u8>();
    let mut offset = 0usize;
    while offset + 12 <= transferred {
        let info = unsafe { &*(bytes.add(offset).cast::<FILE_NOTIFY_INFORMATION>()) };
        let name_bytes = info.FileNameLength as usize;
        if offset + 12 + name_bytes > transferred || !name_bytes.is_multiple_of(2) {
            return Err("Windows returned an invalid local-state change record.".into());
        }
        let name = unsafe {
            String::from_utf16_lossy(std::slice::from_raw_parts(
                info.FileName.as_ptr(),
                name_bytes / 2,
            ))
        };
        record(action_label(info.Action), &name)?;
        if info.NextEntryOffset == 0 {
            break;
        }
        offset = offset.saturating_add(info.NextEntryOffset as usize);
    }
    Ok(())
}

fn action_label(action: u32) -> &'static str {
    match action {
        FILE_ACTION_ADDED => "CREATE",
        FILE_ACTION_REMOVED => "DELETE",
        FILE_ACTION_MODIFIED => "WRITE/METADATA",
        FILE_ACTION_RENAMED_OLD_NAME => "RENAME_FROM",
        FILE_ACTION_RENAMED_NEW_NAME => "RENAME_TO",
        _ => "UNKNOWN",
    }
}

fn format_trace_line(action: &str, relative_path: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!(
        "{millis} action={action} path={} process=unavailable",
        relative_path.replace(['\r', '\n'], "_")
    )
}

fn append_line(path: &Path, line: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    writeln!(file, "{line}").map_err(|error| error.to_string())
}

const fn size_of<T>() -> usize {
    std::mem::size_of::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_root() -> PathBuf {
        PathBuf::from(".tmp").join("tests").join(format!(
            "local-state-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn trace_action_labels_cover_file_lifecycle_without_contents() {
        assert_eq!(action_label(FILE_ACTION_ADDED), "CREATE");
        assert_eq!(action_label(FILE_ACTION_MODIFIED), "WRITE/METADATA");
        assert_eq!(action_label(FILE_ACTION_REMOVED), "DELETE");
        assert_eq!(action_label(FILE_ACTION_RENAMED_OLD_NAME), "RENAME_FROM");
        assert_eq!(action_label(FILE_ACTION_RENAMED_NEW_NAME), "RENAME_TO");
        let line = format_trace_line("WRITE/METADATA", "RobloxCookies.dat");
        assert!(line.contains("path=RobloxCookies.dat"));
        assert!(line.contains("process=unavailable"));
    }

    #[test]
    fn tracer_records_only_fixture_metadata_and_stops_cleanly() {
        let root = unique_root();
        let watched = root.join("watched");
        let log = root.join("logs").join("trace.log");
        fs::create_dir_all(&watched).unwrap();
        let tracer = LocalStateTracer::start(watched.clone(), log.clone()).unwrap();
        assert!(matches!(
            tracer
                .events
                .recv_timeout(std::time::Duration::from_secs(2)),
            Ok(TraceEvent::Started(_))
        ));
        let created = watched.join("SharedState.test");
        let renamed = watched.join("SharedState-renamed.test");
        fs::write(&created, b"fixture-content-must-not-be-recorded").unwrap();
        fs::rename(&created, &renamed).unwrap();
        fs::remove_file(&renamed).unwrap();
        let mut saw_path = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Ok(TraceEvent::Change(line)) = tracer
                .events
                .recv_timeout(std::time::Duration::from_millis(250))
            {
                saw_path |= line.contains("SharedState");
                if saw_path {
                    break;
                }
            }
        }
        tracer.stop();
        drop(tracer);
        let trace = fs::read_to_string(&log).unwrap();
        assert!(saw_path);
        assert!(trace.contains("SharedState"));
        assert!(!trace.contains("fixture-content-must-not-be-recorded"));
        let _ = fs::remove_dir_all(root);
    }
}
