//! Reading the microphone's level — and nothing else.
//!
//! # Why this module has to exist at all
//!
//! The obvious way to meter a microphone is [`crate::devices::capture_peak`],
//! which activates `IAudioMeterInformation` on the capture endpoint. It is two
//! lines and it needs nothing of ours running.
//!
//! It also reads a flat zero, because Windows runs the capture path **on
//! demand**. With no application holding the microphone open there is no signal
//! flowing through the endpoint for the meter to measure, however loudly you
//! speak: fifty consecutive reads, all 0.000.
//!
//! So a level that moves whenever the user talks needs a capture stream of our
//! own. That is what this is.
//!
//! # What it deliberately does not do
//!
//! **It never records, stores, forwards or processes the signal.** It asks
//! Windows for the audio that has arrived since the last look, takes the
//! largest absolute sample value, and throws the buffer away. Nothing is
//! written to disk, sent over the API, or kept in memory beyond a single `f32`.
//!
//! That matters because Lanes promises that the microphone signal is never
//! routed through it, and a capture client is the one piece of code here that
//! could break that promise. It does not.
//!
//! # The visible consequence, which is not a bug
//!
//! Holding a capture stream open lights the **Windows microphone-in-use
//! indicator**, and Lanes appears under the microphone's recent-access
//! list in Privacy settings. There is no way to meter a live microphone without
//! this; it is what "something is listening" is supposed to mean.
//!
//! It is bounded as tightly as the feature allows: the stream is opened only
//! while a client has asked for meters — which in practice means the mixer
//! window is open — and is closed the moment that stops, because metering
//! never runs while nobody is watching.
//!
//! # Threading
//!
//! No thread of its own, deliberately. COM in this process is apartment
//! threaded and every audio call has to stay on the thread that initialised it
//! (see [`crate::com`]). [`InputMeter::peak`] is
//! cheap and non-blocking, so the core's existing meter tick drains the buffer
//! on the thread that owns the apartment, and there is no second thread to get
//! the ordering wrong.

use windows::core::{Result, HSTRING};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, IAudioCaptureClient, IAudioClient, IMMDevice, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_SHAREMODE_SHARED, WAVEFORMATEX,
};
use windows::Win32::System::Com::{CoTaskMemFree, CLSCTX_ALL};

use crate::devices;

/// How much audio Windows may buffer for us, in 100-nanosecond units.
///
/// 200ms. The core reads roughly every 100ms, so this leaves generous headroom
/// for a slow tick without ever discarding audio we would have metered — and a
/// dropped packet here shows up as a meter that misses a syllable, which is
/// precisely the kind of "looks broken, is not" symptom worth avoiding.
const BUFFER_DURATION_100NS: i64 = 2_000_000;

/// How a sample is laid out in the buffer Windows hands back.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SampleFormat {
    /// 32-bit IEEE float, already normalised to -1.0..=1.0. What shared mode
    /// gives you on every machine this has been tried on.
    Float32,
    /// 16-bit signed PCM. Handled because the mix format is not contractually
    /// float, and a meter that silently reports zero on an unusual device would
    /// be indistinguishable from the problem this module exists to solve.
    Int16,
}

/// An open capture stream on one input device, used only to measure its level.
pub struct InputMeter {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    format: SampleFormat,
    channels: usize,
    /// The endpoint this was opened on, so the caller can tell when the
    /// configured device has changed underneath it and reopen.
    device_id: String,
}

