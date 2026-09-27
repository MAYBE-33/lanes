//! Profiles: named sets of how the mixer is set, switched in one action.
//!
//! For example: **Gaming** (everything to the headset), **Movie night** (media to the speakers, chat muted), **Music**
//! (media to the DAC).
//!
//! # What a profile holds, and what it deliberately does not
//!
//! Per channel: **volume, mute and device** - the three things on a strip that
//! describe how it sounds. And the Game/Chat mix, because "Gaming" with the
//! balance left wherever an evening of films put it is not really Gaming.
//!
//! **Not the rules.** Which application belongs to which channel is a fact
//! about the applications - Discord is chat whether it is movie night or not -
//! and a profile that moved apps between channels would make every assignment
//! the user ever made depend on which profile happened to be active. Not the
//! device fallback order either, which is a fact about the hardware on the desk
//! (see `config::Config::device_priority`), and not display state like a
//! collapsed apps list.
//!
//! # "Active" is worked out, not remembered
//!
//! The obvious design stores the name of the last profile switched to and
//! reports it as active forever after. That is a lie the moment anybody touches
//! a fader: the window, and a Stream Deck key lit to say "Gaming", would claim a
//! state the mixer is no longer in.
//!
//! So a profile is active when **the mixer currently matches it**. Nudge a fader
//! and it stops being active; put the fader back and it is active again. The
//! last profile chosen is still stored, but only to break a tie when two
//! profiles happen to hold identical settings.
//!
//! # Channels a profile does not mention are left alone
//!
//! A profile saved before a seventh channel was added says nothing about it, so
//! switching to it leaves that channel exactly as it is, and the channel is not
//! considered when asking whether the profile is active. Equally, a profile
//! naming a channel that has since been removed simply skips it.

use serde::{Deserialize, Serialize};

use crate::config::{Config, DeviceRef};

/// The longest name accepted.
///
/// A profile's name appears in a tray menu and on a Stream Deck key, and 40
/// characters is already more than either can show. The limit exists so a
/// pasted paragraph cannot become a menu item.
pub const MAX_NAME: usize = 40;

/// How close two volumes must be to count as the same.
///
/// Volumes are `f32` and survive a JSON round trip, and a fader moves in 1%
/// steps, so anything under half a step is the same setting.
const SAME_VOLUME: f32 = 0.004;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// As the user typed it. Compared without regard to case, so "gaming" and
    /// "Gaming" are the same profile.
    pub name: String,
    pub channels: Vec<ProfileChannel>,
    /// The Game/Chat balance. `None` in a profile written by hand without one,
    /// which leaves the balance where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_mix: Option<f32>,
}

/// One channel's settings within a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileChannel {
    /// The channel's stable id, never its display name, so renaming a channel
    /// does not orphan every profile that mentions it.
    pub id: String,
    pub volume: f32,
    pub muted: bool,
    /// `None` means the system default, exactly as on a channel.
    #[serde(default)]
    pub device: Option<DeviceRef>,
}

/// Why a profile operation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No profile by that name.
    NotFound(String),
    /// The name is empty, too long, or contains control characters.
    BadName(&'static str),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotFound(name) => write!(f, "no profile called '{name}'"),
            Refusal::BadName(why) => f.write_str(why),
        }
    }
}

/// Tidy a name, or say why it will not do.
pub fn clean_name(name: &str) -> Result<String, Refusal> {
    let name = name.trim();

    if name.is_empty() {
        return Err(Refusal::BadName("a profile needs a name"));
    }
    if name.chars().count() > MAX_NAME {
        return Err(Refusal::BadName("profile names are limited to 40 characters"));
    }
    if name.chars().any(char::is_control) {
        return Err(Refusal::BadName("profile names cannot contain control characters"));
    }

    Ok(name.to_string())
}

fn find<'a>(config: &'a Config, name: &str) -> Option<&'a Profile> {
    config
        .profiles
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name.trim()))
}

/// The mixer as it is now, as a profile called `name`.
pub fn capture(config: &Config, name: &str) -> Profile {
    Profile {
        name: name.to_string(),
        channels: config
            .channels
            .iter()
            .map(|c| ProfileChannel {
                id: c.id.clone(),
                volume: c.volume,
                muted: c.muted,
                device: c.device.clone(),
            })
            .collect(),
        chat_mix: Some(config.chat_mix.value),
    }
}

