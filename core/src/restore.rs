//! Putting the machine back as it was before Lanes.
//!
//! # What Restore means
//!
//! Restore puts Windows audio back the way it was before Lanes: every app at
//! 100% on one device. It does not replay a snapshot. It resets:
//!
//! - **every application** - running now, or ever touched by Lanes - to 100%,
//!   unmuted, and following Windows' default device (no per-app route);
//! - **Windows' default output** to the one it was before Lanes first changed
//!   it, and **each device's own volume and mute** to what they were before
//!   Lanes first changed them (`master::Before`);
//!
//! and then **pauses Lanes**, because otherwise the rules would put every app
//! straight back into its channel the moment it made a sound.
//!
//! Replaying a snapshot sounds equivalent and is not: the newest snapshot might
//! be one taken before a config import, which is a moment *with* Lanes, not
//! before it. Snapshots are still taken and kept, as a record.
//!
//! # Applications that are not running
//!
//! Windows only lets an application's audio settings be changed while it is
//! running. Every application Lanes has ever changed - from the audit log -
//! and every one it has a rule for, that is not running now, goes on a
//! **pending** list. While Lanes is paused, each is reset the first time it
//! plays and then taken off the list. Resuming clears the list: from then on
//! Lanes is managing them again.
//!
//! # Never the global clear
//!
//! `ClearAllPersistedApplicationDefaultEndpoints` is still never called. Every
//! application is reset individually, which also means every change is in the
//! audit log with its old value.
//!
//! # Ordering is load-bearing
//!
//! Levels first, then the device, then levels again once the new session has
//! appeared - the same ordering the engine uses and for the same reason (see
//! `engine::apply_to_group`): moving an app to another device makes Windows
//! create a new session at the app's remembered volume.

use std::collections::{BTreeSet, HashMap};

use winaudio::policy::AudioPolicyConfig;
use winaudio::sessions;
use windows::Win32::Media::Audio::eRender;

use crate::audit;
use crate::config::Config;
use crate::master::Before;
use crate::paths;

const EPSILON: f32 = 0.001;

/// How long to let Windows finish creating the session a device change makes.
const DEVICE_SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Debug, Default)]
pub struct Report {
    pub apps_reset: usize,
    pub already_correct: usize,
    /// Reset when they next play, while Lanes is paused.
    pub pending: Vec<String>,
    pub windows_restored: Vec<String>,
    pub failures: Vec<String>,
}

impl Report {
    pub fn print(&self) {
        println!("Reset {} application(s) to 100%, unmuted, on the default device.", self.apps_reset);
        if self.already_correct > 0 {
            println!("{} were already like that.", self.already_correct);
        }
        for line in &self.windows_restored {
            println!("{line}");
        }
        if !self.pending.is_empty() {
            println!(
                "\n{} application(s) are not running, so Windows will not let them be changed yet.\n\
                 Lanes is paused and will reset each the first time it plays:",
                self.pending.len()
            );
            for name in &self.pending {
                println!("  - {name}");
            }
        }
        println!("\nLanes is paused. Resume it from the tray, the mixer or Settings.");
        if !self.failures.is_empty() {
            println!("\n{} thing(s) could not be put back:", self.failures.len());
            for failure in &self.failures {
                println!("  - {failure}");
            }
        }
    }
}

/// Put Windows audio back as it was before Lanes, and pause Lanes.
///
/// The caller saves the config (which now says `paused`) and, in the core,
/// pulls Master from Windows afterwards.
pub fn reset_windows(config: &mut Config) -> winaudio::Result<Report> {
    let mut report = Report::default();
    config.settings.paused = true;

    let policy = AudioPolicyConfig::connect();

    // Every application playing now.
    let live = sessions::list()?;
    let mut groups: HashMap<String, (String, Vec<u32>)> = HashMap::new();
    for session in &live {
        if session.process_id == 0 || session.full_path == "<system>" {
            continue;
        }
        // A development copy resets only what it may touch - never the
        // machine. See `instance::may_manage`.
        if !crate::instance::may_manage(&session.full_path) {
            continue;
        }
        let entry = groups
            .entry(session.full_path.to_lowercase())
            .or_insert_with(|| (session.executable.clone(), Vec::new()));
        if !entry.1.contains(&session.process_id) {
            entry.1.push(session.process_id);
        }
    }
    for (name, pids) in groups.values() {
        reset_app(name, pids, &policy, &mut report);
    }

    // Windows' own settings, as they were before Lanes first changed them.
    // Never from a development copy: they are the machine's, and the
    // installed Lanes'.
    if !crate::instance::is_separate() {
        restore_windows_devices(&mut report);
    }

    // Everything Lanes has touched that is not running: reset when it plays.
    let running: BTreeSet<String> = groups.values().map(|(name, _)| name.to_lowercase()).collect();
    let pending = pending_from(&touched_executables(), &config.rules, &running);
    report.pending = pending.iter().cloned().collect();
    save_pending(&pending);

    Ok(report)
}

