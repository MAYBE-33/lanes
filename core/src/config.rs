//! Channels, rules, and the file they live in.
//!
//! # Design constraints this file answers to
//!
//! - **Human-readable and hand-editable.** Editing `config.json` by hand and
//!   watching apps get routed is a supported way to use Lanes. That is only
//!   possible if the file is obvious to read, so ids are words and devices
//!   carry their friendly names alongside their ids. `docs/configuration.md`
//!   explains every field.
//! - **Atomic writes.** Temp file then rename, so a crash mid-write cannot
//!   leave a truncated file where a config should be.
//! - **Versioned.** A schema version, so a future build migrates rather than
//!   guesses.
//! - **Never fatal.** A broken config falls back to defaults with a warning
//!   rather than refusing to start, and the broken file is kept for inspection.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths;

pub const SCHEMA_VERSION: u32 = 1;

/// A device the user picked, remembered two ways.
///
/// Friendly name **plus** a stable id, and both earn their place: the id is what Windows actually matches on, and the name is
/// what makes the config readable and what lets us recognise a device that has
/// come back with a new id after being unplugged and replugged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    /// Stable key used by rules and, later, the API. Never renamed.
    pub id: String,
    /// What the user sees. Renameable without breaking anything.
    pub name: String,
    /// 0.0–1.0, applied as a scalar on top of each app's own volume, exactly
    /// as the Windows volume mixer behaves.
    pub volume: f32,
    pub muted: bool,
    /// Preferred output. `None` means "leave apps on the system default".
    pub device: Option<DeviceRef>,
    /// Collapsed state of the apps list in the UI. Stored per channel so it is
    /// remembered between sessions.
    ///
    /// **Defaults to collapsed**, and that default is load-bearing: the mixer
    /// starts condensed and nothing in it opens itself - it opens only when the
    /// user opens it. `serde`'s own default for a `bool` is `false`, which is
    /// why this names a function rather than using `#[serde(default)]`.
    #[serde(default = "default_true")]
    pub collapsed: bool,
    /// Input-only. The Mic channel shows a level and a mute and never routes
    /// the microphone signal anywhere — routing is explicitly out of scope.
    #[serde(default)]
    pub is_input: bool,
}

impl Config {
    /// What a channel's fader actually shows, and what its applications get.
    ///
    /// # Master is a ceiling, not a scalar
    ///
    /// The obvious design multiplies: Master at 75% with Chat at 50% gives
    /// 37.5%, which is how the Windows volume mixer behaves. Lanes does not.
    /// Master is a ceiling — `min(channel, master)`. With every channel at 100%,
    /// moving Master to 75% brings them all to 75%; a channel already at 50% is
    /// unaffected until Master passes below it, and then moves with it. Every
    /// number on screen stays true: a channel reading 50% is at 50%.
    ///
    /// **Nothing is overwritten.** A channel keeps the level it was set to; the
    /// ceiling only limits what is shown and what is applied. Master coming back
    /// up restores every channel to its own value, which a clamp that wrote the
    /// lower number into the config could never do. That is the difference
    /// between a ceiling and a demolition.
    ///
    /// The Mic channel is excluded because it carries no playback at all.
    pub fn effective_volume(&self, channel: &Channel) -> f32 {
        if channel.is_input || channel.id == "master" {
            return channel.volume;
        }

        let ceiling = self.master().map(|m| m.volume).unwrap_or(1.0);
        channel.volume.min(ceiling)
    }
}

impl Channel {
    pub(crate) fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            volume: 1.0,
            muted: false,
            device: None,
            collapsed: true,
            is_input: false,
        }
    }
}

