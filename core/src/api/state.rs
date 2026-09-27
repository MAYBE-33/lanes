//! Building the state clients see.
//!
//! One place, used for the connect push, for deltas, and for the state that
//! rides along with every command reply. If a client can ever observe two
//! differently-shaped views of the same thing, they will eventually disagree.

use std::collections::HashMap;

use winaudio::devices::Device;
use winaudio::sessions::Session;

use crate::api::protocol::{
    ChannelState, ChatMixState, DeviceRef, DeviceState, ProfileState, SessionState, State,
};
use crate::config::Config;
use crate::rules::{self, Resolution};

/// Assemble the full state.
///
/// Takes already-gathered devices and sessions rather than fetching them, so
/// that a caller handling one event does not trigger several independent
/// enumerations of the same thing.
pub fn build(
    config: &Config,
    devices: &[Device],
    sessions: &[Session],
    routing_available: bool,
) -> State {
    let channels = config
        .channels
        .iter()
        .map(|channel| {
            let target = channel.device.as_ref().map(|d| DeviceRef {
                id: d.id.clone(),
                name: d.name.clone(),
            });

            // What this channel is actually on, decided by the same code
            // the engine applies, so the mixer can never report a device the
            // engine did not set - see the note at the top of `routing`.
            let choice = crate::routing::choose(channel, &config.device_priority, devices);

            // A chosen endpoint of `None` means the system default. Name it,
            // so a client can show what that resolves to rather than a blank -
            // the default of the channel's own direction: the Mic channel's
            // default is the default *microphone*, not the default speakers.
            let direction = if channel.is_input { "input" } else { "output" };
            let effective = choice.device.clone().map(DeviceRef::from).or_else(|| {
                devices
                    .iter()
                    .find(|d| d.is_default && d.direction == direction)
                    .map(|d| DeviceRef {
                        id: d.id.clone(),
                        name: d.friendly_name.clone(),
                    })
            });

            ChannelState {
                id: channel.id.clone(),
                name: channel.name.clone(),
                volume: channel.volume,
                effective_volume: config.effective_volume(channel),
                muted: channel.muted,
                is_input: channel.is_input,
                target_device: target,
                effective_device: effective,
                on_fallback: choice.on_fallback,
                collapsed: channel.collapsed,
                removable: !crate::manage::BUILT_IN.contains(&channel.id.as_str()),
            }
        })
        .collect();

    // Group sessions by executable. Browsers and Electron apps own several,
    // and the UI shows one entry per application — so the grouping belongs
    // here, not in every client.
    let mut grouped: HashMap<String, (Session, usize, bool)> = HashMap::new();

    for session in sessions {
        if session.process_id == 0 || session.full_path == "<system>" {
            continue;
        }

        let key = session.full_path.to_lowercase();
        let playing = session.state == "playing";

        grouped
            .entry(key)
            .and_modify(|(_, count, any_playing)| {
                *count += 1;
                *any_playing |= playing;
            })
            .or_insert((session.clone(), 1, playing));
    }

    let mut session_states: Vec<SessionState> = grouped
        .values()
        .filter_map(|(session, count, playing)| {
            let (channel, trim) = match rules::resolve(config, &session.executable, &session.full_path) {
                Resolution::Channel { id, trim } => (Some(id.to_string()), trim),
                Resolution::Unassigned => (None, None),
                // Ignored apps are never shown. That is what the ignore list
                // is for, so filtering here rather than flagging them.
                Resolution::Ignored => return None,
            };

            Some(SessionState {
                executable: session.executable.clone(),
                display_name: session.executable.clone(),
                channel,
                playing: *playing,
                // Exclusive-mode detection is not built, so everything is
                // reported as controllable; see docs/limitations.md. Apps that
                // refuse routing are flagged separately, below.
                controllable: true,
                session_count: *count,
                trim,
                path: session.full_path.clone(),
                routing_refused: crate::engine::refused_device(&session.full_path).is_some(),
            })
        })
        .collect();

    session_states.sort_by_key(|a| a.executable.to_lowercase());

    // Everything Windows is offering, plus anything the configuration still
    // points at which is not plugged in.
    //
    // The absent ones are deliberately derived from the config rather than by
    // asking Windows for endpoints in every state: a typical PC has dozens of
    // render endpoints of which two or three are real, and the rest are
    // sediment. See `routing::remembered_absent`.
    let mut device_states: Vec<DeviceState> = devices
        .iter()
        .map(|d| DeviceState {
            id: d.id.clone(),
            name: d.friendly_name.clone(),
            direction: d.direction.clone(),
            present: true,
            is_default: d.is_default,
        })
        .collect();

    device_states.extend(
        crate::routing::remembered_absent(config, devices)
            .into_iter()
            .map(|absent| DeviceState {
                id: absent.device.id,
                name: absent.device.name,
                direction: absent.direction.to_string(),
                present: false,
                is_default: false,
            }),
    );

    State {
        window: config.settings.window,
        channels,
        sessions: session_states,
        devices: device_states,
        device_priority: config
            .device_priority
            .iter()
            .map(|entry| crate::api::protocol::PriorityState {
                id: entry.id.clone(),
                name: entry.name.clone(),
                enabled: entry.enabled,
                present: devices.iter().any(|d| d.id == entry.id),
            })
            .collect(),
        profiles: ProfileState {
            names: crate::profiles::names(config),
            active: crate::profiles::active(config),
        },
        chat_mix: ChatMixState {
            value: config.chat_mix.value,
            game: config.chat_mix.game.clone(),
            chat: config.chat_mix.chat.clone(),
        },
        hotkeys: config
            .hotkeys
            .iter()
            .map(|binding| {
                use crate::hotkeys::Registration;
                let (status, problem) = match crate::hotkeys::status(&binding.keys) {
                    Some(Registration::Active) => ("active", None),
                    Some(Registration::InUse) => (
                        "in_use",
                        Some("another program is using these keys".to_string()),
                    ),
                    Some(Registration::Failed(why)) => ("failed", Some(why)),
                    None => ("pending", None),
                };
                crate::api::protocol::HotkeyState {
                    keys: binding.keys.clone(),
                    action: binding.action.clone(),
                    status: status.to_string(),
                    problem,
                }
            })
            .collect(),
        ignored: config.ignored.clone(),
        settings: crate::api::protocol::SettingsState {
            notify_new_apps: config.settings.notify_new_apps,
            theme: config.settings.theme,
            close_to_tray: config.settings.close_to_tray,
            paused: config.settings.paused,
        },
        routing_available,
    }
}

