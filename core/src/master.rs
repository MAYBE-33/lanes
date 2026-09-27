//! Master is Windows' default output: its device, its volume slider, its mute.
//!
//! Master is not a number Lanes keeps to itself. Change Master's volume and the
//! slider in the Windows taskbar moves with it; move that slider and Master
//! follows. Specifically:
//!
//! - **Master's device is Windows' default output.** Choosing a device on the
//!   Master strip makes it the default; changing the default anywhere else
//!   (the taskbar's picker, the Sound settings) moves Master to it.
//! - **Master's volume is that device's own slider**, the same number both
//!   ways. Moving Master moves the taskbar slider; moving the slider moves
//!   Master.
//! - **Master's mute is that device's mute**, and still mutes every channel,
//!   as it always has, so apps on other devices go quiet too.
//! - **Master is also a ceiling.** Channels move with it (see
//!   `config::Config::effective_volume`). Windows already applies the slider to every app on that device, so the
//!   engine sets those apps' own levels to make the result come out exactly at
//!   the ceiling rather than cut twice - see `engine::level_for`.
//!
//! # A mirror, kept in step both ways
//!
//! The config still holds Master's volume, mute and device, because profiles,
//! the API and every client already read them there. They are a **mirror** of
//! Windows:
//!
//! - [`pull`] copies Windows into the config: at startup, when the slider or
//!   mute changes elsewhere (`endpointwatch`), and when the default device
//!   changes (`devicewatch`).
//! - [`push`] writes to Windows whatever the config says differently: after
//!   every command, before the engine applies levels, so the engine works from
//!   the new volume. Every write is marked as Lanes' own, so its echo is
//!   ignored.
//!
//! # Safety
//!
//! These are machine-wide settings that outlive Lanes, so before the first
//! change to each one Lanes records what it was, in `snapshots/windows-before.json`
//! ([`Before`]). Restore puts them back (see `restore::reset_windows`).
//!
//! A development copy of Lanes (`instance::is_separate`) **reads** but never
//! writes: its Master follows Windows, and changing it changes nothing. Two
//! copies both setting the default device would fight over it.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use winaudio::endpoint::EndpointVolume;

use crate::audit;
use crate::config::{Config, DeviceRef};
use crate::paths;

/// Volumes are floats; a slider moved by hand never lands exactly.
const EPSILON: f32 = 0.005;

/// What the engine needs to know about Master when it sets an app's level.
#[derive(Debug, Clone, Default)]
pub struct Output {
    /// Windows' default output device, which is Master's.
    pub device: Option<String>,
    /// What its slider does to the signal: a linear factor, 0 to 1. Every
    /// session on this device is multiplied by it before it is heard.
    pub amplitude: f32,
}

impl Output {
    /// Read it from Windows now. Nothing readable means no default device, in
    /// which case nothing is scaled by one.
    pub fn read() -> Self {
        match EndpointVolume::default_output() {
            Ok(Some(volume)) => Self {
                amplitude: volume.amplitude().unwrap_or(1.0),
                device: Some(volume.device_id().to_string()),
            },
            _ => Self {
                device: None,
                amplitude: 1.0,
            },
        }
    }
}

/// Master as Windows and Lanes last agreed it was.
///
/// # Why push compares against this, not against Windows
///
/// Comparing the config with Windows would make every command a chance to undo
/// the user: move the taskbar slider, and a command that arrives before the
/// change notification has been handled would find Windows "wrong" and put the
/// old volume back. Comparing with the last agreed state means [`push`] writes
/// only what a Lanes command actually changed; a change made in Windows is left
/// for [`pull`] to bring in.
#[derive(Debug, Clone, PartialEq)]
struct Synced {
    volume: f32,
    muted: bool,
    device: Option<String>,
}

static SYNCED: Mutex<Option<Synced>> = Mutex::new(None);

fn from_config(config: &Config) -> Option<Synced> {
    config.channel("master").map(|m| Synced {
        volume: m.volume,
        muted: m.muted,
        device: m.device.as_ref().map(|d| d.id.clone()),
    })
}