/// One output device's place in the fallback order.
///
/// See [`Config::device_priority`]. The name is stored for the same reason a
/// channel's device carries one: so an unplugged device can still be shown, and
/// so the file stays readable by the person who may have to edit it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriorityEntry {
    pub id: String,
    pub name: String,
    /// False excludes it from fallback, and keeps it at the bottom of the list.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Maps an executable to a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Executable name, case-insensitive. `*` matches any run of characters,
    /// so `chrome*.exe` works.
    #[serde(rename = "match")]
    pub pattern: String,
    /// Channel id this rule assigns to.
    pub channel: String,
    /// Optional disambiguator: the full path must contain this substring.
    ///
    /// Two different applications can ship the same executable name, and two
    /// installs of the same application can want different channels. This is
    /// how the two are told apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_contains: Option<String>,
    /// Per-app trim within the channel, so one app can sit quieter than its
    /// channel-mates. `None` means "follow the channel exactly".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub schema_version: u32,
    pub channels: Vec<Channel>,
    /// Empty by default. Every application starts in "To be routed" until the
    /// user assigns it — an explicit choice, so the app never silently moves
    /// audio the user did not ask it to move.
    pub rules: Vec<Rule>,
    /// Executables never managed or shown.
    #[serde(default)]
    pub ignored: Vec<String>,
    /// Shared secret for the local API, generated on first use.
    ///
    /// Enough to stop other local processes poking the API casually, which is
    /// all it is for. It is not protection against someone
    /// who can already run code as this user — at that point they can read this
    /// file anyway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_token: Option<String>,
    #[serde(default)]
    pub settings: Settings,
    /// The Game/Chat balance control. See [`ChatMix`].
    #[serde(default)]
    pub chat_mix: ChatMix,
    /// Where every channel goes when its own device is unplugged, in order.
    ///
    /// # Why this is global and not per channel
    ///
    /// Which device is the next-best one is a fact about the *hardware on the
    /// desk*, not about Game or Chat, so six per-channel copies of it could
    /// only ever disagree.
    ///
    /// # How it behaves, like a BIOS boot order
    ///
    /// - A channel whose own device is missing goes to the **first enabled,
    ///   plugged-in** entry here that is not the device it just lost.
    /// - **Disabled entries are never used as a fallback**, and always sit at
    ///   the bottom. Disabling is about fallback only: a channel pointed at a
    ///   disabled device directly still uses it.
    /// - **A device seen for the first time is added at the bottom of the
    ///   enabled entries**, enabled. The list is maintained by the core, so it
    ///   is right however the device arrived.
    /// - Unplugged devices keep their place. That is the point of a priority
    ///   list: the DAC that is off tonight is still first tomorrow.
    ///
    /// Output devices only. The microphone signal is never routed, so there is
    /// nothing for an input to fall back *to*.
    #[serde(default)]
    pub device_priority: Vec<PriorityEntry>,

    /// Named sets of channel settings, switched in one action. See `profiles`
    /// for what one holds and, as importantly, what it leaves alone.
    #[serde(default)]
    pub profiles: Vec<crate::profiles::Profile>,

    /// The profile last saved or switched to.
    ///
    /// **Not** what is reported as active - a profile is active when the mixer
    /// matches it, which this cannot know. It only settles a tie between two
    /// profiles holding identical settings. See `profiles::active`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<String>,

    /// Global hotkeys. **Empty by default** - see `hotkeys` for why.
    #[serde(default)]
    pub hotkeys: Vec<crate::hotkeys::Binding>,

    /// Whether this came from a file we could actually read.
    ///
    /// # What this is defending against
    ///
    /// "The file is not there" and "the file is there and I could not open it"
    /// must not get the same answer. The first is a first run. The second is a
    /// config full of the user's rules that happened to be locked for a few
    /// milliseconds - and answering it with defaults would run the app with
    /// **no rules at all** while the real ones sit on disk, then overwrite them
    /// the first time anything saves.
    ///
    /// So a config that could not be read is marked, [`Config::save`] refuses
    /// to write over the file it failed to read, and the caller can refuse to
    /// start rather than quietly do nothing.
    ///
    /// Not serialised: it describes this load, not the configuration.
    #[serde(skip, default = "yes")]
    pub readable: bool,
}

/// `#[serde(default)]` for a bool is `false`, which is the wrong answer for
/// [`Config::readable`] - a config parsed out of a file was, by definition,
/// readable.
fn yes() -> bool {
    true
}

