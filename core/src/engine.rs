//! Applying channel settings to live audio sessions.
//!
//! # The rule that governs everything here: only act when something changed
//!
//! Switching an app's output device mid-playback causes a brief glitch as the
//! app reinitialises its stream, so a periodic "enforcement" loop that reapplies
//! the same settings would produce audible stuttering for no benefit.
//!
//! Every write in this module is therefore preceded by a read, and skipped when
//! the value already matches. That also keeps the audit log meaningful: it
//! records changes, not heartbeats.
//!
//! # What an application's effective settings are
//!
//! ```text
//! volume = min(channel.volume, master.volume) * chat_mix * (rule.trim or 1.0)
//! muted  = master.muted or channel.muted
//! device = routing::choose(...)       (output channels only)
//! ```
//!
//! The device is the channel's own when it is plugged in, otherwise the first
//! enabled, present entry in the global fallback order - see [`crate::routing`].
//!
//! **Master is a ceiling rather than a scalar** — see
//! [`crate::config::Config::effective_volume`] for what that means. Master is
//! also Windows' own volume slider for the default device, so sessions on that
//! device are compensated for it - see [`level_for`]. Mute is still an OR: if Master is muted, everything is muted
//! regardless of channel state.
//!
//! `chat_mix` is 1.0 unless the Game/Chat balance control is off centre, and
//! then it applies only to the channel being quietened. It multiplies rather
//! than replaces, so each channel's own fader keeps its meaning — see
//! [`crate::config::ChatMix`].

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use winaudio::policy::AudioPolicyConfig;
use winaudio::sessions::{self, Session};
use windows::Win32::Media::Audio::eRender;

use crate::audit;
use crate::config::Config;
use crate::routing;
use crate::rules::{self, Resolution};

/// Volumes are floats that have been through JSON, so compare with a tolerance.
const EPSILON: f32 = 0.001;

#[derive(Debug, Default)]
pub struct Applied {
    pub volumes_changed: usize,
    pub mutes_changed: usize,
    pub devices_changed: usize,
    pub unchanged: usize,
    /// Executables with no rule — the "To be routed" pool.
    pub unassigned: Vec<String>,
    pub ignored: usize,
    pub failures: Vec<String>,
    /// Applications whose output device would not stay set, and which were
    /// therefore left alone. See [`refused_device`].
    pub routing_refused: Vec<String>,
}

// ---------------------------------------------------------------------------
// Applications that will not keep a device
// ---------------------------------------------------------------------------

/// The device each application would not keep, by lower-cased executable path.
///
/// # Why this exists
///
/// An application's output device can be changed by more than Lanes: by the
/// application's own settings (many games choose their own device), by the
/// Windows volume mixer, or by another audio tool. Each time the route changes,
/// the application reopens its audio, which creates a new session; the watcher
/// reports it; and Lanes, still seeing the "wrong" device, would put it back.
/// Two managers doing that to each other never stop, and the application's
/// audio breaks up while they fight.
///
/// So a route is verified after it is set, and when it has not held, Lanes
/// **stops fighting** and says so (`routing_refused` in the API, a mark on the
/// app in the mixer). That is what this memory is for. It lasts as long as the process, and an
/// entry is only honoured for the device it was refused on - a channel pointed
/// somewhere else, or moved to a fallback and back, gets a fresh try, and so
/// does an application the user reassigns.
static REFUSED: LazyLock<Mutex<HashMap<String, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn refused_key(full_path: &str) -> String {
    full_path.to_lowercase()
}

/// The device this application would not keep, if it has refused one.
///
/// `Some(None)` means it would not keep "system default" either, which in
/// practice means an explicit route could not be cleared.
pub fn refused_device(full_path: &str) -> Option<Option<String>> {
    REFUSED.lock().ok()?.get(&refused_key(full_path)).cloned()
}

/// Forget that an application refused a device, so the next apply tries again.
/// For when the user reassigns it: they are asking, so it is worth one more go.
pub fn forget_refusal(executable_or_path: &str) {
    if let Ok(mut refused) = REFUSED.lock() {
        let wanted = executable_or_path.to_lowercase();
        refused.retain(|path, _| path != &wanted && !path.ends_with(&format!("\\{wanted}")));
    }
}

