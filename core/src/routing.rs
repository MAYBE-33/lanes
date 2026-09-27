//! Which output a channel actually uses, and whether that is what was asked
//! for.
//!
//! # Why this is its own module
//!
//! "What device is this channel on" has exactly one answer, and it is
//! [`choose`]. Two pieces of code answering one question, with nothing forcing
//! them to agree, is how a mixer ends up describing a device the engine never
//! set. The engine applies what it returns and the
//! API reports what it returns, so a channel cannot be described as running on
//! something it is not.
//!
//! # What a fallback is for
//!
//! A USB device disappears whenever it is unplugged or powered off, and Windows
//! does not politely leave the applications that were using it in place — it
//! scatters them. A channel silently playing out of the wrong speakers is the
//! most annoying failure a mixer can have, so where a channel goes
//! instead is decided deliberately, and the fact that it is on a stand-in is
//! **explicit state**, not something a client infers by comparing two fields.
//!
//! # One list for the whole application
//!
//! Fallback is one global priority order, like a BIOS boot order: see
//! [`Config::device_priority`]. The rules that
//! keep that list sane — disabled entries at the bottom, new devices appended
//! enabled, unplugged ones keeping their place — live here as [`maintain`] and
//! [`reorder`], so they are tested rather than re-derived by each caller.
//!
//! Nothing here is destructive: a channel's preferred device stays recorded
//! while it is absent, so returning to it when it comes back is not a guess.

use winaudio::devices::Device;

use crate::config::{Channel, Config, DeviceRef, PriorityEntry};

/// What a channel is running on, and whether that is second best.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// The endpoint to use. `None` means "leave it on the system default",
    /// which is a legitimate destination and not a failure.
    pub device: Option<DeviceRef>,
    /// True when the preferred device is absent and this is a substitute.
    pub on_fallback: bool,
}

/// Decide the output for one channel, given the endpoints currently present.
///
/// The order is: the channel's own device, then the first enabled and present
/// entry in the global priority list, then the system default. Only the first
/// of those is not a fallback.
///
/// # A channel with no preferred device is never "on a fallback"
///
/// `device: None` means the user asked for the system default, so following it
/// is doing exactly what was requested. Reporting that as a fallback would put
/// a warning on every channel of a default configuration, and a warning that is
/// always on is one nobody reads.
///
/// # Input channels never consult the list
///
/// The list is outputs. Walking it for the Mic channel would offer a speaker as
/// a microphone.
pub fn choose(channel: &Channel, priority: &[PriorityEntry], active: &[Device]) -> Choice {
    // Master *is* Windows' default output (see `master`), so apps on Master
    // follow the default natively rather than being routed to it. Its device in
    // the config is a mirror of the default, not a destination.
    if channel.id == "master" {
        return Choice {
            device: None,
            on_fallback: false,
        };
    }

    let present = |id: &str| active.iter().any(|d| d.id == id);

    let Some(preferred) = channel.device.as_ref() else {
        return Choice {
            device: None,
            on_fallback: false,
        };
    };

    if present(&preferred.id) {
        return Choice {
            device: Some(preferred.clone()),
            on_fallback: false,
        };
    }

    if !channel.is_input {
        let stand_in = priority
            .iter()
            .filter(|entry| entry.enabled)
            .filter(|entry| entry.id != preferred.id)
            .find(|entry| present(&entry.id));

        if let Some(entry) = stand_in {
            return Choice {
                device: Some(DeviceRef {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                }),
                on_fallback: true,
            };
        }
    }

    // Nothing usable in the list is here. The system default is the end of
    // every chain, implicitly, because it is the only destination guaranteed
    // to exist.
    Choice {
        device: None,
        on_fallback: true,
    }
}

