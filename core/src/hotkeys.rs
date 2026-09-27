//! Global hotkeys: what a binding is, how its keys are written, and whether
//! Windows accepted it.
//!
//! # Where the pieces live, and why
//!
//! - **The bindings are in the config**, as `hotkeys`, and the core owns them
//!   like everything else that persists.
//! - **The tray thread registers them**, with `RegisterHotKey`. Windows
//!   delivers a hotkey as a `WM_HOTKEY` message to the thread that registered
//!   it, and the tray thread is the one with a message loop. It is told the
//!   current list the same way it is told everything else - see
//!   `tray::TrayView` - and reports back which keys Windows refused.
//! - **The core runs the action**, through the same command handler the API
//!   uses, so a hotkey that mutes Chat does exactly what the window's mute
//!   button does.
//!
//! # They ship empty
//!
//! The same reasoning as the rules: nothing happens that the user did not ask
//! for. A global hotkey takes its keys away from every other program on the
//! machine, and a default that happened to be some game's or editor's shortcut
//! would break it for as long as Lanes was running, with nothing to say why.
//!
//! # A key a program already holds is reported, not hidden
//!
//! `RegisterHotKey` fails when another program has registered the same keys,
//! and there is no way to take them. The binding stays in the config - the
//! other program may be closed later - and its status says it is in use, so the
//! window can show it rather than leaving a key that silently does nothing.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// How far one press of a volume key moves a channel.
///
/// Five percent: twenty presses from silence to full, which is about what the
/// Windows volume keys do, and coarse enough that holding one down (Windows
/// repeats nothing for us - see `MOD_NOREPEAT` in the tray) is not needed.
pub const VOLUME_STEP: f32 = 0.05;

/// One hotkey.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    /// Written as the window shows it, modifiers first: `Ctrl+Alt+M`. Stored
    /// in that normalised form whatever form it arrived in.
    pub keys: String,
    pub action: Action,
}

/// What a hotkey does. Every one of these is something the window or the tray
/// can already do; a hotkey is only another way to ask.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Mute or unmute one channel.
    ToggleMute { channel: String },
    /// Mute or unmute Master, which is what "mute all" means here.
    MuteAll,
    /// One channel up by [`VOLUME_STEP`].
    VolumeUp { channel: String },
    /// One channel down by [`VOLUME_STEP`].
    VolumeDown { channel: String },
    /// Step one channel to the next output device.
    CycleDevice { channel: String },
    /// Switch to a saved profile.
    ActivateProfile { name: String },
    /// Open the mixer, or close it if it is open.
    ShowWindow,
    /// Open the tray's quick mixer, or close it if it is open.
    QuickMixer,
}

impl Action {
    /// The channel this action names, if it names one.
    pub fn channel(&self) -> Option<&str> {
        match self {
            Action::ToggleMute { channel }
            | Action::VolumeUp { channel }
            | Action::VolumeDown { channel }
            | Action::CycleDevice { channel } => Some(channel),
            _ => None,
        }
    }
}

/// A parsed key combination, ready for `RegisterHotKey`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    /// `MOD_ALT`, `MOD_CONTROL`, `MOD_SHIFT`, `MOD_WIN`, or'd together.
    pub modifiers: u32,
    /// The virtual-key code.
    pub key: u32,
}

const MOD_ALT: u32 = 0x1;
const MOD_CONTROL: u32 = 0x2;
const MOD_SHIFT: u32 = 0x4;
const MOD_WIN: u32 = 0x8;

/// Every key a binding may use, by the name it is written with.
///
/// Deliberately a list rather than "anything with a virtual-key code": a name
/// has to round-trip through the config and the window, and a key nobody can
/// write down is a key nobody can rebind.
const KEYS: &[(&str, u32)] = &[
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("Pause", 0x13),
    ("ScrollLock", 0x91),
    ("Plus", 0xBB),
    ("Comma", 0xBC),
    ("Minus", 0xBD),
    ("Period", 0xBE),
    ("VolumeMute", 0xAD),
    ("VolumeDown", 0xAE),
    ("VolumeUp", 0xAF),
    ("MediaNext", 0xB0),
    ("MediaPrevious", 0xB1),
    ("MediaStop", 0xB2),
    ("MediaPlayPause", 0xB3),
];

/// The virtual-key code for a key name, and the name as it should be written.
fn key_code(name: &str) -> Option<(u32, String)> {
    let upper = name.to_ascii_uppercase();

    // A to Z, 0 to 9.
    if upper.len() == 1 {
        let c = upper.as_bytes()[0];
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some((c as u32, upper));
        }
    }

    // F1 to F24.
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some((0x70 + n - 1, format!("F{n}")));
        }
    }

    // NumPad0 to NumPad9.
    if let Some(n) = upper.strip_prefix("NUMPAD").and_then(|n| n.parse::<u32>().ok()) {
        if n <= 9 {
            return Some((0x60 + n, format!("NumPad{n}")));
        }
    }

    KEYS.iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(known, code)| (*code, known.to_string()))
}