/// Save the mixer's current settings under `name`.
///
/// **Saving over an existing name replaces it**, in its place in the list and
/// with the spelling just typed. That is what "save" means everywhere else,
/// and it is idempotent, which the API promises of every command that can be.
///
/// Returns the name as stored.
pub fn save(config: &mut Config, name: &str) -> Result<String, Refusal> {
    let name = clean_name(name)?;
    let profile = capture(config, &name);

    match config
        .profiles
        .iter_mut()
        .find(|p| p.name.eq_ignore_ascii_case(&name))
    {
        Some(existing) => *existing = profile,
        None => config.profiles.push(profile),
    }

    config.active_profile = Some(name.clone());
    Ok(name)
}

/// Switch the mixer to the profile called `name`.
///
/// Only changes the config. The caller saves and applies it, which is what
/// every other command does and keeps this testable without touching Windows.
///
/// Returns the name as stored.
pub fn activate(config: &mut Config, name: &str) -> Result<String, Refusal> {
    let profile = find(config, name)
        .cloned()
        .ok_or_else(|| Refusal::NotFound(name.trim().to_string()))?;

    for saved in &profile.channels {
        let Some(channel) = config.channel_mut(&saved.id) else {
            continue;
        };

        channel.volume = saved.volume.clamp(0.0, 1.0);
        channel.muted = saved.muted;

        // The microphone signal is never routed, and the API refuses to give
        // the Mic channel a device. A hand-edited profile must not be a way
        // round that.
        if !channel.is_input {
            channel.device = saved.device.clone();
        }
    }

    if let Some(mix) = profile.chat_mix {
        config.chat_mix.value = mix.clamp(-1.0, 1.0);
    }

    config.active_profile = Some(profile.name.clone());
    Ok(profile.name)
}

/// Remove the profile called `name`.
pub fn delete(config: &mut Config, name: &str) -> Result<String, Refusal> {
    let index = config
        .profiles
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(name.trim()))
        .ok_or_else(|| Refusal::NotFound(name.trim().to_string()))?;

    let removed = config.profiles.remove(index);

    if config
        .active_profile
        .as_deref()
        .is_some_and(|a| a.eq_ignore_ascii_case(&removed.name))
    {
        config.active_profile = None;
    }

    Ok(removed.name)
}

/// Does the mixer currently match this profile?
pub fn matches(config: &Config, profile: &Profile) -> bool {
    let channels_match = profile.channels.iter().all(|saved| {
        let Some(channel) = config.channel(&saved.id) else {
            // A channel that no longer exists cannot disagree.
            return true;
        };

        let same_device = channel.is_input
            || channel.device.as_ref().map(|d| d.id.as_str())
                == saved.device.as_ref().map(|d| d.id.as_str());

        (channel.volume - saved.volume).abs() < SAME_VOLUME
            && channel.muted == saved.muted
            && same_device
    });

    let mix_matches = profile
        .chat_mix
        .is_none_or(|mix| (config.chat_mix.value - mix).abs() < SAME_VOLUME);

    channels_match && mix_matches
}

/// The profile the mixer is in, if it is in one. See the module docs.
pub fn active(config: &Config) -> Option<String> {
    // The one last chosen wins a tie, so saving "Music" as a copy of "Gaming"
    // and switching to it shows "Music" rather than whichever came first.
    if let Some(last) = config.active_profile.as_deref().and_then(|n| find(config, n)) {
        if matches(config, last) {
            return Some(last.name.clone());
        }
    }

    config
        .profiles
        .iter()
        .find(|p| matches(config, p))
        .map(|p| p.name.clone())
}

