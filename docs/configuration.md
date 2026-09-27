# Configuration

Everything Lanes remembers lives in one file:

```
%LOCALAPPDATA%\Lanes\config.json
```

(Or, for a portable copy, in a `Lanes` folder beside `Lanes.exe`.)

It is plain JSON and it is meant to be readable. Hand-editing it is a supported way to work, and the core reloads it when a new audio session appears, so most edits take effect without a restart. For anything beyond a quick edit, quit Lanes first: the running core saves its own copy whenever something changes, and would write over yours.

---

## Before you edit it

**Write it as UTF-8 with no byte order mark.** A BOM makes the file unparseable (`expected value at line 1 column 1`) and the app will move it aside and start from defaults. Windows PowerShell's `Set-Content -Encoding utf8` and `Out-File` both add one; use `[System.IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($false)))` instead, or any editor set to "UTF-8" rather than "UTF-8 with BOM".

**A broken file is never silently discarded.** If the config cannot be parsed it is renamed to `config.json.broken` and the app starts from defaults rather than refusing to run. Your rules are still in that file, so recovering them is a matter of fixing the syntax and renaming it back.

**Writes are atomic** — a temporary file and a rename — so a crash mid-save cannot leave a half-written config.

---

## A worked example

```json
{
  "schema_version": 1,
  "channels": [
    {
      "id": "master",
      "name": "Master",
      "volume": 1.0,
      "muted": false,
      "device": null,
      "collapsed": true,
      "is_input": false
    },
    {
      "id": "media",
      "name": "Media",
      "volume": 0.4,
      "muted": false,
      "device": {
        "id": "{0.0.0.00000000}.{6f1c2d3e-4a5b-4c6d-8e7f-a1b2c3d4e5f6}",
        "name": "Speakers (Realtek(R) Audio)"
      },
      "collapsed": true,
      "is_input": false
    },
    {
      "id": "mic",
      "name": "Mic",
      "volume": 1.0,
      "muted": false,
      "device": null,
      "collapsed": true,
      "is_input": true
    }
  ],
  "rules": [
    { "match": "Spotify.exe", "channel": "media" },
    { "match": "chrome*.exe", "channel": "media", "trim": 0.8 },
    { "match": "Discord.exe", "channel": "chat", "path_contains": "\\Discord\\" }
  ],
  "ignored": ["SomeLauncher.exe"],
  "api_token": "…generated on first use…",
  "settings": {
    "start_with_windows": false,
    "start_minimised": false,
    "close_to_tray": true,
    "pool_collapsed": true
  },
  "chat_mix": { "value": 0.0, "game": "game", "chat": "chat" }
}
```

---

## Top level

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | number | Currently **1**. A file from a *newer* version is refused rather than guessed at — see below |
| `channels` | array | The mixer strips. Ships with six |
| `rules` | array | Executable → channel. **Empty by default** |
| `ignored` | array of strings | Executables never managed or shown |
| `api_token` | string | Shared secret for the local API, generated on first use |
| `settings` | object | Lifecycle and UI preferences |
| `chat_mix` | object | The Game/Chat balance control |
| `device_priority` | array | The global device fallback order - see below |
| `profiles` | array | Saved profiles - see below |
| `active_profile` | string | The profile last saved or switched to. Only breaks ties - see below |
| `hotkeys` | array | Global hotkeys. **Empty by default** - see below |

**`schema_version` is checked in one direction only.** A file claiming a version higher than this build understands is refused, because guessing at a format from the future is how configurations get silently corrupted. An older version loads normally: every field added since has a default.

**`rules` is empty by default, deliberately.** Every application starts in "To be routed" until you assign it. Nothing is ever moved that you did not ask to be moved.

**`api_token` is the only secret in the project**, and it never leaves this file — it is not in the repository and not in any build. Deleting it makes the app generate a new one, which disconnects any Stream Deck plugin until it is re-read.

---

## `channels`

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | Stable key used by rules and the API. **Never rename this** — rename `name` instead |
| `name` | string | What you see. Rename freely |
| `volume` | 0.0–1.0 | The channel's level. For Master, see "Master" below |
| `muted` | bool | |
| `device` | object or `null` | Preferred output. `null` leaves apps on the system default. For Master, see below |
| `collapsed` | bool | Whether the apps list is collapsed. Defaults to `true` |
| `is_input` | bool | Input-only. The Mic channel shows a level and a mute and never routes the microphone signal |

`device` is `{ "id": …, "name": … }`. The **id** is what is used; the **name** is stored so the file stays readable, and so an unplugged device can still be shown by name in the picker.