/// It holds its device now - moved by hand, or the channel changed - so the
/// refusal no longer describes it.
fn clear_refused(full_path: &str) {
    if let Ok(mut refused) = REFUSED.lock() {
        refused.remove(&refused_key(full_path));
    }
}

fn mark_refused(full_path: &str, device: Option<&str>) {
    if let Ok(mut refused) = REFUSED.lock() {
        refused.insert(refused_key(full_path), device.map(str::to_string));
    }
}

/// Whether this application has already refused exactly this device.
fn already_refused(full_path: &str, device: Option<&str>) -> bool {
    refused_device(full_path).is_some_and(|refused| refused.as_deref() == device)
}

/// The levels a group of sessions should be running at.
///
/// **No device.** The output is decided separately, by `routing::choose`,
/// because deciding it needs the list of endpoints currently present and only
/// one of the two apply paths does device work at all. Carrying a device field
/// that `apply_levels_only` never reads would be an invitation to read it.
struct Target {
    channel_id: String,
    /// How loud the application should be **heard**, as a linear factor: its
    /// channel under Master's ceiling, the Game/Chat mix and its trim. What each
    /// of its sessions is set to depends on the device it is on - see
    /// [`level_for`].
    audible: f32,
    muted: bool,
    /// Master as it was read for this apply. See [`level_for`].
    master: crate::master::Output,
}

/// The level to give one session so that it is heard at `audible`.
///
/// # Why the device matters
///
/// Master is Windows' default output's own slider (see `master`), and Windows multiplies every session on that device by it. A
/// session there set straight to `audible` would be cut twice - once by its
/// channel's ceiling, once by the slider. So a session on Master's device gets
/// `audible` divided by what the slider does (its amplitude), and anything on
/// another device, which the slider does not touch, gets `audible` as it is.
///
/// `audible` never exceeds the amplitude for a playback channel (the ceiling is
/// applied in that unit), so the division never asks for more than full volume,
/// except through rounding - hence the clamp. A slider at zero silences the
/// device whatever the session says; full volume is as good as anything there.
fn level_for(audible: f32, device: &str, master: &crate::master::Output) -> f32 {
    let on_master = master.device.as_deref() == Some(device);
    let level = if !on_master {
        audible
    } else if master.amplitude > 0.0001 {
        audible / master.amplitude
    } else {
        1.0
    };
    level.clamp(0.0, 1.0)
}

fn target_for(config: &Config, session: &Session, master: &crate::master::Output) -> Option<Target> {
    // Paused after a Restore: Lanes leaves every application exactly as Restore
    // put it until the user resumes. See `restore::reset_windows`.
    if config.settings.paused {
        return None;
    }

    // A development copy of Lanes leaves real applications alone - see
    // `instance::may_manage`. Every apply path comes through here.
    if !crate::instance::may_manage(&session.full_path) {
        return None;
    }

    let resolution = rules::resolve(config, &session.executable, &session.full_path);

    let (channel_id, trim) = match resolution {
        Resolution::Channel { id, trim } => (id, trim),
        Resolution::Ignored | Resolution::Unassigned => return None,
    };

    let channel = config.channel(channel_id)?;

    // The Mic channel never routes audio and never scales playback; it exists
    // for level display and mute only. Guarding here rather than at the call
    // site means no future caller can forget.
    if channel.is_input {
        return None;
    }

    let master_muted = config.master().map(|m| m.muted).unwrap_or(false);

    // The ceiling, in what Master's slider actually does to the sound rather
    // than the number it shows: Windows' slider follows an audio taper, so at
    // "50" it is well under half the amplitude. Capping at the amplitude keeps
    // a channel on another device exactly as loud as the same channel on
    // Master's, and never louder than Master lets anything be.
    let own = if channel.id == "master" { 1.0 } else { channel.volume };
    let ceiling = master.amplitude.clamp(0.0, 1.0);

    Some(Target {
        channel_id: channel.id.clone(),
        audible: (own.min(ceiling) * config.chat_mix.factor_for(&channel.id) * trim.unwrap_or(1.0))
            .clamp(0.0, 1.0),
        muted: master_muted || channel.muted,
        master: master.clone(),
    })
}

