//! Audio session enumeration and per-session volume.
//!
//! All documented API, all stable. This is the half of the project that is not
//! a gamble.

use serde::{Deserialize, Serialize};
// `Interface` must be in scope for `.cast()` — QueryInterface between COM
// interfaces on the same object. Without it the method simply does not exist.
use std::collections::HashMap;

use windows::core::{Interface, Result};
use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    eRender, AudioSessionStateActive, AudioSessionStateExpired, AudioSessionStateInactive,
    IAudioSessionControl2, IAudioSessionManager2, ISimpleAudioVolume,
};
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::devices;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub process_id: u32,
    pub executable: String,
    pub full_path: String,
    pub volume: f32,
    pub muted: bool,
    pub state: String,
    /// Friendly name of the endpoint this session was found on. For humans.
    pub on_device: String,
    /// Endpoint ID of the same device. For machines — friendly names are not
    /// unique and change when hardware is renamed, so anything that has to
    /// survive a restart keys on this.
    pub on_device_id: String,
}

/// Map a PID to its executable path.
///
/// Uses `PROCESS_QUERY_LIMITED_INFORMATION`, which a standard user can open on
/// their own processes. Anything requiring more access than that would breach
/// the project's no-elevation rule.
fn process_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buffer = [0u16; MAX_PATH as usize];
        let mut size = buffer.len() as u32;

        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );

        let _ = CloseHandle(handle);

        if result.is_ok() {
            Some(String::from_utf16_lossy(&buffer[..size as usize]))
        } else {
            None
        }
    }
}

fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

