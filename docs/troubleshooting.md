# Troubleshooting

## The mixer opens but shows no devices and no applications

The window is a client of the core; if there is no core, there is nothing to
show. The window starts one itself when it cannot find one, so this should fix
itself within a few seconds of opening - give it that long before concluding
anything.

If it persists:

- Check that `Lanes.exe` sits **beside** `Lanes.Window.exe`. The window looks
  for the core next to itself and nowhere else, deliberately: searching the PATH
  could find a different copy.
- Open `%LOCALAPPDATA%\Lanes\logs\` and read the newest file. A core that
  started and then stopped will have said why.
- `Lanes.exe --status` from a terminal reports what the core can see.

**Pinning the window to the taskbar is fine.** It starts the whole
application either way.

## A channel's device button has turned amber

That channel's chosen device is **not plugged in**, and it is playing through a
stand-in: the first enabled, plugged-in device in the **Audio devices** list, or
the system default if none is. Hover over the button and it says which device
is missing.

It is not an error. Plug the device back in and the channel returns to it on its
own, and the button goes back to grey.

To change where channels fall back to, open **Audio devices** (below "To be
routed" in the wide layout, a button at the top in the narrow ones). Move
devices up and down to set the order; **Disable** keeps one out of fallback
altogether.

What happened, and when, is in the audit log — every device appearing or going,
and every channel that moved because of it, is recorded as `devices-changed`
whether or not any application needed moving:

```powershell
Select-String devices-changed "$env:LOCALAPPDATA\Lanes\logs\audit-*.jsonl"
```

## My audio is coming out of the wrong device

Put everything back as it was before Lanes:

```bash
Lanes.exe --restore
```

Every application goes back to **100%, unmuted, following Windows' default
device**, and Windows' own default output device and each device's volume go
back to how they were before Lanes first changed them. Applications that are not
running are reset the next time they play. Then **Lanes pauses** - your
channels are not applied, or they would put everything straight back - until
you choose **Resume Lanes** in the tray, the mixer or Settings. Your channels,
rules and profiles are all kept.

The same is in the tray menu (Settings > Restore Windows audio settings, which
asks first) and in the Settings window (General). If Lanes is running,
`--restore` asks it to do this; if not, it does it itself.

### Why restore is a command-line flag and not just a button

Because the thing you need to undo might be the reason the window won't open. A recovery option that requires a working UI isn't a recovery option.

### "N applications are not running"

Expected, and not a failure.

Windows only lets an application's audio settings be changed **while that application is running**. So Restore lists them, stays paused, and resets each one the first time it plays - Lanes keeps that list in `snapshots\restore-pending.json`. Resuming Lanes before they have played clears the list: from then on your channels apply to them again.

### Lanes is paused

It was told to restore Windows audio. Nothing is wrong: choose **Resume Lanes** in the tray menu (at the top while paused), in the banner across the mixer, or in Settings > General.

## Why did my settings survive uninstalling the app?

Because Windows stores them, not us.

When an application is assigned an output device, Windows persists that against the *application* — not the running process, and not Lanes. It survives the app closing, a reboot, and Lanes being deleted from your machine.

This is worth understanding, because it means **deleting Lanes does not undo what it did.** Run `--restore` first, then uninstall. [install.md](install.md) puts the steps in the right order.

## An app ignores its assigned device, or has an amber "!"

Some applications — games especially — manage their own output device and will override or revert whatever is assigned to them.

Lanes checks each route after setting it. When an application has changed it straight back, Lanes **stops moving it** rather than fight - two managers undoing each other would make the app's audio break up - and marks its chip with an amber **!**. Set the output device inside that application's own settings instead. Volume and mute still work normally. Assigning the app to a channel again, changing the channel's device, or restarting Lanes gives it one more try.

If *every* app keeps being moved back and forth, check that only one copy of Lanes is running, and that no other audio tool (EarTrumpet's per-app devices, a headset vendor's software) is managing the same apps.

## An app doesn't appear at all

Two likely reasons:

- **It isn't playing anything.** Applications only appear once they have an active audio session. Discord, for instance, creates no session until there's actual voice activity.
- **It uses WASAPI exclusive mode.** These bypass the Windows audio engine entirely and cannot be seen or controlled by any application, including Windows' own volume mixer.

## "Per-app device routing is unavailable on this Windows build"

Lanes uses an interface Microsoft has never documented, and it has changed between Windows releases.

If it stops working after a Windows update, volume and mute control will continue to work — only device routing is affected, and the app will say so rather than failing silently.

**Tray menu > Settings > Open settings... > Diagnostics** shows whether routing is available. If it is not, please open an issue with your Windows version (`winver`); [interop.md](interop.md) describes what has to be checked.

## A hotkey does nothing

Open **tray menu > Settings > Hotkeys...** and read the status beside it.

- **In use elsewhere** - another program registered those keys first, and
  Windows gives them to whoever asked first. Choose different keys, or close
  the other program. Lanes only tries again when the list changes or Lanes
  restarts, so after closing the other program, restart Lanes (tray > Settings
  > Quit Lanes, then start it again).
- **Not saved** - Lanes refused the list; the reason is shown above it.
- **Waiting for Lanes** - it has not been registered yet. If it stays that way,
  Lanes is probably running without its tray (`--serve`), which registers no
  hotkeys at all.

Pressing a hotkey while the settings window is recording keys records it rather
than running it. That is deliberate: it is how you rebind a key that is already
bound.

## The quick mixer

Left-click the tray icon for a small panel with every channel's fader and mute.
It closes when you click anywhere else, press Esc, or click the icon again.
Right-click the icon for the menu. Double-click opens the full mixer.

## Windows says "Windows protected your PC"

SmartScreen warns about programs it has not seen many people run, and every release starts at zero. Choose **More info** → **Run anyway**. It is not a sign that anything is wrong. See [install.md](install.md) for Smart App Control, which is stricter.

## Where everything lives

Everything is under one folder:

```
%LOCALAPPDATA%\Lanes\
  config.json           channels, rules and settings
  port                  the API's port number
  snapshots\
    first-run.json      state before Lanes ever ran — never overwritten
    windows-before.json Windows' default device and volumes before Lanes changed them
    snapshot-*.json     timestamped captures, most recent 20 kept
  logs\
    audit-YYYY-MM-DD.jsonl   every change made, with its previous value