/// Bring every live session into line with the config.
///
/// `reason` is recorded in the audit log so a later reader can tell a startup
/// sweep from a user action from a rule firing.
pub fn apply_all(config: &Config, reason: &str) -> winaudio::Result<Applied> {
    let mut report = Applied::default();
    let master = crate::master::Output::read();
    let live = sessions::list()?;
    let policy = AudioPolicyConfig::connect();

    // What is actually plugged in, read once for the whole sweep.
    //
    // Every channel's destination is decided against this list, so a device
    // that vanished takes its channels to their fallbacks in the same pass
    // rather than leaving applications pointed at an endpoint Windows will
    // scatter them off.
    let active = winaudio::devices::list(eRender).unwrap_or_default();

    // Group sessions by executable.
    //
    // Browsers and Electron apps own several sessions, and rerouting an app
    // creates another one while abandoning the old. A channel
    // applies to the application, so the grouping has to happen before any
    // decision is made about it.
    let mut by_exe: HashMap<String, Vec<&Session>> = HashMap::new();
    for session in &live {
        if session.process_id == 0 || session.full_path == "<system>" {
            continue;
        }
        by_exe
            .entry(session.full_path.to_lowercase())
            .or_default()
            .push(session);
    }

    for group in by_exe.values() {
        let Some(first) = group.first() else { continue };

        let resolution = rules::resolve(config, &first.executable, &first.full_path);

        match resolution {
            Resolution::Ignored => {
                report.ignored += 1;
                continue;
            }
            Resolution::Unassigned => {
                report.unassigned.push(first.executable.clone());
                continue;
            }
            Resolution::Channel { .. } => {}
        }

        let Some(target) = target_for(config, first, &master) else {
            continue;
        };

        // One answer to "which device", shared with what the API reports.
        let choice = config
            .channel(&target.channel_id)
            .map(|channel| routing::choose(channel, &config.device_priority, &active))
            .unwrap_or(routing::Choice {
                device: None,
                on_fallback: false,
            });

        apply_to_group(
            group,
            &target,
            choice.device.as_ref().map(|d| d.id.as_str()),
            &policy,
            reason,
            &mut report,
        );
    }

    report.unassigned.sort();
    report.unassigned.dedup();
    Ok(report)
}

/// How long to let Windows finish moving an app to a new device.
///
/// See [`apply_to_group`]. Chosen as "long enough to observe working, short
/// enough not to be felt" — it only ever elapses when a device actually
/// changed, which is rare and already causes an audible reinitialisation
/// glitch of its own.
const DEVICE_SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

