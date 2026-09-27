//! Edits to the config that are more than setting one field: renaming a
//! channel, the ignore list, per-app trim, and export and import.
//!
//! Kept apart from `config.rs`, which is about the file - its shape, loading
//! and saving - and apart from the API handlers, so the rules here can be
//! tested without a running core.

use crate::config::{Config, WindowPlacement};

/// The longest channel name accepted. A strip header is about this wide at
/// the compact layout's narrowest.
pub const MAX_CHANNEL_NAME: usize = 24;

/// How many channels there may be, the six that ship included.
///
/// Channels are renameable and one more can be added to the six that ship. The
/// limit is not arbitrary tidiness: every layout of the mixer has been drawn and
/// checked for seven columns or rows at most. Raise it only after checking the
/// window's three layouts at the new count.
pub const MAX_CHANNELS: usize = 7;

/// The channels that ship, which can be renamed but not removed.
///
/// Master and Mic mean something - Master is the ceiling over every playback
/// channel, Mic is the one input - and Game and Chat are what the mix control
/// defaults to. Removing any of them would leave the rest of the app holding a
/// meaning with nothing to attach it to.
pub const BUILT_IN: [&str; 6] = ["master", "game", "chat", "media", "aux", "mic"];

/// Why an edit was refused. Carried to the client as the error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NotFound(String),
    Invalid(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotFound(why) | Refusal::Invalid(why) => f.write_str(why),
        }
    }
}