/// Every profile's name, in the order they were created.
pub fn names(config: &Config) -> Vec<String> {
    config.profiles.iter().map(|p| p.name.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str) -> DeviceRef {
        DeviceRef {
            id: id.into(),
            name: format!("Speakers ({id})"),
        }
    }

    #[test]
    fn saving_and_switching_round_trips_every_setting() {
        let mut config = Config::default();
        config.channel_mut("media").unwrap().volume = 0.4;
        config.channel_mut("media").unwrap().device = Some(device("speakers"));
        config.channel_mut("chat").unwrap().muted = true;
        config.chat_mix.value = 0.3;
        save(&mut config, "Movie night").unwrap();

        // Change everything the profile holds.
        config.channel_mut("media").unwrap().volume = 1.0;
        config.channel_mut("media").unwrap().device = None;
        config.channel_mut("chat").unwrap().muted = false;
        config.chat_mix.value = 0.0;
        assert_eq!(active(&config), None);

        activate(&mut config, "movie NIGHT").unwrap();

        let media = config.channel("media").unwrap();
        assert_eq!(media.volume, 0.4);
        assert_eq!(media.device, Some(device("speakers")));
        assert!(config.channel("chat").unwrap().muted);
        assert_eq!(config.chat_mix.value, 0.3);
        assert_eq!(active(&config).as_deref(), Some("Movie night"));
    }

    #[test]
    fn touching_a_fader_ends_the_profile_and_putting_it_back_restores_it() {
        let mut config = Config::default();
        save(&mut config, "Gaming").unwrap();
        assert_eq!(active(&config).as_deref(), Some("Gaming"));

        config.channel_mut("game").unwrap().volume = 0.8;
        assert_eq!(active(&config), None, "a moved fader is no longer the profile");

        config.channel_mut("game").unwrap().volume = 1.0;
        assert_eq!(active(&config).as_deref(), Some("Gaming"));
    }

    #[test]
    fn saving_over_a_name_replaces_it_in_place() {
        let mut config = Config::default();
        save(&mut config, "Gaming").unwrap();
        save(&mut config, "Music").unwrap();

        config.channel_mut("game").unwrap().volume = 0.5;
        save(&mut config, "GAMING").unwrap();

        assert_eq!(names(&config), vec!["GAMING", "Music"]);
        assert_eq!(config.profiles[0].channels[1].volume, 0.5);
    }

    #[test]
    fn the_last_chosen_wins_a_tie() {
        let mut config = Config::default();
        save(&mut config, "Gaming").unwrap();
        save(&mut config, "Music").unwrap();

        // Identical settings; the one just saved is the one reported.
        assert_eq!(active(&config).as_deref(), Some("Music"));
        activate(&mut config, "Gaming").unwrap();
        assert_eq!(active(&config).as_deref(), Some("Gaming"));
    }

    #[test]
    fn channels_the_profile_does_not_mention_are_left_alone() {
        let mut config = Config::default();
        save(&mut config, "Old").unwrap();
        config.profiles[0].channels.retain(|c| c.id != "aux");

        config.channel_mut("aux").unwrap().volume = 0.2;
        activate(&mut config, "Old").unwrap();

        assert_eq!(config.channel("aux").unwrap().volume, 0.2);
        assert_eq!(active(&config).as_deref(), Some("Old"));
    }

    #[test]
    fn a_profile_naming_a_removed_channel_still_applies() {
        let mut config = Config::default();
        save(&mut config, "Old").unwrap();
        config.channels.retain(|c| c.id != "aux");

        assert!(activate(&mut config, "Old").is_ok());
        assert_eq!(active(&config).as_deref(), Some("Old"));
    }

    #[test]
    fn a_profile_cannot_route_the_microphone() {
        let mut config = Config::default();
        save(&mut config, "Sneaky").unwrap();
        config.profiles[0]
            .channels
            .iter_mut()
            .find(|c| c.id == "mic")
            .unwrap()
            .device = Some(device("speakers"));

        activate(&mut config, "Sneaky").unwrap();
        assert_eq!(config.channel("mic").unwrap().device, None);
    }

    #[test]
    fn deleting_the_last_chosen_forgets_it() {
        let mut config = Config::default();
        save(&mut config, "Gaming").unwrap();
        assert_eq!(delete(&mut config, " gaming ").unwrap(), "Gaming");

        assert!(config.profiles.is_empty());
        assert_eq!(config.active_profile, None);
        assert_eq!(
            delete(&mut config, "Gaming"),
            Err(Refusal::NotFound("Gaming".into()))
        );
    }

    #[test]
    fn switching_to_an_unknown_profile_changes_nothing() {
        let mut config = Config::default();
        config.channel_mut("game").unwrap().volume = 0.3;

        assert!(matches!(
            activate(&mut config, "Nope"),
            Err(Refusal::NotFound(_))
        ));
        assert_eq!(config.channel("game").unwrap().volume, 0.3);
    }

    #[test]
    fn names_are_checked() {
        assert_eq!(clean_name("  Gaming  ").unwrap(), "Gaming");
        assert!(clean_name("   ").is_err());
        assert!(clean_name(&"x".repeat(MAX_NAME + 1)).is_err());
        assert!(clean_name("two\nlines").is_err());
        assert!(clean_name(&"é".repeat(MAX_NAME)).is_ok(), "characters, not bytes");
    }

    #[test]
    fn a_profile_without_a_mix_leaves_the_mix_alone() {
        let mut config = Config::default();
        save(&mut config, "Handwritten").unwrap();
        config.profiles[0].chat_mix = None;

        config.chat_mix.value = -0.5;
        activate(&mut config, "Handwritten").unwrap();

        assert_eq!(config.chat_mix.value, -0.5);
        assert_eq!(active(&config).as_deref(), Some("Handwritten"));
    }
}