/// Apply volume and mute only, for a change that cannot have moved anything.
///
/// # Why this exists: a fader has to be cheap
///
/// [`apply_all`] costs roughly **100ms** per call, measured. Almost none of
/// that is the volume: it is one WinRT activation for the undocumented routing
/// interface, plus a `GetPersistedDefaultAudioEndpoint` for every process, none
/// of which can have changed when the user moved a slider.
///
/// Dragging a fader sends a command per step. At 100ms each, a drag across the
/// full travel would leave the core seconds behind the user's hand, and the
/// audio would still be catching up long after the fader stopped. The window
/// also rations what it sends; this is the half that keeps the sound in step.
///
/// `only_channel` narrows it further: a channel's own volume can only affect
/// the applications in it. `None` means every channel, which is what a Master
/// change or a mute-all needs.
pub fn apply_levels_only(
    config: &Config,
    only_channel: Option<&str>,
    reason: &str,
) -> winaudio::Result<Applied> {
    let mut report = Applied::default();
    let master = crate::master::Output::read();
    let live = sessions::list()?;

    // Grouped by executable for the same reason as in `apply_all`: a channel
    // applies to an application, and browsers own several sessions each.
    let mut by_exe: HashMap<String, Vec<&Session>> = HashMap::new();
    for session in &live {
        if session.process_id == 0 || session.full_path == "<system>" {
            continue;
        }
        by_exe
            .entry(session.full_path.to_lowercase())
            .or_default()
            .push(session);
    }

    // Decide everything first, touch the audio graph once.
    let mut work: Vec<(Vec<u32>, Target, String)> = Vec::new();

    for group in by_exe.values() {
        let Some(first) = group.first() else { continue };

        let Resolution::Channel { id, .. } =
            rules::resolve(config, &first.executable, &first.full_path)
        else {
            continue;
        };

        if let Some(wanted) = only_channel {
            if id != wanted {
                continue;
            }
        }

        let Some(target) = target_for(config, first, &master) else {
            continue;
        };

        let mut pids: Vec<u32> = group.iter().map(|s| s.process_id).collect();
        pids.sort_unstable();
        pids.dedup();

        work.push((pids, target, first.executable.clone()));
    }

    // One sweep of the audio graph for every process, rather than one each.
    // This is the difference between a fader that follows the hand and one
    // that runs seconds behind it.
    let all_pids: Vec<u32> = work
        .iter()
        .flat_map(|(pids, _, _)| pids.iter().copied())
        .collect();
    let controls = sessions::device_volume_controls_for(&all_pids)?;

    for (pids, target, name) in &work {
        for pid in pids {
            let Some(found) = controls.get(pid) else {
                continue;
            };
            set_levels(found, target, name, reason, &mut report);
        }
    }

    Ok(report)
}

/// Apply a channel to every session an application owns.
///
/// # Ordering is load-bearing
///
/// Changing an application's output device does not move its session. Windows
/// creates a **new** session on the new device, asynchronously, and the new
/// session starts at the application's *persisted* volume — not at whatever we
/// just wrote to the old session. Setting the device first and the volume
/// second looks obviously correct and is not:
///
/// 1. device change requested; Windows begins moving the app
/// 2. volume written to the sessions that exist *right now* — the old ones
/// 3. milliseconds later the new session appears, at the persisted volume
///
/// The result would be an app left at the wrong volume while every call
/// reported success. Two things prevent it, and both are needed:
///
/// - **Levels are applied first.** Windows persists per-app volume, so setting
///   it before the move means any session created by the move inherits the
///   right value.
/// - **Levels are applied again after a device change**, once Windows has had
///   [`DEVICE_SETTLE`] to create the new session, because persistence is not
///   instantaneous either.
///
/// The session watcher would eventually correct this by itself when the new
/// session raises its event. Relying on that would leave one-shot commands like
/// `--apply` quietly wrong, which is worse than a 300ms pause.
fn apply_to_group(
    group: &[&Session],
    target: &Target,
    device: Option<&str>,
    policy: &winaudio::Result<AudioPolicyConfig>,
    reason: &str,
    report: &mut Applied,
) {
    let mut pids: Vec<u32> = group.iter().map(|s| s.process_id).collect();
    pids.sort_unstable();
    pids.dedup();

    let name = group[0].executable.clone();
    let path = group[0].full_path.clone();

    // 1. Levels first, so a session created by the device move inherits them.
    apply_levels(&pids, target, &name, reason, report);

    // 2. Device.
    let mut device_changed = false;
    let mut moved: Option<u32> = None;

    if let Ok(policy) = policy {
        for &pid in &pids {
            let current = policy.get_endpoint(pid, eRender).ok().flatten();

            if current.as_deref() == device {
                report.unchanged += 1;
                clear_refused(&path);
                continue;
            }

            // It has already refused this device. Setting it again would only
            // restart its audio and start the loop described at `REFUSED`.
            if already_refused(&path, device) {
                report.routing_refused.push(name.clone());
                continue;
            }

            match policy.set_endpoint(pid, eRender, device) {
                Ok(()) => {
                    audit::change(
                        &name,
                        "endpoint",
                        current,
                        device.map(str::to_string),
                        &format!("{reason}:{}", target.channel_id),
                    );
                    report.devices_changed += 1;
                    device_changed = true;
                    moved.get_or_insert(pid);
                }
                Err(e) => {
                    audit::failure(
                        &name,
                        "endpoint",
                        device.map(str::to_string),
                        reason,
                        &e.to_string(),
                    );
                    report.failures.push(format!("{name} routing: {e}"));
                }
            }
        }
    }

    // 3. Catch the session the move created.
    if device_changed {
        std::thread::sleep(DEVICE_SETTLE);
        apply_levels(&pids, target, &name, reason, report);

        // 4. Did it hold? Read it back once the settle has passed, and if
        //    Windows says the application is not where it was just put, stop.
        //    One restart of its audio, rather than one every third of a second.
        if let (Ok(policy), Some(pid)) = (policy, moved) {
            let now = policy.get_endpoint(pid, eRender).ok().flatten();
            if now.as_deref() == device {
                clear_refused(&path);
            } else {
                mark_refused(&path, device);
                audit::failure(
                    &name,
                    "endpoint",
                    device.map(str::to_string),
                    reason,
                    "set, but something changed it back; Lanes will leave its device alone",
                );
                report.routing_refused.push(name.clone());
            }
        }
    }
}
/// Set volume and mute on every session these processes own.
///
/// Idempotent, and silent when nothing needs changing — which matters because
/// this runs twice whenever a device moves, and the audit log must not fill
/// with duplicate entries for a single user action.
fn apply_levels(pids: &[u32], target: &Target, name: &str, reason: &str, report: &mut Applied) {
    for &pid in pids {
        let controls = match sessions::device_volume_controls_for(&[pid]) {
            Ok(mut c) => c.remove(&pid).unwrap_or_default(),
            Err(e) => {
                report.failures.push(format!("{name} volume lookup: {e}"));
                continue;
            }
        };

        set_levels(&controls, target, name, reason, report);
    }
}