impl Config {
    /// Rename a channel. Only the name changes: the id, which rules, profiles
    /// and hotkeys refer to, never does - so nothing that names the channel
    /// breaks.
    pub fn rename_channel(&mut self, id: &str, name: &str) -> Result<String, Refusal> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Refusal::Invalid("a channel needs a name".into()));
        }
        if name.chars().count() > MAX_CHANNEL_NAME {
            return Err(Refusal::Invalid(format!(
                "channel names are limited to {MAX_CHANNEL_NAME} characters"
            )));
        }
        if name.chars().any(char::is_control) {
            return Err(Refusal::Invalid(
                "channel names cannot contain control characters".into(),
            ));
        }

        let channel = self
            .channel_mut(id)
            .ok_or_else(|| Refusal::NotFound(format!("no channel called '{id}'")))?;
        channel.name = name.to_string();
        Ok(channel.name.clone())
    }

    /// Choose which two channels the Game/Chat mix balances.
    ///
    /// Stored rather than assumed, so a user whose voice chat lives on Aux can
    /// say so. Both must be different playback channels, and neither can be
    /// Master, which is a ceiling over the others rather than a peer of them.
    ///
    /// **The mix goes back to centre.** Centre attenuates nothing, so this can
    /// never make anything quieter. Keeping the position instead would turn a
    /// channel down the moment it was named, when the user asked for a
    /// partner, not a level.
    pub fn set_mix_channels(&mut self, game: &str, chat: &str) -> Result<(), Refusal> {
        for id in [game, chat] {
            let channel = self
                .channel(id)
                .ok_or_else(|| Refusal::NotFound(format!("no channel called '{id}'")))?;
            if channel.is_input || channel.id == "master" {
                return Err(Refusal::Invalid(format!(
                    "{} cannot be one side of the Game/Chat mix",
                    channel.name
                )));
            }
        }
        if game == chat {
            return Err(Refusal::Invalid(
                "the two sides of the mix must be different channels".into(),
            ));
        }

        self.chat_mix = crate::config::ChatMix {
            value: 0.0,
            game: game.to_string(),
            chat: chat.to_string(),
        };
        Ok(())
    }

    /// Add a playback channel, just before the Mic channel, and return its id.
    ///
    /// The id is made from the name - lower-case letters and digits - so a
    /// hand-edited config stays readable, with a number added if that id is
    /// taken. It is never changed afterwards, however the channel is renamed.
    pub fn add_channel(&mut self, name: &str) -> Result<String, Refusal> {
        if self.channels.len() >= MAX_CHANNELS {
            return Err(Refusal::Invalid(format!(
                "Lanes has room for {MAX_CHANNELS} channels; remove the added one first"
            )));
        }

        // Validated through rename's rules, on a throwaway, so the two can
        // never disagree about what a good name is.
        let mut probe = crate::config::Config::default();
        let name = probe.rename_channel("aux", name)?;

        let stem: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        let stem = if stem.is_empty() { "channel".to_string() } else { stem };

        let mut id = stem.clone();
        let mut n = 2;
        while self.channels.iter().any(|c| c.id == id) {
            id = format!("{stem}{n}");
            n += 1;
        }

        let at = self
            .channels
            .iter()
            .position(|c| c.is_input)
            .unwrap_or(self.channels.len());
        self.channels
            .insert(at, crate::config::Channel::new(&id, &name));
        Ok(id)
    }

    /// Remove an added channel.
    ///
    /// Everything that pointed at it is tidied rather than left dangling: its
    /// applications' rules go, so they are back in "To be routed" - their audio
    /// is left where it is, the same as unassigning - and hotkeys that named it
    /// go with them. Profiles simply stop mentioning it, which they already
    /// handle. A mix control that named it goes back to Game and Chat.
    pub fn remove_channel(&mut self, id: &str) -> Result<String, Refusal> {
        if BUILT_IN.contains(&id) {
            return Err(Refusal::Invalid(format!(
                "'{id}' is one of the channels Lanes ships with; it can be renamed but not removed"
            )));
        }
        let index = self
            .channels
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| Refusal::NotFound(format!("no channel called '{id}'")))?;

        let removed = self.channels.remove(index);
        self.rules.retain(|r| r.channel != removed.id);
        self.hotkeys
            .retain(|h| h.action.channel() != Some(removed.id.as_str()));
        for profile in &mut self.profiles {
            profile.channels.retain(|c| c.id != removed.id);
        }
        if self.chat_mix.game == removed.id || self.chat_mix.chat == removed.id {
            self.chat_mix = crate::config::ChatMix::default();
        }
        Ok(removed.name)
    }

    /// Stop managing and showing an application.
    ///
    /// **Its rule is kept.** Ignoring is "leave this alone", not "forget where
    /// it goes": stop ignoring it and it goes straight back to its channel.
    /// Nor is its audio moved back - the same principle as `--unassign` (D1 in
    /// the known issues). Returns false when it was already ignored.
    pub fn ignore(&mut self, executable: &str) -> bool {
        let executable = executable.trim();
        if executable.is_empty()
            || self
                .ignored
                .iter()
                .any(|i| i.eq_ignore_ascii_case(executable))
        {
            return false;
        }
        self.ignored.push(executable.to_string());
        true
    }

    /// Manage and show an ignored application again. False when it was not
    /// ignored by that exact name.
    pub fn unignore(&mut self, executable: &str) -> bool {
        let before = self.ignored.len();
        self.ignored
            .retain(|i| !i.eq_ignore_ascii_case(executable.trim()));
        self.ignored.len() != before
    }

    /// Set how loud one application is within its channel, as a fraction of
    /// the channel. `None`, or anything at or above full, clears it.
    ///
    /// Only an application with a rule can have a trim, because the trim lives
    /// on the rule - it is part of "where this app goes and how". Returns the
    /// channel it applies within, which is all a level change needs to touch.
    pub fn set_trim(&mut self, executable: &str, trim: Option<f32>) -> Result<String, Refusal> {
        let rule = self
            .rules
            .iter_mut()
            .find(|r| r.pattern.eq_ignore_ascii_case(executable.trim()))
            .ok_or_else(|| {
                Refusal::NotFound(format!(
                    "{executable} is not in a channel; assign it to one first"
                ))
            })?;

        rule.trim = trim
            .map(|t| t.clamp(0.0, 1.0))
            .filter(|t| *t < 0.999);
        Ok(rule.channel.clone())
    }

    /// This config as it should be written to an export file.
    ///
    /// **Without the API token**, which is the one secret the app has and
    /// which must not travel in a file someone might share or keep in a cloud
    /// folder. **Without the window's placement**, which describes this
    /// machine's monitors and would put the mixer off-screen on another.
    pub fn exported(&self) -> Config {
        let mut out = self.clone();
        out.api_token = None;
        out.settings.window = None;
        out
    }

    /// Take everything an export carries from `imported`, keeping what
    /// belongs to this computer.
    ///
    /// Kept from this config: the API token (so connected clients are not cut
    /// off), the window's placement, and the start-with-Windows setting, which
    /// mirrors a registry entry the import has no business writing. Everything
    /// else - channels, rules, ignore list, profiles, hotkeys, the mix, the
    /// device order, and the other preferences - is replaced.
    ///
    /// Refuses a file from a newer version of Lanes, for the same reason
    /// `Config::load` does: guessing at a format from the future is how a
    /// config gets silently mangled.
    pub fn import(&mut self, imported: Config) -> Result<(), Refusal> {
        if imported.schema_version > crate::config::SCHEMA_VERSION {
            return Err(Refusal::Invalid(format!(
                "that file is from a newer version of Lanes (format {}, this one reads up to {})",
                imported.schema_version,
                crate::config::SCHEMA_VERSION
            )));
        }
        if imported.channels.is_empty() {
            return Err(Refusal::Invalid(
                "that file has no channels; it does not look like a Lanes export".into(),
            ));
        }

        let token = self.api_token.take();
        let window: Option<WindowPlacement> = self.settings.window;
        let start_with_windows = self.settings.start_with_windows;

        *self = Config {
            readable: true,
            schema_version: crate::config::SCHEMA_VERSION,
            api_token: token,
            ..imported
        };
        self.settings.window = window;
        self.settings.start_with_windows = start_with_windows;
        Ok(())
    }
}