fn remember(config: &Config) {
    if let Ok(mut synced) = SYNCED.lock() {
        *synced = from_config(config);
    }
}

/// Copy Windows' default output into the config's Master. True if anything
/// changed, in which case the caller saves and reapplies levels.
pub fn pull(config: &mut Config) -> bool {
    let changed = pull_inner(config);
    remember(config);
    changed
}

fn pull_inner(config: &mut Config) -> bool {
    let Ok(Some(volume)) = EndpointVolume::default_output() else {
        return false;
    };
    let scalar = volume.scalar().unwrap_or(1.0);
    let muted = volume.muted().unwrap_or(false);
    let id = volume.device_id().to_string();
    let name = device_name(&id);

    let Some(master) = config.channel_mut("master") else {
        return false;
    };

    let mut changed = false;
    if (master.volume - scalar).abs() > EPSILON {
        master.volume = scalar;
        changed = true;
    }
    if master.muted != muted {
        master.muted = muted;
        changed = true;
    }
    if master.device.as_ref().map(|d| d.id.as_str()) != Some(id.as_str()) {
        master.device = Some(DeviceRef { id, name });
        changed = true;
    }
    changed
}

/// Write to Windows whatever the config's Master says differently.
///
/// Device first: a new default device has its own slider, so the volume is
/// written to the device Master is now on. Failures are logged and otherwise
/// ignored - the engine still applies the channels, and the next [`pull`]
/// puts the mirror right.
pub fn push(config: &Config, reason: &str) {
    if crate::instance::is_separate() {
        return;
    }
    let Some(master) = config.channel("master") else {
        return;
    };

    // Only what a command changed since Windows and Lanes last agreed. See
    // `Synced`. Before the first pull there is nothing to compare with, and
    // every difference from Windows counts.
    let last = SYNCED.lock().ok().and_then(|s| s.clone());
    let wanted = from_config(config);
    if last.is_some() && last == wanted {
        return;
    }
    let device_changed = last
        .as_ref()
        .is_none_or(|l| l.device.as_deref() != master.device.as_ref().map(|d| d.id.as_str()));
    let volume_changed = last
        .as_ref()
        .is_none_or(|l| (l.volume - master.volume).abs() > EPSILON);
    let muted_changed = last.as_ref().is_none_or(|l| l.muted != master.muted);
    remember(config);

    // The device.
    if let Some(wanted) = master.device.as_ref().filter(|_| device_changed) {
        let current = winaudio::endpoint::default_output_id().ok().flatten();
        if current.as_deref() != Some(wanted.id.as_str()) && device_present(&wanted.id) {
            Before::record_default(current.as_deref());
            match winaudio::default_device::set_default_output(&wanted.id) {
                Ok(()) => audit::change("<windows>", "default-output", current, Some(wanted.id.clone()), reason),
                Err(e) => audit::failure("<windows>", "default-output", Some(wanted.id.clone()), reason, &e.to_string()),
            }
        }
    }

    // The volume and mute, of whatever is the default now.
    let Ok(Some(volume)) = EndpointVolume::default_output() else {
        return;
    };
    let id = volume.device_id().to_string();

    let scalar = volume.scalar().unwrap_or(master.volume);
    if volume_changed && (scalar - master.volume).abs() > EPSILON {
        Before::record_endpoint(&volume);
        match volume.set_scalar(master.volume) {
            Ok(()) => audit::change(&endpoint_target(&id), "volume", Some(format!("{scalar:.2}")), Some(format!("{:.2}", master.volume)), reason),
            Err(e) => audit::failure(&endpoint_target(&id), "volume", Some(format!("{:.2}", master.volume)), reason, &e.to_string()),
        }
    }

    let muted = volume.muted().unwrap_or(master.muted);
    if muted_changed && muted != master.muted {
        Before::record_endpoint(&volume);
        match volume.set_muted(master.muted) {
            Ok(()) => audit::change(&endpoint_target(&id), "mute", Some(muted.to_string()), Some(master.muted.to_string()), reason),
            Err(e) => audit::failure(&endpoint_target(&id), "mute", Some(master.muted.to_string()), reason, &e.to_string()),
        }
    }
}

