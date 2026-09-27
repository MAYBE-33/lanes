//! Watching Master's device for its volume slider or mute changing elsewhere.
//!
//! Master is Windows' default output's own volume (see `master`). The user can
//! move that slider from the taskbar, a keyboard's volume keys, or another
//! app, and Master has to follow - otherwise Lanes shows one number and Windows
//! another, and the ceiling Lanes applies to every channel is computed from a
//! volume that is no longer true.
//!
//! # Lanes' own changes are ignored
//!
//! Windows reports every change, including the ones Lanes makes. Each of those
//! is marked with `winaudio::endpoint::LANES_CONTEXT`, which comes back here,
//! so the echo is dropped and only other people's changes reach the core.
//!
//! # Bursts collapse
//!
//! Dragging the taskbar slider fires a notification per step. The callback
//! sends at most one event until the core has handled it (`pending`); the core
//! then reads the volume as it is *by then*, so a whole drag becomes a few
//! passes rather than hundreds. Nothing is lost: the core always reads the
//! current value, not the one that triggered the event.
//!
//! # Following the default device
//!
//! The registration is on one device. When the default changes, the core calls
//! [`EndpointWatcher::refresh`], which moves it to the new one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use windows::core::{implement, Result};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::AUDIO_VOLUME_NOTIFICATION_DATA;
use winaudio::endpoint::{EndpointVolume, LANES_CONTEXT};

use crate::watcher::Event;

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeNotifier {
    tx: Sender<Event>,
    pending: Arc<AtomicBool>,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeNotifier_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        // A Windows-owned thread: look, send at most one event, return.
        let ours = !data.is_null() && unsafe { (*data).guidEventContext } == LANES_CONTEXT;
        if !ours && !self.pending.swap(true, Ordering::AcqRel) {
            let _ = self.tx.send(Event::MasterChanged);
        }
        Ok(())
    }
}

/// Holds the registration on the current default output.
pub struct EndpointWatcher {
    tx: Sender<Event>,
    pending: Arc<AtomicBool>,
    current: Option<(EndpointVolume, IAudioEndpointVolumeCallback)>,
}

impl EndpointWatcher {
    pub fn register(tx: Sender<Event>) -> Self {
        let mut watcher = Self {
            tx,
            pending: Arc::new(AtomicBool::new(false)),
            current: None,
        };
        watcher.refresh();
        watcher
    }

    /// Called by the core when it starts handling a `MasterChanged`, so the
    /// next change raises a new event.
    pub fn handled(&self) {
        self.pending.store(false, Ordering::Release);
    }

    /// Watch whatever is the default output now. Cheap when it has not moved.
    pub fn refresh(&mut self) {
        let default = winaudio::endpoint::default_output_id().ok().flatten();
        let watching = self.current.as_ref().map(|(volume, _)| volume.device_id().to_string());
        if default == watching {
            return;
        }

        self.release();

        let Some(id) = default else { return };
        let Ok(volume) = EndpointVolume::open(&id) else { return };
        let callback: IAudioEndpointVolumeCallback = VolumeNotifier {
            tx: self.tx.clone(),
            pending: self.pending.clone(),
        }
        .into();

        if unsafe { volume.interface().RegisterControlChangeNotify(&callback) }.is_ok() {
            self.current = Some((volume, callback));
        }
    }

    fn release(&mut self) {
        if let Some((volume, callback)) = self.current.take() {
            unsafe {
                let _ = volume.interface().UnregisterControlChangeNotify(&callback);
            }
        }
    }
}

impl Drop for EndpointWatcher {
    fn drop(&mut self) {
        // The callback holds a Sender into the core loop; Windows must not be
        // left calling it after the loop is gone. Same reason as DeviceWatcher.
        self.release();
    }
}