/// Enumerate every audio session on every active output device.
///
/// Note that browsers and Electron apps produce **several** sessions each,
/// roughly one per renderer. They are returned individually here rather than
/// grouped — grouping by executable is the core's job, and the raw truth is
/// more useful for diagnosis.
pub fn list() -> Result<Vec<Session>> {
    let mut sessions = Vec::new();

    unsafe {
        let enumerator = devices::enumerator()?;
        let collection = enumerator
            .EnumAudioEndpoints(eRender, windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE)?;

        for i in 0..collection.GetCount()? {
            let device = collection.Item(i)?;
            let device_name =
                devices::friendly_name(&device).unwrap_or_else(|_| "<unknown>".into());
            let device_id = device
                .GetId()
                .ok()
                .and_then(|id| id.to_string().ok())
                .unwrap_or_default();

            let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let list = match manager.GetSessionEnumerator() {
                Ok(l) => l,
                Err(_) => continue,
            };

            for s in 0..list.GetCount()? {
                let control = match list.GetSession(s) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let control2: IAudioSessionControl2 = match control.cast() {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let pid = control2.GetProcessId().unwrap_or(0);

                // Guard clauses, not bare patterns.
                //
                // `Ok(AudioSessionStateActive) => ...` looks like a comparison
                // but is not: these are constants, not enum variants, so in
                // pattern position the name binds a fresh variable that matches
                // ANYTHING. Every session would have been reported as
                // "playing". rustc catches it as a non_upper_case_globals
                // warning, which is easy to dismiss as style noise — it is not.
                let state = match control.GetState() {
                    Ok(s) if s == AudioSessionStateActive => "playing",
                    Ok(s) if s == AudioSessionStateInactive => "idle",
                    Ok(s) if s == AudioSessionStateExpired => "expired",
                    _ => "unknown",
                };

                let (volume, muted) = match control.cast::<ISimpleAudioVolume>() {
                    Ok(v) => (
                        v.GetMasterVolume().unwrap_or(-1.0),
                        v.GetMute().map(|m| m.as_bool()).unwrap_or(false),
                    ),
                    Err(_) => (-1.0, false),
                };

                let full_path = process_path(pid).unwrap_or_else(|| "<system>".into());

                sessions.push(Session {
                    process_id: pid,
                    executable: file_name(&full_path),
                    full_path,
                    volume,
                    muted,
                    state: state.to_string(),
                    on_device: device_name.clone(),
                    on_device_id: device_id.clone(),
                });
            }
        }
    }

    sessions.sort_by_key(|a| a.executable.to_lowercase());
    Ok(sessions)
}

/// Re-exported so callers can hold the controls these functions return without
/// depending on the `windows` crate directly.
pub use windows::Win32::Media::Audio::ISimpleAudioVolume as VolumeControl;

/// Volume controls for several processes at once, keyed by process id.
///
/// # Why this exists
///
/// [`volume_controls`] enumerates every active render endpoint and every
/// session on each of them — and does it again for every process asked about.
/// Applying a channel's volume to six applications therefore meant six full
/// sweeps of the audio graph, on top of the one that found the sessions in the
/// first place.
///
/// Doing that per process would be most of the cost of a volume change, and a
/// fader drag sends a command per step - at over 100ms each the core would run
/// seconds behind the hand moving it. One sweep answers for every process.
pub fn volume_controls_for(pids: &[u32]) -> Result<HashMap<u32, Vec<ISimpleAudioVolume>>> {
    let mut found: HashMap<u32, Vec<ISimpleAudioVolume>> = HashMap::new();
    if pids.is_empty() {
        return Ok(found);
    }

    unsafe {
        let enumerator = devices::enumerator()?;
        let collection = enumerator
            .EnumAudioEndpoints(eRender, windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE)?;

        for i in 0..collection.GetCount()? {
            let device = collection.Item(i)?;

            let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let list = match manager.GetSessionEnumerator() {
                Ok(l) => l,
                Err(_) => continue,
            };

            for s in 0..list.GetCount()? {
                let control = match list.GetSession(s) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let control2: IAudioSessionControl2 = match control.cast() {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let pid = control2.GetProcessId().unwrap_or(0);
                if !pids.contains(&pid) {
                    continue;
                }

                if let Ok(v) = control.cast::<ISimpleAudioVolume>() {
                    found.entry(pid).or_default().push(v);
                }
            }
        }
    }

    Ok(found)
}

/// A session's volume control, and the output device the session is on.
///
/// The device matters because Master is the default device's own volume:
/// Windows applies that volume to every session on the device, so
/// the engine sets sessions there differently from sessions elsewhere. See
/// `engine::level_for` in the core.
pub struct DeviceVolume {
    pub device: String,
    pub control: ISimpleAudioVolume,
}

/// Like [`volume_controls_for`], with each control's device.
pub fn device_volume_controls_for(pids: &[u32]) -> Result<HashMap<u32, Vec<DeviceVolume>>> {
    let mut found: HashMap<u32, Vec<DeviceVolume>> = HashMap::new();
    if pids.is_empty() {
        return Ok(found);
    }

    unsafe {
        let enumerator = devices::enumerator()?;
        let collection = enumerator
            .EnumAudioEndpoints(eRender, windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE)?;

        for i in 0..collection.GetCount()? {
            let device = collection.Item(i)?;
            let device_id = match device.GetId().ok().and_then(|id| id.to_string().ok()) {
                Some(id) => id,
                None => continue,
            };

            let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let list = match manager.GetSessionEnumerator() {
                Ok(l) => l,
                Err(_) => continue,
            };

            for s in 0..list.GetCount()? {
                let control = match list.GetSession(s) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let control2: IAudioSessionControl2 = match control.cast() {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let pid = control2.GetProcessId().unwrap_or(0);
                if !pids.contains(&pid) {
                    continue;
                }

                if let Ok(v) = control.cast::<ISimpleAudioVolume>() {
                    found.entry(pid).or_default().push(DeviceVolume {
                        device: device_id.clone(),
                        control: v,
                    });
                }
            }
        }
    }

    Ok(found)
}

/// Every session of every process, with its device - for Restore, which puts
/// everything back rather than what one channel holds.
pub fn all_device_volume_controls() -> Result<HashMap<u32, Vec<DeviceVolume>>> {
    let pids: Vec<u32> = list()?
        .iter()
        .map(|s| s.process_id)
        .filter(|pid| *pid != 0)
        .collect();
    device_volume_controls_for(&pids)
}

/// Find **every** `ISimpleAudioVolume` belonging to a PID, across **every**
/// active output device.
///
/// # Why this searches all devices, and why that is not obvious
///
/// When an application is rerouted to another device, Windows does not move
/// its session. The old session **stays on the old device** as an idle husk,
/// and a **new session is created on the new device at full volume**. So an app
/// routed away from the default ends up with:
///
/// - a stale, idle session on the old device, and
/// - the live, playing session on the new one.
///
/// Searching only the default device would find the husk, set *its* volume,
/// and read back the value it had just written — reporting complete success
/// while the audible volume never changed. A correct return code and a correct
/// read-back, proving nothing at all.
///
/// The consequence for the core is concrete: a channel's volume must be
/// applied to every session an executable owns, on every device, and reapplied
/// as new sessions appear.
pub fn volume_controls(pid: u32) -> Result<Vec<ISimpleAudioVolume>> {
    let mut controls = Vec::new();

    unsafe {
        let enumerator = devices::enumerator()?;
        let collection = enumerator
            .EnumAudioEndpoints(eRender, windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE)?;

        for i in 0..collection.GetCount()? {
            let device = collection.Item(i)?;

            let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let list = match manager.GetSessionEnumerator() {
                Ok(l) => l,
                Err(_) => continue,
            };

            for s in 0..list.GetCount()? {
                let control = match list.GetSession(s) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let control2: IAudioSessionControl2 = match control.cast() {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                if control2.GetProcessId().unwrap_or(0) == pid {
                    if let Ok(v) = control.cast::<ISimpleAudioVolume>() {
                        controls.push(v);
                    }
                }
            }
        }
    }

    Ok(controls)
}

/// Re-exported so callers can hold a meter without depending on the `windows`
/// crate directly.
pub use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation as MeterHandle;

/// The meter interfaces belonging to a set of processes, keyed by process id.
///
/// # Why hold these rather than read peaks directly
///
/// [`peak_levels_for`] sweeps every active endpoint and every session on it to
/// find the processes it was asked about — and a meter has to be read thirty
/// times a second to look alive. Enumerating the whole audio graph that often
/// costs about **12% of one core**, measured, where metering's budget is 1%.
///
/// The interfaces themselves are cheap to keep and cheap to call. Enumerating
/// is the expensive part, and the set of sessions only changes when an
/// application starts or stops making sound — which the session watcher already
/// reports. So the caller holds these and calls [`peak_of`] per tick.
///
/// A session that ends leaves an interface that fails or reads zero, which
/// [`peak_of`] treats as silence; the caller refreshes on the watcher's signal
/// and on a slow backstop anyway.
pub fn meters_for(pids: &[u32]) -> Result<HashMap<u32, Vec<MeterHandle>>> {
    let mut found: HashMap<u32, Vec<MeterHandle>> = HashMap::new();
    if pids.is_empty() {
        return Ok(found);
    }

    unsafe {
        let enumerator = devices::enumerator()?;
        let collection = enumerator
            .EnumAudioEndpoints(eRender, windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE)?;

        for i in 0..collection.GetCount()? {
            let device = collection.Item(i)?;

            let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let list = match manager.GetSessionEnumerator() {
                Ok(l) => l,
                Err(_) => continue,
            };

            for s in 0..list.GetCount()? {
                let control = match list.GetSession(s) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let control2: IAudioSessionControl2 = match control.cast() {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let pid = control2.GetProcessId().unwrap_or(0);
                if !pids.contains(&pid) {
                    continue;
                }

                if let Ok(meter) = control.cast::<MeterHandle>() {
                    found.entry(pid).or_default().push(meter);
                }
            }
        }
    }

    Ok(found)
}

/// The loudest of a set of held meters.
///
/// Reading a peak **resets it**, so there must be exactly one caller per tick.
/// A meter whose session has ended reads as silence rather than as an error:
/// the caller cannot do anything useful about it, and a level meter is the
/// wrong place to report a device problem.
pub fn peak_of(meters: &[MeterHandle]) -> f32 {
    let mut peak = 0.0f32;
    for meter in meters {
        unsafe {
            if let Ok(value) = meter.GetPeakValue() {
                peak = peak.max(value);
            }
        }
    }
    peak
}

/// Peak levels for several processes at once, keyed by process id.
///
/// # Why this exists
///
/// [`peak_level`] sweeps every active render endpoint and every session on each
/// of them, and does it again for every process asked about. Metering six
/// applications therefore meant six full sweeps of the audio graph, ten times a
/// second, which put a hard ceiling on how often a meter could be updated —
/// and a meter that updates ten times a second looks like it is stuttering.
///
/// One sweep answers for every process, so the rate can be raised to something
/// that actually looks live.
///
/// Reading a peak **resets it**, so this must be the only caller per tick;
/// two readers would each see part of the signal and both would under-read.
pub fn peak_levels_for(pids: &[u32]) -> Result<HashMap<u32, f32>> {
    let mut peaks: HashMap<u32, f32> = HashMap::new();
    if pids.is_empty() {
        return Ok(peaks);
    }

    unsafe {
        let enumerator = devices::enumerator()?;
        let collection = enumerator
            .EnumAudioEndpoints(eRender, windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE)?;

        for i in 0..collection.GetCount()? {
            let device = collection.Item(i)?;

            let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let list = match manager.GetSessionEnumerator() {
                Ok(l) => l,
                Err(_) => continue,
            };

            for s in 0..list.GetCount()? {
                let control = match list.GetSession(s) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let control2: IAudioSessionControl2 = match control.cast() {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let pid = control2.GetProcessId().unwrap_or(0);
                if !pids.contains(&pid) {
                    continue;
                }

                if let Ok(meter) = control.cast::<IAudioMeterInformation>() {
                    if let Ok(value) = meter.GetPeakValue() {
                        let entry = peaks.entry(pid).or_insert(0.0);
                        *entry = entry.max(value);
                    }
                }
            }
        }
    }

    Ok(peaks)
}

/// Peak level for a process, 0.0–1.0, across every session it owns.
///
/// `IAudioMeterInformation` reports the peak since the last read, which is what
/// a level meter wants — an average would under-read transients and make a
/// clearly audible channel look quiet.
///
/// Returns the loudest of the app's sessions. Reading is destructive in the
/// sense that the peak resets, so two readers would each see part of the
/// signal. The core's meter loop uses [`peak_of`] on held handles instead.
pub fn peak_level(pid: u32) -> Result<f32> {
    let mut peak: f32 = 0.0;

    unsafe {
        let enumerator = devices::enumerator()?;
        let collection = enumerator
            .EnumAudioEndpoints(eRender, windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE)?;

        for i in 0..collection.GetCount()? {
            let device = collection.Item(i)?;

            let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let list = match manager.GetSessionEnumerator() {
                Ok(l) => l,
                Err(_) => continue,
            };

            for s in 0..list.GetCount()? {
                let control = match list.GetSession(s) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let control2: IAudioSessionControl2 = match control.cast() {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                if control2.GetProcessId().unwrap_or(0) != pid {
                    continue;
                }

                if let Ok(meter) = control.cast::<IAudioMeterInformation>() {
                    if let Ok(value) = meter.GetPeakValue() {
                        peak = peak.max(value);
                    }
                }
            }
        }
    }

    Ok(peak)
}
