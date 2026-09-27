# Local API

Lanes' core exposes a local API on loopback. **The mixer window is a client of this API with no privileged access**, which is what makes the [Stream Deck plugin](https://github.com/MAYBE-33/lanes-streamdeck) a second client rather than a second implementation - and what lets a script, or a tool of your own, do anything the window can.

**Stable since 1.0.** The Stream Deck plugin depends on it. Add to it freely; change or remove nothing a client can see without a new `protocol_version`. The Rust types in `core/src/api/protocol.rs` are the contract; this page describes them.

## Connecting

The API is running whenever Lanes is (the tray app serves it). `Lanes.exe --serve` runs the core and API without the tray, for development.

It binds `127.0.0.1` only — **never** any other interface.

### Finding the port

The default is **8477**, but it is not guaranteed: if that port is taken the server scans upward. The port it actually bound is written to:

```
%LOCALAPPDATA%\Lanes\port
```

Read that file rather than assuming a number. A portable copy (a `portable.txt` beside `Lanes.exe`) keeps this and the config in a `Lanes` folder beside the executable instead.

### Authenticating

Every request carries a `token`, read from `api_token` in:

```
%LOCALAPPDATA%\Lanes\config.json
```

It is generated on first use. It is checked on **every** request, not just at connect, so a client cannot hold a connection open across a token change.

This is a guard against other local processes poking the API casually, which is all it is meant to be. It is not protection against someone who can already run code as you — at that point they can read the file.

### Web pages are refused

**Any request carrying an `Origin` header gets `403 Forbidden`**, WebSocket or HTTP, and so does one whose `Host` is anything but `127.0.0.1`, `localhost` or `[::1]`.

Loopback keeps other machines out; it does nothing about the browser on this one. Browsers do not apply cross-origin rules to WebSockets, so without this any page in any tab could open `ws://127.0.0.1:8477/` and be handed the full state on connect - every application playing sound, every device, every profile - since the token guards commands, not the state push. Programs (the window, the Stream Deck plugin, curl, a script) send no `Origin`; browsers always do on a WebSocket and on any cross-site request. The `Host` check covers DNS rebinding, where a hostile page makes *same-origin* requests that carry no `Origin` but do carry its own name in `Host`.

A client written as a web page therefore cannot talk to Lanes directly. That is deliberate; the Stream Deck plugin's settings panels, which are web pages, ask the plugin, and the plugin asks Lanes.

## Two transports, one port

| | Use it for |
| --- | --- |
| **WebSocket** `ws://127.0.0.1:<port>/` | The primary interface. Bidirectional, with state pushed as it changes |
| **HTTP POST** `http://127.0.0.1:<port>/` | One-shot commands, so a Stream Deck button can be a plain HTTP call |

An HTTP `GET` with no body returns full state, which makes the server pleasant to poke at with a browser while developing.

## Requests

```json
{
  "id": 42,
  "token": "…",
  "command": "set_channel_volume",
  "channel": "media",
  "value": 0.4
}
```

`id` is optional and echoed back so replies can be correlated. `command` selects the operation; its parameters sit alongside it at the top level.

## Replies

Every reply carries **the resulting state**, so a client never has to guess whether a command worked or make a second round trip to find out:

```json
{ "id": 42, "ok": true, "state": { … } }
```

On failure:

```json
{
  "id": 42,
  "ok": false,
  "error": { "code": "not_found", "message": "no channel called 'nope'" }
}
```

| Code | Meaning |
| --- | --- |
| `unauthorised` | Missing or wrong token |
| `bad_request` | Malformed JSON, or a command this build does not know |
| `not_found` | Named a channel, device or app that does not exist |
| `not_implemented` | A command reserved but not built (none in 1.0) |
| `audio_error` | Windows refused, or per-app routing is unavailable |

## Commands