/// Keep the priority list in step with the devices that exist.
///
/// Returns true when it changed anything, so the caller knows to save.
///
/// - **An output device seen for the first time is added at the bottom of the
///   enabled entries, enabled** — the user's words were "placed to the bottom
///   of the list but not disabled by default", and the disabled entries are
///   pinned below everything, so the bottom of the usable list is where it
///   goes.
/// - **A present device whose name has changed has its name refreshed**, so an
///   unplugged device is later shown by the name it last had.
/// - **Nothing is ever removed.** An unplugged device keeps its place.
/// - The very first time, when the list is empty, the current system default
///   goes first. Any order is a guess at that point; that one is the guess
///   Windows has already made.
///
/// Only output devices are considered; `outputs` may be the full list and
/// inputs are skipped.
pub fn maintain(priority: &mut Vec<PriorityEntry>, outputs: &[Device]) -> bool {
    let before = priority.clone();

    let mut arrivals: Vec<&Device> = outputs
        .iter()
        .filter(|d| d.direction == "output")
        .filter(|d| !priority.iter().any(|e| e.id == d.id))
        .collect();

    if priority.is_empty() {
        // Stable: the default first, everything else in the order Windows gave.
        arrivals.sort_by_key(|d| !d.is_default);
    }

    for device in outputs.iter().filter(|d| d.direction == "output") {
        if let Some(entry) = priority.iter_mut().find(|e| e.id == device.id) {
            if entry.name != device.friendly_name {
                entry.name = device.friendly_name.clone();
            }
        }
    }

    let insert_at = priority.iter().take_while(|e| e.enabled).count();
    for (offset, device) in arrivals.into_iter().enumerate() {
        priority.insert(
            insert_at + offset,
            PriorityEntry {
                id: device.id.clone(),
                name: device.friendly_name.clone(),
                enabled: true,
            },
        );
    }

    normalise(priority);
    *priority != before
}

/// Apply an order and enabled flags a client asked for.
///
/// `wanted` is `(id, enabled)` in the order the client wants. Anything it does
/// not mention is kept, after the ones it does, so a client working from a
/// slightly stale list cannot delete a device by omission. Ids the list has
/// never heard of are ignored: this reorders devices, it does not invent them.
///
/// The result is normalised, so however the request was arranged, disabled
/// entries end up at the bottom.
pub fn reorder(priority: &[PriorityEntry], wanted: &[(String, bool)]) -> Vec<PriorityEntry> {
    let mut out: Vec<PriorityEntry> = Vec::new();

    for (id, enabled) in wanted {
        if out.iter().any(|e| &e.id == id) {
            continue;
        }
        if let Some(existing) = priority.iter().find(|e| &e.id == id) {
            out.push(PriorityEntry {
                enabled: *enabled,
                ..existing.clone()
            });
        }
    }

    for existing in priority {
        if !out.iter().any(|e| e.id == existing.id) {
            out.push(existing.clone());
        }
    }

    normalise(&mut out);
    out
}

/// Enabled first, disabled last, each group keeping its order; no duplicates.
fn normalise(priority: &mut Vec<PriorityEntry>) {
    let mut seen: Vec<String> = Vec::new();
    priority.retain(|e| {
        if seen.contains(&e.id) {
            false
        } else {
            seen.push(e.id.clone());
            true
        }
    });

    // A stable sort on one boolean is exactly "move the disabled ones to the
    // bottom without disturbing anything else".
    priority.sort_by_key(|e| !e.enabled);
}

/// A device the configuration remembers, which is not currently plugged in.
#[derive(Debug, Clone, PartialEq)]
pub struct Absent {
    pub device: DeviceRef,
    /// "output" or "input".
    pub direction: &'static str,
}

/// Devices the configuration refers to that Windows is not currently offering.
///
/// # Why this is derived from the config rather than enumerated
///
/// The obvious implementation is to ask Windows for endpoints in every state
/// rather than only the active ones. **On a PC that has been in use for a
/// while, that is dozens of endpoints of which two or three are real.** The
/// rest are years of sediment — HDMI outputs for monitors long gone, old jacks,
/// headsets sold years ago, and the virtual devices of uninstalled audio
/// software. Offering those in a picker would bury the real devices in
/// phantoms.
///
/// Every device the user has actually met is in the config — as a channel's
/// device or as an entry in the priority list — so the set that matters is
/// already known. That is what this returns.
///
/// **The limitation this accepts:** a device that is unplugged and has never
/// been plugged in while Lanes was running cannot be chosen. Plugging it in once
/// is enough, after which the priority list remembers it for good.
pub fn remembered_absent(config: &Config, active: &[Device]) -> Vec<Absent> {
    let mut out: Vec<Absent> = Vec::new();
    let mut add = |device: DeviceRef, direction: &'static str| {
        if active.iter().any(|d| d.id == device.id) || out.iter().any(|a| a.device.id == device.id) {
            return;
        }
        out.push(Absent { device, direction });
    };

    for channel in &config.channels {
        if let Some(device) = &channel.device {
            add(device.clone(), if channel.is_input { "input" } else { "output" });
        }
    }

    for entry in &config.device_priority {
        add(
            DeviceRef {
                id: entry.id.clone(),
                name: entry.name.clone(),
            },
            "output",
        );
    }

    out
}