```

Nothing is written anywhere else. No services, no scheduled tasks, no drivers, and the only registry value is the standard `Run` key for start-on-boot, under `HKCU`.

### Reading the audit log

One JSON object per line, so it can be read directly or queried:

```json
{"at":"2026-10-01T12:15:16+01:00","target":"Spotify.exe","field":"volume","old":"0.15","new":"1.00","reason":"restore","ok":true,"error":null}
```

`old` and `new` are always recorded, which is what makes it possible to reconstruct what happened. Failed attempts are logged too, with `ok: false` and the error — "we tried to move Discord and Windows refused" is exactly the sort of thing that cannot be reconstructed from a log that only records successes.

Changes that would have made no difference are not logged at all, so what remains is what actually happened.

## Working out why an app is where it is

**Tray menu > Settings > Open settings... > Diagnostics** lists every audio
session Windows reports, which channel it is in and the rule that put it there -
or that it is ignored, or waiting to be routed - and the last forty changes Lanes
made, with old and new values. "Open folder" goes to the settings, snapshots and
logs.

An app you ignored by mistake is under **Ignored apps** in the same window.

## Useful commands

| Command | What it does |
| --- | --- |
| `Lanes.exe --status` | Current devices, apps, volumes and routing |
| `Lanes.exe --snapshot "note"` | Capture state before you change something |
| `Lanes.exe --restore` | Put Windows audio back as it was before Lanes, and pause |
| `Lanes.exe --list-snapshots` | Show saved snapshots |
| `Lanes.exe --channels` | Channels, their settings and their apps |
| `Lanes.exe --unrouted` | Apps playing with no channel |
| `Lanes.exe --help` | Everything else |

`Lanes.exe` is a desktop program, so the output appears in the terminal you ran it from but a shell may not wait for it - the prompt can come back first. To capture the output, use PowerShell: `.\Lanes.exe --status | Out-File status.txt` works, while redirecting with `>` in `cmd` captures nothing.

## Starting from scratch

Close the app, then delete `%LOCALAPPDATA%\Lanes\`.

**Run `--restore` first if the app has changed anything**, for the reason above: deleting the folder destroys the snapshots, and without them there is no record of what your settings were.
