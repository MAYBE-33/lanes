//! Per-application audio endpoint routing — the undocumented part.
//!
//! # Read this before changing anything here
//!
//! This is the riskiest, least googleable code in the project, and the single
//! capability the whole design rests on. If you are reading this because
//! per-app routing suddenly stopped working after a Windows update, start at
//! [`AudioPolicyConfig::connect`] and the two IIDs below.
//!
//! ## What this is
//!
//! Windows' Settings app can send an individual application's audio to a
//! specific output device. It does that through `IAudioPolicyConfigFactory`,
//! an interface Microsoft has never documented. There is no header for it, no
//! MSDN page, and no stability guarantee. We use it anyway because it is the
//! only way to do per-app routing without installing a virtual audio driver,
//! and avoiding that driver is the entire architectural bet of Lanes: no driver
//! means no added latency, no elevation, and nothing that can break the audio
//! stack.
//!
//! ## Why this is hand-written against a raw vtable
//!
//! The interface is `IInspectable`-based (WinRT), not classic COM. There is no
//! `.winmd` metadata for it that the `windows` crate could generate bindings
//! from, so the vtable is declared by hand in `AudioPolicyConfigVtbl`.
//!
//! **The vtable layout is load-bearing.** The three methods we care about sit
//! at the *end* of a long interface. Every preceding slot must be present and
//! correctly counted, or calls land on the wrong function pointer — which does
//! not fail cleanly, it corrupts or crashes. That is why 19 unused slots are
//! declared explicitly rather than omitted.
//!
//! ## The two IIDs
//!
//! The interface's IID changed in Windows 11 21H2. Both are tried, newest
//! first, because guessing the Windows version is less reliable than simply
//! asking for each and seeing which one the system hands back.
//!
//! ## Source of this knowledge
//!
//! The IIDs, the vtable order and the device-ID format were learned from
//! EarTrumpet, the most complete public reference:
//! `EarTrumpet/Interop/MMDeviceAPI/IAudioPolicyConfigFactoryVariantFor21H2.cs`
//! and `EarTrumpet/DataModel/WindowsAudio/Internal/AudioPolicyConfigService.cs`.
//! This Rust binding is an independent implementation of that interface; no
//! EarTrumpet code is included. See `THIRD-PARTY-NOTICES.md`. Known to work on
//! Windows 11 build 26200.

use std::ffi::c_void;

use windows::core::{Result, GUID, HRESULT, HSTRING};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Media::Audio::{eCapture, eConsole, eMultimedia, EDataFlow, ERole};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The WinRT runtime class that hands out the factory.
const RUNTIME_CLASS: &str = "Windows.Media.Internal.AudioPolicyConfig";

/// IID used by Windows 11 21H2 and later, including build 26200.
const IID_VARIANT_21H2: GUID = GUID::from_u128(0xab3d4648_e242_459f_b02f_541c70306324);

/// IID used by Windows 10 and pre-21H2 Windows 11.
const IID_VARIANT_DOWNLEVEL: GUID = GUID::from_u128(0x2a59116d_6c4f_45e0_a74f_707e3fef9258);

// The device ID passed to SetPersistedDefaultAudioEndpoint is NOT the plain
// endpoint ID that IMMDevice::GetId returns. It must be wrapped into a device
// interface path, or the call appears to succeed and silently does nothing.
//
// This is the single most likely cause of "it returns S_OK but the audio did
// not move". See `wrap_device_id`.
const MMDEVAPI_TOKEN: &str = r"\\?\SWD#MMDEVAPI#";
const DEVINTERFACE_AUDIO_RENDER: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";
const DEVINTERFACE_AUDIO_CAPTURE: &str = "#{2eef81be-33fa-4800-9670-1cd474972c3f}";

// ---------------------------------------------------------------------------
// The raw vtable
// ---------------------------------------------------------------------------

