//! Keeping the command line usable in a program that has no console.
//!
//! # Why this exists
//!
//! Built as a **console** program, `Lanes.exe` would get a console window
//! whether anyone wanted one or not: double-clicking it would put a black
//! command prompt in front of the mixer — and, far worse, **closing that window
//! would kill the core**, taking the tray icon, the session watcher and every
//! routing rule with it. The rules would not be lost; nothing would be left
//! running to apply them, which looks exactly the same.
//!
//! So it is a desktop program: `#![windows_subsystem = "windows"]` on the crate
//! root. That has one cost: such a program has no
//! standard output at all, so `--restore`, `--status` and `--help` would print
//! into nothing. Given `--restore` is the documented way out of a broken audio
//! configuration, printing nothing is not an acceptable trade.
//!
//! So this reattaches to the console of whoever launched us, if there was one.
//! Run from a terminal, output appears there as before. Double-clicked, there
//! is no parent console, nothing is attached, and no window appears.
//!
//! # The part that is easy to get wrong
//!
//! `AttachConsole` on its own is not enough. A windows-subsystem program starts
//! with **no standard handles**, and Rust's `println!` resolves `GetStdHandle`
//! once, lazily, on first use — so if the handles are still empty at that
//! moment, every later write is silently discarded and the attach looks like it
//! did nothing.
//!
//! The handles therefore have to be pointed at the console explicitly, and
//! **before anything prints**. [`attach_to_parent`] is called as the first
//! statement in `main` for that reason.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Console::{
    AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_HANDLE,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};

/// Attach to the launching terminal's console, if there is one.
///
/// **Call this before anything writes to stdout or stderr.** Returns `true`
/// when a console was attached, meaning the program was run from a command line
/// and its output will be seen.
///
/// Failure is the normal case rather than an error: it means the executable was
/// double-clicked, and a desktop program with nothing to print to is exactly
/// what is wanted there.
pub fn attach_to_parent() -> bool {
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return false;
        }

        // `CONOUT$` and `CONIN$` are the console's own streams, reachable by
        // name once attached. Opening them is what gives us handles to install;
        // without this the attach succeeds and every `println!` still vanishes.
        //
        // **Only where there is not already a usable handle.** Installing them
        // unconditionally would break redirection: a caller that had piped our
        // output - a PowerShell pipeline, or any program capturing it - would
        // have its pipe replaced by the console, and the text would go somewhere
        // the caller was not looking. Inherited handles are the caller's
        // instruction about where output should go, and they win.
        let mut attached = false;

        if !usable(STD_OUTPUT_HANDLE) {
            if let Some(handle) = open(w!("CONOUT$"), true) {
                let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
                attached = true;
            }
        } else {
            attached = true;
        }

        if !usable(STD_ERROR_HANDLE) {
            if let Some(handle) = open(w!("CONOUT$"), true) {
                let _ = SetStdHandle(STD_ERROR_HANDLE, handle);
            }
        }

        if !usable(STD_INPUT_HANDLE) {
            if let Some(handle) = open(w!("CONIN$"), false) {
                let _ = SetStdHandle(STD_INPUT_HANDLE, handle);
            }
        }

        attached
    }
}

/// Whether a standard handle already points somewhere output can go.
///
/// A windows-subsystem process usually starts with these empty, but not always
/// — a caller that redirected our output passes real handles in, and those are
/// an instruction about where the output belongs.
///
/// # Safety
///
/// Calls `GetStdHandle`, which is safe for any of the three standard ids.
unsafe fn usable(which: STD_HANDLE) -> bool {
    match GetStdHandle(which) {
        Ok(handle) => !handle.is_invalid() && handle != INVALID_HANDLE_VALUE,
        Err(_) => false,
    }
}

/// Open one of the console's named streams.
///
/// # Safety
///
/// `name` must be a null-terminated wide string naming a console stream.
unsafe fn open(name: PCWSTR, writable: bool) -> Option<HANDLE> {
    let access = if writable {
        FILE_GENERIC_READ | FILE_GENERIC_WRITE
    } else {
        FILE_GENERIC_READ
    };

    let handle = CreateFileW(
        name,
        access.0,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        None,
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL,
        None,
    )
    .ok()?;

    (handle != INVALID_HANDLE_VALUE).then_some(handle)
}