| Command | Parameters | Notes |
| --- | --- | --- |
| `get_state` | — | Full state. Also pushed automatically on connect |
| `set_channel_volume` | `channel`, `value` (0–1) | Sets `volume`. Master's ceiling still applies on top. **For `master` this moves Windows' own volume slider** - see "Master is the Windows default output" |
| `adjust_channel_volume` | `channel`, `delta` | Relative, clamped to 0–1 |
| `set_channel_mute` | `channel`, `muted` | |
| `toggle_channel_mute` | `channel` | |
| `set_device_priority` | `entries` | The global fallback order: `[{ "id", "enabled" }]`, in priority order. See below |
| `set_channel_device` | `channel`, `device_id` (or `null`) | `null` returns it to the system default. **For `master` it makes that device Windows' default output**; `null` does nothing, since Master already is the default |
| `cycle_channel_device` | `channel` | Steps through outputs, including "system default" as a position. Refused for the Mic channel, as `set_channel_device` is |
| `assign_app` | `executable`, `channel` | Creates a persistent rule |
| `unassign_app` | `executable` | Returns the app to "To be routed" |
| `set_chat_mix` | `value` (−1 to 1) | Game/Chat balance. 0 is centre — see below |
| `adjust_chat_mix` | `delta` | Relative, clamped to −1…1 |
| `set_chat_mix_channels` | `game`, `chat` | Which two channels the mix balances; `game` is the side a positive value favours. The mix returns to centre. Refused for the same channel twice, Master, an input or an unknown id |
| `mute_all` | `muted` | Mutes **Master**, not each channel — see below |
| `restore_windows_settings` | — | Puts Windows audio back as it was before Lanes and **pauses** Lanes. See "Restore and pausing" |
| `resume` | — | Stops being paused: channels apply again, at once. Harmless when not paused |
| `set_channel_collapsed` | `channel`, `collapsed` | Display state. Touches no audio setting — see below |
| `set_window_placement` | `x`, `y`, `width`, `height`, `maximised` | Where the mixer window was left. Physical pixels - see below |
| `subscribe_meters` | `enabled` | Per-connection |
| `activate_profile` | `name` | Switch to a saved profile. Case-insensitive. `not_found` if there is none |
| `save_profile` | `name` | Save the mixer as it is now. An existing name is **replaced**. `bad_request` for an empty, over-long (40) or control-character name |
| `delete_profile` | `name` | Remove a saved profile. Changes no audio setting |
| `set_hotkeys` | `bindings` | Replace every global hotkey. See below |
| `rename_channel` | `channel`, `name` | Change a channel's display name. Its id never changes |
| `ignore_app` | `executable` | Never manage or show it. Its rule is kept; its audio is left where it is |
| `unignore_app` | `executable` | Manage it again; its rule applies at once. `not_found` if it was not ignored |
| `set_app_trim` | `executable`, `trim` | Its level within its channel, 0-1; `null` follows the channel. Needs a rule |
| `set_notify_new_apps` | `enabled` | Show a notice when an unassigned app starts playing. On by default for new installs; held back while the user is in a full-screen game or presentation |
| `set_theme` | `theme` | `"system"`, `"dark"` or `"light"`. Stored for the windows; touches no audio |
| `add_channel` | `name` | Add a playback channel before Mic. Seven channels at most. Its id is made from the name |
| `remove_channel` | `channel` | Remove the added channel. Refused for the six that ship. Its apps go back to "To be routed" without their audio moving; hotkeys naming it go |
| `get_diagnostics` | — | The reply carries `diagnostics`: every session and why, the last 40 changes, folders |
| `show_window` | — | Open the mixer window, or bring it forward. What a second launch of `Lanes.exe` sends |
| `quit` | — | Shut Lanes down cleanly. What `Lanes.exe --quit` and the uninstaller send |
| `export_config` | — | The reply carries `config`: everything but the API token and window placement |
| `import_config` | `config` | Replace the configuration with an export. See below |

### `adjust_channel_volume` matters more than it looks