/// The mixer window's last size and position.
///
/// See [`Settings::window`] for why this lives in the config at all, and why
/// the units are physical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Restored maximised. The size below is still the un-maximised one, which
    /// is what Windows itself stores, so un-maximising lands somewhere sensible.
    #[serde(default)]
    pub maximised: bool,
}

/// A single balance control sitting across two channels.
///
/// Sonar and the wireless headsets that popularised this call it "ChatMix": one
/// slider that trades game audio against voice chat without having to reach for
/// two faders mid-fight.
///
/// # What it does, and the two designs it is not
///
/// **It attenuates one side only.** Sliding toward `game` quietens `chat` and
/// leaves `game` alone; sliding toward `chat` does the reverse. At centre it
/// attenuates nothing at all.
///
/// That last property is the reason for this design over the alternatives:
///
/// * **Each channel's own fader keeps its meaning.** The mix is a multiplier on
///   top, so Chat at 70% is still Chat at 70% — of whatever the mix leaves it.
///   Two controls that both claim to set "the chat volume" would be a UI where
///   neither number can be trusted.
/// * **Centre is a true no-op.** A design that redistributes a fixed total
///   between the two would make the centre position *change* both channels the
///   moment their faders differ, so returning the slider to the middle would
///   not return the sound to where it was.
/// * **Nothing ever gets louder than its fader.** Boosting the favoured side
///   was rejected for the same reason the app applies volume as a scalar: this
///   is a manager for Windows' own per-application volumes, and it must never
///   exceed what the user set.
///
/// # The arithmetic
///
/// ```text
/// value  0.0        both channels untouched
/// value +0.6        chat runs at 40% of its own level, game untouched
/// value -0.25       game runs at 75% of its own level, chat untouched
/// ```
///
/// So the factor applied to the far channel is `1.0 - value.abs()`, and it
/// multiplies with master and the channel's own volume rather than replacing
/// either.
///
/// # Why the channel ids are stored
///
/// Channels are renameable and a seventh can be added, so hard-coding "game"
/// and "chat" would break for anyone who reorganises them. The defaults match
/// the shipped channels; the fields exist so that a user whose voice chat lives
/// on Aux can say so.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMix {
    /// -1.0 to 1.0, where 0.0 is centre and attenuates nothing.
    ///
    /// Positive favours [`ChatMix::game`] by quietening [`ChatMix::chat`];
    /// negative does the reverse. The magnitude is how much is taken off the
    /// other side.
    #[serde(default)]
    pub value: f32,
    /// The channel favoured when `value` is positive.
    #[serde(default = "default_game")]
    pub game: String,
    /// The channel favoured when `value` is negative.
    #[serde(default = "default_chat")]
    pub chat: String,
}

impl Default for ChatMix {
    fn default() -> Self {
        Self {
            value: 0.0,
            game: default_game(),
            chat: default_chat(),
        }
    }
}

impl ChatMix {
    /// The multiplier this control applies to `channel_id`.
    ///
    /// Always 1.0 for any channel the control does not name, and always 1.0 for
    /// both of them at centre — so a config that has never touched this is
    /// indistinguishable from one without the feature.
    pub fn factor_for(&self, channel_id: &str) -> f32 {
        let value = self.value.clamp(-1.0, 1.0);

        // Guard against a hand-edited config naming the same channel twice,
        // which would otherwise attenuate it in both directions.
        if self.game == self.chat {
            return 1.0;
        }

        let attenuated = if value > 0.0 {
            &self.chat
        } else if value < 0.0 {
            &self.game
        } else {
            return 1.0;
        };

        if channel_id == attenuated {
            1.0 - value.abs()
        } else {
            1.0
        }
    }
}

fn default_game() -> String {
    "game".into()
}

fn default_chat() -> String {
    "chat".into()
}