/// Compare two states and describe only what moved.
///
/// Returning `None` when nothing changed is the point: clients get deltas
/// after the initial push, and a "delta" that always contains
/// everything is just a full push wearing a different name — it would also make
/// an idle core chatter at every connected client for no reason.
pub fn diff(previous: &State, current: &State) -> Option<crate::api::protocol::Event> {
    let channels = (previous.channels != current.channels).then(|| current.channels.clone());
    let sessions = (previous.sessions != current.sessions).then(|| current.sessions.clone());
    let devices = (previous.devices != current.devices).then(|| current.devices.clone());
    let device_priority = (previous.device_priority != current.device_priority)
        .then(|| current.device_priority.clone());
    let profiles = (previous.profiles != current.profiles).then(|| current.profiles.clone());
    let chat_mix = (previous.chat_mix != current.chat_mix).then(|| current.chat_mix.clone());
    let hotkeys = (previous.hotkeys != current.hotkeys).then(|| current.hotkeys.clone());
    let ignored = (previous.ignored != current.ignored).then(|| current.ignored.clone());
    let settings = (previous.settings != current.settings).then(|| current.settings.clone());

    if channels.is_none()
        && sessions.is_none()
        && devices.is_none()
        && device_priority.is_none()
        && profiles.is_none()
        && chat_mix.is_none()
        && hotkeys.is_none()
        && ignored.is_none()
        && settings.is_none()
    {
        return None;
    }

    Some(crate::api::protocol::Event::Delta {
        channels,
        sessions,
        devices,
        device_priority,
        profiles,
        chat_mix,
        hotkeys,
        ignored,
        settings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use winaudio::devices::Device;

    fn device(id: &str, name: &str, direction: &str) -> Device {
        Device {
            id: id.into(),
            friendly_name: name.into(),
            is_default: true,
            direction: direction.into(),
        }
    }

    #[test]
    fn a_channel_on_the_default_reports_the_default_of_its_own_direction() {
        let config = Config::default();
        let devices = [
            device("{speakers}", "Speakers (Desk Speakers)", "output"),
            device("{mic}", "Microphone (Blue Yeti)", "input"),
        ];
        let state = build(&config, &devices, &[], true);
        let named = |id: &str| {
            state
                .channels
                .iter()
                .find(|c| c.id == id)
                .and_then(|c| c.effective_device.as_ref())
                .map(|d| d.name.clone())
        };
        assert_eq!(named("game").as_deref(), Some("Speakers (Desk Speakers)"));
        assert_eq!(named("mic").as_deref(), Some("Microphone (Blue Yeti)"));
    }
}