On a Stream Deck **Neo** — keys, no dials — a relative adjustment is the *only* way volume can be controlled. A button does not know the current value, so it cannot compute an absolute one. Prefer this over `set_channel_volume` for anything driven by a key.

### `set_chat_mix` attenuates one side only

One slider across two channels, so game audio can be traded against voice chat without reaching for two faders mid-fight.

```text
value  0.0     both channels untouched
value +0.6     chat runs at 40% of its own level, game untouched
value -0.25    game runs at 75% of its own level, chat untouched
```

Positive favours Game, negative favours Chat, and the factor applied to the far channel is `1 - |value|`. It **multiplies** with master and the channel's own volume rather than replacing either.

Three properties worth relying on:

- **Centre is a true no-op.** A client can return the slider to 0.0 and be certain the sound is exactly where it was. A design that redistributed a fixed total between the two would change both channels at centre the moment their faders differed.
- **Each channel's own fader keeps its meaning.** Chat at 70% is still Chat at 70% — of whatever the mix leaves it. Two controls both claiming to set "the chat volume" would be a UI where neither number can be trusted.
- **Nothing ever gets louder than its fader.** The favoured side is never boosted, for the same reason volume is applied as a scalar: this is a manager for Windows' own per-app volumes and must never exceed what the user set.

The state reports which two channels the control spans, rather than assuming Game and Chat, because channels are renameable and a user may keep voice chat on Aux. `set_chat_mix_channels` changes them. It puts the mix back to centre, because centre attenuates nothing: keeping the old position would turn a channel down the moment it was named.

### Profiles

A profile is every channel's **volume, mute and device**, plus the Game/Chat
mix, saved under a name. It deliberately does **not** hold the rules (which
app is on which channel), the device fallback order, or display state: those
are facts about the applications and the hardware, not about the evening.

**`profiles.active` is the profile the mixer currently matches, not the one
last chosen.** Switch to "Gaming" and it reads `"Gaming"`; move any fader
Gaming holds and it becomes `null`; move the fader back and it is `"Gaming"`
again. That is what lets a Stream Deck key or a menu tick be trusted without
the client keeping its own idea of what is on. When two profiles hold
identical settings, the one saved or switched to most recently is reported.

A profile saved before a channel existed leaves that channel alone when
switched to, and ignores it when deciding whether it is active.

### Hotkeys

```json
{ "command": "set_hotkeys",
  "bindings": [
    { "keys": "ctrl+alt+m", "action": { "kind": "toggle_mute", "channel": "chat" } },
    { "keys": "Ctrl+Alt+F10", "action": { "kind": "quick_mixer" } }
  ] }
```

`kind` is one of `toggle_mute`, `volume_up`, `volume_down`, `cycle_device`
(these four take a `channel`), `activate_profile` (takes a `name`), `mute_all`,
`show_window` (open or close the mixer) and `quick_mixer` (open or close the
flyout). Volume keys move 5% a press.

**The whole list is replaced, and refused as a whole** if any binding's keys
cannot be read, two share keys, or one names a channel that does not exist -
with the reason, such as `Shift+M needs Ctrl, Alt or Win with it`. Keys are
stored normalised, modifiers first: send `ctrl+alt+m`, get `Ctrl+Alt+M`.

**Every binding needs Ctrl, Alt or Win**, except F13 to F24 alone. A global
hotkey takes its keys from every other program, so a plain key would stop
being typeable anywhere while Lanes runs.

In the state, each binding carries a `status`:

| `status` | Meaning |
| --- | --- |
| `active` | Registered with Windows and working |
| `in_use` | Another program holds these keys; pressing them does nothing here |
| `failed` | Windows refused for another reason, given in `problem` |
| `pending` | Not registered yet - or there is no tray (`--serve`), which registers none |

The tray registers them, so a core run with `--serve` saves them but never
makes them live.

### Export and import

`export_config` answers with the whole configuration under `config`, **minus
the API token** - the one secret, which must not travel in a file - **and the
window's placement**, which describes this machine's monitors.

