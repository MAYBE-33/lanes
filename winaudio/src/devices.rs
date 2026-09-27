//! Audio endpoint enumeration — fully documented Windows APIs, no surprises.

use windows::core::{Result, HSTRING};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, STGM_READ};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub friendly_name: String,
    pub is_default: bool,
    /// "output" or "input" — spelled out rather than a flag, because this
    /// lands in JSON snapshots a human may have to read under pressure.
    pub direction: String,
}

pub fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

/// Peak level on a **capture** endpoint, 0.0-1.0. `None` uses the default
/// input device.
///
/// # Why this is an endpoint meter and not a session meter
///
/// Playback is metered per session, because a channel's level is "how loud are
/// the apps in it". A microphone has no equivalent: there is one signal
/// arriving at one endpoint, and what the Mic strip should show is whether that
/// endpoint is hearing anything. `IAudioMeterInformation` activated on the
/// device answers exactly that, and needs no capture client of our own.
///
/// # The limitation this carries, which is not a bug
///
/// **The meter only moves while something is capturing.** Windows runs the
/// capture path on demand, so with no application holding the microphone open
/// this reads a legitimate 0.0 however loudly you speak. That is why the Mic
/// channel's meter uses [`crate::capture`] instead, which opens a stream of its
/// own; this is kept as the cheaper diagnostic read.
///
/// Reading is non-destructive for the endpoint meter, unlike the per-session
/// peak in [`crate::sessions::peak_level`].
pub fn capture_peak(device_id: Option<&str>) -> Result<f32> {
    unsafe {
        let enumerator = enumerator()?;

        let device: IMMDevice = match device_id {
            Some(id) => enumerator.GetDevice(&HSTRING::from(id))?,
            // eConsole rather than eCommunications: the Mic strip is a mixer
            // control, not a telephony one, and eConsole is what the user
            // picked as their everyday input.
            None => enumerator.GetDefaultAudioEndpoint(eCapture, eConsole)?,
        };

        let meter: IAudioMeterInformation = device.Activate(CLSCTX_ALL, None)?;
        meter.GetPeakValue()
    }
}

/// Read an endpoint's human-readable name.
///
/// Endpoint IDs are GUIDs and mean nothing to a person, so anything shown to
/// one - the mixer, the audit log, `--status` - carries this as well.
pub fn friendly_name(device: &IMMDevice) -> Result<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ)?;
        let value = store.GetValue(&PKEY_Device_FriendlyName)?;
        Ok(value.to_string())
    }
}

pub fn list(flow: EDataFlow) -> Result<Vec<Device>> {
    let direction = if flow == eCapture { "input" } else { "output" };

    unsafe {
        let enumerator = enumerator()?;

        let default_id = enumerator
            .GetDefaultAudioEndpoint(flow, eConsole)
            .ok()
            .and_then(|d| d.GetId().ok())
            .and_then(|id| id.to_string().ok());

        let collection = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        let count = collection.GetCount()?;

        let mut devices = Vec::with_capacity(count as usize);

        for i in 0..count {
            let device = collection.Item(i)?;
            let id = device.GetId()?.to_string()?;
            let name = friendly_name(&device).unwrap_or_else(|_| "<name unavailable>".into());

            devices.push(Device {
                is_default: default_id.as_deref() == Some(id.as_str()),
                id,
                friendly_name: name,
                direction: direction.to_string(),
            });
        }

        Ok(devices)
    }
}

/// Every active endpoint, outputs first.
pub fn list_all() -> Result<Vec<Device>> {
    let mut all = list(eRender)?;
    all.extend(list(eCapture)?);
    Ok(all)
}

/// Resolve a user-supplied device argument.
///
/// Accepts a full endpoint ID, or a case-insensitive substring of a friendly
/// name — so `Lanes --set-device media speakers` works, and nobody has to
/// retype a GUID.
pub fn resolve(needle: &str) -> Result<Option<Device>> {
    let all = list_all()?;

    if let Some(exact) = all.iter().find(|d| d.id == needle) {
        return Ok(Some(exact.clone()));
    }

    let lowered = needle.to_lowercase();
    let matches: Vec<_> = all
        .iter()
        .filter(|d| d.friendly_name.to_lowercase().contains(&lowered))
        .collect();

    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches[0].clone())),
        _ => {
            eprintln!("'{needle}' is ambiguous — it matches:");
            for d in matches {
                eprintln!("  {} [{}]", d.friendly_name, d.id);
            }
            Ok(None)
        }
    }
}
