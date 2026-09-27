//! The local API contract.
//!
//! This module **is** the contract. `docs/api.md` describes it for humans, but
//! the types here are what every client actually talks to. It is stable:
//! the Stream Deck plugin, which lives in its own repository and ships on its
//! own schedule, depends on it. Add to it; do not change what is there.
//!
//! # Shape
//!
//! Clients send [`Request`]s carrying an `id`. The server answers each with a
//! [`Reply`] carrying the same `id`, **and the resulting state**, so a client
//! never has to guess whether a command worked, or issue a second round trip to
//! find out.
//!
//! Independently, the server pushes [`Event`]s: full state on connect, deltas
//! thereafter, and meter levels only while someone has asked for them.
//!
//! # Idempotence
//!
//! Every command that can be idempotent is. `set_channel_volume` to the value
//! it already holds succeeds and changes nothing — it does not error, and it
//! does not write to the audit log. This matters for a Stream Deck, whose
//! buttons are pressed optimistically and whose state may briefly disagree with
//! the core's.

use serde::{Deserialize, Serialize};

/// Bumped when the wire format changes incompatibly.
///
/// Sent in [`Event::Welcome`] so a client can refuse to talk to a core it does
/// not understand, rather than misinterpreting it.
pub const PROTOCOL_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Client -> server
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// Echoed back in the reply. Clients that never need to correlate may omit
    /// it; the reply then carries `null`.
    #[serde(default)]
    pub id: Option<u64>,
    /// The shared token from `config.json`.
    ///
    /// Checked on every request rather than only at connect. It is cheap, and
    /// it means a client cannot hold a connection open across a token change.
    #[serde(default)]
    pub token: Option<String>,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Full state, on demand. Sent automatically on connect too.
    GetState,

    SetChannelVolume {
        channel: String,
        value: f32,
    },
    /// Relative change, clamped to 0..=1.
    ///
    /// **This is what a Stream Deck needs**, and on the target hardware — a
    /// Neo, with keys and no dials — it is the *only* way volume can be
    /// controlled at all. An absolute-value command cannot express "a bit
    /// louder" from a button that does not know the current value.
    AdjustChannelVolume {
        channel: String,
        delta: f32,
    },
    SetChannelMute {
        channel: String,
        muted: bool,
    },
    ToggleChannelMute {
        channel: String,
    },
    SetChannelDevice {
        channel: String,
        /// `None` returns the channel to the system default.
        #[serde(default)]
        device_id: Option<String>,
    },
    /// Set the global fallback order: every output device, in priority order,
    /// each enabled or not.
    ///
    /// Replaces the order rather than moving one entry, so a client sends what
    /// it wants and does not have to read-modify-write. It is forgiving in the
    /// ways that matter: entries it does not mention are kept after the ones
    /// it does, so a client with a stale list cannot delete a device by
    /// omission, and unknown ids are ignored. Disabled entries are moved to the
    /// bottom whatever order they arrive in.
    SetDevicePriority {
        #[serde(default)]
        entries: Vec<PriorityInput>,
    },
    /// Step to the next available output device.
    ///
    /// Useful from a single button, where picking from a list is not possible.
    CycleChannelDevice {
        channel: String,
    },

    AssignApp {
        executable: String,
        channel: String,
    },
    UnassignApp {
        executable: String,
    },

    MuteAll {
        muted: bool,
    },
    RestoreWindowsSettings,
    /// Stop being paused after a Restore: manage applications again, and apply
    /// every channel at once. Harmless when not paused.
    Resume,

    /// Set the Game/Chat balance. -1.0 to 1.0, 0.0 is centre.
    ///
    /// Positive quietens Chat; negative quietens Game. Out-of-range values are
    /// clamped rather than rejected, so a client that scales a dial slightly
    /// past the end does not get an error it cannot act on.
    SetChatMix {
        value: f32,
    },
    /// Choose the two channels the Game/Chat mix balances. `game` is the side
    /// a positive value favours. The mix returns to centre. Refused for the
    /// same channel twice, Master, an input, or an unknown id.
    SetChatMixChannels {
        game: String,
        chat: String,
    },
    /// Nudge the Game/Chat balance.
    ///
    /// **This is what a Stream Deck needs**, for the same reason as
    /// [`Command::AdjustChannelVolume`]: a key that does not know the current
    /// value cannot express "a bit more game" any other way. On the target
    /// hardware — a Neo, with keys and no dials — it is the only usable form.
    AdjustChatMix {
        delta: f32,
    },

    /// Remember where the mixer window is.
    ///
    /// Sent when the window closes. Physical pixels, virtual-screen
    /// coordinates - see `config::Settings::window`.
    SetWindowPlacement {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        #[serde(default)]
        maximised: bool,
    },

    /// Show or hide a channel's application list.
    ///
    /// Display state, stored because it has to survive the window closing.
    /// Changing it touches no audio setting.
    SetChannelCollapsed {
        channel: String,
        collapsed: bool,
    },

    /// Bring the mixer window up, or focus it if it is already open.
    ///
    /// **This is what makes relaunching the program do the right thing.**
    /// Starting a second copy shows the running one's window rather than
    /// refusing; with the core already listening on loopback, the second copy
    /// sends this and exits.
    ShowWindow,

    /// Shut the core down.
    ///
    /// A full exit is deliberately hard to reach by accident - closing the
    /// window does not do it (unless "Close to tray" is off), and there is no
    /// bare "Exit" in the tray menu. This is the programmatic equivalent of the
    /// Quit item inside Settings, and it exists for two callers: `--quit` from
    /// a terminal, and the uninstaller, which cannot replace files that a
    /// running process holds open.
    Quit,

    /// Start or stop meter pushes for *this connection only*.
    ///
    /// Metering is the one thing in the core that needs a timer, so it runs
    /// only while a client has asked for it and stops when the last one
    /// disconnects. An idle Stream Deck must not cause continuous metering
    /// work.
    SubscribeMeters {
        enabled: bool,
    },

    /// Switch the mixer to a saved profile: every channel's volume, mute and
    /// device it names, and the Game/Chat mix. Names are matched without
    /// regard to case. See `profiles` for what a profile holds.
    ActivateProfile {
        name: String,
    },
    /// Save the mixer as it is now under `name`. An existing profile with that
    /// name is replaced, so saving twice is the same as saving once.
    SaveProfile {
        name: String,
    },
    /// Remove a saved profile. Changes no audio setting.
    DeleteProfile {
        name: String,
    },

    /// Replace every global hotkey.
    ///
    /// The whole list, like `set_device_priority`, so a client sends what it
    /// wants rather than editing one entry at a time. Refused as a whole if any
    /// binding's keys cannot be read, two bindings share keys, or one names a
    /// channel that does not exist - a list half-applied is harder to reason
    /// about than one refused with the reason. Keys are stored normalised:
    /// send `alt+ctrl+m`, get back `Ctrl+Alt+M`.
    SetHotkeys {
        #[serde(default)]
        bindings: Vec<crate::hotkeys::Binding>,
    },

    /// Rename a channel. Only its display name changes; its id, which rules,
    /// profiles and hotkeys use, never does.
    RenameChannel {
        channel: String,
        name: String,
    },

    /// Stop managing and showing an application. Its rule, if it has one, is
    /// kept for when it is un-ignored; its audio is left where it is.
    IgnoreApp {
        executable: String,
    },
    UnignoreApp {
        executable: String,
    },

    /// How loud one application is within its channel, as a fraction of the
    /// channel: 0.0 to 1.0, or `null` to follow the channel exactly. The
    /// application must be assigned to a channel.
    SetAppTrim {
        executable: String,
        #[serde(default)]
        trim: Option<f32>,
    },

    /// What the core can see and has done: the reply carries `diagnostics`.
    GetDiagnostics,

    /// The configuration as a file would hold it: the reply carries `config`,
    /// without the API token or the window's placement.
    ExportConfig,

    /// Replace the configuration with an exported one. Keeps this computer's
    /// token, window placement and start-with-Windows setting; snapshots the
    /// audio first and keeps the previous config beside the new one.
    ImportConfig {
        config: serde_json::Value,
    },

    /// Turn the new-application notice on or off.
    SetNotifyNewApps {
        enabled: bool,
    },

    /// Light, dark, or follow Windows. Changes no audio; open windows redraw.
    SetTheme {
        theme: crate::config::Theme,
    },

    /// Add a playback channel, before the Mic channel. Seven channels at most.
    /// The reply's state carries it; its id is made from the name.
    AddChannel {
        name: String,
    },

    /// Remove a channel that was added. The six that ship cannot be removed.
    /// Its apps return to "To be routed" without their audio moving, and
    /// hotkeys that named it are removed.
    RemoveChannel {
        channel: String,
    },
}

