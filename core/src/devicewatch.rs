//! Watching for devices appearing and disappearing.
//!
//! # Why this is a first-class feature and not an edge case
//!
//! A USB device is absent whenever it is unplugged or powered off - a USB
//! headset, a DAC, headphones plugged into a USB microphone. When an endpoint goes away Windows does not leave the
//! applications that were using it politely waiting — it scatters them, and the
//! assignments can be lost rather than merely inactive.
//!
//! So this subscribes to `IMMNotificationClient` and hands every arrival and
//! departure to the core loop, which re-decides each channel's destination
//! through [`crate::routing::choose`].
//!
//! # Property changes are deliberately ignored
//!
//! `OnPropertyValueChanged` fires constantly — a volume change on an endpoint
//! is a property change — and acting on it would mean re-applying every channel
//! several times a second, which is both wasteful and audible: re-routing a
//! live stream causes a glitch, which is why the engine only ever writes when
//! something actually differs.
//!
//! Only add, remove, state change and default change are reported. Those four
//! are the ones that can alter which endpoint a channel should be on.
//!
//! # Do almost nothing in the callback
//!
//! The same rule as the session watcher, for the same reason: these run on a
//! Windows-owned COM thread. The callback sends one enum down a channel and
//! returns.

use std::sync::mpsc::Sender;

use windows::core::{implement, Result, PCWSTR};
use windows::Win32::Media::Audio::{
    eConsole, EDataFlow, ERole, IMMNotificationClient, IMMNotificationClient_Impl,
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::DEVICE_STATE;

use crate::watcher::Event;

/// The COM object Windows calls into when the endpoint set changes.
#[implement(IMMNotificationClient)]
struct DeviceNotifier {
    tx: Sender<Event>,
}

impl DeviceNotifier {
    /// Report a change. Never fails the caller.
    ///
    /// A failed send means the core has shut down, which is normal during
    /// exit. Returning an error here would surface inside the Windows audio
    /// engine, which has no use for it.
    fn changed(&self) {
        let _ = self.tx.send(Event::DevicesChanged);
    }
}

impl IMMNotificationClient_Impl for DeviceNotifier_Impl {
    fn OnDeviceStateChanged(&self, _device_id: &PCWSTR, _new_state: DEVICE_STATE) -> Result<()> {
        // The state is not inspected. "Became unplugged" and "became active"
        // both mean the set of usable endpoints is different from what every
        // channel was last decided against, and re-deciding is cheap because
        // the engine writes nothing when nothing differs.
        self.changed();
        Ok(())
    }

    fn OnDeviceAdded(&self, _device_id: &PCWSTR) -> Result<()> {
        self.changed();
        Ok(())
    }

    fn OnDeviceRemoved(&self, _device_id: &PCWSTR) -> Result<()> {
        self.changed();
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        _flow: EDataFlow,
        role: ERole,
        _device_id: &PCWSTR,
    ) -> Result<()> {
        // Three roles fire for one user action, so only the console role is
        // taken. Acting on all three would triple the work for a single change
        // of the default device.
        //
        // This matters beyond tidiness: a default changed outside the app must
        // be followed without fighting the user, and every channel that follows
        // the default needs its reported device updated when it moves.
        if role == eConsole {
            self.changed();
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _device_id: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
        // Deliberately nothing. See the module documentation.
        Ok(())
    }
}

/// Holds the registration for as long as it is alive.
pub struct DeviceWatcher {
    enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator,
    client: IMMNotificationClient,
}

impl DeviceWatcher {
    /// Subscribe to endpoint changes, delivering to the core's queue.
    pub fn register(tx: Sender<Event>) -> Result<Self> {
        let enumerator = winaudio::devices::enumerator()?;
        let client: IMMNotificationClient = DeviceNotifier { tx }.into();

        unsafe {
            enumerator.RegisterEndpointNotificationCallback(&client)?;
        }

        Ok(Self { enumerator, client })
    }
}

impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        // Unregistering matters more here than it looks: the callback holds a
        // Sender into the core loop, and Windows holds this object. Leaving it
        // registered past shutdown means Windows calling into a channel whose
        // receiver is gone.
        unsafe {
            let _ = self
                .enumerator
                .UnregisterEndpointNotificationCallback(&self.client);
        }
    }
}