`import_config` takes such a file's contents. It keeps this machine's token,
window placement and start-with-Windows setting, and replaces everything else.
Before changing anything it **snapshots the audio** (so Restore can undo what
the import applies) and **copies the current config to
`config.json.before-import`**; if either fails, nothing is imported. A file from
a newer version of Lanes, or one with no channels, is refused.

### Master is the Windows default output

Master is not a number Lanes keeps to itself:

- **Its device is Windows' default output.** `set_channel_device` on `master`
  makes that device the default (the "Set Default" of the Sound control panel:
  the console and multimedia roles, not communications); `cycle_channel_device`
  on `master` moves the default to the next output. When the default changes
  anywhere else, Master's device follows, in a delta.
- **Its volume is that device's own slider** - the taskbar's - the same number
  both ways. Setting Master moves the slider; moving the slider moves Master,
  and every client hears it as a delta.
- **Its mute is that device's mute**, and still mutes every channel.
- **It is still a ceiling.** `effective_volume` on each channel is still
  `min(volume, master)`. Windows applies the slider to every app on that device,
  so Lanes sets those apps' own levels to come out at the ceiling rather than
  be cut twice; apps on other devices are capped directly. The ceiling is
  applied at what the slider does to the signal, which follows Windows' volume
  curve, so a channel set a little below Master's number may be trimmed
  slightly.
- Apps in the Master channel are not routed: they follow the default, which is
  Master's device.

Before Lanes first changes the default device or a device's volume, it records
the old value in `snapshots\windows-before.json`, and Restore puts it back.

### Restore and pausing

`restore_windows_settings` means *before Lanes*: every application to 100%, unmuted, following the default device; the default output
and each device's volume and mute as recorded before Lanes first changed them.
Then `settings.paused` is `true` and Lanes applies no channel until `resume`,
or the channels would put everything straight back. Applications that were not
running are reset the first time they play while paused. A client should show
that Lanes is paused and offer `resume`.

### `mute_all` mutes Master

Not every channel individually. Muting each channel would destroy the user's per-channel mute states, and unmuting afterwards could not restore them.

### Idempotence

Every command that can be idempotent is. Setting a volume to the value it already holds succeeds, changes nothing, and writes nothing to the audit log. This matters for a Stream Deck, whose buttons are pressed optimistically and whose view of state may briefly lag the core's.

## Events

Pushed by the server, unprompted.

### `welcome`

First message on every connection.

```json
{ "event": "welcome", "protocol_version": 1, "authenticated": true }
```

Check `protocol_version` and refuse to continue if you do not understand it, rather than misinterpreting later messages.

### `state`

Full state, on connect and on `get_state`.

### `delta`

Only what changed. **Sections that did not change are omitted entirely** — a delta containing everything would just be a full push under another name, and would make an idle core chatter at every client.

```json
{ "event": "delta", "channels": [ … ] }
```

The sections a delta can carry are `channels`, `sessions`, `devices`,
`device_priority`, `profiles`, `chat_mix`, `hotkeys`, `ignored` and
`settings`. `hotkeys` arrives in a delta when the tray reports which keys it
registered.

### `meters`

Per-channel levels, 0–1, about thirty times a second. Only while subscribed.

```json
{ "event": "meters", "levels": [ { "channel": "media", "level": 0.0473 } ] }
```

A channel's level is the **loudest** session in it, not the average — the question a meter answers is "is this channel making noise", and averaging lets one quiet app drag the needle down on a channel that is plainly audible.

**Levels are linear peak values, not decibels.** That is the honest number, but it is not what a meter should draw: normal speech peaks around 0.1–0.3 linear, which renders as a bar that barely leaves the floor and looks broken. Apply a logarithmic curve in the client, where the choice of scale belongs.

