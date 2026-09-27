//! Start with Windows.
//!
//! # The only registry value this app ever writes
//!
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, and nothing else. Not
//! `HKLM`, not audio driver keys, not device properties, not anything under
//! `MMDevices`. That is a design rule, and it is what makes uninstalling honest:
//! untick the setting, quit, delete the exe and the `%LOCALAPPDATA%` folder, and
//! nothing remains.
//!
//! No scheduled task and no service. The app manages this value itself, so it
//! works the same whether Lanes was installed or is being run portably.
//!
//! # Why the stale-path check exists
//!
//! The `Run` value stores an absolute path. This is a portable executable
//! people are expected to move. Once moved, the stored path points at nothing
//! and start-on-boot silently stops working — the user would find out weeks
//! later, if ever.
//!
//! So [`reconcile`] runs at startup: if the setting is on but the recorded path
//! is not where we are, it is corrected quietly.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, REG_VALUE_TYPE,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Lanes";

/// The executable a registered command line starts.
///
/// Handles the quoted form this module writes, and an unquoted one in case the
/// entry was written by hand.
fn registered_exe(command: &str) -> &str {
    let command = command.trim();
    match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => command.split(' ').next().unwrap_or(command),
    }
}

/// Whether a registered entry that differs from ours should be replaced.
///
/// Yes if it starts this very executable (only the flag differs), or if the
/// executable it names has gone. No if it names another copy that still
/// exists - that copy is somebody's start-on-boot and not ours to take. See
/// [`reconcile`].
fn needs_correcting(registered: &str, ours: &std::path::Path) -> bool {
    let theirs = std::path::Path::new(registered_exe(registered));

    let same = theirs
        .to_string_lossy()
        .eq_ignore_ascii_case(&ours.to_string_lossy());

    same || !theirs.exists()
}

/// The command line we register.
///
/// `--minimised` is appended when "Start minimised" is on: one setting
/// affecting the registered command, not a second registry value to keep in
/// sync.
fn command_line(minimised: bool) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.to_string_lossy();

    Some(if minimised {
        format!("\"{path}\" --minimised")
    } else {
        format!("\"{path}\"")
    })
}

fn open(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
    let mut key = HKEY::default();

    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            Some(0),
            access,
            &mut key,
        )
    };

    (result == ERROR_SUCCESS).then_some(key)
}

/// What is currently registered, if anything.
pub fn current() -> Option<String> {
    read(VALUE_NAME)
}

/// Read one value from the Run key.
fn read(value_name: &str) -> Option<String> {
    let key = open(KEY_READ)?;

    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;

    // First call with a null buffer asks how much space is needed.
    let probe = unsafe {
        RegQueryValueExW(
            key,
            &HSTRING::from(value_name),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };

    if probe != ERROR_SUCCESS || size == 0 {
        unsafe {
            let _ = RegCloseKey(key);
        }
        return None;
    }

    let mut buffer = vec![0u8; size as usize];

    let read = unsafe {
        RegQueryValueExW(
            key,
            &HSTRING::from(value_name),
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr()),
            Some(&mut size),
        )
    };

    unsafe {
        let _ = RegCloseKey(key);
    }

    if read != ERROR_SUCCESS {
        return None;
    }

    // REG_SZ is UTF-16 with a trailing NUL.
    let wide: Vec<u16> = buffer
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|c| *c != 0)
        .collect();

    Some(String::from_utf16_lossy(&wide))
}

pub fn is_enabled() -> bool {
    current().is_some()
}

pub fn enable(minimised: bool) -> Result<(), String> {
    let command = command_line(minimised).ok_or("could not determine this executable's path")?;
    let key = open(KEY_WRITE).ok_or("could not open the Run key for writing")?;

    let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = unsafe {
        std::slice::from_raw_parts(wide.as_ptr() as *const u8, std::mem::size_of_val(&wide[..]))
    };

    let result = unsafe {
        RegSetValueExW(
            key,
            &HSTRING::from(VALUE_NAME),
            Some(0),
            REG_SZ,
            Some(bytes),
        )
    };

    unsafe {
        let _ = RegCloseKey(key);
    }

    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("registry write failed ({result:?})"))
    }
}

pub fn disable() -> Result<(), String> {
    let Some(key) = open(KEY_WRITE) else {
        // No key means nothing to remove, which is the desired end state.
        return Ok(());
    };

    let result = unsafe { RegDeleteValueW(key, PCWSTR(HSTRING::from(VALUE_NAME).as_ptr())) };
    unsafe {
        let _ = RegCloseKey(key);
    }

    // "Not found" is success here: the caller wanted it gone.
    if result == ERROR_SUCCESS || result == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!("registry delete failed ({result:?})"))
    }
}

/// Correct a stale registered path.
///
/// Called at startup. Silent when nothing needs doing — this is housekeeping,
/// not something to announce every launch.
///
/// # Stale means gone, not different
///
/// A *different* path is not the same as a stale one. Treating them alike would
/// mean that starting a second copy of Lanes - a portable one on a USB stick,
/// or a development build - silently re-pointed the installed copy's
/// start-on-boot entry at it, and the next boot would start whichever copy
/// happened to run last, on that copy's settings.
///
/// So a registered path is replaced only when the executable it names no
/// longer exists. When it names *this* executable, the entry is still
/// rewritten if the `--minimised` flag has drifted from the setting.
///
/// Returns `Some(old_path)` when a correction was made, so the caller can log
/// it. A change to a registry value is still a change, and the audit log should
/// carry it.
pub fn reconcile(minimised: bool) -> Option<String> {
    let registered = current()?;
    let expected = command_line(minimised)?;

    if registered == expected {
        return None;
    }

    let ours = std::env::current_exe().ok()?;
    if !needs_correcting(&registered, &ours) {
        return None;
    }

    match enable(minimised) {
        Ok(()) => Some(registered),
        Err(_) => None,
    }
}

#[cfg(test)]
mod reconcile_tests {
    use super::*;

    #[test]
    fn the_executable_is_read_out_of_either_form() {
        assert_eq!(
            registered_exe(r#""C:\Program Files\Lanes\Lanes.exe" --minimised"#),
            r"C:\Program Files\Lanes\Lanes.exe"
        );
        assert_eq!(registered_exe(r"C:\Lanes\Lanes.exe --minimised"), r"C:\Lanes\Lanes.exe");
    }

    #[test]
    fn another_copy_that_still_exists_is_left_alone() {
        // This test binary certainly exists, so it stands in for "the
        // installed copy" while a different path plays "the copy running now".
        let installed = std::env::current_exe().unwrap();
        let registered = format!("\"{}\" --minimised", installed.display());
        let portable = std::path::Path::new(r"D:\usb\Lanes\Lanes.exe");

        assert!(!needs_correcting(&registered, portable));
    }

    #[test]
    fn a_path_that_has_gone_is_corrected() {
        let registered = r#""C:\nowhere\that\exists\Lanes.exe" --minimised"#;
        let ours = std::env::current_exe().unwrap();

        assert!(needs_correcting(registered, &ours));
    }

    #[test]
    fn our_own_entry_with_a_drifted_flag_is_corrected() {
        let ours = std::env::current_exe().unwrap();
        let registered = format!("\"{}\"", ours.display().to_string().to_uppercase());

        assert!(needs_correcting(&registered, &ours));
    }
}