/// The output device after `current` in Windows' list, wrapping round - what
/// "next output device" means for Master, whose device is the default.
pub fn next_output(current: Option<&str>) -> Option<DeviceRef> {
    let outputs: Vec<_> = winaudio::devices::list(windows::Win32::Media::Audio::eRender)
        .ok()?
        .into_iter()
        .collect();
    if outputs.is_empty() {
        return None;
    }
    let next = match current.and_then(|id| outputs.iter().position(|d| d.id == id)) {
        Some(i) => (i + 1) % outputs.len(),
        None => 0,
    };
    Some(DeviceRef {
        id: outputs[next].id.clone(),
        name: outputs[next].friendly_name.clone(),
    })
}

fn device_present(id: &str) -> bool {
    winaudio::devices::list(windows::Win32::Media::Audio::eRender)
        .map(|all| all.iter().any(|d| d.id == id))
        .unwrap_or(false)
}

fn device_name(id: &str) -> String {
    winaudio::devices::list(windows::Win32::Media::Audio::eRender)
        .ok()
        .and_then(|all| all.into_iter().find(|d| d.id == id))
        .map(|d| d.friendly_name)
        .unwrap_or_else(|| "Default output".into())
}

/// How an output device appears in the audit log, which is keyed by "target".
fn endpoint_target(id: &str) -> String {
    format!("<device {}>", device_name(id))
}

// ---------------------------------------------------------------------------
// What Windows was like before Lanes changed it
// ---------------------------------------------------------------------------

/// Windows' own audio settings as they were before Lanes first changed each
/// one: the default output, and each device's volume and mute.
///
/// **First writer wins, per setting**: a device's entry is written the first
/// time Lanes changes that device and never again, so however many times
/// Master moves afterwards, this still says what the machine was like before.
/// Kept beside the snapshots, and like `first-run.json` never overwritten
/// wholesale.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Before {
    /// The default output before Lanes first changed it. `None` inside `Some`
    /// would mean "there was none"; absent means Lanes never changed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_output: Option<Option<String>>,
    /// Each device's slider and mute, by device id.
    #[serde(default)]
    pub endpoints: BTreeMap<String, EndpointBefore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointBefore {
    pub volume: f32,
    pub muted: bool,
}

impl Before {
    pub fn path() -> std::path::PathBuf {
        paths::snapshots_dir().join("windows-before.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let _ = paths::ensure_dir(&paths::snapshots_dir());
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let path = Self::path();
            let temp = path.with_extension("json.tmp");
            if std::fs::write(&temp, text).is_ok() {
                let _ = std::fs::rename(&temp, &path);
            }
        }
    }

    fn record_default(current: Option<&str>) {
        let mut before = Self::load();
        if before.default_output.is_none() {
            before.default_output = Some(current.map(str::to_string));
            before.save();
        }
    }

    fn record_endpoint(volume: &EndpointVolume) {
        let mut before = Self::load();
        if !before.endpoints.contains_key(volume.device_id()) {
            before.endpoints.insert(
                volume.device_id().to_string(),
                EndpointBefore {
                    volume: volume.scalar().unwrap_or(1.0),
                    muted: volume.muted().unwrap_or(false),
                },
            );
            before.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_of_before_survives_a_round_trip_and_reads_absent_as_untouched() {
        let mut before = Before::default();
        assert!(before.default_output.is_none());
        before.default_output = Some(Some("{headset}".into()));
        before.endpoints.insert("{speakers}".into(), EndpointBefore { volume: 0.42, muted: false });

        let text = serde_json::to_string(&before).unwrap();
        let back: Before = serde_json::from_str(&text).unwrap();
        assert_eq!(back.default_output, Some(Some("{headset}".into())));
        assert_eq!(back.endpoints["{speakers}"].volume, 0.42);

        // An empty file, or one from before this existed, means nothing was changed.
        let empty: Before = serde_json::from_str("{}").unwrap();
        assert!(empty.default_output.is_none() && empty.endpoints.is_empty());
    }
}
