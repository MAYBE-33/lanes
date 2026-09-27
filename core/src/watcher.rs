//! Watching for new audio sessions.
//!
//! # Subscribe, never poll
//!
//! Everything in Lanes is event-driven, with no polling loops and no timers
//! running when nothing is happening. An idle Lanes should cost ~0% CPU with no
//! measurable wakeups, which a poll loop cannot meet.
//!
//! So this registers an `IAudioSessionNotification` callback with Windows and
//! sleeps until Windows says something happened.
//!
//! # The gotcha that makes registration silently do nothing
//!
//! `RegisterSessionNotification` appears to succeed but **never delivers a
//! callback** unless `GetSessionEnumerator` has been called on the same
//! `IAudioSessionManager2` first. This is undocumented, widely hit, and looks
//! exactly like "the event never fired" rather than "you skipped a step".
//!
//! [`Watcher::register`] therefore calls the enumerator and deliberately
//! discards the result. **Do not remove that call as dead code.**
//!
//! # Do almost nothing in the callback
//!
//! `OnSessionCreated` runs on a Windows-owned COM thread while the audio engine
//! is waiting. Enumerating sessions or touching the policy interface from
//! inside it risks deadlock. The callback therefore only sends the process id
//! down a channel; all real work happens on our own thread.

use std::sync::mpsc::{channel, Receiver, Sender};

use windows::core::Interface;
use windows::core::{implement, Ref, Result};
use windows::Win32::Media::Audio::{
    eRender, IAudioSessionControl, IAudioSessionControl2, IAudioSessionManager2,
    IAudioSessionNotification, IAudioSessionNotification_Impl, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::CLSCTX_ALL;

/// What a Windows callback reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    SessionCreated { process_id: u32 },
    /// An endpoint appeared, vanished, changed state, or became the default.
    ///
    /// Deliberately carries no device id. Every channel's destination is
    /// decided against the whole list of what is present, so "something
    /// changed, look again" is the entire useful content - and it means bursts
    /// of these, which Windows does send, collapse into one re-decision.
    DevicesChanged,
    /// Master's device had its volume or mute changed by something other than
    /// Lanes - the taskbar slider, volume keys. See `endpointwatch`.
    MasterChanged,
}

/// The COM object Windows calls into.
#[implement(IAudioSessionNotification)]
struct Notifier {
    tx: Sender<Event>,
}

impl IAudioSessionNotification_Impl for Notifier_Impl {
    fn OnSessionCreated(&self, new_session: Ref<'_, IAudioSessionControl>) -> Result<()> {
        // Keep this short. See the module docs: we are on a Windows COM thread
        // and the audio engine is waiting on us.
        if let Some(session) = new_session.as_ref() {
            if let Ok(control2) = session.cast::<IAudioSessionControl2>() {
                if let Ok(pid) = unsafe { control2.GetProcessId() } {
                    // A failed send means the receiver has shut down, which is
                    // normal during exit. Never propagate it — returning an
                    // error here would surface inside the Windows audio engine.
                    let _ = self.tx.send(Event::SessionCreated { process_id: pid });
                }
            }
        }
        Ok(())
    }
}

/// Holds registrations open. Unregisters on drop.
///
/// Keeping the managers alive matters: dropping an `IAudioSessionManager2`
/// tears down its registration, and the callbacks simply stop with no error
/// anywhere to explain why.
pub struct Watcher {
    /// Kept so [`Watcher::refresh`] can build notifiers for devices that turn
    /// up later.
    tx: Sender<Event>,
    /// Device id alongside the registration, so refreshing can tell what it is
    /// already subscribed to without re-registering and double-reporting.
    registrations: Vec<(String, IAudioSessionManager2, IAudioSessionNotification)>,
    pub events: Receiver<Event>,
}

impl Watcher {
    /// Register on every active output device.
    ///
    /// Notifications are per-device, so a single registration would miss
    /// sessions created on any other endpoint. Devices appearing later are
    /// picked up by [`Watcher::refresh`].
    pub fn register() -> Result<Self> {
        let (tx, events) = channel();
        Self::build(tx, events)
    }

    /// Register, delivering events to a caller-supplied sender.
    ///
    /// The API server funnels session events into the same queue as client
    /// requests, so that the core thread has one place to wait rather than
    /// needing to select across several.
    pub fn register_with(tx: Sender<Event>) -> Result<Self> {
        // The returned receiver is unused in this mode; events go to `tx`.
        let (_unused_tx, events) = channel();
        Self::build(tx, events)
    }

    fn build(tx: Sender<Event>, events: Receiver<Event>) -> Result<Self> {
        let mut watcher = Self {
            tx,
            registrations: Vec::new(),
            events,
        };

        watcher.refresh();

        if watcher.registrations.is_empty() {
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "could not register for session notifications on any output device",
            ));
        }

        Ok(watcher)
    }

    /// Bring the registrations into line with the devices that exist now.
    ///
    /// # Why this has to exist
    ///
    /// Session notifications are **per device**. A registration made at startup
    /// covers the endpoints present at startup and nothing else, so an
    /// application that starts playing on a DAC plugged in afterwards would
    /// never be seen - it would sit in "To be routed" at full volume while its
    /// rule sat unused in the config.
    ///
    /// Idempotent: devices already registered are left alone, so calling it on
    /// every device event costs nothing when nothing relevant changed.
    /// Registrations for endpoints that have gone are dropped, which also
    /// releases the manager objects Windows can no longer service.
    pub fn refresh(&mut self) {
        let present: Vec<String> = winaudio::devices::list(eRender)
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.id)
            .collect();

        // Let go of anything that is no longer there.
        self.registrations.retain(|(id, manager, notifier)| {
            if present.iter().any(|p| p == id) {
                return true;
            }
            unsafe {
                let _ = manager.UnregisterSessionNotification(notifier);
            }
            false
        });

        unsafe {
            let Ok(enumerator) = winaudio::devices::enumerator() else {
                return;
            };
            let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
                return;
            };
            let Ok(count) = collection.GetCount() else {
                return;
            };

            for i in 0..count {
                let Ok(device) = collection.Item(i) else {
                    continue;
                };
                let Some(id) = device.GetId().ok().and_then(|id| id.to_string().ok()) else {
                    continue;
                };

                if self.registrations.iter().any(|(known, _, _)| known == &id) {
                    continue;
                }

                let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                // REQUIRED. Registration silently delivers nothing without it.
                // See the module documentation. Not dead code.
                let _ = manager.GetSessionEnumerator();

                let notifier: IAudioSessionNotification = Notifier {
                    tx: self.tx.clone(),
                }
                .into();

                if manager.RegisterSessionNotification(&notifier).is_ok() {
                    self.registrations.push((id, manager, notifier));
                }
            }
        }
    }

    pub fn device_count(&self) -> usize {
        self.registrations.len()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        for (_id, manager, notifier) in &self.registrations {
            unsafe {
                let _ = manager.UnregisterSessionNotification(notifier);
            }
        }
    }
}
