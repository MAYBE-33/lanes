//! Windows audio, wrapped just enough to be usable.
//!
//! This crate is the **single** place Lanes talks to Windows audio. Keeping it
//! apart from the core means the risky interop has one home, one set of docs,
//! and one thing to fix when a Windows update breaks it - and that any other
//! tool can reuse it without the rest of Lanes.
//!
//! That matters most for [`policy`] and [`default_device`], the undocumented
//! parts. `docs/interop.md` explains them for someone who has to repair them.
//!
//! # Layout
//!
//! | Module | Risk |
//! |--------|------|
//! | [`com`] | Apartment lifetime. Small, but get it wrong and the process crashes on exit |
//! | [`devices`] | Documented API. Safe |
//! | [`capture`] | Documented API. The microphone meter's own capture stream |
//! | [`sessions`] | Documented API. Safe |
//! | [`endpoint`] | Documented API. A device's own volume - Lanes' Master |
//! | [`policy`] | **Undocumented.** Read its module docs before touching it |
//! | [`default_device`] | **Undocumented.** Setting Windows' default output |
//!
//! Everything here is Windows-only by design. A port to another platform would
//! be a separate backend, not a compromise of this one.

pub mod capture;
pub mod com;
pub mod default_device;
pub mod devices;
pub mod endpoint;
pub mod policy;
pub mod sessions;

pub use windows::core::{Error, Result};
