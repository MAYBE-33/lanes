//! Where Lanes keeps its state.
//!
//! Everything lives under one folder so that uninstalling is "delete the exe
//! and this folder", with nothing left behind. No services, no scheduled
//! tasks, no `HKLM`, no COM registration.

use std::path::{Path, PathBuf};

/// Folder name under `%LOCALAPPDATA%`. If you fork Lanes under another name,
/// change this so the two never share state.
const APP_FOLDER: &str = "Lanes";

/// Marker that switches the app to portable mode.
///
/// If a file with this name sits next to the executable, state goes beside the
/// exe instead of in `%LOCALAPPDATA%` - for running Lanes from a USB stick, or
/// a development build beside an installed copy. The decision is made once,
/// here, so no module has its own idea of where state lives.
const PORTABLE_MARKER: &str = "portable.txt";

/// The root of all application state.
pub fn root() -> PathBuf {
    if let Some(portable) = portable_root() {
        return portable;
    }

    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        // Falling back to the current directory is deliberate rather than
        // panicking: losing the environment variable should degrade to
        // something usable, not stop the app from starting.
        .unwrap_or_else(|_| PathBuf::from("."));

    base.join(APP_FOLDER)
}

fn portable_root() -> Option<PathBuf> {
    Some(portable_dir()?.join(APP_FOLDER))
}

/// The folder the executable sits in, when this is a portable copy.
pub fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join(PORTABLE_MARKER).exists().then(|| dir.to_path_buf())
}

/// Timestamped state captures.
pub fn snapshots_dir() -> PathBuf {
    root().join("snapshots")
}

/// The state of the machine before this app ever touched it.
///
/// **Written once and never overwritten.** Every other snapshot is a
/// convenience; this one is the only record of what the machine looked like
/// before Lanes existed on it, and it can never be recreated.
pub fn first_run_snapshot() -> PathBuf {
    snapshots_dir().join("first-run.json")
}

/// Where the API server publishes the port it actually bound.
///
/// Written somewhere known so clients find the port without configuration —
/// which matters because it is not fixed: if the default (8477) is taken, the
/// server scans upward.
pub fn port_file() -> PathBuf {
    root().join("port")
}

/// Audit log of every change the app makes.
pub fn logs_dir() -> PathBuf {
    root().join("logs")
}

/// Channel and rule configuration. See `docs/configuration.md`.
pub fn config_file() -> PathBuf {
    root().join("config.json")
}

/// Create a folder and any missing parents.
pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}
