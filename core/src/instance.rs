//! Making sure only one core runs at a time.
//!
//! # Why this matters more than it looks
//!
//! Two running cores do not merely duplicate work — they **fight**. Each
//! registers its own session callbacks and each applies its own view of the
//! config, so if one loaded the file before a hand-edit and the other after,
//! they take turns overwriting each other's device assignments. The result is
//! continuous re-routing, and re-routing a live stream causes an audible glitch
//! every time. The machine would stutter until one of them was killed.
//!
//! # Scope
//!
//! This guards the long-running modes only (the tray app and `--watch`).
//! One-shot commands like `--status` and `--snapshot` are read-mostly, and two
//! `--apply` runs converge on the same values rather than fighting, so
//! exclusivity would only get in the way.
//!
//! A second launch that finds the lock taken asks the running instance to show
//! its window, over the local API (see `api::oneshot`), and exits.
//!
//! # Development copies
//!
//! One exception exists for people working on Lanes: a **portable** build
//! started with `LANES_SEPARATE_INSTANCE=1` runs beside an installed Lanes,
//! with its own lock, and changes only the executables listed in a
//! `test-apps.txt` beside it. `docs/building.md` explains how to use it.

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

/// `Local\` rather than `Global\`: the scope we want is this user's logon
/// session, which is also the scope of the audio settings being managed. It
/// additionally avoids the privileges `Global\` can require — and this app is
/// never allowed to need elevation.
///
/// If you fork Lanes under another name, change this too, or your fork and an
/// installed Lanes will each refuse to start while the other runs.
const MUTEX_NAME: &str = r"Local\Lanes.SingleInstance";

/// The environment variable that lets a portable development copy run beside
/// an installed one. See [`mutex_name`].
const SEPARATE_INSTANCE: &str = "LANES_SEPARATE_INSTANCE";

/// Whether this is a second copy - a portable development copy started with
/// `LANES_SEPARATE_INSTANCE=1` - running beside an installed Lanes.
pub fn is_separate() -> bool {
    std::env::var(SEPARATE_INSTANCE).is_ok_and(|v| v == "1") && crate::paths::portable_dir().is_some()
}

/// The executables a separate copy may touch, read once from `test-apps.txt`
/// beside it. `None` when this is not a separate copy, which may touch anything.
static ALLOWED: std::sync::LazyLock<Option<Vec<String>>> = std::sync::LazyLock::new(|| {
    if !is_separate() {
        return None;
    }
    let file = crate::paths::portable_dir()?.join("test-apps.txt");
    let text = std::fs::read_to_string(file).unwrap_or_default();
    Some(parse_allowed(&text))
});

fn parse_allowed(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or("").trim().to_lowercase())
        .filter(|line| !line.is_empty())
        .collect()
}

fn allowed_by(allowed: &[String], full_path: &str) -> bool {
    let name = full_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(full_path)
        .to_lowercase();
    allowed.contains(&name)
}

/// Whether this copy of Lanes may change this application's audio.
///
/// # Why a second copy is hands-off
///
/// A separate copy exists so changes can be tried out beside your own Lanes
/// without quitting it. But two managers of one application cannot both win:
/// if both had a rule for the same game, each would undo the other's output
/// device and levels on every new session the other caused, several times a
/// second, and the game's audio would break up.
///
/// So a second copy manages **only** the executables named in `test-apps.txt`
/// beside it, one per line - typically a silent test player or two - and leaves
/// every other application exactly as it finds it, whatever its rules say. With no file it
/// touches nothing at all. The installed copy is never affected.
pub fn may_manage(full_path: &str) -> bool {
    match ALLOWED.as_ref() {
        None => true,
        Some(allowed) => allowed_by(allowed, full_path),
    }
}

/// The lock this process competes for.
///
/// # A developer switch, and why it is safe
///
/// Always [`MUTEX_NAME`], with one exception: a **portable** copy started with
/// `LANES_SEPARATE_INSTANCE=1` gets a lock of its own, named after its folder.
///
/// A portable copy keeps all its state beside its executable, so it cannot
/// touch the installed copy's config, and [`may_manage`] keeps it away from
/// every application not on its list. Both conditions are required so an
/// ordinary install can never end up running twice: the variable alone does
/// nothing without the portable marker.
fn mutex_name() -> String {
    let separate = std::env::var(SEPARATE_INSTANCE).is_ok_and(|v| v == "1");

    match crate::paths::portable_dir() {
        Some(dir) if separate => {
            // FNV-1a over the folder, so two portable copies in different
            // places are also kept apart from each other.
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for byte in dir.to_string_lossy().to_lowercase().bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
            format!("{MUTEX_NAME}.{hash:016x}")
        }
        _ => MUTEX_NAME.to_string(),
    }
}

/// Held for as long as this process is the single instance.
pub struct InstanceLock {
    handle: HANDLE,
}

pub enum Acquired {
    /// This process now owns the lock.
    Yes(InstanceLock),
    /// Another core is already running.
    AlreadyRunning,
}

/// Try to become the single running instance.
///
/// The mutex is never *waited* on — we only care whether it already existed.
/// Waiting would mean a second launch silently blocking until the first exits,
/// which looks identical to a hang.
pub fn acquire() -> Acquired {
    unsafe {
        let handle = match CreateMutexW(None, true, &HSTRING::from(mutex_name())) {
            Ok(h) => h,
            Err(e) => {
                // Failing to create the mutex is not a reason to refuse to run.
                // The consequence of being wrong here is a duplicate instance,
                // which is bad; the consequence of refusing is an app that will
                // not start at all, which is worse.
                eprintln!("warning: could not create the single-instance lock: {e}");
                eprintln!("  continuing, but check that no other copy is running");
                return Acquired::Yes(InstanceLock {
                    handle: HANDLE::default(),
                });
            }
        };

        // CreateMutexW succeeds whether or not the mutex already existed; the
        // last error is the only thing that distinguishes them.
        if windows::Win32::Foundation::GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return Acquired::AlreadyRunning;
        }

        Acquired::Yes(InstanceLock { handle })
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{allowed_by, parse_allowed};

    #[test]
    fn a_test_copy_touches_only_what_it_is_told_to() {
        let allowed = parse_allowed("# silent players\ntonetwo.exe\n  PowerShell.exe  # the notice test\n\n");
        assert_eq!(allowed, ["tonetwo.exe", "powershell.exe"]);
        assert!(allowed_by(&allowed, r"C:\Temp\scratch\tonetwo.exe"));
        assert!(allowed_by(&allowed, r"C:\Windows\System32\WindowsPowerShell\v1.0\POWERSHELL.EXE"));
        assert!(!allowed_by(&allowed, r"C:\Games\Example\Game.exe"));
        // No file, nothing allowed.
        assert!(!allowed_by(&parse_allowed(""), r"C:\Temp\tonetwo.exe"));
    }
}
