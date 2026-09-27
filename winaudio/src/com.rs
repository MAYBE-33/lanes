//! COM apartment lifetime.
//!
//! # The hazard this exists to prevent
//!
//! Every COM interface must be released **before** `CoUninitialize`. Releasing
//! one afterwards runs `IUnknown::Release` against an apartment that no longer
//! exists, and the process dies with `STATUS_ACCESS_VIOLATION` — *after*
//! producing entirely correct output, which makes it look like a Core Audio
//! problem rather than a teardown-ordering one.
//!
//! # How to use it
//!
//! Create the guard first, do all COM work in a scope *inside* it, and let the
//! guard drop last:
//!
//! ```no_run
//! # use winaudio::com;
//! let _com = com::initialize()?;
//! {
//!     // Every COM object lives and dies in here.
//! }
//! // _com drops last, calling CoUninitialize with nothing left to release.
//! # Ok::<(), windows::core::Error>(())
//! ```
//!
//! Rust drops local bindings in reverse declaration order, so a guard declared
//! first is dropped last. That gives the right ordering by construction —
//! provided no COM object outlives the function holding the guard. Storing one
//! in a `static`, leaking it, or moving it into a detached thread defeats this.

use windows::core::Result;
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
};

/// Which apartment to join.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apartment {
    /// Single-threaded apartment. **Use this for the undocumented policy
    /// interop.** It matches what EarTrumpet does, and undocumented WinRT
    /// surfaces can behave differently across apartment models — this is not a
    /// free choice.
    SingleThreaded,
    /// Multi-threaded apartment, for background work that touches only the
    /// documented session and device APIs.
    MultiThreaded,
}

impl From<Apartment> for COINIT {
    fn from(value: Apartment) -> Self {
        match value {
            Apartment::SingleThreaded => COINIT_APARTMENTTHREADED,
            Apartment::MultiThreaded => COINIT_MULTITHREADED,
        }
    }
}

/// Holds the apartment open. Calls `CoUninitialize` on drop.
///
/// Deliberately not `Clone`, not `Send` and not `Sync`: COM apartments are
/// per-thread, and a guard that could move between threads would uninitialise
/// the wrong one.
pub struct ComGuard {
    _not_send: std::marker::PhantomData<*const ()>,
}

/// Join a single-threaded apartment - what Lanes uses on every thread that
/// touches audio.
pub fn initialize() -> Result<ComGuard> {
    initialize_as(Apartment::SingleThreaded)
}

pub fn initialize_as(apartment: Apartment) -> Result<ComGuard> {
    unsafe { CoInitializeEx(None, apartment.into()).ok()? };
    Ok(ComGuard {
        _not_send: std::marker::PhantomData,
    })
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
