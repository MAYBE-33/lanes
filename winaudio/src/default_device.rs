//! Setting Windows' default output device - the other undocumented part.
//!
//! # Why this exists
//!
//! Lanes' Master channel *is* Windows' default output: its device button sets
//! the default, and its fader is that device's own volume
//! (see `endpoint`). Windows offers no documented way for a program to change
//! the default device. Every tool that does it - the Sound control panel's own
//! "Set Default", EarTrumpet, SoundSwitch - uses the interface declared here.
//!
//! # The interface
//!
//! `IPolicyConfig`, IID `F8679F50-850A-41CF-9C72-430F290290C8`, created from
//! the class `CPolicyConfigClient`, CLSID `870AF99C-171D-4F9E-AF0D-E63DF40C2BC9`.
//! This IID has been stable from Windows 7 through 8 and from Windows 10 RS1 to
//! today; two short-lived Windows 10 builds (TH1, TH2) used others, and are not
//! supported. The IID, CLSID and method order were learned from EarTrumpet's
//! C# declaration of the interface (`IPolicyConfig.cs`); this Rust binding is
//! an independent implementation. See `THIRD-PARTY-NOTICES.md`.
//!
//! **The slot order is load-bearing**, exactly as in `policy.rs`: eight methods
//! this code never calls, then `GetPropertyValue`, `SetPropertyValue`,
//! `SetDefaultEndpoint`, `SetEndpointVisibility`. The unused ones are declared
//! so that `SetDefaultEndpoint` lands in the right slot; their signatures do
//! not matter because they are never called. Unlike `policy.rs`, this is a
//! classic `IUnknown` interface, so the `windows` crate's `interface` macro can
//! lay the table out from the declaration order.
//!
//! # What "broken" looks like
//!
//! `CoCreateInstance` failing with `REGDB_E_CLASSNOTREG` or `E_NOINTERFACE`
//! means a Windows update changed the class or the IID. A call that returns
//! success and changes nothing means the slot order is wrong. **Read the
//! default back after setting it** - [`set_default_output`] does.

// The interface's methods are named as Windows names them, and most are never
// called - they exist only to hold their slots in the table.
#![allow(non_snake_case, dead_code)]

use windows::core::{GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, PCWSTR};
use windows::Win32::Media::Audio::{eConsole, eMultimedia, ERole};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

use crate::{endpoint, Error, Result};

const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

#[windows_core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: IUnknown {
    fn Unused1(&self) -> HRESULT;
    fn Unused2(&self) -> HRESULT;
    fn Unused3(&self) -> HRESULT;
    fn Unused4(&self) -> HRESULT;
    fn Unused5(&self) -> HRESULT;
    fn Unused6(&self) -> HRESULT;
    fn Unused7(&self) -> HRESULT;
    fn Unused8(&self) -> HRESULT;
    fn GetPropertyValue(&self, device: PCWSTR, key: *const core::ffi::c_void, value: *mut core::ffi::c_void) -> HRESULT;
    fn SetPropertyValue(&self, device: PCWSTR, key: *const core::ffi::c_void, value: *mut core::ffi::c_void) -> HRESULT;
    fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
    fn SetEndpointVisibility(&self, device: PCWSTR, visible: i16) -> HRESULT;
}

/// Whether the interface can be created on this Windows - without changing
/// anything. A diagnostic: call it to tell "Windows changed the interface"
/// apart from "the call failed for some other reason".
pub fn probe() -> Result<()> {
    unsafe {
        let _config: IPolicyConfig = CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)?;
    }
    Ok(())
}

/// Make this device Windows' default output, and check that it took.
///
/// Sets the `eConsole` and `eMultimedia` roles - what "Set Default" in the
/// Sound control panel sets. The communications role is left alone: it is a
/// separate choice in Windows ("Default Communication Device") that some people
/// deliberately point at a headset.
pub fn set_default_output(device_id: &str) -> Result<()> {
    let wide = HSTRING::from(device_id);
    unsafe {
        let config: IPolicyConfig = CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)?;
        for role in [eConsole, eMultimedia] {
            config.SetDefaultEndpoint(PCWSTR(wide.as_ptr()), role).ok()?;
        }
    }

    match endpoint::default_output_id()? {
        Some(now) if now == device_id => Ok(()),
        now => Err(Error::new(
            windows::Win32::Foundation::E_FAIL,
            format!(
                "Windows accepted the new default device but still reports {}",
                now.as_deref().unwrap_or("none")
            ),
        )),
    }
}
