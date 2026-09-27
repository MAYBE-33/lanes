# Installing Lanes

For someone who just wants to use it. If you are building from source, see
[building.md](building.md).

---

## What you need

**Windows 10 version 1809 or later, 64-bit.** That is all.

There is nothing to install first — no .NET, no Visual C++ redistributable, no
drivers, no audio devices to set up. Both programs carry everything they need
inside themselves. This is checked by the build, not assumed: `build-release.ps1`
refuses to produce a release whose binaries depend on a redistributable.

**No administrator rights**, at any point. The installer puts the app in your
own user folder and touches nothing machine-wide. If something ever asks you to
elevate, that is a bug worth reporting.

---

## Installing

1. Download `Lanes-x.y.z-setup.exe` from the
   [Releases page](https://github.com/MAYBE-33/lanes/releases).
2. Double-click it.
3. **Windows will warn you** — see below.
4. Click through. It installs in a few seconds.

It goes to `%LOCALAPPDATA%\Programs\Lanes`, with a Start Menu entry and
an optional desktop shortcut.

The installer offers to **start Lanes when you sign in**. That setting
lives in the app afterwards, in the tray icon's menu, so you can change your
mind without reinstalling.

### The SmartScreen warning

On first run you may see:

> **Windows protected your PC** — Microsoft Defender SmartScreen prevented an
> unrecognised app from starting.

Click **More info**, then **Run anyway**.

This is not a sign of anything wrong. SmartScreen warns about any program it
has not yet seen many people run, and each new release starts again at zero, so
**it can come back after an update**. The warning fades as a release is
downloaded more.

### Smart App Control

If **Smart App Control** is turned on (Windows Security > App & browser
control), it blocks unsigned programs outright, with no "Run anyway". Releases
that are code-signed pass it; if the release you downloaded is not signed yet,
Smart App Control will refuse it. See [code-signing.md](code-signing.md) for how
Lanes' releases are signed.

---

## Using it

Double-click the Start Menu shortcut. The mixer window opens and a tray icon
appears.

- **Closing the window does not quit the app.** The core keeps running in the
  tray so your channels stay applied. That is the point of it.
- **To quit properly**, right-click the tray icon and use the Quit item in its
  Settings menu. This is deliberately not a one-click action, because quitting
  stops your audio routing.
- **Double-click the tray icon** to reopen the window. So does running the
  program again — from the Start Menu, the desktop, anywhere. It will not start
  a second copy.

Every application playing sound starts in **"To be routed"** on the left. Drag
it onto a channel to assign it, which creates a rule that survives restarts and
reboots. Nothing is moved that you did not ask to be moved.

A few more things, none of which you need on day one:

- **Left-click the tray icon** for the quick mixer: every channel's fader and
  mute in a small panel. Click anywhere else to close it.
- **Right-click an app** in the mixer to set how loud it is within its channel,
  take it out of its channel, or ignore it altogether. In "To be routed", the
  same menu puts it straight into a channel.
- **Ctrl-click several apps**, then drag any one of them, to move them all at
  once. A line at the foot of the window confirms the rule that was made.
- **A channel's menu** - the "⋯" that appears in its top corner, or right-click
  its name - renames it and chooses what the Game/Chat mix balances it
  against. Double-clicking the name also renames it. A seventh channel can be
  added in Settings > Channels.
- **Profiles** (the Profile card, or the tray menu) save and switch whole mixer
  setups.
- **Tray menu > Settings > Open settings...** has hotkeys, the ignored apps,
  light or dark (Lanes follows Windows unless told otherwise), an optional
  notice when a new app starts playing, backup, and diagnostics.

---

## Where your settings live

```
%LOCALAPPDATA%\Lanes\
    config.json        your channels, rules and settings. Meant to be readable
    snapshots\         what your audio looked like before Lanes changed it
    logs\              every change made, with old and new values
```

`snapshots\first-run.json` is captured on Lanes' very first run, before it
changes anything, and is never overwritten. `snapshots\windows-before.json`
records Windows' default output and each device's volume before Lanes first
changed them, and is what **Restore Windows audio settings** puts back.

---

## Portable mode

To keep everything beside the program instead of in `%LOCALAPPDATA%` - on a USB
stick, say - put an empty file called **`portable.txt`** in the same folder as
`Lanes.exe` and `Lanes.Window.exe`. Settings, snapshots and logs then live in a
`Lanes` folder beside them.

Only one Lanes runs at a time, installed or portable, because two would fight
over the same applications. Quit one before starting the other.

A portable copy never takes over an installed copy's start-on-boot entry. Its
"Start with Windows" tick in the tray reflects whichever copy has the entry
(see [limitations.md](limitations.md)).

---

## Upgrading

Run the new installer. It asks the running Lanes to quit, replaces the program,
and keeps your settings.

---

## Uninstalling

Settings → Apps → Installed apps → **Lanes** → Uninstall. Or the
Uninstall shortcut in the Start Menu folder.

It stops the app, removes the program and the shortcuts, and **asks** whether to
remove your settings as well. Answer **No** if you might reinstall — that folder
holds the record of your original audio setup, and nothing else has a copy.

### Put my audio back the way it was

**Do this before uninstalling**, while the app is still there.

Open the tray menu, Settings, **Restore Windows audio settings**. Every application goes back to **100%, unmuted, following Windows' default
device**, and Windows' own default output device and each device's volume go
back to how they were before Lanes first changed them. Applications that are not
running are reset the next time they play. Then **Lanes pauses** - your
channels are not applied, or they would put everything straight back - until
you choose **Resume Lanes** in the tray, the mixer or Settings. Your channels,
rules and profiles are all kept.

Or from a terminal:

```
"%LOCALAPPDATA%\Programs\Lanes\Lanes.exe" --restore
```

**Why this matters.** Windows stores per-application volume and output device
against the application, not against this program. Those settings **outlive
Lanes**. Removing the app does not undo them — it just removes the thing
that was managing them, leaving every assignment in place with nothing to
explain it. Restore is the way back.

One limit, which is Windows' and not ours: an application can only be changed
while it is running. Restore resets the rest as they next play, **while Lanes is
still installed and paused** - so if you are uninstalling, open the apps you use
once after restoring, then uninstall.

---

## If something goes wrong

See [troubleshooting.md](troubleshooting.md), which the installer also copies
next to the program.