// ---------------------------------------------------------------------------
// Server -> client
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Reply {
    pub id: Option<u64>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
    /// The state after the command ran, so a client never has to ask.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<State>,
    /// Only in the reply to `get_diagnostics`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Diagnostics>,
    /// Only in the reply to `export_config`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

/// What `get_diagnostics` answers with.
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostics {
    /// Where the config, snapshots and logs are.
    pub state_folder: String,
    pub log_folder: String,
    pub portable: bool,
    /// Whether the undocumented per-app routing interface answered.
    pub routing_available: bool,
    /// Every audio session Windows reports, and what the core makes of it.
    pub sessions: Vec<DiagnosticSession>,
    /// The most recent audit-log entries, newest last.
    pub recent: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticSession {
    pub executable: String,
    pub path: String,
    pub process_id: u32,
    /// `playing`, `idle` or `expired`.
    pub state: String,
    pub volume: f32,
    pub muted: bool,
    /// The channel it resolves to; `null` if unassigned or ignored.
    pub channel: Option<String>,
    /// The pattern of the rule that put it there.
    pub rule: Option<String>,
    pub ignored: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Missing or wrong token.
    Unauthorised,
    /// Malformed JSON, or a command this build does not know.
    BadRequest,
    /// Named a channel, device or app that does not exist.
    NotFound,
    /// A real command that this build has not implemented yet.
    ///
    /// Not currently produced. Kept because it is part of the published set of
    /// error codes: a client may handle it, and a command reserved ahead of its
    /// implementation would use it.
    #[allow(dead_code)]
    NotImplemented,
    /// Windows refused, or the undocumented interop is unavailable.
    AudioError,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// First message on every connection, before any state.
    Welcome {
        protocol_version: u32,
        /// False when the token was wrong — the connection then closes.
        authenticated: bool,
    },
    /// Complete state. Sent on connect and on request.
    State { state: State },
    /// Only what changed. Fields that did not change are omitted entirely,
    /// which is what distinguishes this from a full push.
    Delta {
        #[serde(skip_serializing_if = "Option::is_none")]
        channels: Option<Vec<ChannelState>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sessions: Option<Vec<SessionState>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        devices: Option<Vec<DeviceState>>,
        /// Carried in deltas so a newly connected device appears in an open
        /// settings list straight away, as everything else in the window does.
        #[serde(skip_serializing_if = "Option::is_none")]
        device_priority: Option<Vec<PriorityState>>,
        /// Carried so a profile switched from the tray or a Stream Deck shows
        /// in an open window, and so a profile stops showing as active the
        /// moment a fader moves anywhere.
        #[serde(skip_serializing_if = "Option::is_none")]
        profiles: Option<ProfileState>,
        /// The Game/Chat balance. In deltas as well as full state, so a mix
        /// changed by one client - or by switching profile - moves every
        /// other client's control.
        #[serde(skip_serializing_if = "Option::is_none")]
        chat_mix: Option<ChatMixState>,
        /// Carried so a key the tray could not register shows as such in an
        /// open window as soon as the tray has tried.
        #[serde(skip_serializing_if = "Option::is_none")]
        hotkeys: Option<Vec<HotkeyState>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ignored: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        settings: Option<SettingsState>,
    },
    /// Per-channel levels, 0.0–1.0. Only while subscribed.
    Meters { levels: Vec<MeterLevel> },
    /// The core is stopping. Close; there is nothing left to talk to.
    ///
    /// A client cannot tell a shutdown from a crash or a dropped socket by the
    /// disconnect alone, and the difference matters: one should be reconnected
    /// to, the other should not. The mixer window would otherwise sit there
    /// saying "Reconnecting…" forever, holding its executable open — which is
    /// precisely what stops an uninstaller deleting it.
    Shutdown,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct State {
    /// Where the window was last left, if it has ever been closed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<crate::config::WindowPlacement>,
    pub channels: Vec<ChannelState>,
    pub sessions: Vec<SessionState>,
    pub devices: Vec<DeviceState>,
    /// The global fallback order. See `config::Config::device_priority`.
    pub device_priority: Vec<PriorityState>,
    pub profiles: ProfileState,
    /// The Game/Chat balance control.
    pub chat_mix: ChatMixState,
    /// Global hotkeys, and whether each is live.
    pub hotkeys: Vec<HotkeyState>,
    /// Executables never managed or shown.
    pub ignored: Vec<String>,
    /// Preferences a client can change.
    pub settings: SettingsState,
    /// True when per-app device routing is unavailable on this Windows build.
    ///
    /// The app keeps working as a volume mixer in that case and says so
    /// plainly; it never fails silently. The UI shows a
    /// banner driven by this flag.
    pub routing_available: bool,
}

/// The Game/Chat balance, as clients see it.
///
/// The two channel ids are reported rather than assumed so a client can label
/// the ends of the slider with the channels' real names, which the user may
/// have renamed.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChatMixState {
    /// -1.0 to 1.0. 0.0 is centre and attenuates nothing.
    pub value: f32,
    /// The channel favoured at +1.0; the one quietened at -1.0.
    pub game: String,
    /// The channel favoured at -1.0; the one quietened at +1.0.
    pub chat: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChannelState {
    pub id: String,
    pub name: String,
    /// What the user set this channel to. **Setting a volume sets this.**
    pub volume: f32,
    /// What the channel is actually running at, after Master's ceiling.
    ///
    /// Master limits every playback channel rather than scaling it, so a
    /// channel set to 100% with Master at 75% *runs* at 75% while still
    /// remembering that it was set to 100%. A fader should show this; a command
    /// that sets a volume should set [`ChannelState::volume`].
    ///
    /// Equal to `volume` for Master itself and for input channels.
    pub effective_volume: f32,
    pub muted: bool,
    pub is_input: bool,
    /// What the user asked for. `None` means system default.
    pub target_device: Option<DeviceRef>,
    /// What is actually in use. Differs from `target_device` when the preferred
    /// device is absent and a fallback is active.
    pub effective_device: Option<DeviceRef>,
    /// True when running on a fallback because the preferred device is missing.
    ///
    /// A channel silently playing out of the wrong speakers is the most
    /// annoying failure this app can have, so this is explicit state rather
    /// than something a client has to infer by comparing the two device
    /// fields.
    pub on_fallback: bool,
    /// Whether this channel's application list is shown collapsed.
    ///
    /// Purely a client's display state, and it lives here for one reason: it
    /// is **remembered between sessions**, and the core owns everything that
    /// persists. A window that kept it to itself
    /// would forget every time it closed - which it is designed to do - and
    /// two clients would disagree about the same channel.
    pub collapsed: bool,
    /// Whether `remove_channel` will accept it: only a channel that was added.
    /// Reported rather than left for clients to work out, so no client keeps a
    /// copy of which channels ship.
    pub removable: bool,
}

/// One entry in a `set_device_priority` request.
#[derive(Debug, Clone, Deserialize)]
pub struct PriorityInput {
    pub id: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

/// One output device's place in the fallback order, as clients see it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PriorityState {
    pub id: String,
    pub name: String,
    /// False: never used as a fallback, and listed at the bottom.
    pub enabled: bool,
    /// Plugged in right now. An unplugged device keeps its place.
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DeviceRef {
    pub id: String,
    pub name: String,
}

/// The config's device reference and the wire's are the same two fields, and
/// they are deliberately separate types: one is a storage format that must stay
/// readable by a human editing `config.json`, the other is a published contract
/// a Stream Deck plugin depends on. Letting one drift is fine; letting them
/// drift *silently* is not, which is what this conversion prevents — adding a
/// field to either side fails here rather than somewhere subtler.
impl From<crate::config::DeviceRef> for DeviceRef {
    fn from(from: crate::config::DeviceRef) -> Self {
        let crate::config::DeviceRef { id, name } = from;
        Self { id, name }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SessionState {
    pub executable: String,
    pub display_name: String,
    /// `None` means it is in the "To be routed" pool.
    pub channel: Option<String>,
    pub playing: bool,
    /// False for exclusive-mode sessions and apps that refuse routing.
    ///
    /// Always true in 1.0 — exclusive-mode detection is not built (see
    /// `docs/limitations.md`). Apps that refuse routing are reported
    /// separately, by `routing_refused`.
    pub controllable: bool,
    /// How many sessions this executable owns. Browsers and Electron apps have
    /// several; the UI shows one entry.
    pub session_count: usize,
    /// Its level within its channel, when it has been set below the channel.
    pub trim: Option<f32>,
    /// The executable's full path, so a client can show the application's own
    /// icon. Sessions are grouped by it, so it is the same for every session
    /// this entry stands for.
    pub path: String,
    /// True when the application would not keep the output device Lanes gave
    /// it - the application, or some other program, changed it straight back -
    /// so Lanes has stopped trying. Its volume and mute are still Lanes'.
    pub routing_refused: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DeviceState {
    pub id: String,
    pub name: String,
    pub direction: String,
    pub present: bool,
    pub is_default: bool,
}

/// Preferences a client can change through the API.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SettingsState {
    /// Whether an unassigned application starting to play shows a notice.
    pub notify_new_apps: bool,
    /// `system`, `dark` or `light`.
    pub theme: crate::config::Theme,
    /// Lanes has put Windows audio back as it was before Lanes and is leaving it
    /// alone until `resume`. Clients should say so, with a way to resume.
    pub paused: bool,
    /// Whether closing the mixer leaves Lanes running. When false, the mixer
    /// quits Lanes as it closes - the core cannot tell a window closed with X
    /// from one that exited for any other reason, so the window does it.
    pub close_to_tray: bool,
}

/// One global hotkey, as clients see it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HotkeyState {
    /// Normalised: `Ctrl+Alt+M`.
    pub keys: String,
    pub action: crate::hotkeys::Action,
    /// `active`: registered and live. `in_use`: another program holds these
    /// keys, so pressing them does nothing here. `failed`: Windows refused for
    /// another reason, given in `problem`. `pending`: the tray has not reported
    /// yet - or there is no tray, as with `--serve`, which registers none.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProfileState {
    /// Every saved profile, in the order they were created.
    pub names: Vec<String>,
    /// The profile the mixer currently **matches**, not the one last chosen.
    ///
    /// Move any fader a profile holds and this becomes `null`; move it back and
    /// the profile is active again. That is what lets a Stream Deck key or a
    /// menu tick be trusted. See `profiles::active`.
    pub active: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MeterLevel {
    pub channel: String,
    pub level: f32,
}

impl Reply {
    pub fn ok(id: Option<u64>, state: State) -> Self {
        Self {
            id,
            ok: true,
            error: None,
            state: Some(state),
            diagnostics: None,
            config: None,
        }
    }

    pub fn err(id: Option<u64>, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            error: Some(ApiError {
                code,
                message: message.into(),
            }),
            state: None,
            diagnostics: None,
            config: None,
        }
    }
}
