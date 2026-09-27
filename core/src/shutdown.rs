//! Stopping the whole process, from a thread that is not the one holding it up.
//!
//! # Why quitting needs its own module
//!
//! Without this, `--quit` would report "Lanes has stopped", the core would
//! print `[Quit] shutting down`, and the process would carry on running. Every
//! part of that is locally correct and the overall result is wrong.
//!
//! The reason is the threading. The **main thread** runs the tray's message
//! loop; the **core thread** runs the API server, owns the config and handles
//! commands. A quit arriving over the API is handled on the core thread, so it
//! ends `core_loop` and nothing else — the main thread is still sitting in
//! `MsgWaitForMultipleObjectsEx` waiting for a tray click that will never come.
//! Only the tray menu's own Quit would ever reach the code that returns from
//! the message loop.
//!
//! # Why a flag and a posted message, and not one of them
//!
//! The flag alone is not enough: the message loop waits with no timeout when
//! nothing is pending, which is what keeps the idle CPU cost at zero, so it
//! could sit there indefinitely without ever looking at the flag.
//!
//! The posted message alone is not enough either: `WM_NULL` wakes the loop but
//! carries no meaning, and posted messages can be coalesced or arrive while the
//! loop is mid-iteration.
//!
//! Together they are exactly right — the flag says *what*, the message says
//! *now* — and the idle behaviour is unchanged, because nothing is posted until
//! somebody actually asks to quit.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// The thread running the tray's message loop.
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);

/// Whether a quit has been asked for.
static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Record the message loop's thread. Call this on that thread, before the core
/// thread is started.
pub fn remember_main_thread() {
    use windows::Win32::System::Threading::GetCurrentThreadId;

    MAIN_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
}

/// Ask the process to stop. Safe to call from any thread, and more than once.
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
    wake();
}

/// Wake the tray's message loop so it looks at whatever it has been left.
///
/// The same flag-and-message arrangement as a quit, used for the tray menu as
/// well: the core thread leaves the menu's new contents where
/// the loop will find them, then calls this. See `tray::publish`.
pub fn wake() {
    use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_NULL};

    let thread = MAIN_THREAD.load(Ordering::SeqCst);
    if thread == 0 {
        // No message loop: a one-shot command line run, or the API running
        // without a tray. There is nothing to wake, and whatever was left for
        // the loop is simply never collected.
        return;
    }

    // A failure here means the thread has already gone. For a quit that is the
    // outcome being asked for; for a menu update there is no menu left.
    unsafe {
        let _ = PostThreadMessageW(thread, WM_NULL, Default::default(), Default::default());
    }
}

/// Whether a quit has been asked for. Checked by the message loop.
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}