impl InputMeter {
    /// Open a capture stream on `device_id`, or on the default input device.
    ///
    /// Opening is the expensive part and the part that can fail — an absent
    /// device, a format we cannot read, another application holding the device
    /// in exclusive mode. Doing it once and keeping the handle means [`InputMeter::peak`]
    /// cannot fail in a way worth reporting.
    pub fn open(device_id: Option<&str>) -> Result<Self> {
        unsafe {
            let enumerator = devices::enumerator()?;

            let device: IMMDevice = match device_id {
                Some(id) => enumerator.GetDevice(&HSTRING::from(id))?,
                // eConsole, not eCommunications: the Mic strip is a mixer
                // control, and eConsole is the device the user chose for
                // everyday use.
                None => enumerator.GetDefaultAudioEndpoint(eCapture, eConsole)?,
            };

            let id = device.GetId()?.to_string()?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

            // Shared mode, so we never take the device away from the
            // application that actually wants it. A mixer must not be able to
            // stop Discord hearing you.
            let mix_format = client.GetMixFormat()?;
            let format = describe(&*mix_format);
            let channels = (*mix_format).nChannels as usize;

            let result = client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                0,
                BUFFER_DURATION_100NS,
                0,
                mix_format,
                None,
            );

            // GetMixFormat allocates; Initialize copies what it needs.
            CoTaskMemFree(Some(mix_format as *const _));
            result?;

            let (Some(format), true) = (format, channels > 0) else {
                return Err(windows::core::Error::from(
                    windows::Win32::Foundation::E_NOTIMPL,
                ));
            };

            let capture: IAudioCaptureClient = client.GetService()?;
            client.Start()?;

            Ok(Self {
                client,
                capture,
                format,
                channels,
                device_id: id,
            })
        }
    }

    /// The endpoint this stream is reading.
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// The loudest sample since the last call, 0.0-1.0.
    ///
    /// Drains everything Windows has buffered, so calling it on a regular tick
    /// keeps the stream from overflowing. Errors are reported as silence: a
    /// microphone unplugged mid-call is an ordinary event, and the meter is not
    /// the right place to raise it — the core's device handling is.
    pub fn peak(&self) -> f32 {
        let mut peak: f32 = 0.0;

        unsafe {
            loop {
                let Ok(available) = self.capture.GetNextPacketSize() else {
                    break;
                };
                if available == 0 {
                    break;
                }

                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames: u32 = 0;
                let mut flags: u32 = 0;

                if self
                    .capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .is_err()
                {
                    break;
                }

                // A silent packet carries no meaningful data — Windows is
                // telling us to treat it as zeroes rather than read the buffer.
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 == 0 && !data.is_null() && frames > 0
                {
                    peak = peak.max(scan(data, frames as usize * self.channels, self.format));
                }

                let _ = self.capture.ReleaseBuffer(frames);
            }
        }

        peak.min(1.0)
    }
}

impl Drop for InputMeter {
    /// Stops the stream, which is what puts the microphone-in-use indicator
    /// out. Worth being explicit about: a leaked `InputMeter` leaves the
    /// operating system telling the user they are being listened to.
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

/// Work out how samples are laid out, or `None` if we cannot read this format.
///
/// `WAVE_FORMAT_EXTENSIBLE` does not say on its own what the samples are, and
/// reading its sub-format GUID needs a Windows feature this crate does not
/// otherwise pull in. The bit depth answers the question well enough: shared
/// mode hands out 32-bit float in every case observed, and 16-bit PCM is the
/// only other layout in practical use.
fn describe(format: &WAVEFORMATEX) -> Option<SampleFormat> {
    /// `WAVE_FORMAT_IEEE_FLOAT`. Spelled out because the constant is not
    /// exported by the `Win32_Media_Audio` feature this crate enables, and
    /// pulling in another feature for one `u16` is not worth it.
    const IEEE_FLOAT: u16 = 0x0003;

    // Written as `if`, not `match`. A bare constant in a match pattern is a
    // BINDING, not a comparison — it matches everything and shadows the
    // scrutinee - the same trap `sessions::list` documents. rustc flags it only
    // as an unreachable-pattern warning, which is easy to wave away.
    if format.wFormatTag == IEEE_FLOAT || format.wBitsPerSample == 32 {
        Some(SampleFormat::Float32)
    } else if format.wBitsPerSample == 16 {
        Some(SampleFormat::Int16)
    } else {
        None
    }
}

/// Largest absolute sample in a buffer.
///
/// # Safety
///
/// `data` must point to at least `samples` values of `format`'s type, which is
/// what `GetBuffer` guarantees for the frame count it reports.
unsafe fn scan(data: *const u8, samples: usize, format: SampleFormat) -> f32 {
    let mut peak: f32 = 0.0;

    match format {
        SampleFormat::Float32 => {
            let values = std::slice::from_raw_parts(data as *const f32, samples);
            for v in values {
                // Denormals and NaN can appear in a live capture buffer; abs()
                // of NaN is NaN, and max() with NaN would poison the result.
                if v.is_finite() {
                    peak = peak.max(v.abs());
                }
            }
        }
        SampleFormat::Int16 => {
            let values = std::slice::from_raw_parts(data as *const i16, samples);
            for v in values {
                peak = peak.max((*v as f32 / i16::MAX as f32).abs());
            }
        }
    }

    peak
}