/// While paused: if this process belongs to an application still waiting to be
/// reset, reset it now and take it off the list. True if it was reset.
pub fn reset_if_pending(pid: u32) -> bool {
    let mut pending = load_pending();
    if pending.is_empty() {
        return false;
    }
    let Ok(live) = sessions::list() else { return false };
    let Some(session) = live.iter().find(|s| s.process_id == pid) else {
        return false;
    };
    if !crate::instance::may_manage(&session.full_path) {
        return false;
    }
    let key = session.executable.to_lowercase();
    if !pending.remove(&key) {
        return false;
    }

    let mut report = Report::default();
    reset_app(&session.executable, &[pid], &AudioPolicyConfig::connect(), &mut report);
    save_pending(&pending);
    true
}

/// Stop being paused: Lanes manages applications again, so nothing is pending.
pub fn resume(config: &mut Config) {
    config.settings.paused = false;
    let _ = std::fs::remove_file(pending_path());
}

/// One application back to 100%, unmuted, following the default device.
fn reset_app(
    name: &str,
    pids: &[u32],
    policy: &winaudio::Result<AudioPolicyConfig>,
    report: &mut Report,
) {
    let changed_levels = reset_levels(name, pids, report);

    let mut moved = false;
    if let Ok(policy) = policy {
        for &pid in pids {
            let current = policy.get_endpoint(pid, eRender).ok().flatten();
            if current.is_none() {
                continue;
            }
            match policy.set_endpoint(pid, eRender, None) {
                Ok(()) => {
                    audit::change(name, "endpoint", current, None, "restore");
                    moved = true;
                }
                Err(e) => {
                    audit::failure(name, "endpoint", None, "restore", &e.to_string());
                    report.failures.push(format!("{name} device: {e}"));
                }
            }
        }
    }

    if moved {
        std::thread::sleep(DEVICE_SETTLE);
        reset_levels(name, pids, report);
    }

    if changed_levels || moved {
        report.apps_reset += 1;
    } else {
        report.already_correct += 1;
    }
}

/// Every session these processes own, on any device, to 100% and unmuted.
fn reset_levels(name: &str, pids: &[u32], report: &mut Report) -> bool {
    let controls = match sessions::device_volume_controls_for(pids) {
        Ok(c) => c,
        Err(e) => {
            report.failures.push(format!("{name} volume lookup: {e}"));
            return false;
        }
    };

    let mut changed = false;
    for found in controls.values().flatten() {
        unsafe {
            let volume = found.control.GetMasterVolume().unwrap_or(-1.0);
            let muted = found.control.GetMute().map(|m| m.as_bool()).unwrap_or(false);

            if (volume - 1.0).abs() >= EPSILON {
                match found.control.SetMasterVolume(1.0, std::ptr::null()) {
                    Ok(()) => {
                        audit::change(name, "volume", Some(format!("{volume:.2}")), Some("1.00".into()), "restore");
                        changed = true;
                    }
                    Err(e) => report.failures.push(format!("{name} volume: {e}")),
                }
            }
            if muted {
                match found.control.SetMute(false, std::ptr::null()) {
                    Ok(()) => {
                        audit::change(name, "mute", Some("true".into()), Some("false".into()), "restore");
                        changed = true;
                    }
                    Err(e) => report.failures.push(format!("{name} mute: {e}")),
                }
            }
        }
    }
    changed
}