/// Hand-declared vtable for `IAudioPolicyConfigFactory`.
///
/// Slot layout, which must not be disturbed:
///
/// | Slots | What |
/// |-------|------|
/// | 0–2   | `IUnknown` — QueryInterface, AddRef, Release |
/// | 3–5   | `IInspectable` — GetIids, GetRuntimeClassName, GetTrustLevel |
/// | 6–24  | 19 methods we do not use (volume groups, ringer, chat context) |
/// | 25    | `SetPersistedDefaultAudioEndpoint` |
/// | 26    | `GetPersistedDefaultAudioEndpoint` |
/// | 27    | `ClearAllPersistedApplicationDefaultEndpoints` — **never called** |
#[repr(C)]
struct AudioPolicyConfigVtbl {
    // --- IUnknown ---
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,

    // --- IInspectable ---
    get_iids: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut GUID) -> HRESULT,
    get_runtime_class_name: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    get_trust_level: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,

    /// Slots 6–24. Present purely so the three methods below land at the right
    /// offsets. Do not remove, do not reorder, do not call.
    ///
    /// In declaration order these are: add_CtxVolumeChange,
    /// remove_CtxVolumeChanged, add_RingerVibrateStateChanged,
    /// remove_RingerVibrateStateChange, SetVolumeGroupGainForId,
    /// GetVolumeGroupGainForId, GetActiveVolumeGroupForEndpointId,
    /// GetVolumeGroupsForEndpoint, GetCurrentVolumeContext,
    /// SetVolumeGroupMuteForId, GetVolumeGroupMuteForId, SetRingerVibrateState,
    /// GetRingerVibrateState, SetPreferredChatApplication,
    /// ResetPreferredChatApplication, GetPreferredChatApplication,
    /// GetCurrentChatApplications, add_ChatContextChanged,
    /// remove_ChatContextChanged.
    _unused_slots_6_to_24: [*const c_void; 19],

    /// Slot 25. `device_id` is an `HSTRING`; a null HSTRING clears the
    /// assignment for that process, returning it to the system default.
    set_persisted_default_audio_endpoint:
        unsafe extern "system" fn(*mut c_void, u32, EDataFlow, ERole, *mut c_void) -> HRESULT,

    /// Slot 26. Returns an `HSTRING` the caller owns.
    get_persisted_default_audio_endpoint:
        unsafe extern "system" fn(*mut c_void, u32, EDataFlow, ERole, *mut *mut c_void) -> HRESULT,

    /// Slot 27. **NEVER CALL THIS.**
    ///
    /// It clears persisted per-application endpoint assignments *globally*,
    /// including every assignment the user made outside this app, through
    /// Windows' own Settings UI. There is no undo. Lanes' Restore clears each
    /// application's assignment individually instead, with `set_endpoint(None)`.
    ///
    /// It is declared solely because omitting it would be a lie about the
    /// interface's shape. It is not wrapped, and no code path reaches it.
    _clear_all_persisted_endpoints_never_call: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

// ---------------------------------------------------------------------------
// Activation
// ---------------------------------------------------------------------------

// `runtimeobject.lib`, NOT `combase.lib`.
//
// RoGetActivationFactory is exported by combase.dll, so linking against
// "combase" is the obvious guess — and it fails, because the Windows SDK ships
// no combase.lib. The import library for the WinRT surface is runtimeobject.lib,
// which forwards to that DLL. The error if you get this wrong is
// `LNK1181: cannot open input file 'combase.lib'`.
#[link(name = "runtimeobject")]
extern "system" {
    /// Declared directly rather than via the `windows` crate, so the exact
    /// raw-pointer form is visible at the call site — this is the one call
    /// most likely to need adjusting if a future Windows build changes things.
    ///
    /// The crate's own binding is generic over `T: Interface`, which our
    /// hand-rolled vtable type deliberately is not.
    fn RoGetActivationFactory(
        activatable_class_id: *mut c_void,
        iid: *const GUID,
        factory: *mut *mut c_void,
    ) -> HRESULT;
}