/// Write volume and mute to controls already in hand.
///
/// Split out of [`apply_levels`] so the batched path can reuse it without
/// re-enumerating the audio graph. Every write is preceded by a read and
/// skipped when it would change nothing — re-applying a value a session
/// already has is audible on some devices, and pointless on all of them.
fn set_levels(
    controls: &[winaudio::sessions::DeviceVolume],
    target: &Target,
    name: &str,
    reason: &str,
    report: &mut Applied,
) {
    {
        for found in controls {
            let control = &found.control;
            let level = level_for(target.audible, &found.device, &target.master);
            unsafe {
                let volume_now = control.GetMasterVolume().unwrap_or(-1.0);
                let muted_now = control.GetMute().map(|m| m.as_bool()).unwrap_or(false);

                if (volume_now - level).abs() >= EPSILON {
                    match control.SetMasterVolume(level, std::ptr::null()) {
                        Ok(()) => {
                            audit::change(
                                name,
                                "volume",
                                Some(format!("{volume_now:.2}")),
                                Some(format!("{level:.2}")),
                                &format!("{reason}:{}", target.channel_id),
                            );
                            report.volumes_changed += 1;
                        }
                        Err(e) => {
                            audit::failure(
                                name,
                                "volume",
                                Some(format!("{level:.2}")),
                                reason,
                                &e.to_string(),
                            );
                            report.failures.push(format!("{name} volume: {e}"));
                        }
                    }
                } else {
                    report.unchanged += 1;
                }

                if muted_now != target.muted {
                    match control.SetMute(target.muted, std::ptr::null()) {
                        Ok(()) => {
                            audit::change(
                                name,
                                "mute",
                                Some(muted_now.to_string()),
                                Some(target.muted.to_string()),
                                &format!("{reason}:{}", target.channel_id),
                            );
                            report.mutes_changed += 1;
                        }
                        Err(e) => {
                            report.failures.push(format!("{name} mute: {e}"));
                        }
                    }
                }
            }
        }
    }
}

