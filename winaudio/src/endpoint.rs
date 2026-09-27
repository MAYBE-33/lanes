//! The volume of an output device itself - the number on the taskbar's slider.
//!
//! Fully documented Windows API (`IAudioEndpointVolume`). This is what Lanes'
//! Master fader is: moving Master moves the Windows slider of the device Master
//! is set to, and moving that slider moves Master. Anything less would be two
//! volume controls claiming to be the same one.
//!
//! # Two ways of reading one volume
//!
//! - [`EndpointVolume::scalar`] is the slider position, 0 to 1. It follows an
//!   audio taper - half-way is not half the amplitude - and it is the number a
//!   person sees, so it is what Master shows.
//! - [`EndpointVolume::amplitude`] is what that position actually does to the
//!   signal, from the level in decibels. It is what the engine needs to keep
//!   applications on this device at the level their channel promises, since
//!   Windows applies this factor to every one of them on top of their own.
//!
//! # Telling our changes from everyone else's
//!
//! Every write carries [`LANES_CONTEXT`]. Windows hands that back in the change
//! notification, which is how a watcher can ignore the echo of Lanes' own
//! change and react only to the slider being moved somewhere else.

use windows::core::{GUID, HSTRING};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDevice};
use windows::Win32::System::Com::CLSCTX_ALL;

use crate::devices;
use crate::Result;

/// Marks a volume change as Lanes' own. Arbitrary, fixed, and unique to Lanes.
pub const LANES_CONTEXT: GUID = GUID::from_u128(0x6c616e65_736d_6173_7465_725f766f6c75);

/// The id of Windows' default output device, if there is one.
///
/// `eConsole`, the role the taskbar's device picker and "Set Default" in the
/// Sound control panel both set.
pub fn default_output_id() -> Result<Option<String>> {
    unsafe {
        let enumerator = devices::enumerator()?;
        match enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
            Ok(device) => Ok(Some(device.GetId()?.to_string()?)),
            Err(_) => Ok(None),
        }
    }
}

/// One output device's own volume and mute.
pub struct EndpointVolume {
    inner: IAudioEndpointVolume,
    device_id: String,
}

impl EndpointVolume {
    /// The volume control of this device.
    pub fn open(device_id: &str) -> Result<Self> {
        unsafe {
            let enumerator = devices::enumerator()?;
            let device: IMMDevice = enumerator.GetDevice(&HSTRING::from(device_id))?;
            let inner: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            Ok(Self {
                inner,
                device_id: device_id.to_string(),
            })
        }
    }

    /// The volume control of Windows' default output device.
    pub fn default_output() -> Result<Option<Self>> {
        match default_output_id()? {
            Some(id) => Ok(Some(Self::open(&id)?)),
            None => Ok(None),
        }
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// The slider position, 0 to 1.
    pub fn scalar(&self) -> Result<f32> {
        unsafe { self.inner.GetMasterVolumeLevelScalar() }
    }

    /// What the slider position does to the signal: a linear factor, 0 to 1.
    pub fn amplitude(&self) -> Result<f32> {
        let db = unsafe { self.inner.GetMasterVolumeLevel()? };
        Ok(10f32.powf(db / 20.0).clamp(0.0, 1.0))
    }

    pub fn muted(&self) -> Result<bool> {
        unsafe { Ok(self.inner.GetMute()?.as_bool()) }
    }

    /// Move the slider. Marked as Lanes' own; see [`LANES_CONTEXT`].
    pub fn set_scalar(&self, level: f32) -> Result<()> {
        unsafe {
            self.inner
                .SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), &LANES_CONTEXT)?;
        }
        Ok(())
    }

    pub fn set_muted(&self, muted: bool) -> Result<()> {
        unsafe {
            self.inner.SetMute(muted, &LANES_CONTEXT)?;
        }
        Ok(())
    }

    /// The interface itself, for registering a change notification.
    pub fn interface(&self) -> &IAudioEndpointVolume {
        &self.inner
    }
}