/// Read the raw handle value out of an `HSTRING`.
///
/// **Subtle, and easy to get wrong.** `HSTRING` is `repr(transparent)` over a
/// pointer, so `&hstring as *const HSTRING` yields the address *of the
/// wrapper* — a pointer to a pointer. The ABI wants the contained handle
/// itself.
///
/// Passing the address instead makes `RoGetActivationFactory` fail, and the
/// failure is indistinguishable from "this Windows does not support the
/// interface" - a binding bug that looks exactly like a platform limit.
///
/// `transmute_copy` reads the handle out without consuming the `HSTRING`, so
/// the caller keeps ownership and `Drop` still frees it.
unsafe fn hstring_abi(value: &HSTRING) -> *mut c_void {
    std::mem::transmute_copy(value)
}

/// Which IID the system accepted. Worth reporting, because it tells you
/// immediately whether you are on the modern or legacy interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Windows 11 21H2 and later.
    For21H2,
    /// Windows 10 / pre-21H2.
    Downlevel,
}

impl Variant {
    pub fn name(self) -> &'static str {
        match self {
            Variant::For21H2 => "21H2+ (ab3d4648-e242-459f-b02f-541c70306324)",
            Variant::Downlevel => "downlevel (2a59116d-6c4f-45e0-a74f-707e3fef9258)",
        }
    }
}

/// A connected `IAudioPolicyConfigFactory`.
///
/// Holds a raw COM pointer. Released on drop — and, per the hazard documented
/// in [`crate::com`], that must happen before `CoUninitialize`.
pub struct AudioPolicyConfig {
    ptr: *mut c_void,
    variant: Variant,
}

impl AudioPolicyConfig {
    /// Attempt to connect, trying the modern IID first.
    ///
    /// **This is the project's highest-risk operation.** If it fails on a
    /// machine, per-app device routing is unavailable there and the app must
    /// degrade to being a per-app volume mixer with a clear explanation —
    /// never a silent failure, never a crash.
    pub fn connect() -> Result<Self> {
        let class_id = HSTRING::from(RUNTIME_CLASS);
        let mut attempts = Vec::new();

        for (iid, variant) in [
            (IID_VARIANT_21H2, Variant::For21H2),
            (IID_VARIANT_DOWNLEVEL, Variant::Downlevel),
        ] {
            let mut factory: *mut c_void = std::ptr::null_mut();

            let hr = unsafe { RoGetActivationFactory(hstring_abi(&class_id), &iid, &mut factory) };

            if hr.is_ok() && !factory.is_null() {
                return Ok(Self {
                    ptr: factory,
                    variant,
                });
            }

            attempts.push(format!("{} -> {hr:?}", variant.name()));
        }

        // Report the ACTUAL HRESULT from each attempt, not a generic failure.
        //
        // A bare E_FAIL would hide the real error and make a binding bug look
        // identical to an unsupported machine - the one distinction whoever is
        // reading this most needs to make.
        //
        // Codes worth recognising:
        //   0x80040154 REGDB_E_CLASSNOTREG — the runtime class is genuinely
        //              absent. This is what a real "unsupported" looks like.
        //   0x80004002 E_NOINTERFACE       — class exists, IID does not match.
        //              Likely a new IID on a newer Windows build.
        //   0x800401F0 CO_E_NOTINITIALIZED — our fault: COM not initialised.
        Err(windows::core::Error::new(
            E_FAIL,
            format!(
                "IAudioPolicyConfigFactory could not be activated. Attempts: {}",
                attempts.join("; ")
            ),
        ))
    }

    pub fn variant(&self) -> Variant {
        self.variant
    }

    unsafe fn vtbl(&self) -> &AudioPolicyConfigVtbl {
        &**(self.ptr as *mut *mut AudioPolicyConfigVtbl as *mut &AudioPolicyConfigVtbl)
    }

