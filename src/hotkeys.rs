use std::{
    ptr::null_mut,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
    },
    WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY},
};

const PRIMARY_ID: i32 = 0x524D_4101;
const SECONDARY_ID: i32 = 0x524D_4102;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    FocusPrimary,
    FocusSecondary,
}

pub struct HotkeyManager {
    pub events: mpsc::Receiver<HotkeyEvent>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HotkeyManager {
    pub fn start() -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let (event_tx, events) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let handle = thread::Builder::new()
            .name("rmal-global-hotkeys".into())
            .spawn(move || {
                let modifiers = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
                let primary =
                    unsafe { RegisterHotKey(null_mut(), PRIMARY_ID, modifiers, b'1' as u32) } != 0;
                let secondary =
                    unsafe { RegisterHotKey(null_mut(), SECONDARY_ID, modifiers, b'2' as u32) }
                        != 0;
                if !(primary && secondary) {
                    if primary {
                        unsafe { UnregisterHotKey(null_mut(), PRIMARY_ID) };
                    }
                    if secondary {
                        unsafe { UnregisterHotKey(null_mut(), SECONDARY_ID) };
                    }
                    let _ = ready_tx.send(Err(
                        "Ctrl+Alt+1 or Ctrl+Alt+2 is already registered by another application."
                            .into(),
                    ));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                while !thread_stop.load(Ordering::Acquire) {
                    let mut message: MSG = unsafe { std::mem::zeroed() };
                    while unsafe {
                        PeekMessageW(&mut message, null_mut(), WM_HOTKEY, WM_HOTKEY, PM_REMOVE)
                    } != 0
                    {
                        let event = match message.wParam as i32 {
                            PRIMARY_ID => Some(HotkeyEvent::FocusPrimary),
                            SECONDARY_ID => Some(HotkeyEvent::FocusSecondary),
                            _ => None,
                        };
                        if let Some(event) = event {
                            let _ = event_tx.send(event);
                        }
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                unsafe {
                    UnregisterHotKey(null_mut(), PRIMARY_ID);
                    UnregisterHotKey(null_mut(), SECONDARY_ID);
                }
            })
            .map_err(|error| error.to_string())?;
        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(Self {
                events,
                stop,
                thread: Some(handle),
            }),
            Ok(Err(message)) => {
                let _ = handle.join();
                Err(message)
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                let _ = handle.join();
                Err("Timed out while registering global hotkeys.".into())
            }
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