/// Apply the config to one newly-created session.
///
/// This is what the session watcher calls. Kept separate from [`apply_all`]
/// because a new session should not trigger a sweep of every other application
/// — that would be exactly the "periodic enforcement" this module avoids, just
/// triggered by a different clock.
pub fn apply_to_pid(config: &Config, pid: u32, reason: &str) -> winaudio::Result<Applied> {
    let mut report = Applied::default();
    let master = crate::master::Output::read();

    let live = sessions::list()?;
    let group: Vec<&Session> = live
        .iter()
        .filter(|s| s.process_id == pid && s.process_id != 0)
        .collect();

    let Some(first) = group.first() else {
        return Ok(report);
    };

    match rules::resolve(config, &first.executable, &first.full_path) {
        Resolution::Ignored => {
            report.ignored += 1;
            return Ok(report);
        }
        Resolution::Unassigned => {
            report.unassigned.push(first.executable.clone());
            return Ok(report);
        }
        Resolution::Channel { .. } => {}
    }

    let Some(target) = target_for(config, first, &master) else {
        return Ok(report);
    };

    // The same decision the full sweep makes, against the same list.
    //
    // This is the path a newly created session takes, which is the one that
    // matters most while a device is missing: an application starting up while
    // the DAC is off must land on the fallback, not on an endpoint that is not
    // there.
    let active = winaudio::devices::list(eRender).unwrap_or_default();
    let choice = config
        .channel(&target.channel_id)
        .map(|channel| routing::choose(channel, &config.device_priority, &active))
        .unwrap_or(routing::Choice {
            device: None,
            on_fallback: false,
        });

    let policy = AudioPolicyConfig::connect();
    apply_to_group(
        &group,
        &target,
        choice.device.as_ref().map(|d| d.id.as_str()),
        &policy,
        reason,
        &mut report,
    );

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::level_for;
    use crate::master::Output;

    fn master_on(device: &str, amplitude: f32) -> Output {
        Output {
            device: Some(device.into()),
            amplitude,
        }
    }

    #[test]
    fn apps_on_masters_device_are_not_cut_twice() {
        // Master's slider at a point that halves the sound.
        let master = master_on("{speakers}", 0.5);

        // A channel at or above the ceiling runs at full on the device, so the
        // slider alone decides: heard at 0.5.
        assert_eq!(level_for(0.5, "{speakers}", &master), 1.0);
        // A channel below it keeps its own level once the slider is undone:
        // 0.25 heard means 0.5 set, times the slider's 0.5.
        assert!((level_for(0.25, "{speakers}", &master) - 0.5).abs() < 1e-6);
        // Elsewhere the slider does nothing, so the level is the level.
        assert!((level_for(0.25, "{headset}", &master) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn a_silent_slider_or_no_default_device_asks_for_nothing_impossible() {
        assert_eq!(level_for(0.0, "{speakers}", &master_on("{speakers}", 0.0)), 1.0);
        let none = Output { device: None, amplitude: 1.0 };
        assert!((level_for(0.3, "{speakers}", &none) - 0.3).abs() < 1e-6);
        // Rounding can put audible a hair over the amplitude; never above full.
        assert_eq!(level_for(0.5001, "{speakers}", &master_on("{speakers}", 0.5)), 1.0);
    }

    use super::*;

    // One test for the whole memory, because it is process-wide state and
    // tests run in parallel: separate tests touching it would race.
    #[test]
    fn a_refusal_is_remembered_per_device_and_forgotten_on_request() {
        let path = r"C:\Games\Example\Game.exe";
        let headset = Some("{headset}");

        assert!(!already_refused(path, headset));
        mark_refused(path, headset);

        // Case does not matter, the device does.
        assert!(already_refused(&path.to_uppercase(), headset));
        assert!(!already_refused(path, Some("{speakers}")));
        assert!(!already_refused(path, None));

        // Holding a device clears it.
        clear_refused(path);
        assert!(!already_refused(path, headset));

        // Reassigning by executable name clears it too.
        mark_refused(path, headset);
        forget_refusal("Game.exe");
        assert_eq!(refused_device(path), None);
    }
}