#[cfg(test)]
// Tests build configs field by field; it reads better than update syntax here.
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::config::Rule;

    fn with_rule(pattern: &str, channel: &str) -> Config {
        let mut config = Config::default();
        config.rules.push(Rule {
            pattern: pattern.into(),
            channel: channel.into(),
            path_contains: None,
            trim: None,
        });
        config
    }

    #[test]
    fn the_mix_can_balance_any_two_playback_channels() {
        let mut config = Config::default();
        config.chat_mix.value = 0.6;

        config.set_mix_channels("game", "aux").unwrap();
        assert_eq!(
            (config.chat_mix.game.as_str(), config.chat_mix.chat.as_str()),
            ("game", "aux")
        );
        // Back to centre, where it attenuates nothing.
        assert_eq!(config.chat_mix.value, 0.0);

        assert!(matches!(config.set_mix_channels("game", "game"), Err(Refusal::Invalid(_))));
        assert!(matches!(config.set_mix_channels("master", "chat"), Err(Refusal::Invalid(_))));
        assert!(matches!(config.set_mix_channels("game", "mic"), Err(Refusal::Invalid(_))));
        assert!(matches!(config.set_mix_channels("game", "nope"), Err(Refusal::NotFound(_))));
        // A refusal changes nothing.
        assert_eq!(config.chat_mix.chat, "aux");
    }

    #[test]
    fn renaming_changes_the_name_and_nothing_else() {
        let mut config = Config::default();
        assert_eq!(config.rename_channel("aux", "  Voice  ").unwrap(), "Voice");
        let aux = config.channel("aux").unwrap();
        assert_eq!((aux.id.as_str(), aux.name.as_str()), ("aux", "Voice"));

        assert!(config.rename_channel("aux", "   ").is_err());
        assert!(config.rename_channel("aux", &"x".repeat(MAX_CHANNEL_NAME + 1)).is_err());
        assert!(matches!(
            config.rename_channel("nope", "X"),
            Err(Refusal::NotFound(_))
        ));
    }

    #[test]
    fn a_seventh_channel_goes_before_the_mic_and_no_eighth() {
        let mut config = Config::default();
        assert_eq!(config.add_channel("  Voice Chat 2 ").unwrap(), "voicechat2");
        let ids: Vec<_> = config.channels.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["master", "game", "chat", "media", "aux", "voicechat2", "mic"]);
        assert_eq!(config.channel("voicechat2").unwrap().name, "Voice Chat 2");

        assert!(config.add_channel("Another").is_err(), "seven is the limit");
    }

    #[test]
    fn a_channel_id_never_collides() {
        let mut config = Config::default();
        config.channels.retain(|c| c.id != "aux");
        assert_eq!(config.add_channel("Game").unwrap(), "game2");
        config.channels.retain(|c| c.id != "game2");
        assert_eq!(config.add_channel("!!!").unwrap(), "channel", "no letters or digits to make an id from");
    }

    #[test]
    fn a_new_channel_needs_a_good_name() {
        let mut config = Config::default();
        assert!(config.add_channel("   ").is_err());
        assert!(config.add_channel(&"x".repeat(MAX_CHANNEL_NAME + 1)).is_err());
    }

    #[test]
    fn removing_an_added_channel_tidies_what_named_it() {
        let mut config = with_rule("Discord.exe", "chat");
        let id = config.add_channel("Voice").unwrap();
        config.rules.push(Rule {
            pattern: "TeamSpeak.exe".into(),
            channel: id.clone(),
            path_contains: None,
            trim: None,
        });
        config.hotkeys.push(crate::hotkeys::Binding {
            keys: "Ctrl+Alt+V".into(),
            action: crate::hotkeys::Action::ToggleMute { channel: id.clone() },
        });
        config.chat_mix.chat = id.clone();
        crate::profiles::save(&mut config, "Evening").unwrap();

        assert_eq!(config.remove_channel(&id).unwrap(), "Voice");
        assert!(config.channel(&id).is_none());
        assert_eq!(config.rules.len(), 1, "only its own app's rule goes");
        assert!(config.hotkeys.is_empty());
        assert_eq!(config.chat_mix.chat, "chat", "the mix goes back to its default");
        assert!(config.profiles[0].channels.iter().all(|c| c.id != id));
    }

    #[test]
    fn a_shipped_channel_cannot_be_removed() {
        let mut config = Config::default();
        for id in BUILT_IN {
            assert!(config.remove_channel(id).is_err());
        }
        assert_eq!(config.channels.len(), 6);
        assert!(matches!(config.remove_channel("nope"), Err(Refusal::NotFound(_))));
    }

    #[test]
    fn ignoring_keeps_the_rule_and_does_not_duplicate() {
        let mut config = with_rule("Discord.exe", "chat");
        assert!(config.ignore("Discord.exe"));
        assert!(!config.ignore("discord.EXE"), "already ignored, any case");
        assert_eq!(config.ignored, vec!["Discord.exe"]);
        assert_eq!(config.rules.len(), 1, "the rule survives");

        assert!(config.unignore("DISCORD.exe"));
        assert!(config.ignored.is_empty());
        assert!(!config.unignore("Discord.exe"));
    }

    #[test]
    fn trim_needs_a_rule_and_full_clears_it() {
        let mut config = with_rule("Spotify.exe", "media");
        assert_eq!(config.set_trim("spotify.exe", Some(0.6)).unwrap(), "media");
        assert_eq!(config.rules[0].trim, Some(0.6));

        config.set_trim("Spotify.exe", Some(1.0)).unwrap();
        assert_eq!(config.rules[0].trim, None, "full is no trim at all");

        config.set_trim("Spotify.exe", Some(-3.0)).unwrap();
        assert_eq!(config.rules[0].trim, Some(0.0), "clamped, not refused");

        assert!(matches!(
            config.set_trim("brave.exe", Some(0.5)),
            Err(Refusal::NotFound(_))
        ));
    }

    #[test]
    fn an_export_carries_no_secret_and_no_window() {
        let mut config = Config::default();
        config.api_token = Some("secret".into());
        config.settings.window = Some(WindowPlacement {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
            maximised: false,
        });

        let out = config.exported();
        assert_eq!(out.api_token, None);
        assert_eq!(out.settings.window, None);
        assert!(!serde_json::to_string(&out).unwrap().contains("secret"));
    }

    #[test]
    fn an_import_replaces_the_mix_and_keeps_the_machine() {
        let mut here = Config::default();
        here.api_token = Some("mine".into());
        here.settings.start_with_windows = true;
        here.settings.window = Some(WindowPlacement {
            x: 10,
            y: 20,
            width: 900,
            height: 600,
            maximised: false,
        });

        let mut there = with_rule("Discord.exe", "chat");
        there.settings.start_with_windows = false;
        there.channel_mut("media").unwrap().volume = 0.3;

        here.import(there).unwrap();

        assert_eq!(here.rules.len(), 1);
        assert_eq!(here.channel("media").unwrap().volume, 0.3);
        assert_eq!(here.api_token.as_deref(), Some("mine"));
        assert!(here.settings.start_with_windows, "a registry mirror is not imported");
        assert_eq!(here.settings.window.map(|w| w.width), Some(900));
    }

    #[test]
    fn an_import_from_the_future_or_of_nothing_is_refused() {
        let mut here = with_rule("Discord.exe", "chat");

        let mut future = Config::default();
        future.schema_version = crate::config::SCHEMA_VERSION + 1;
        assert!(here.import(future).is_err());

        let mut empty = Config::default();
        empty.channels.clear();
        assert!(here.import(empty).is_err());

        assert_eq!(here.rules.len(), 1, "a refused import changes nothing");
    }
}