    /// Route one process's audio to a specific endpoint.
    ///
    /// `endpoint_id` is the plain ID from `IMMDevice::GetId`; wrapping is
    /// handled here. Pass `None` to clear this process's assignment and return
    /// it to the system default — that is the safe, per-application inverse of
    /// the global clear we never call.
    ///
    /// Both `eConsole` and `eMultimedia` roles are set, matching what Windows'
    /// own UI does. Setting only one leaves the app half-routed, with some
    /// streams still going to the old device.
    pub fn set_endpoint(
        &self,
        process_id: u32,
        flow: EDataFlow,
        endpoint_id: Option<&str>,
    ) -> Result<()> {
        let wrapped = endpoint_id.map(|id| HSTRING::from(wrap_device_id(id, flow)));

        // Null HSTRING == clear the assignment.
        // Same handle-vs-address trap as in `connect` — see `hstring_abi`.
        let abi = match &wrapped {
            Some(h) => unsafe { hstring_abi(h) },
            None => std::ptr::null_mut(),
        };

        for role in [eConsole, eMultimedia] {
            let hr = unsafe {
                (self.vtbl().set_persisted_default_audio_endpoint)(
                    self.ptr, process_id, flow, role, abi,
                )
            };
            hr.ok()?;
        }

        Ok(())
    }

    /// Read a process's persisted endpoint assignment.
    ///
    /// Returns `None` when the process has no explicit assignment, i.e. it
    /// follows the system default.
    pub fn get_endpoint(&self, process_id: u32, flow: EDataFlow) -> Result<Option<String>> {
        let mut raw: *mut c_void = std::ptr::null_mut();

        let hr = unsafe {
            (self.vtbl().get_persisted_default_audio_endpoint)(
                self.ptr,
                process_id,
                flow,
                eMultimedia,
                &mut raw,
            )
        };
        hr.ok()?;

        if raw.is_null() {
            return Ok(None);
        }

        // We own the returned HSTRING. Taking it into an owned HSTRING means
        // Drop frees it for us.
        let owned: HSTRING = unsafe { std::mem::transmute(raw) };
        let value = owned.to_string();

        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(unwrap_device_id(&value)))
        }
    }
}

impl Drop for AudioPolicyConfig {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { (self.vtbl().release)(self.ptr) };
            self.ptr = std::ptr::null_mut();
        }
    }
}

// ---------------------------------------------------------------------------
// Device ID wrapping
// ---------------------------------------------------------------------------

/// Turn a plain endpoint ID into the device interface path the API expects.
///
/// `{0.0.0.00000000}.{guid}` becomes
/// `\\?\SWD#MMDEVAPI#{0.0.0.00000000}.{guid}#{e6327cad-…}`.
///
/// Get this wrong and the call still returns success while doing nothing -
/// which is why routing is verified by listening, and by reading the route
/// back, never by the return code alone.
fn wrap_device_id(endpoint_id: &str, flow: EDataFlow) -> String {
    let suffix = if flow == eCapture {
        DEVINTERFACE_AUDIO_CAPTURE
    } else {
        DEVINTERFACE_AUDIO_RENDER
    };
    format!("{MMDEVAPI_TOKEN}{endpoint_id}{suffix}")
}

/// Inverse of [`wrap_device_id`], for displaying what came back.
fn unwrap_device_id(value: &str) -> String {
    let mut s = value;
    if let Some(rest) = s.strip_prefix(MMDEVAPI_TOKEN) {
        s = rest;
    }
    for suffix in [DEVINTERFACE_AUDIO_RENDER, DEVINTERFACE_AUDIO_CAPTURE] {
        if let Some(rest) = s.strip_suffix(suffix) {
            s = rest;
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the tests need eRender; the production paths take the flow from
    // their caller. Scoped here so the non-test build stays warning-free.
    use windows::Win32::Media::Audio::eRender;

    #[test]
    fn wrapping_round_trips() {
        let id = "{0.0.0.00000000}.{6f1c2d3e-4a5b-4c6d-8e7f-a1b2c3d4e5f6}";
        let wrapped = wrap_device_id(id, eRender);
        assert!(wrapped.starts_with(MMDEVAPI_TOKEN));
        assert!(wrapped.ends_with(DEVINTERFACE_AUDIO_RENDER));
        assert_eq!(unwrap_device_id(&wrapped), id);
    }

    #[test]
    fn unwrapping_tolerates_plain_ids() {
        let id = "{0.0.0.00000000}.{abc}";
        assert_eq!(unwrap_device_id(id), id);
    }
}