/// Application settings: lifecycle, window, notices, appearance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Where the mixer window was last left.
    ///
    /// # Why the core keeps this
    ///
    /// For the same reason it keeps the apps list's collapse state: the window
    /// is destroyed every time it closes, so it cannot remember anything about
    /// itself. It reopens at the size, position and monitor it was left at, and
    /// the core is the only thing still running to remember them.
    ///
    /// The numbers are physical pixels in virtual-screen coordinates - what
    /// `GetWindowPlacement` deals in - and deliberately not logical units. A
    /// logical size means nothing without saying which monitor's scaling it was
    /// logical *in*.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowPlacement>,

    /// Mirrors the HKCU Run key. The registry is the source of truth — this is
    /// what the user last asked for, and `autostart::reconcile` keeps them in
    /// step when the executable moves.
    #[serde(default)]
    pub start_with_windows: bool,
    #[serde(default)]
    pub start_minimised: bool,
    /// **On by default**: clicking X closes the window and leaves the core in
    /// the tray. Turned off, X quits.
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    /// Collapsed state of the "To be routed" pool.
    ///
    /// **Collapsed by default.** Once channels are configured the pool is
    /// normally empty and the user should not have to look at it. It never
    /// opens itself, even when something new arrives — the collapsed rail
    /// carries a count instead, which is how an unrouted app stays noticeable
    /// without the window rearranging itself.
    #[serde(default = "default_true")]
    pub pool_collapsed: bool,
    /// Show a small notice when an application with no channel starts
    /// playing, with a button per channel to put it in one.
    ///
    /// **On by default**: it is the clearest way to learn that an app is
    /// playing outside Lanes' control. It never takes focus, stays quiet while a game or presentation is
    /// full-screen, and carries its own "Turn off" link. A config that already
    /// says `false` keeps it off.
    #[serde(default = "default_true")]
    pub notify_new_apps: bool,
    /// Lanes has put Windows audio back as it was before Lanes, and is leaving
    /// it alone until the user resumes. See `restore::reset_windows`.
    ///
    /// Saved, so a restart while paused stays paused: resuming is the user's
    /// decision, and a reboot is not one.
    #[serde(default)]
    pub paused: bool,
    /// Light or dark windows.
    ///
    /// **Follows Windows by default**, with a manual override for people whose desk lighting disagrees with their
    /// taskbar. The core stores it only because the windows are destroyed
    /// every time they close and so cannot remember it themselves; nothing in
    /// the core looks at it.
    #[serde(default)]
    pub theme: Theme,
}

/// The windows' colour scheme. See [`Settings::theme`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Whatever Windows' "app mode" is set to, following it when it changes.
    #[default]
    System,
    Dark,
    Light,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // No remembered placement until the window has been closed once,
            // so a first run opens where WindowStartupLocation puts it.
            window: None,
            start_with_windows: false,
            start_minimised: false,
            close_to_tray: true,
            pool_collapsed: true,
            notify_new_apps: true,
            paused: false,
            theme: Theme::System,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut mic = Channel::new("mic", "Mic");
        mic.is_input = true;

        Self {
            readable: true,
            schema_version: SCHEMA_VERSION,
            channels: vec![
                Channel::new("master", "Master"),
                Channel::new("game", "Game"),
                Channel::new("chat", "Chat"),
                Channel::new("media", "Media"),
                Channel::new("aux", "Aux"),
                mic,
            ],
            // Deliberately empty. See the field's documentation.
            rules: Vec::new(),
            ignored: Vec::new(),
            api_token: None,
            settings: Settings::default(),
            chat_mix: ChatMix::default(),
            device_priority: Vec::new(),
            profiles: Vec::new(),
            active_profile: None,
            hotkeys: Vec::new(),
        }
    }
}