**Six channels ship, and one more can be added** - in **Settings > Channels**, or with `add_channel` - for seven at most. An added channel goes just before Mic, with an id made from its name (`Voice Chat` becomes `voicechat`), which never changes afterwards however it is renamed. **Only the added channel can be removed.** Master is the ceiling over every playback channel and Mic is the one input, so the shipped six stay. Removing the added channel takes its apps' rules with it - those apps go back to "To be routed", playing exactly as they were - along with any hotkeys that named it.

### `device_priority` — where channels go when their device is unplugged

One list for the whole application, in priority order, like a BIOS boot order:

```json
"device_priority": [
  { "id": "{0.0.0.00000000}.{6f1c…}", "name": "Headphones (USB Audio)",      "enabled": true },
  { "id": "{0.0.0.00000000}.{9d8e…}", "name": "Speakers (Realtek(R) Audio)", "enabled": true }
]
```

When a channel's `device` is not plugged in, it uses the first entry here that
is **enabled and plugged in** (and is not the device it just lost). If there is
none, it follows the system default. The mixer's device button turns **amber**
whenever a channel is on a stand-in.

- **`enabled: false`** excludes a device from fallback, and it is always kept at
  the bottom. A channel pointed at a disabled device *directly* still uses it.
- **Lanes maintains the list.** Every output device it sees is added at the
  bottom of the enabled entries, enabled. Nothing is removed: an unplugged
  device keeps its place.
- **`device` is never rewritten while it is absent.** Plug it back in and the
  channel returns to it.
- **A channel with `device: null` is never "on a fallback".** It asked for the
  system default and is getting it.

Change it in the window under **Audio devices**, with `set_device_priority`, or
by hand here.

### Master

**Master is a ceiling, not a multiplier.** An application's level is:

```text
level = min(channel.volume, master.volume) × chat_mix × (rule.trim or 1.0)
muted = master.muted  or  channel.muted
```

With every channel at 100%, Master at 75% brings them all to 75%. A channel
already at 50% is untouched until Master drops below 50%, and then moves with
it. A channel's own `volume` is never overwritten: raise Master again and every
channel returns to its own level.

**Master is also Windows' default output.** Its `device` is the Windows default
output device, its `volume` is that device's own volume slider (the one in the
taskbar), and its `muted` is that device's mute. Lanes keeps them in step both
ways, so the values here are a mirror of Windows rather than settings of their
own: Lanes reads them back from Windows at startup and whenever Windows changes
them, so editing them by hand here has no lasting effect. Apps on the default device are set
so that Windows' slider and the ceiling together give the right level, rather
than cutting it twice.


**`collapsed` defaults to `true`, and that default is load-bearing.** Nothing in this window opens itself. Note that `#[serde(default)]` on a `bool` would give `false`, which is the wrong answer here — the code uses an explicit `default_true`, and there are tests asserting it, including one against a config written before the field existed.

---

## `rules`

Matched against a session's executable when it appears.

| Field | Type | Meaning |
| --- | --- | --- |
| `match` | string | Executable name, case-insensitive. `*` matches any run of characters |
| `channel` | string | The channel **id** to assign to |
| `path_contains` | string, optional | The full path must contain this substring |
| `trim` | 0.0–1.0, optional | Per-app trim within the channel |

**More specific rules win**, and ties go to whichever appears first in the file — so hand-editing behaves predictably. Specificity counts the qualifiers: a rule with `path_contains` beats one without, and a literal name beats a wildcard.

`path_contains` exists because two different applications can ship the same executable name, and two installs of the same application can want different channels.

**`trim`** is set in the mixer by right-clicking an app in a channel, or with `set_app_trim`. The app's chip then shows the percentage, so it is clear why it is quieter than its channel-mates.

The glob is hand-written rather than a dependency: there is one metacharacter, and a crate whose edge cases differ from what someone expects while hand-editing would be worse than twenty lines that do exactly what they say.

---

## `settings`

| Field | Default | Meaning |
| --- | --- | --- |
| `window` | absent | Where the mixer window was last left: physical pixels on the virtual screen, and whether it was maximised. Absent until the window has been closed once |
| `start_with_windows` | `false` | Mirrors the `HKCU` Run key. **The registry is the source of truth**; this records what you last asked for, and the app corrects a stale path if the executable moves |
| `start_minimised` | `false` | Start in the tray with no window. Adds `--minimised` to the Run entry |
| `close_to_tray` | `true` | Clicking X closes the window and leaves the core running. Turned off, X quits |
| `pool_collapsed` | `true` | Whether "To be routed" is collapsed to its rail |
| `notify_new_apps` | `true` | Show a notice when an app with no channel starts playing. See below |
| `paused` | `false` | Lanes restored Windows audio and is leaving it alone until you resume. Set by Restore, cleared by Resume |
| `theme` | `"system"` | `"system"`, `"dark"` or `"light"`. See below |

`close_to_tray` and `pool_collapsed` both default to **true** for the same reason as `collapsed` above.