**Input channels are metered from a capture stream**, because the obvious alternative does not work: `IAudioMeterInformation` on a capture endpoint reads a flat zero unless some other application already has the microphone open, since Windows runs the capture path on demand. So while a client is subscribed to meters and the configuration has an input channel, the core **holds the microphone open**, and Windows shows its microphone-in-use indicator. The signal is measured and discarded — never recorded, stored or forwarded — and the stream is closed as soon as the last subscriber goes away. Unsubscribing, or simply disconnecting, is enough.

**Metering is the only thing in the core that needs a timer.** It runs only while a client has asked for it, and stops when the last subscriber disconnects, so an idle Stream Deck causes no metering work at all.

## State

```json
{
  "channels": [ {
    "id": "media", "name": "Media",
    "volume": 1.0, "effective_volume": 0.75, "muted": false, "is_input": false,
    "target_device":    { "id": "{0.0.0…}", "name": "Speakers (Realtek(R) Audio)" },
    "effective_device": { "id": "{0.0.0…}", "name": "Speakers (Realtek(R) Audio)" },
    "on_fallback": false, "collapsed": true
  } ],
  "sessions": [ {
    "executable": "brave.exe", "display_name": "brave.exe",
    "channel": null, "playing": true,
    "controllable": true, "session_count": 2, "routing_refused": false,
    "path": "C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe"
  } ],
  "devices": [ {
    "id": "{0.0.0…}", "name": "Speakers (Realtek(R) Audio)",
    "direction": "output", "present": true, "is_default": false
  } ],
  "profiles": { "names": [], "active": null },
  "chat_mix": { "value": 0.0, "game": "game", "chat": "chat" },
  "settings": { "notify_new_apps": true, "theme": "system", "paused": false, "close_to_tray": true },
  "routing_available": true
}
```

Things worth knowing:

- **`volume` is what the user set; `effective_volume` is what the channel runs at.** Master is a **ceiling** over every playback channel rather than a scalar on it: a channel set to 100% with Master at 75% runs at 75% and still remembers it was set to 100%, so raising Master restores it. **Show `effective_volume` on a fader; send `volume` when setting one.** They are equal for Master itself and for input channels.
- **`channel: null`** means the app is in the "To be routed" pool. It is playing at its own volume, outside the user's control — worth surfacing prominently.
- **`path`** is the executable's full path, so a client can show the application's own icon. Sessions are grouped by it, so one entry has one path.
- **`routing_refused`** is true when the application did not keep the output device Lanes gave it - something changed it straight back - so Lanes has stopped moving it rather than fight. Its volume and mute are still applied. Assigning it to a channel again gives it one more try.
- **`settings.paused`** is true after a Restore, until `resume`. See "Restore and pausing".
- **`settings.close_to_tray`** false means a client that closes should quit Lanes (`quit`); the mixer does.
- **`settings.theme`** is `system`, `dark` or `light`. The core stores it only because windows are destroyed when they close; `system` means "follow Windows' app mode", which the client reads for itself.
- **`session_count`** is how many audio sessions the executable owns. Browsers and Electron apps have several; they are grouped into one entry here so every client does not have to do it.
- **`on_fallback`** is explicit rather than something you infer by comparing the two device fields. A channel silently playing out of the wrong speakers is the most annoying possible failure, so it is stated.
- **`window`** is where the mixer window was last closed, and is absent until it has been closed once. **The numbers are physical pixels in virtual-screen coordinates**, which is what Win32 window placement deals in — deliberately not logical units, because a logical size means nothing without also saying which monitor's scaling it was logical in. A client restoring it should check the rectangle still overlaps a display and fall back to its default position if not; a laptop undocked between sessions is the ordinary way that happens, and a window restored onto a monitor that is gone cannot be reached with a mouse.
- **`collapsed`** is whether a client should show that channel's application list closed. It is the one piece of pure display state the core stores, and it is here for a specific reason: it is remembered between sessions, and the mixer window is *destroyed* every time it closes, so it cannot remember anything itself. Storing it centrally also means two clients agree about the same channel. `set_channel_collapsed` saves the config and runs no audio call.
- **`chat_mix.game` and `chat_mix.chat`** are channel **ids**, so a client can label the ends of the slider with those channels' current names. Do not assume they are `"game"` and `"chat"`.
- **`routing_available: false`** means per-app device routing is unavailable on this Windows build. The app still works as a volume mixer; show a banner and hide device controls rather than failing silently.