impl Config {
    pub fn channel(&self, id: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.id == id)
    }

    pub fn channel_mut(&mut self, id: &str) -> Option<&mut Channel> {
        self.channels.iter_mut().find(|c| c.id == id)
    }

    /// The Master channel scales every other channel.
    pub fn master(&self) -> Option<&Channel> {
        self.channel("master")
    }

    /// How many times to retry a config that will not open.
    ///
    /// The realistic cause is this file's own atomic save: the rename window
    /// between the temporary file and the real one is a fraction of a
    /// millisecond, but a second process reading at exactly that moment gets a
    /// sharing violation. A short retry turns that into a non-event.
    const READ_ATTEMPTS: usize = 5;
    const RETRY_PAUSE: std::time::Duration = std::time::Duration::from_millis(40);

    /// Load from disk, falling back to defaults.
    ///
    /// A missing file is normal — it means first run. A *corrupt* file is not,
    /// and is handled by moving it aside and starting fresh rather than
    /// refusing to start: config trouble must not stop the app from running,
    /// and keeping the bad file means the user can still recover their rules by
    /// hand. A file that exists but cannot be *opened* is different again -
    /// see [`Config::readable`].
    pub fn load() -> Self {
        let path = paths::config_file();

        if !path.exists() {
            return Self::default();
        }

        let mut last: Option<std::io::Error> = None;

        for attempt in 0..Self::READ_ATTEMPTS {
            match Self::read(&path) {
                Ok(config) => return config,

                // Corrupt, or written by a newer build. The file is genuinely
                // unusable, so setting it aside and starting fresh is right -
                // and keeping it is what lets the user get their rules back by
                // hand.
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                    eprintln!("warning: could not parse {}: {e}", path.display());

                    let quarantine = path.with_extension("json.broken");
                    match std::fs::rename(&path, &quarantine) {
                        Ok(()) => {
                            eprintln!("  kept the unreadable file at {}", quarantine.display())
                        }
                        Err(e) => eprintln!("  could not set it aside: {e}"),
                    }

                    eprintln!("  starting from defaults; no rules, everything unassigned");
                    return Self::default();
                }

                // Could not open it. The contents may be perfectly good, so
                // nothing is moved, nothing is assumed, and it is tried again.
                Err(e) => {
                    last = Some(e);
                    if attempt + 1 < Self::READ_ATTEMPTS {
                        std::thread::sleep(Self::RETRY_PAUSE);
                    }
                }
            }
        }

        eprintln!(
            "ERROR: {} exists but could not be opened after {} attempts.",
            path.display(),
            Self::READ_ATTEMPTS
        );
        if let Some(e) = last {
            eprintln!("  {e}");
        }
        eprintln!("  NOT starting from defaults: your channels and rules are still in that file,");
        eprintln!("  and running without them would apply nothing and then overwrite them.");

        Self {
            readable: false,
            ..Self::default()
        }
    }

    fn read(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let config: Config = serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        if config.schema_version > SCHEMA_VERSION {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "config uses schema version {} but this build understands up to {}",
                    config.schema_version, SCHEMA_VERSION
                ),
            ));
        }

        Ok(config)
    }

    /// Write atomically: temp file, then rename over the original.
    ///
    /// **Refuses when this config did not come from the file.** See
    /// [`Config::readable`]: writing defaults over a config we merely failed to
    /// open is how a transient lock turns into permanent data loss.
    pub fn save(&self) -> std::io::Result<()> {
        if !self.readable {
            return Err(std::io::Error::other(
                "refusing to overwrite a config that could not be read",
            ));
        }

        let path = paths::config_file();
        paths::ensure_dir(&paths::root())?;

        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;

        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, json)?;
        std::fs::rename(&temp, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_starts_condensed() {
        // The window opens clean: nothing expands itself, ever. A regression
        // here would make the app noisier on every launch, which is precisely
        // what it exists to avoid.
        let c = Config::default();
        assert!(c.settings.pool_collapsed, "the pool must start collapsed");
        assert!(
            c.channels.iter().all(|ch| ch.collapsed),
            "every channel apps list must start collapsed"
        );
    }

    #[test]
    fn an_older_config_without_the_flags_still_starts_condensed() {
        // serde's default for bool is false, which would be the wrong answer
        // for all three of these. Hence the explicit default_true.
        let json = r#"{"schema_version":1,"channels":[{"id":"media","name":"Media","volume":1.0,"muted":false,"device":null}],"rules":[]}"#;
        let parsed: Config = serde_json::from_str(json).unwrap();
        assert!(parsed.settings.pool_collapsed);
        assert!(parsed.settings.close_to_tray);
        assert!(parsed.channels[0].collapsed);
    }

    #[test]
    fn close_to_tray_defaults_on() {
        // A regression here would silently change what the X button does.
        assert!(Config::default().settings.close_to_tray);
    }

    #[test]
    fn missing_settings_block_still_defaults_close_to_tray_on() {
        // An older config file has no "settings" key at all. serde's default
        // for bool is false, which would be the wrong answer — hence the
        // explicit default_true.
        let json = r#"{"schema_version":1,"channels":[],"rules":[]}"#;
        let parsed: Config = serde_json::from_str(json).unwrap();
        assert!(parsed.settings.close_to_tray);
    }

    #[test]
    fn notices_default_on_and_an_old_choice_is_kept() {
        let fresh: Config = serde_json::from_str(r#"{"schema_version":1,"channels":[],"rules":[],"settings":{}}"#).unwrap();
        assert!(fresh.settings.notify_new_apps);
        assert!(!fresh.settings.paused);
        let chose_off: Config = serde_json::from_str(
            r#"{"schema_version":1,"channels":[],"rules":[],"settings":{"notify_new_apps":false}}"#,
        )
        .unwrap();
        assert!(!chose_off.settings.notify_new_apps);
    }

    #[test]
    fn theme_follows_windows_unless_told_otherwise() {
        // A config from before the setting existed follows Windows, and the
        // three values are the lowercase words the window reads back.
        let json = r#"{"schema_version":1,"channels":[],"rules":[],"settings":{}}"#;
        let parsed: Config = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.settings.theme, Theme::System);

        let light: Theme = serde_json::from_str(r#""light""#).unwrap();
        assert_eq!(light, Theme::Light);
        assert_eq!(serde_json::to_string(&Theme::Dark).unwrap(), r#""dark""#);
    }

    #[test]
    fn defaults_have_six_channels_and_no_rules() {
        let c = Config::default();
        assert_eq!(c.channels.len(), 6);
        assert!(
            c.rules.is_empty(),
            "everything must start in To be routed, unassigned"
        );
    }

    #[test]
    fn mic_is_the_only_input_channel() {
        let c = Config::default();
        let inputs: Vec<_> = c.channels.iter().filter(|ch| ch.is_input).collect();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].id, "mic");
    }

    #[test]
    fn round_trips_through_json() {
        let c = Config::default();
        let text = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back.channels.len(), c.channels.len());
    }
    // --- ChatMix ---------------------------------------------------------

    /// Centre must be a true no-op. This is the property the whole design was
    /// chosen for: a user who never touches the control must not be able to
    /// tell it exists.
    #[test]
    fn chat_mix_at_centre_changes_nothing() {
        let mix = ChatMix::default();
        assert_eq!(mix.factor_for("game"), 1.0);
        assert_eq!(mix.factor_for("chat"), 1.0);
        assert_eq!(mix.factor_for("media"), 1.0);
    }

    /// Sliding toward Game quietens Chat and leaves Game alone. Nothing ever
    /// gets louder than its own fader.
    #[test]
    fn chat_mix_toward_game_only_attenuates_chat() {
        let mix = ChatMix {
            value: 0.6,
            ..ChatMix::default()
        };
        assert_eq!(mix.factor_for("game"), 1.0);
        assert!((mix.factor_for("chat") - 0.4).abs() < 1e-6);
    }

    #[test]
    fn chat_mix_toward_chat_only_attenuates_game() {
        let mix = ChatMix {
            value: -0.25,
            ..ChatMix::default()
        };
        assert!((mix.factor_for("game") - 0.75).abs() < 1e-6);
        assert_eq!(mix.factor_for("chat"), 1.0);
    }

    /// The control names two channels and must not touch any others.
    #[test]
    fn chat_mix_leaves_other_channels_alone() {
        let mix = ChatMix {
            value: 1.0,
            ..ChatMix::default()
        };
        for other in ["master", "media", "aux", "mic"] {
            assert_eq!(mix.factor_for(other), 1.0, "{other} was attenuated");
        }
    }

    /// A client that overshoots the end of its slider gets clamped, not an
    /// error it cannot act on — and never a negative volume.
    #[test]
    fn chat_mix_clamps_out_of_range_values() {
        let mix = ChatMix {
            value: 4.0,
            ..ChatMix::default()
        };
        assert_eq!(mix.factor_for("chat"), 0.0);
        assert_eq!(mix.factor_for("game"), 1.0);
    }

    /// A hand-edited config naming the same channel at both ends would
    /// otherwise attenuate it whichever way the slider moved.
    #[test]
    fn chat_mix_ignores_a_config_naming_one_channel_twice() {
        let mix = ChatMix {
            value: 0.8,
            game: "game".into(),
            chat: "game".into(),
        };
        assert_eq!(mix.factor_for("game"), 1.0);
    }

    /// A config written before this feature existed must load, and must behave
    /// as though the control is centred.
    #[test]
    fn a_config_without_chat_mix_loads_centred() {
        let json = r#"{
            "schema_version": 1,
            "channels": [],
            "rules": []
        }"#;
        let config: Config = serde_json::from_str(json).expect("should load");
        assert_eq!(config.chat_mix.value, 0.0);
        assert_eq!(config.chat_mix.game, "game");
        assert_eq!(config.chat_mix.chat, "chat");
        assert_eq!(config.chat_mix.factor_for("chat"), 1.0);
    }
    // --- Master as a ceiling ---------------------------------------------

    fn with_master(master: f32, chat: f32) -> Config {
        let mut config = Config::default();
        for channel in &mut config.channels {
            match channel.id.as_str() {
                "master" => channel.volume = master,
                "chat" => channel.volume = chat,
                _ => {}
            }
        }
        config
    }

    /// The user's own description: everything at 100%, Master to 75%, and
    /// every channel follows it down.
    #[test]
    fn master_pulls_channels_down_with_it() {
        let config = with_master(0.75, 1.0);
        let chat = config.channel("chat").expect("chat exists");
        assert_eq!(config.effective_volume(chat), 0.75);
    }

    /// And the other half of it: a channel already below the ceiling is left
    /// alone.
    #[test]
    fn a_channel_below_the_ceiling_is_untouched() {
        let config = with_master(0.75, 0.5);
        let chat = config.channel("chat").expect("chat exists");
        assert_eq!(config.effective_volume(chat), 0.5);
    }

    /// "...until Master moves below 50%, at which point Chat would move with
    /// it."
    #[test]
    fn a_channel_follows_once_the_ceiling_passes_it() {
        let config = with_master(0.3, 0.5);
        let chat = config.channel("chat").expect("chat exists");
        assert_eq!(config.effective_volume(chat), 0.3);
    }

    /// The property that makes this a ceiling and not a clamp: the channel
    /// keeps what it was set to, so raising Master restores it.
    #[test]
    fn the_ceiling_never_overwrites_what_the_user_set() {
        let mut config = with_master(0.2, 0.8);
        let chat = config.channel("chat").expect("chat exists");
        assert_eq!(config.effective_volume(chat), 0.2);
        assert_eq!(chat.volume, 0.8, "the channel's own value must survive");

        // Master comes back up.
        for channel in &mut config.channels {
            if channel.id == "master" {
                channel.volume = 1.0;
            }
        }
        let chat = config.channel("chat").expect("chat exists");
        assert_eq!(config.effective_volume(chat), 0.8);
    }

    /// Master is not limited by itself, and the microphone carries no playback
    /// for a playback ceiling to apply to.
    #[test]
    fn master_and_the_microphone_are_exempt() {
        let config = with_master(0.4, 1.0);

        let master = config.channel("master").expect("master exists");
        assert_eq!(config.effective_volume(master), 0.4);

        let mic = config.channel("mic").expect("mic exists");
        assert_eq!(config.effective_volume(mic), mic.volume);
    }
}