`notify_new_apps` defaults to **true**: a new application with no channel gets
one notice per run of Lanes, in the corner,
with a button per channel. It never takes focus, and it is **held back while a
game, video or presentation is full-screen** or Windows is in Focus / quiet
hours - Windows' own "is the user busy" check - and counted as shown, so it
does not appear the moment the game is minimised. The notice has a **Turn off
these notices** link, the same setting as Settings > General. A config that
already says `false` keeps it off.

`theme` is `"system"` unless chosen in Settings > General: the windows follow
Windows' own app mode (Settings > Personalisation > Colours), and change with
it. `"dark"` or `"light"` fixes them. Nothing in the core reads it; it is here
because the windows are destroyed when they close and have nowhere else to
keep it. Any other value is read as `"system"` by the windows.

---

## `chat_mix`

One slider across two channels, so game audio can be traded against voice chat without reaching for two faders.

| Field | Default | Meaning |
| --- | --- | --- |
| `value` | `0.0` | −1.0 to 1.0. **0.0 is centre and attenuates nothing** |
| `game` | `"game"` | The channel **id** favoured when `value` is positive |
| `chat` | `"chat"` | The channel **id** favoured when `value` is negative |

```text
value  0.0     both channels untouched
value +0.6     chat runs at 40% of its own level, game untouched
value -0.25    game runs at 75% of its own level, chat untouched
```

It **attenuates one side only** and multiplies with the channel's own volume, so each fader keeps its meaning and returning the slider to centre returns the sound to exactly where it was. The favoured side is never boosted.

`game` and `chat` are stored rather than hard-coded because channels are renameable and a seventh can be added — if your voice chat lives on Aux, say so. The window does it without editing this file: open a channel's menu (the **⋯** on its header, or right-click its name) and choose what to balance it against. That puts the mix back to centre. Naming the **same** channel in both fields by hand disables the control rather than attenuating that channel in both directions; the menu and the API refuse it.

---

## `profiles`

Named sets of channel settings, switched in one action from the window, the
tray or the API.

```json
"profiles": [
  {
    "name": "Movie night",
    "channels": [
      { "id": "media", "volume": 0.6, "muted": false,
        "device": { "id": "{0.0.0.00000000}.{9d8e…}", "name": "Speakers (Realtek(R) Audio)" } },
      { "id": "chat",  "volume": 1.0, "muted": true, "device": null }
    ],
    "chat_mix": -0.3
  }
],
"active_profile": "Movie night"
```

- **`channels[].id` is the channel's id, not its name**, so renaming a channel
  does not break a profile.
- **A channel a profile does not mention is left alone** when switching to it.
  That is how a hand-written profile can touch only Media.
- **Leave out `chat_mix`** and switching leaves the mix where it is.
- **The Mic channel's device is never applied**, even if a profile names one:
  the microphone signal is never routed.
- **`active_profile` is not what the window shows as active.** A profile is
  shown as active only while the mixer actually matches it. This field only
  decides which of two identical profiles is named. It is removed when that
  profile is deleted.

Profiles hold volume, mute and device. Rules, the fallback order and display
state are deliberately not part of them.

---

## `hotkeys`

```json
"hotkeys": [
  { "keys": "Ctrl+Alt+M",   "action": { "kind": "toggle_mute", "channel": "chat" } },
  { "keys": "Ctrl+Alt+Up",  "action": { "kind": "volume_up", "channel": "media" } },
  { "keys": "Ctrl+Alt+F10", "action": { "kind": "quick_mixer" } }
]
```

Set them in **tray menu > Settings > Hotkeys...**, where you press the keys
rather than type them. The kinds and the rule that every binding needs Ctrl, Alt
or Win are listed in [api.md](api.md) under Hotkeys.

**Empty by default**, for the same reason `rules` is: a global hotkey takes its
keys away from every other program, and a default that happened to be some
game's shortcut would break it with nothing to say why.

A hand-edited binding whose keys cannot be read, or that names a channel that no
longer exists, simply does nothing; the settings window shows it as refused.

---

## What is *not* in this file

- **Snapshots** of your audio settings: `%LOCALAPPDATA%\Lanes\snapshots\`. `first-run.json` is written once, the first time Lanes runs, and never overwritten. `windows-before.json` records Windows' default output and each device's volume before Lanes first changed them - what Restore puts back.
- **The audit log** of every change made: `%LOCALAPPDATA%\Lanes\logs\audit-YYYY-MM-DD.jsonl`.
- **The API port**: `%LOCALAPPDATA%\Lanes\port`, a plain number, so clients can find the server without configuration.

Uninstalling is deleting the executable and the `%LOCALAPPDATA%\Lanes` folder. Note that this does **not** undo per-app device assignments, because Windows stores those itself — use **Restore Windows audio settings**, or `--restore`, before deleting anything. See [troubleshooting.md](troubleshooting.md).
