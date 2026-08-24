use std::ptr::null;
use windows::{
    Win32::{
        Media::Audio::{
            IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume,
            MMDeviceEnumerator, eConsole, eRender,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
        },
    },
    core::Interface,
};

#[derive(Debug, Clone, Copy)]
pub struct AudioState {
    pub volume_percent: u8,
    pub muted: bool,
    pub sessions: usize,
}

struct ComScope(bool);

impl ComScope {
    fn enter() -> Self {
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        Self(initialized)
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn volumes_for_pid(pid: u32) -> Result<Vec<ISimpleAudioVolume>, String> {
    let _com = ComScope::enter();
    let enumerator: IMMDeviceEnumerator = unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .map_err(|error| format!("Core Audio device enumeration failed: {error}"))?
    };
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
        .map_err(|error| format!("The default audio output is unavailable: {error}"))?;
    let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None) }
        .map_err(|error| format!("Core Audio session management is unavailable: {error}"))?;
    let sessions = unsafe { manager.GetSessionEnumerator() }
        .map_err(|error| format!("Audio sessions could not be enumerated: {error}"))?;
    let count = unsafe { sessions.GetCount() }
        .map_err(|error| format!("Audio session count is unavailable: {error}"))?;
    let mut result = Vec::new();
    for index in 0..count {
        let Ok(control) = (unsafe { sessions.GetSession(index) }) else {
            continue;
        };
        let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
            continue;
        };
        if unsafe { control2.GetProcessId() }.ok() != Some(pid) {
            continue;
        }
        if let Ok(volume) = control.cast::<ISimpleAudioVolume>() {
            result.push(volume);
        }
    }
    if result.is_empty() {
        Err(format!(
            "No active Core Audio session is currently mapped to Roblox PID {pid}. Start in-game audio and retry."
        ))
    } else {
        Ok(result)
    }
}

pub fn state(pid: u32) -> Result<AudioState, String> {
    let volumes = volumes_for_pid(pid)?;
    let first = &volumes[0];
    let volume = unsafe { first.GetMasterVolume() }
        .map_err(|error| format!("Audio volume is unavailable: {error}"))?;
    let muted = unsafe { first.GetMute() }
        .map_err(|error| format!("Audio mute state is unavailable: {error}"))?
        .as_bool();
    Ok(AudioState {
        volume_percent: (volume.clamp(0.0, 1.0) * 100.0).round() as u8,
        muted,
        sessions: volumes.len(),
    })
}

pub fn set(pid: u32, volume_percent: u8, muted: bool) -> Result<usize, String> {
    let volumes = volumes_for_pid(pid)?;
    let scalar = volume_scalar(volume_percent);
    for volume in &volumes {
        unsafe {
            volume
                .SetMasterVolume(scalar, null())
                .map_err(|error| format!("Audio volume could not be changed: {error}"))?;
            volume
                .SetMute(muted, null())
                .map_err(|error| format!("Audio mute state could not be changed: {error}"))?;
        }
    }
    Ok(volumes.len())
}

fn volume_scalar(volume_percent: u8) -> f32 {
    f32::from(volume_percent.min(100)) / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_percent_is_always_clamped_before_core_audio() {
        assert_eq!(volume_scalar(255), 1.0);
        assert_eq!(volume_scalar(35), 0.35);
    }
}
