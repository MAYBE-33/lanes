//! Capturing the machine's audio state.
//!
//! # Keyed on executable, never on PID
//!
//! This is the central design decision. Process IDs are unusable for a
//! restore, for two reasons:
//!
//! 1. **PIDs do not survive a restart.** Close Spotify and reopen it and the
//!    entry no longer refers to anything.
//! 2. **PIDs are reused.** Replaying an old snapshot could route a completely
//!    unrelated process that happens to have inherited the number. That is not
//!    a failed restore; it is a restore that breaks something new.
//!
//! So state is grouped by executable path, which is stable across restarts and
//! is the same key the rule engine uses.
//!
//! # What "restore" can and cannot reach
//!
//! `SetPersistedDefaultAudioEndpoint` addresses an application **by PID**, so
//! an application can only be changed while it is running. Windows resolves
//! that PID to an application identity and persists the setting against it.
//!
//! The consequence is unavoidable and must not be hidden: **an application can
//! only be reset while it is running.** Restore handles the rest by pausing
//! Lanes and resetting each remaining application the next time it plays (see
//! `restore`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use winaudio::devices::{self, Device};
use winaudio::policy::AudioPolicyConfig;
use winaudio::sessions;
use windows::Win32::Media::Audio::eRender;

use crate::audit;
use crate::paths;

/// Bumped when the on-disk shape changes.
///
/// A schema version, so a future build can migrate an old file instead of
/// refusing to start or, worse, misreading it.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub taken_at: String,
    /// Why this was captured: `first-run`, `manual`, `pre-change`.
    pub reason: String,
    pub system_default_output: Option<String>,
    pub system_default_input: Option<String>,
    pub devices: Vec<Device>,
    /// One entry per executable, keyed by lowercased full path.
    pub apps: Vec<AppState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    /// Lowercased full path — the stable identity.
    pub path: String,
    /// `Spotify.exe`, for humans reading the file.
    pub executable: String,
    /// The persisted output endpoint, or `None` for "follows system default".
    ///
    /// `None` is a real, restorable value, not missing data: restoring it means
    /// actively clearing any assignment the app has acquired since.
    pub endpoint: Option<String>,
    /// Per-session levels. An app commonly owns several.
    pub sessions: Vec<SessionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub device_id: String,
    pub volume: f32,
    pub muted: bool,
}

/// Read the machine's current audio state.
pub fn capture(reason: &str) -> winaudio::Result<Snapshot> {
    let device_list = devices::list_all()?;
    let live_sessions = sessions::list()?;
    let policy = AudioPolicyConfig::connect();

    if let Err(e) = &policy {
        // Not fatal. A snapshot covering volumes is worth far more than no
        // snapshot, and routing being unreadable is itself worth recording.
        eprintln!("warning: routing could not be read, snapshot covers volumes only: {e}");
    }

    // Group by executable path. Sessions belonging to one app collapse into a
    // single entry, which is what makes the result restorable later.
    let mut by_path: BTreeMap<String, AppState> = BTreeMap::new();

    for session in &live_sessions {
        // PID 0 is the system's own per-device session. It has no executable
        // and nothing meaningful to restore.
        if session.process_id == 0 || session.full_path == "<system>" {
            continue;
        }

        let key = session.full_path.to_lowercase();

        let entry = by_path.entry(key.clone()).or_insert_with(|| AppState {
            path: key,
            executable: session.executable.clone(),
            endpoint: policy
                .as_ref()
                .ok()
                .and_then(|p| p.get_endpoint(session.process_id, eRender).ok())
                .flatten(),
            sessions: Vec::new(),
        });

        entry.sessions.push(SessionState {
            device_id: session.on_device_id.clone(),
            volume: session.volume,
            muted: session.muted,
        });
    }

    let default_of = |dir: &str| {
        device_list
            .iter()
            .find(|d| d.is_default && d.direction == dir)
            .map(|d| d.id.clone())
    };

    Ok(Snapshot {
        schema_version: SCHEMA_VERSION,
        taken_at: audit::now_iso(),
        reason: reason.to_string(),
        system_default_output: default_of("output"),
        system_default_input: default_of("input"),
        devices: device_list,
        apps: by_path.into_values().collect(),
    })
}

/// Write a snapshot atomically: temp file, then rename.
///
/// The config file is written the same way. It matters at least as much here —
/// a snapshot half-written during a crash is worse than none,
/// because it looks like a usable safety net and is not.
fn write_atomic(path: &Path, snapshot: &Snapshot) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(snapshot).map_err(std::io::Error::other)?;

    let temp = path.with_extension("tmp");
    std::fs::write(&temp, json)?;
    std::fs::rename(&temp, path)
}

/// Save a timestamped snapshot.
pub fn save(snapshot: &Snapshot) -> std::io::Result<PathBuf> {
    let dir = paths::snapshots_dir();
    paths::ensure_dir(&dir)?;

    let stamp = snapshot.taken_at.replace([':', '+'], "-").replace('.', "-");
    let path = dir.join(format!("snapshot-{stamp}.json"));

    write_atomic(&path, snapshot)?;
    prune(&dir);
    Ok(path)
}

/// How many timestamped snapshots to keep, excluding `first-run.json`.
const KEEP_RECENT: usize = 20;

/// Delete the oldest snapshots beyond [`KEEP_RECENT`].
///
/// **`first-run.json` is never considered**, because it lives at a fixed name
/// this filter does not match. That is load-bearing, not incidental: it is the
/// only record of the machine before this app touched it, and it cannot be
/// recreated.
fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let mut snapshots: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("snapshot-"))
        .collect();

    if snapshots.len() <= KEEP_RECENT {
        return;
    }

    snapshots.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    for old in &snapshots[..snapshots.len() - KEEP_RECENT] {
        let _ = std::fs::remove_file(old.path());
    }
}

/// Write `first-run.json` if it does not already exist.
///
/// Returns the path when it was created, `None` when one was already there.
///
/// **Refuses to overwrite.** This runs on every startup, and a bug that let it
/// rewrite the file would quietly destroy the original state the moment the app
/// was launched a second time — replacing the record of "before Lanes"
/// with a record of "after Lanes", which is worthless.
pub fn ensure_first_run() -> winaudio::Result<Option<PathBuf>> {
    let path = paths::first_run_snapshot();

    if path.exists() {
        return Ok(None);
    }

    let snapshot = capture("first-run")?;

    paths::ensure_dir(&paths::snapshots_dir())
        .map_err(|e| winaudio::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))?;

    write_atomic(&path, &snapshot)
        .map_err(|e| winaudio::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))?;

    audit::change(
        "<machine>",
        "first-run-snapshot",
        None,
        Some(path.display().to_string()),
        "first-run",
    );

    Ok(Some(path))
}

pub fn load(path: &Path) -> std::io::Result<Snapshot> {
    let text = std::fs::read_to_string(path)?;
    let snapshot: Snapshot = serde_json::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    if snapshot.schema_version > SCHEMA_VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "snapshot uses schema version {} but this build understands up to {}. \
                 Refusing to guess at its meaning.",
                snapshot.schema_version, SCHEMA_VERSION
            ),
        ));
    }

    Ok(snapshot)
}

