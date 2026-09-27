# The window

`Lanes.Window.exe` - C# 13 on .NET 10, WPF - is everything you see: the mixer,
the tray's quick mixer, Settings, and the new-app notice. It is a **pure client
of the local API** ([api.md](api.md)). It holds no authoritative state and never
calls Windows audio; everything it shows arrives from the core, and everything
it does is a command to the core. Its source is `ui-wpf/`.

---

## One executable, four windows

`App.xaml.cs` picks the window from the command line:

| Started with | Window | Started by |
| --- | --- | --- |
| (nothing) | `MainWindow` - the mixer | Start Menu, taskbar pin, tray "Open", double-click on the tray icon |
| `--flyout` | `FlyoutWindow` - the quick mixer | A left click on the tray icon |
| `--settings` | `SettingsWindow` | Tray menu > Settings > Open settings... |
| `--arrival <exe>` | `ArrivalWindow` - the new-app notice | The core, when an unassigned app starts playing |

Each is its own process and **exits when its window closes**, so its memory is
returned rather than kept in a hidden window. The core keeps running in the
tray. The core starts these windows (`lifecycle::spawn_window` in the core) and
keeps each process handle so it never opens two of the same.

---

## Talking to the core

| File | Job |
| --- | --- |
| `Api/CoreClient.cs` | The whole connection: finds the port and token (honouring portable mode), keeps a WebSocket open and reconnects on its own, merges deltas into the last full state, and raises events on the UI thread. Sends are queued **in order, one at a time**, and each is tracked until the core replies. |
| `Api/Models.cs` | The API's state as C# records, deserialised by name. |
| `Api/CoreProcess.cs` | Starts the core when none is running (so a window pinned to the taskbar still works), and sets the taskbar identity that groups the window with the core. |

Two rules that are easy to break:

- **Wait for replies before exiting.** A window that sends a command and then
  closes must not exit until the core has answered: Lanes keeps pushing updates,
  so there is always unread data in the socket, and a process exiting with
  unread data makes Windows *reset* the connection - discarding the command.
  `App.OnExit` calls `CoreClient.FlushAll`, which waits up to 0.6 s.
- **Update in place; never rebuild what the user may be holding.** State arrives
  up to thirty times a second while meters run. Controls are created when the
  *set* of channels or apps changes and are updated in place otherwise. A strip
  or chip rebuilt on every push would be destroyed mid-drag.

---

## The mixer

| File | Job |
| --- | --- |
| `MainWindow.xaml(.cs)` | The mixer: the header band (Game/Chat mix, profile, audio override), the channel strips, "To be routed", "Audio devices", the paused and routing banners, drag and drop, rule confirmations. |
| `Controls/ChannelStrip.cs` | One channel: name, fader with meter, mute, device button, apps list, the "⋯" menu. The same control in every layout, and in the quick mixer (`Minimal`). |
| `Controls/Fader.cs` | The fader, drawn by hand: meter behind the track, detents with magnetism, Master's ceiling as a dashed line. |
| `Controls/Detents.cs`, `CapPaint.cs` | The detent scale and the handle's paint, shared by the faders and the mix so they behave identically. |
| `Controls/ChatMixSlider.cs` | The Game/Chat balance. |
| `Controls/AppChip.cs`, `AppIcons.cs` | An application: its icon, name, playing state, right-click menu, Ctrl multi-select, drag. |
| `Controls/PoolPanel.cs` | "To be routed". |
| `Controls/DevicePanel.cs`, `DeviceButton.cs` | The global fallback order, and a channel's device picker. |
| `Controls/ProfileButton.cs` | Switch, save and delete profiles. |
| `Controls/PopupMenu.cs`, `ChoiceButton.cs`, `Toggle.cs`, `KeyCapture.cs`, `Icons.cs`, `Tree.cs` | Small shared pieces. `KeyCapture` records a hotkey by being pressed, holding a low-level keyboard hook only while it is listening. |

### The fader holds its own value while it is held

Every drag step sends a command, and the core echoes the resulting state back.
A fader that simply displayed the echo would fight the hand holding it. So while
the pointer is down, a fader shows *its own* value and ignores what arrives; after
release it keeps its value until an echo agrees (or two seconds pass). Commands
are rationed to one per 45 ms while dragging, plus the final value, so the core
never falls behind.

**Detents** pull the handle to 0, 25, 50, 75 and 100% on release - unless the
handle was held still for 350 ms first, which means the user found the value
they wanted. Clicking a detent's paddle jumps straight to it.

### Meters

Levels arrive as linear peaks about thirty times a second. The fader eases
towards each one (instant rise, 0.18 s fall), draws on a logarithmic scale with
a -60 dB floor - speech peaks around 0.1-0.3 linear, which would barely leave
the floor drawn straight - and redraws nothing once the drawn level matches.
Meters run only while the mixer is open (`subscribe_meters`).

---

## Layout

Three layouts, chosen from the window's size by `Layout.cs`:

| Layout | When | What changes |
| --- | --- | --- |
| **Full** | 900 logical px wide and up | "To be routed" and "Audio devices" as a column on the left; strips across the rest |
| **Compact** | 560-900 | Both side panels become buttons in the header band, opening as overlays; device buttons drop to an icon |
| **Stacked** | below 560, or portrait below 1000 wide | Channels become full-width rows; faders lie horizontal |

Each boundary has **hysteresis**: a width at which the layout gives way and a
wider one (48 px more) at which it comes back, so resting the window edge on a
boundary cannot make it flicker.

The stacked layout's horizontal fader is the vertical one **rotated 90° with a
`LayoutTransform`**. WPF measures the rotated box and delivers mouse input in the
control's own coordinates, so no fader code knows about the rotation.

### The window fits rather than scrolls

`Controls/FitWindow.cs` scales the whole mixer down uniformly, with a render
transform, until it fits: a mixer is a dashboard, and a channel you have to
scroll to is one you have stopped watching. Content is laid out at the width it
will have *after* scaling, so shrinking makes faders shorter, not names more
truncated. Below 0.7 the scale stops and a scrollbar appears, because at some
size scrolling beats illegible text.

### Monitors and DPI

The window is per-monitor DPI aware (v2, `app.manifest`). WPF rescales the
content when it crosses to a display with different scaling, but not the
window's frame - `DpiFollow.cs` applies Windows' suggested size once the drag
ends. `Placement.cs` saves the window's position in physical pixels with Win32
`GetWindowPlacement`, and on the next start refuses a position that is no longer
on any display.

---

## Look

The visual direction is "Quiet": no panels, whitespace grouping, type-led
hierarchy, one muted accent per channel used only on the strip's top band and
the meter.

- **Every colour is in `Theme.xaml`** (dark) and **`ThemeLight.xaml`** (light,
  merged over it). A hex literal anywhere else is a bug.
- `Themes.cs` follows Windows' app mode unless the user chose a theme. **A theme
  change rebuilds the open window** rather than recolouring it: colours are
  looked up once when each control is made.
- `TitleBar.cs` makes the title bar dark or light to match.
- **Nothing expands itself.** Apps lists and side panels start collapsed and open
  only when the user opens them; an unrouted app is signalled by an amber count,
  not by rearranging the window.

---

## Building and running it on its own

```powershell
dotnet build ui-wpf -c Release
```

Run `Lanes.Window.exe` beside a `Lanes.exe`: with no core running, it starts one.
For working on the window beside an installed Lanes, see
[building.md](building.md).
