# Known limitations

What Lanes 1.0 does not do, or does imperfectly, stated plainly. Some are
Windows' limits and cannot be fixed from outside; the rest are open for anyone
who wants to take them on.

## Windows' limits

**Applications using WASAPI exclusive mode cannot be controlled.** They bypass
the Windows audio engine entirely - so no volume mixer, Windows' own included,
can see or change them. They do not appear in Lanes at all, which looks the same
as "not playing". Lanes reports every session it sees as controllable
(`controllable: true` in the API); detecting and marking exclusive-mode apps is
not built.

**Some applications choose their own output device.** Games especially may
ignore or undo the device Lanes assigns. Lanes notices, stops re-assigning that
app rather than fighting it, and marks it with an amber **!**. Set the device
in the application's own settings; volume and mute still work.

**An application's audio settings can only be changed while it is running.**
Restore handles this by pausing Lanes and resetting each remaining application
the next time it plays.

**Applications appear only once they make a sound.** Voice chat apps, for
example, often create no audio session until someone speaks.

**Settings Lanes changes outlive it.** Windows stores each application's volume
and output device itself, so uninstalling Lanes does not undo them. Use
**Restore Windows audio settings** first - see [install.md](install.md).

## Lanes' own gaps

| Area | Limitation |
| --- | --- |
| Device changes | After moving an app to another device, Lanes waits a fixed 300 ms for Windows to create the app's new session before re-applying its volume. On a heavily loaded machine that may occasionally be too short, leaving the app at its previous level until its next change. A bounded wait for the new session would be better. |
| Hotkeys | A hotkey another program was holding is not retried when that program lets go; change the hotkey list or restart Lanes. Windows gives no notice, and polling would cost idle CPU. |
| Tray icon | Apps in "To be routed" are not shown on the tray icon. The mixer's pool turns amber and the new-app notice appears, but with both closed nothing shows. |
| Config safety | A config file that cannot be parsed is moved aside to `config.json.broken` and Lanes starts from defaults. No rolling known-good copies are kept (an import keeps `config.json.before-import`). |
| Audit log | One file per day, never deleted. Small, but unbounded. |
| API port | 8477, or the next free port above it. It cannot be chosen. |
| Faders | A fader moved by another client (a Stream Deck key, a hotkey) jumps to its new value rather than gliding. |
| Portable copies | A portable copy's "Start with Windows" tick reflects whichever copy owns the start-on-boot entry. The Stream Deck plugin needs a portable copy's folder typed into its settings. |
| `--watch` | The command-line watch mode ends on Ctrl+C without its tidy shutdown; Windows cleans up on exit. |
| Platform | Windows 10 (1809) and later, x64 only. |

## By design

- **Removing an app's rule does not move its audio back.** It means "stop
  managing this app", not "put it back". Restore is the way back.
- **Idle sessions can linger on a previous device** after an app is rerouted.
  Windows expires them; they hold no settings.
- **Lanes never processes audio.** No EQ, no effects, no virtual devices. That
  is what keeps latency at zero and Lanes driver-free.