/// The default output device, and each device's own volume and mute, back to
/// what they were before Lanes first changed them.
///
/// Only what Lanes changed is touched: `Before` records a setting the first
/// time Lanes changes it, so a device Lanes never touched is not in it. The
/// default device falls back to the first-run snapshot's record when Lanes
/// changed it before `Before` existed.
fn restore_windows_devices(report: &mut Report) {
    let before = Before::load();
    let present: Vec<String> = winaudio::devices::list(windows::Win32::Media::Audio::eRender)
        .map(|all| all.into_iter().map(|d| d.id).collect())
        .unwrap_or_default();

    let wanted_default = match &before.default_output {
        Some(recorded) => recorded.clone(),
        None => crate::snapshot::load(&paths::first_run_snapshot())
            .ok()
            .and_then(|s| s.system_default_output),
    };
    if let Some(wanted) = wanted_default.filter(|id| present.contains(id)) {
        let current = winaudio::endpoint::default_output_id().ok().flatten();
        if current.as_deref() != Some(wanted.as_str()) {
            match winaudio::default_device::set_default_output(&wanted) {
                Ok(()) => {
                    audit::change("<windows>", "default-output", current, Some(wanted.clone()), "restore");
                    report.windows_restored.push("Put the default output device back.".into());
                }
                Err(e) => report.failures.push(format!("default output device: {e}")),
            }
        }
    }

    for (id, was) in &before.endpoints {
        if !present.contains(id) {
            continue;
        }
        let Ok(volume) = winaudio::endpoint::EndpointVolume::open(id) else {
            continue;
        };
        let now = volume.scalar().unwrap_or(was.volume);
        if (now - was.volume).abs() > 0.005 {
            match volume.set_scalar(was.volume) {
                Ok(()) => {
                    audit::change("<device>", "volume", Some(format!("{now:.2}")), Some(format!("{:.2}", was.volume)), "restore");
                    report.windows_restored.push(format!("Put a device's volume back to {:.0}%.", was.volume * 100.0));
                }
                Err(e) => report.failures.push(format!("device volume: {e}")),
            }
        }
        if volume.muted().unwrap_or(was.muted) != was.muted {
            let _ = volume.set_muted(was.muted);
        }
    }
}

// ---------------------------------------------------------------------------
// The pending list
// ---------------------------------------------------------------------------

/// Every executable Lanes has ever changed, from the audit logs - lower-cased.
fn touched_executables() -> BTreeSet<String> {
    let mut touched = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(paths::logs_dir()) else {
        return touched;
    };
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
        for line in text.lines() {
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let field = row.get("field").and_then(|f| f.as_str()).unwrap_or("");
            let target = row.get("target").and_then(|t| t.as_str()).unwrap_or("");
            if matches!(field, "volume" | "mute" | "endpoint") && !target.starts_with('<') && !target.is_empty() {
                touched.insert(target.to_lowercase());
            }
        }
    }
    touched
}

/// Which applications still need resetting: those Lanes touched or has a rule
/// for, less those just reset because they were running. Wildcard rules name no
/// single application and are skipped.
fn pending_from(
    touched: &BTreeSet<String>,
    rules: &[crate::config::Rule],
    running: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut pending: BTreeSet<String> = touched.clone();
    for rule in rules {
        if !rule.pattern.contains(['*', '?']) {
            pending.insert(rule.pattern.to_lowercase());
        }
    }
    pending.retain(|name| !running.contains(name));
    pending
}

fn pending_path() -> std::path::PathBuf {
    paths::snapshots_dir().join("restore-pending.json")
}

fn load_pending() -> BTreeSet<String> {
    std::fs::read_to_string(pending_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_pending(pending: &BTreeSet<String>) {
    if pending.is_empty() {
        let _ = std::fs::remove_file(pending_path());
        return;
    }
    let _ = paths::ensure_dir(&paths::snapshots_dir());
    if let Ok(text) = serde_json::to_string_pretty(pending) {
        let _ = std::fs::write(pending_path(), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rule;

    fn rule(pattern: &str) -> Rule {
        Rule {
            pattern: pattern.into(),
            channel: "game".into(),
            path_contains: None,
            trim: None,
        }
    }

    #[test]
    fn pending_is_everything_touched_or_ruled_that_did_not_just_get_reset() {
        let touched: BTreeSet<String> = ["discord.exe", "game.exe"].map(String::from).into();
        let rules = [rule("Spotify.exe"), rule("*.exe"), rule("Discord.exe")];
        let running: BTreeSet<String> = ["discord.exe".to_string()].into();

        let pending = pending_from(&touched, &rules, &running);
        // Discord was running, so it was reset already; the wildcard names no app.
        assert_eq!(
            pending.into_iter().collect::<Vec<_>>(),
            ["game.exe", "spotify.exe"]
        );
    }
}