/// One thing a device event changed, in a form that can be printed and logged.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceChange {
    /// A device name, `<system>`, or a channel id.
    pub target: String,
    /// `present`, `default-output`, or `effective-device`.
    pub field: &'static str,
    pub from: String,
    pub to: String,
}

impl DeviceChange {
    /// How it reads on the console.
    pub fn line(&self) -> String {
        match self.field {
            "present" if self.to == "false" => format!("- {} went away", self.target),
            "present" => format!("+ {} appeared", self.target),
            "default-output" => format!("default output: {} -> {}", self.from, self.to),
            _ => format!("{}: {} -> {}", self.target, self.from, self.to),
        }
    }
}

/// Everything a change in the set of devices means, described.
///
/// Pure: no printing, no audit log, no Windows. It is the half of
/// "what did that unplug do" that can be tested, and it decides channel moves
/// with [`choose`] - the same function the engine applies - so the record
/// cannot describe a move the engine did not make.
///
/// Three kinds of change, in this order: devices that went or appeared, the
/// system default output moving, and every playback channel whose destination
/// is different, marked when the new one is a fallback. The last is the one a
/// person most needs to find afterwards: "why did Game come out of the
/// speakers last night?"
pub fn device_changes(config: &Config, before: &[Device], after: &[Device]) -> Vec<DeviceChange> {
    let mut out = Vec::new();

    for gone in before.iter().filter(|b| !after.iter().any(|a| a.id == b.id)) {
        out.push(DeviceChange {
            target: gone.friendly_name.clone(),
            field: "present",
            from: "true".into(),
            to: "false".into(),
        });
    }

    for arrived in after.iter().filter(|a| !before.iter().any(|b| b.id == a.id)) {
        out.push(DeviceChange {
            target: arrived.friendly_name.clone(),
            field: "present",
            from: "false".into(),
            to: "true".into(),
        });
    }

    let default_output = |list: &[Device]| {
        list.iter()
            .find(|d| d.is_default && d.direction == "output")
            .map(|d| d.friendly_name.clone())
            .unwrap_or_else(|| "(none)".into())
    };
    let (was, is) = (default_output(before), default_output(after));
    if was != is {
        out.push(DeviceChange {
            target: "<system>".into(),
            field: "default-output",
            from: was,
            to: is,
        });
    }

    let describe = |choice: &Choice, list: &[Device]| {
        let place = match &choice.device {
            Some(d) => list
                .iter()
                .find(|x| x.id == d.id)
                .map(|x| x.friendly_name.clone())
                .unwrap_or_else(|| d.name.clone()),
            None => "system default".into(),
        };
        if choice.on_fallback {
            format!("{place} [fallback]")
        } else {
            place
        }
    };

    for channel in config.channels.iter().filter(|c| !c.is_input) {
        let old = choose(channel, &config.device_priority, before);
        let new = choose(channel, &config.device_priority, after);
        if old == new {
            continue;
        }

        out.push(DeviceChange {
            target: channel.id.clone(),
            field: "effective-device",
            from: describe(&old, before),
            to: describe(&new, after),
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str) -> Device {
        present(id, &format!("Device {id}"), false)
    }

    fn present(id: &str, name: &str, default: bool) -> Device {
        Device {
            id: id.to_string(),
            friendly_name: name.to_string(),
            is_default: default,
            direction: "output".to_string(),
        }
    }

    fn reference(id: &str) -> DeviceRef {
        DeviceRef {
            id: id.to_string(),
            name: format!("Device {id}"),
        }
    }

    fn entry(id: &str, enabled: bool) -> PriorityEntry {
        PriorityEntry {
            id: id.to_string(),
            name: format!("Device {id}"),
            enabled,
        }
    }

    fn ids(list: &[PriorityEntry]) -> Vec<(&str, bool)> {
        list.iter().map(|e| (e.id.as_str(), e.enabled)).collect()
    }

    fn channel(device: Option<&str>) -> Channel {
        Channel {
            id: "media".into(),
            name: "Media".into(),
            volume: 1.0,
            muted: false,
            device: device.map(reference),
            collapsed: true,
            is_input: false,
        }
    }

    // --- choose ------------------------------------------------------------

    #[test]
    fn no_preference_follows_the_system_default_and_is_not_a_fallback() {
        let choice = choose(&channel(None), &[entry("a", true)], &[device("a")]);
        assert_eq!(choice.device, None);
        assert!(!choice.on_fallback, "the default is what was asked for");
    }

    #[test]
    fn a_present_preference_is_used_whatever_the_list_says() {
        let choice = choose(
            &channel(Some("a")),
            &[entry("b", true), entry("a", true)],
            &[device("a"), device("b")],
        );
        assert_eq!(choice.device, Some(reference("a")));
        assert!(!choice.on_fallback);
    }

    #[test]
    fn an_absent_preference_goes_to_the_first_present_enabled_entry() {
        let choice = choose(
            &channel(Some("dac")),
            &[entry("dac", true), entry("speakers", true), entry("b", true)],
            &[device("b")],
        );
        assert_eq!(choice.device, Some(reference("b")), "speakers are absent too");
        assert!(choice.on_fallback);
    }

    #[test]
    fn the_list_is_ordered() {
        let choice = choose(
            &channel(Some("dac")),
            &[entry("speakers", true), entry("b", true)],
            &[device("b"), device("speakers")],
        );
        assert_eq!(choice.device, Some(reference("speakers")));
    }

    #[test]
    fn a_disabled_device_is_never_a_fallback() {
        let choice = choose(
            &channel(Some("dac")),
            &[entry("speakers", false), entry("b", true)],
            &[device("speakers"), device("b")],
        );
        assert_eq!(
            choice.device,
            Some(reference("b")),
            "speakers are present, but excluded"
        );
    }

    #[test]
    fn a_disabled_device_chosen_directly_is_still_used() {
        let choice = choose(
            &channel(Some("speakers")),
            &[entry("speakers", false)],
            &[device("speakers")],
        );
        assert_eq!(choice.device, Some(reference("speakers")));
        assert!(!choice.on_fallback, "disabling is about fallback only");
    }

    #[test]
    fn nothing_usable_ends_at_the_system_default() {
        let choice = choose(&channel(Some("dac")), &[entry("b", false)], &[device("b")]);
        assert_eq!(choice.device, None);
        assert!(choice.on_fallback);
    }

    #[test]
    fn the_mic_never_falls_back_to_a_speaker() {
        let mut mic = channel(Some("gone-mic"));
        mic.is_input = true;
        let choice = choose(&mic, &[entry("speakers", true)], &[device("speakers")]);
        assert_eq!(choice.device, None);
    }

    #[test]
    fn the_preference_is_not_forgotten_while_it_is_absent() {
        let strip = channel(Some("dac"));
        let _ = choose(&strip, &[entry("b", true)], &[device("b")]);
        assert_eq!(strip.device, Some(reference("dac")));
    }

    // --- maintain ------------------------------------------------------------

    #[test]
    fn the_first_list_puts_the_system_default_first() {
        let mut list = Vec::new();
        let changed = maintain(
            &mut list,
            &[present("speakers", "Speakers", false), present("iems", "IEMs", true)],
        );
        assert!(changed);
        assert_eq!(ids(&list), vec![("iems", true), ("speakers", true)]);
    }

    #[test]
    fn a_new_device_goes_to_the_bottom_of_the_enabled_ones_enabled() {
        let mut list = vec![entry("a", true), entry("b", true), entry("off", false)];
        maintain(
            &mut list,
            &[device("a"), device("b"), device("off"), device("new")],
        );
        assert_eq!(
            ids(&list),
            vec![("a", true), ("b", true), ("new", true), ("off", false)],
            "above the disabled ones, not below them"
        );
    }

    #[test]
    fn unplugged_devices_keep_their_place() {
        let mut list = vec![entry("dac", true), entry("speakers", true)];
        let changed = maintain(&mut list, &[device("speakers")]);
        assert!(!changed, "nothing to do: an absent device is not removed");
        assert_eq!(ids(&list), vec![("dac", true), ("speakers", true)]);
    }

    #[test]
    fn inputs_are_never_added() {
        let mut list = Vec::new();
        let mut mic = device("mic");
        mic.direction = "input".into();
        maintain(&mut list, &[mic, device("speakers")]);
        assert_eq!(ids(&list), vec![("speakers", true)]);
    }

    #[test]
    fn nothing_changes_twice() {
        let mut list = Vec::new();
        let outputs = [device("a"), device("b")];
        assert!(maintain(&mut list, &outputs));
        assert!(!maintain(&mut list, &outputs), "idempotent, so no needless save");
    }

    // --- reorder -------------------------------------------------------------

    #[test]
    fn reorder_applies_the_order_asked_for() {
        let list = vec![entry("a", true), entry("b", true), entry("c", true)];
        let out = reorder(
            &list,
            &[("c".into(), true), ("a".into(), true), ("b".into(), true)],
        );
        assert_eq!(ids(&out), vec![("c", true), ("a", true), ("b", true)]);
    }

    #[test]
    fn disabling_sends_a_device_to_the_bottom() {
        let list = vec![entry("a", true), entry("b", true), entry("c", true)];
        let out = reorder(
            &list,
            &[("a".into(), false), ("b".into(), true), ("c".into(), true)],
        );
        assert_eq!(ids(&out), vec![("b", true), ("c", true), ("a", false)]);
    }

    #[test]
    fn a_stale_request_cannot_delete_a_device() {
        let list = vec![entry("a", true), entry("new", true)];
        let out = reorder(&list, &[("a".into(), true)]);
        assert_eq!(ids(&out), vec![("a", true), ("new", true)]);
    }

    #[test]
    fn unknown_ids_are_ignored() {
        let list = vec![entry("a", true)];
        let out = reorder(&list, &[("ghost".into(), true), ("a".into(), true)]);
        assert_eq!(ids(&out), vec![("a", true)]);
    }

    // --- remembered_absent ----------------------------------------------------

    #[test]
    fn remembered_devices_that_are_gone_are_listed_once_each() {
        let mut config = Config::default();
        config.channels[1].device = Some(reference("dac"));
        config.channels[2].device = Some(reference("dac"));
        config.device_priority = vec![entry("dac", true), entry("speakers", true)];

        let absent = remembered_absent(&config, &[device("speakers")]);

        assert_eq!(absent.len(), 1, "remembered twice, listed once");
        assert_eq!(absent[0].device.id, "dac");
        assert_eq!(absent[0].direction, "output");
    }

    // --- device_changes ----------------------------------------------------------

    /// A typical two-device setup: four channels on a headset, Media on the
    /// speakers, and the priority list headset then speakers.
    fn machine_config() -> Config {
        let mut config = Config::default();
        for channel in config.channels.iter_mut() {
            if channel.is_input {
                continue;
            }
            channel.device = Some(reference(if channel.id == "media" {
                "speakers"
            } else {
                "iems"
            }));
        }
        config.device_priority = vec![entry("iems", true), entry("speakers", true)];
        config
    }

    #[test]
    fn unplugging_the_iems_moves_their_channels_and_leaves_media_alone() {
        let config = machine_config();
        let before = [present("iems", "IEMs", true), present("speakers", "Speakers", false)];
        let after = [present("speakers", "Speakers", true)];

        let changes = device_changes(&config, &before, &after);

        assert!(changes.contains(&DeviceChange {
            target: "IEMs".into(),
            field: "present",
            from: "true".into(),
            to: "false".into(),
        }));
        assert!(changes.contains(&DeviceChange {
            target: "game".into(),
            field: "effective-device",
            from: "IEMs".into(),
            to: "Speakers [fallback]".into(),
        }));
        assert!(!changes.iter().any(|c| c.target == "media"));
    }

    #[test]
    fn plugging_them_back_in_brings_the_channels_home() {
        let config = machine_config();
        let before = [present("speakers", "Speakers", true)];
        let after = [present("iems", "IEMs", true), present("speakers", "Speakers", false)];

        let changes = device_changes(&config, &before, &after);

        assert!(changes.contains(&DeviceChange {
            target: "chat".into(),
            field: "effective-device",
            from: "Speakers [fallback]".into(),
            to: "IEMs".into(),
        }));
    }

    #[test]
    fn a_default_change_alone_is_still_recorded() {
        let config = Config::default();
        let before = [present("a", "A", true), present("b", "B", false)];
        let after = [present("a", "A", false), present("b", "B", true)];

        let changes = device_changes(&config, &before, &after);

        assert_eq!(
            changes,
            vec![DeviceChange {
                target: "<system>".into(),
                field: "default-output",
                from: "A".into(),
                to: "B".into(),
            }]
        );
    }
}
