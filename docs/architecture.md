# Architecture

How Lanes is put together, and **why it is shaped this way**. For every command
and state field see [api.md](api.md); for the config file,
[configuration.md](configuration.md); for the window, [window.md](window.md);
for the undocumented Windows interfaces, [interop.md](interop.md).

---

## The one idea everything follows from

**Lanes never touches the audio signal.** It manages Windows' own
per-application settings - each app's volume, mute and output device - with a
mixer on top. The audio goes `app → Windows audio engine → hardware` exactly as
it would without Lanes, which is why there is no driver, no virtual device, no
added latency and no elevation. Every other decision below is downstream of that
one.

The cost is that Lanes can only do what Windows' per-application settings can
do. It cannot mix, process or monitor audio, and it cannot control applications
that bypass the audio engine (see [limitations.md](limitations.md)).

---

## The programs

```
                 ┌─────────────────────────────────────────────┐
                 │ Lanes.exe - the core                  (Rust) │
                 │                                             │
  Windows  ◄─────┤  engine ─ routing ─ rules ─ config.json     │
  audio          │  watchers (sessions, devices, volume)       │
  (Core Audio,   │  snapshot / restore / audit log             │
   undocumented  │  tray icon, hotkeys, start-on-boot          │
   routing API)  │  local API on 127.0.0.1  ◄────────┐         │
                 └───────────────────────────────────┼─────────┘
                                                     │ WebSocket / HTTP
                          ┌──────────────────────────┼─────────────────────┐
                          │                          │                     │
          ┌───────────────▼──────────┐   ┌───────────▼──────────┐  ┌───────▼──────┐
          │ Lanes.Window.exe (C#/WPF)│   │ Stream Deck plugin    │  │ curl, scripts│
          │ mixer, quick mixer,      │   │ (Node, inside Stream  │  │              │
          │ settings, new-app notice │   │  Deck; own repository)│  │              │
          └──────────────────────────┘   └───────────────────────┘  └──────────────┘
```

- **The core** runs from sign-in to quit, lives in the tray, and is the only
  thing that talks to Windows audio or writes the config.
- **The window** is started on demand, one process per window, and **exits when
  it closes**, so its memory goes back. It is a pure client of the API.