## Honest gaps

- **`controllable` is always `true`.** Detection of exclusive-mode sessions is not built; see [limitations.md](limitations.md). Apps that refuse routing are reported separately, by `routing_refused`.
- **Unplugged devices are only the ones the configuration remembers.** `present: false` entries come from what the config points at, not from every endpoint Windows has ever seen - see "Devices that are not plugged in" below.

## Example

```bash
PORT=$(cat "$LOCALAPPDATA/Lanes/port")
TOKEN=$(python -c "import json,os;print(json.load(open(os.environ['LOCALAPPDATA']+r'\Lanes\config.json'))['api_token'])")

curl -X POST "http://127.0.0.1:$PORT/" \
  -d "{\"token\":\"$TOKEN\",\"command\":\"adjust_channel_volume\",\"channel\":\"media\",\"delta\":-0.1}"
```

## Implementation note: no async runtime

The server uses blocking `tungstenite` on dedicated threads rather than `tokio`, and the reason is COM.

Every Windows audio call must happen on the thread that initialised the COM apartment, and we use a single-threaded apartment deliberately — the undocumented routing interface is sensitive to apartment model. An async runtime that moves futures between worker threads would violate that, and the failure mode is not a clean error.

So **one thread owns COM and all state**, and connections talk to it over channels. With one UI window and one Stream Deck plugin there is no concurrency pressure an async runtime would relieve, and this makes the threading rule impossible to break by accident rather than merely documented.

## Device fallback order

When a channel's own device is unplugged, it goes to the first **enabled,
plugged-in** device in one global list, then to the system default. The list is
in the state as `device_priority`, and in deltas whenever it changes:

```json
"device_priority": [
  { "id": "{0.0.0.00000000}.{6f1c…}", "name": "Headphones (USB Audio)",      "enabled": true,  "present": true },
  { "id": "{0.0.0.00000000}.{9d8e…}", "name": "Speakers (Realtek(R) Audio)", "enabled": true,  "present": true }
]
```

`set_device_priority` sends the whole order back:

```json
{ "command": "set_device_priority",
  "entries": [ { "id": "{…9d8e…}", "enabled": true },
               { "id": "{…6f1c…}", "enabled": false } ] }
```

It is deliberately forgiving:

- **Entries not mentioned are kept**, after the ones that are, so a client
  working from a stale list cannot delete a device by leaving it out.
- **Unknown ids are ignored.** It reorders devices; it does not invent them.
- **Disabled entries always end up at the bottom**, whatever order they arrive
  in. Disabled means "never a fallback" - a channel pointed at a disabled device
  directly still uses it.

The core maintains the list itself: every output device present at startup, or
plugged in later, is added at the bottom of the enabled entries, and nothing is
ever removed - an unplugged device keeps its place and reports `present: false`.

### What a client can rely on

- **`effective_device` is what the core actually applied**, not a guess. The
  same function decides what the engine writes and what this reports.
- **`on_fallback` is explicit.** Do not infer it by comparing `target_device`
  with `effective_device`: a channel with no preferred device follows the system
  default and is **not** on a fallback, because that is what was asked for.
- **The preferred device is never overwritten while it is absent.**

### Devices that are not plugged in

`devices` includes entries with `present: false` for any device the
configuration still refers to - a channel's device, or an entry in the fallback
order. They are safe to offer in a picker.

It does **not** include every endpoint Windows has ever seen. A PC that has been
in use for a while typically has dozens of render endpoints of which two or
three are real, the rest being years of sediment. A device that is unplugged and has never been plugged in
while Lanes was running cannot be chosen; plugging it in once is enough.