/// Read a combination like `ctrl + alt + m` into a chord and its normal
/// spelling, `Ctrl+Alt+M`.
///
/// # What is refused
///
/// **A binding must include Ctrl, Alt or Win**, unless it is one of F13 to F24,
/// which no keyboard used for typing has and which exist for exactly this. A
/// global hotkey takes its keys from every program, so `M` alone would stop the
/// letter M being typed anywhere while Lanes runs - and `Shift+M` would stop
/// the capital. That includes the media and volume keys: taking the volume
/// keys away from Windows is a decision worth two keys, not one.
pub fn parse(text: &str) -> Result<(Chord, String), String> {
    let parts: Vec<&str> = text
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();

    let Some((key_name, modifier_names)) = parts.split_last() else {
        return Err("no keys given".into());
    };

    let mut modifiers = 0;
    for name in modifier_names {
        modifiers |= match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => MOD_CONTROL,
            "alt" => MOD_ALT,
            "shift" => MOD_SHIFT,
            "win" | "windows" | "super" => MOD_WIN,
            other => return Err(format!("'{other}' is not a modifier (use Ctrl, Alt, Shift or Win)")),
        };
    }

    let (key, key_text) =
        key_code(key_name).ok_or_else(|| format!("'{key_name}' is not a key Lanes can bind"))?;

    let spare_key = (0x7C..=0x87).contains(&key); // F13 to F24
    if modifiers & (MOD_CONTROL | MOD_ALT | MOD_WIN) == 0 && !spare_key {
        return Err(format!(
            "{key_text} needs Ctrl, Alt or Win with it - on its own it would stop working everywhere else"
        ));
    }

    let mut text = String::new();
    for (flag, name) in [
        (MOD_CONTROL, "Ctrl"),
        (MOD_ALT, "Alt"),
        (MOD_SHIFT, "Shift"),
        (MOD_WIN, "Win"),
    ] {
        if modifiers & flag != 0 {
            text.push_str(name);
            text.push('+');
        }
    }
    text.push_str(&key_text);

    Ok((Chord { modifiers, key }, text))
}

/// Whether Windows accepted a binding's keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// Registered and live.
    Active,
    /// Windows refused: another program holds these keys.
    InUse,
    /// Refused for some other reason, which is carried.
    Failed(String),
}

/// What the tray thread last reported, by normalised keys.
///
/// A static rather than core-loop state so the state builder can read it
/// without being handed yet another argument; it is written only by the core
/// loop, when the tray reports.
static STATUS: Mutex<Option<HashMap<String, Registration>>> = Mutex::new(None);

/// Record what the tray thread reported.
pub fn record(results: Vec<(String, Registration)>) {
    if let Ok(mut status) = STATUS.lock() {
        *status = Some(results.into_iter().collect());
    }
}

/// The status of one binding: `Some` once the tray has reported on it.
pub fn status(keys: &str) -> Option<Registration> {
    STATUS
        .lock()
        .ok()
        .and_then(|status| status.as_ref().and_then(|s| s.get(keys).cloned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_normalised_modifiers_first() {
        let (chord, text) = parse(" alt + ctrl +m ").unwrap();
        assert_eq!(text, "Ctrl+Alt+M");
        assert_eq!(chord.modifiers, MOD_CONTROL | MOD_ALT);
        assert_eq!(chord.key, 'M' as u32);
    }

    #[test]
    fn named_and_numbered_keys() {
        assert_eq!(parse("Ctrl+F5").unwrap().0.key, 0x74);
        assert_eq!(parse("win+numpad3").unwrap().1, "Win+NumPad3");
        assert_eq!(parse("Ctrl+Shift+pageup").unwrap().1, "Ctrl+Shift+PageUp");
        assert_eq!(parse("Alt+7").unwrap().0.key, '7' as u32);
    }

    #[test]
    fn a_key_alone_is_refused_but_a_spare_function_key_is_not() {
        assert!(parse("M").is_err());
        assert!(parse("Shift+M").is_err(), "Shift alone still steals the capital");
        assert!(parse("VolumeUp").is_err());
        assert_eq!(parse("F13").unwrap().1, "F13");
        assert!(parse("F12").is_err());
    }

    #[test]
    fn nonsense_is_explained() {
        assert!(parse("").unwrap_err().contains("no keys"));
        assert!(parse("Ctrl+Hyper+M").unwrap_err().contains("not a modifier"));
        assert!(parse("Ctrl+Banana").unwrap_err().contains("not a key"));
        assert!(parse("Ctrl+F25").is_err());
    }

    #[test]
    fn bindings_round_trip_through_json() {
        let binding = Binding {
            keys: "Ctrl+Alt+M".into(),
            action: Action::ToggleMute {
                channel: "chat".into(),
            },
        };
        let json = serde_json::to_string(&binding).unwrap();
        assert_eq!(
            json,
            r#"{"keys":"Ctrl+Alt+M","action":{"kind":"toggle_mute","channel":"chat"}}"#
        );
        assert_eq!(serde_json::from_str::<Binding>(&json).unwrap(), binding);

        let bare = r#"{"keys":"Ctrl+Alt+W","action":{"kind":"show_window"}}"#;
        assert_eq!(
            serde_json::from_str::<Binding>(bare).unwrap().action,
            Action::ShowWindow
        );
    }
}