- **The [Stream Deck plugin](https://github.com/MAYBE-33/lanes-streamdeck)** is
  another client with the same standing, in its own repository.

### Why the window is a separate program

The window is purely a client of the core over the local API, with no
privileged knowledge. Inside one process that is a rule kept by discipline;
**across a process boundary it is a fact**. The window cannot reach the core's
state even by accident, so anything it needs has to be added to the API - which
is exactly what made the Stream Deck plugin cheap: it is a second client of an
interface that was already complete.

It also means the window's toolkit can be replaced without touching the audio
code, and that the part which runs all day - the core, a few MB on disk and
about 20 MB in memory - stays small, while the window, which carries .NET
inside it, only costs memory while it is open.

---

## Inside the core

### The crates

| Crate | What it is |
| --- | --- |
| `winaudio/` | Windows audio, wrapped just enough: devices, sessions and per-session volume, a device's own volume (`endpoint`), the microphone level (`capture`), COM apartment lifetime (`com`) - and the two undocumented interfaces, **`policy`** (per-app routing) and **`default_device`**. See [interop.md](interop.md). |
| `core/` | The application: everything else. Builds `Lanes.exe`. |

### The modules

| Module | Job |
| --- | --- |
| `config` | Channels, rules, settings, profiles, hotkeys - and the file they live in. Written atomically; a corrupt file is set aside, a locked one stops startup rather than being overwritten. |
| `rules` | Which channel an application belongs in, from its executable name or path. Most specific rule wins. |
| `routing` | **The only answer to "which device is this channel on"**: its own device if present, else the first enabled present device in the global fallback order, else the system default. The engine applies what it returns and the API reports what it returns, so they cannot disagree. |
| `master`, `endpointwatch` | **Master is Windows' default output**: its device, its own volume slider and its mute, kept in step both ways. `master::push` writes what a Lanes command changed; `master::pull` brings in what changed in Windows, which `endpointwatch` hears about. |
| `engine` | Applying a channel to live sessions: level, mute, device. Every write is preceded by a read and skipped if nothing would change. |
| `watcher`, `devicewatch` | Windows' notifications for new sessions and for devices appearing and vanishing. Event-driven; there is no polling anywhere in the core. |
| `api::server` | The core loop and the local API (see below). |
| `api::state` | The state clients see, built from the config and the audio graph. |
| `api::protocol` | The API contract, as Rust types. |
| `snapshot`, `restore`, `audit` | The safety layer (see below). |
| `profiles`, `manage` | Profiles; edits bigger than one field (rename, add or remove a channel, import). |
| `tray`, `lifecycle`, `hotkeys`, `autostart`, `instance`, `shutdown`, `console` | Being a tray application: the icon and menu, hotkeys, start-on-boot, one instance at a time, a clean exit, and a command line that works from a desktop program. |

### Threads, and the single owner

```
main thread ─── tray: menu, icon, hotkeys (RegisterHotKey needs a message loop)
                  │ TrayCommand
                  ▼
core thread ─── core_loop: the ONLY thread that owns the config and talks to
                Windows audio. Everything arrives here as a CoreEvent:
                  API requests · new sessions · device changes ·
                  Master's volume changing · tray commands · meter ticks
                  ▲
accept thread ─ one thread per API connection (WebSocket or HTTP)
COM callbacks ─ Windows' notification threads, forwarded straight into the queue
meter ticker ── exists only while a client has asked for meters
```

**One thread owns the config.** Every source of change - a click, a Stream Deck
key, a hotkey, a new app starting, a device unplugged - becomes an event on one
queue, handled in order. Two independent load-modify-save cycles racing each
other, with the loser silently dropping the user's change, cannot happen.

**COM is apartment-threaded and stays on that thread.** That is also why the
API uses blocking sockets on their own threads rather than an async runtime: an
executor that moves work between threads would break the COM rule in a way
that fails silently. Interfaces are dropped before `CoUninitialize`; see
[interop.md](interop.md).

---

## What happens when…

### …a key or a fader changes a volume

1. The client sends `adjust_channel_volume` (Stream Deck) or
   `set_channel_volume` (window) with the token.
2. The core loop updates the config, saves it, and runs
   `engine::apply_levels_only` for that channel: one sweep of the audio graph,
   levels only, because a volume change cannot have moved a device. (A full
   apply costs roughly 100 ms; this path is a few, which is what lets a fader
   follow a hand.)
3. The reply carries the whole resulting state; every other client gets a
   delta with the sections that changed.

### …Master changes

`set_channel_volume` on `master` also moves Windows' own volume slider for the
default device (`master::push`). Moving that slider in Windows raises a
callback (`endpointwatch`), and `master::pull` copies it into the config; Lanes'
own writes carry a marker so their echo is ignored. Because Windows applies the
slider to every app on that device, the engine sets those apps' own levels to
compensate (`engine::level_for`), so the channel ceiling is not applied twice.

### …an application starts playing

1. Windows raises a new-session notification; the watcher puts it on the queue.
2. `engine::apply_to_pid` resolves the app's rule. Unassigned apps are left
   exactly as they are and appear in "To be routed" (and, if enabled, the
   new-app notice opens).
3. For an assigned app: **levels first, then the device**, then levels again
   300 ms later. Changing an app's device makes Windows create a new session at
   the app's remembered volume, so levels applied only before the move would
   land on a session that is about to be abandoned.
4. Then the route is **read back**. If something changed it straight back - the
   app itself, or another tool - Lanes stops moving that app and flags it
   (`routing_refused`) rather than fight.

### …a device is unplugged

`devicewatch` reports it; the core re-registers session notifications on the
devices that exist now and runs a full apply. `routing::choose` sends each
channel that was on the missing device to its fallback, and the state marks it
`on_fallback`, which the window and the Stream Deck show in amber. Plugging it
back in reverses it the same way.

---

## The safety layer

Lanes changes settings that **outlive it**: Windows remembers an app's volume and
device against the app, not against Lanes, so uninstalling Lanes undoes nothing.

- **Snapshot** (`snapshot`): on first run, before Lanes changes anything, it
  records every app's volume, mute and device and the system default.
  `first-run.json` is written once and never overwritten. Before Lanes first
  changes Windows' default device or a device's volume, the old value goes to
  `windows-before.json` (`master::Before`).
- **Restore** (`restore`; tray menu, Settings, `Lanes.exe --restore`): puts
  Windows back as it was before Lanes - every app to 100%, unmuted, following
  the default device; the default output and device volumes as recorded - and
  then **pauses** Lanes, resetting apps that were not running as they next play.
  One application at a time, never by process id, and never with the global
  `ClearAllPersistedApplicationDefaultEndpoints`.
- **Audit log** (`audit`): every change, with its old and new value and why,
  one JSON line each, a file per day. Failures are logged as carefully as
  successes.

---

## Security and privacy

- The API binds **127.0.0.1 only**. Commands need a **token** from the config,
  which only the user can read.
- Requests from **web pages are refused** (any `Origin` header, or a `Host`
  other than loopback), because a browser would otherwise let any open tab read
  the state pushed to every new WebSocket.
- The only registry write is the user's own start-on-boot entry under `HKCU`.
  Nothing runs elevated, and the installer is per-user.
- **Lanes sends nothing anywhere.** It makes no network connections beyond its
  own loopback API, has no telemetry and no update check. The microphone meter
  reads the signal level and discards the audio; nothing is recorded.

---

## Working on Lanes beside an installed copy

A **portable** build (`portable.txt` beside it) keeps its state beside its own
executable. Started with `LANES_SEPARATE_INSTANCE=1` it takes its own
single-instance lock and runs beside an installed Lanes - and then it is
**hands-off**: it changes only the executables named in `test-apps.txt` beside
it, never Windows' default device or volumes, and every window and its tray
tooltip say "(test copy)". Two copies managing the same application would undo
each other forever. See [building.md](building.md).

---

## Where to start when something breaks

| Symptom | Look at |
| --- | --- |
| Routing silently does nothing after a Windows update | [interop.md](interop.md) and `winaudio/src/policy.rs` |
| An app ends up on the wrong device | `routing.rs` (the decision), then the audit log for who changed it |
| An app ends up at the wrong volume | `engine.rs`: the ordering note on `apply_to_group`, and `level_for` for apps on Master's device |
| Something keeps being changed back and forth | The audit log, per second - and whether another copy of Lanes or another audio tool is running |
| A client shows something the core does not believe | `api/state.rs` |
| The window misbehaves on a second monitor | `ui-wpf/DpiFollow.cs`, `ui-wpf/Placement.cs` |
| Anything that "reported success but nothing happened" | Read the value back. The commonest failure in this area is a success code with the wrong result |
