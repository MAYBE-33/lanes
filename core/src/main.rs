#![windows_subsystem = "windows"]
//! Lanes — core.
//!
//! `Lanes.exe` owns everything: the channels, the rules, the config file and
//! every call into Windows audio. It runs in the tray, applies each channel's
//! volume, mute and output device to the applications assigned to it, and
//! serves a local API (WebSocket and HTTP on 127.0.0.1) that the mixer window
//! and the Stream Deck plugin use. The window is a separate program and a pure
//! client of that API, so it cannot reach into the core's state even by
//! accident. `docs/architecture.md` describes how the pieces fit.
//!
//! # Layers
//!
//! - **Safety** — [`snapshot`], [`restore`], [`audit`]. Windows persists the
//!   settings Lanes changes, so they outlive Lanes itself. Everything it changes
//!   is logged with its old value, and Restore can put Windows back as it was.
//! - **State** — [`config`], [`rules`], [`routing`], [`profiles`], [`manage`].
//! - **Applying it** — [`engine`], [`master`], and the watchers that tell the
//!   core when something changed: [`watcher`] (new audio sessions),
//!   [`devicewatch`] (devices plugged in or removed), [`endpointwatch`] (the
//!   Windows volume slider).
//! - **Surfaces** — [`api`], [`tray`], [`hotkeys`], [`lifecycle`], and this
//!   file's command line.
//!
//! # This is a desktop program, not a console one
//!
//! `#![windows_subsystem = "windows"]` above is load-bearing. Without it
//! Windows opens a console window for this process, and closing that console
//! would kill the core, the tray icon, the session watcher and every routing
//! rule with it.
//!
//! The command line still works: see [`console`] for how the output finds its
//! way back to a terminal when there is one.

mod api;
mod audit;
mod autostart;
mod commands;
mod config;
mod console;
mod devicewatch;
mod endpointwatch;
mod engine;
mod hotkeys;
mod instance;
mod lifecycle;
mod manage;
mod master;
mod paths;
mod profiles;
mod restore;
mod routing;
mod rules;
mod shutdown;
mod snapshot;
mod tray;
mod watcher;

const USAGE: &str = r#"
Lanes

CHANNELS AND ROUTING
  --channels                    Show channels, their settings and their apps
  --unrouted                    Show applications with no channel ("To be routed")
  --assign <exe> <channel>      Create a rule, e.g. --assign Spotify.exe media
  --unassign <exe>              Remove an app's rule; it returns to unrouted
  --set-volume <channel> <0-1>  Set a channel's volume
  --set-mute <channel> <on|off> Mute or unmute a channel
  --set-device <channel> <name> Send a channel to an output device
  --clear-device <channel>      Return a channel to the system default
  --apply                       Apply the config to everything playing now
  --watch                       Apply, then keep applying as new apps start
  --serve [port]                Run the local API (WebSocket + HTTP) on loopback
  --tray                        Same as no arguments
  --minimised                   Run without opening the window (start-on-boot uses this)
  --ensure-core                 Start the core if it is not running; open nothing.
                                The mixer window uses this when it finds no core.
  --quit                        Stop a running instance (what the uninstaller uses)
  --autostart [on|off|status]   Start with Windows (HKCU Run key; the only one written)

SAFETY
  --snapshot [note]             Capture current audio state
  --restore                     Put Windows audio back as it was before Lanes, and pause
  --list-snapshots              List saved snapshots
  --status                      Current devices, apps, volumes and routing

  --help

Double-clicking the executable is the normal way in: it runs the tray, the core
and the window. The flags above exist for scripting and for what start-on-boot
registers; you should not need them.

--restore is a flag, not just a button, so a broken state can be fixed from a
terminal when there is no working window.

Channels start EMPTY: every application sits in "To be routed" until you assign
it. Nothing is moved that you did not ask to be moved.

Config lives at %LOCALAPPDATA%\Lanes\config.json and is meant to be
hand-edited.
"#;

fn main() {
    // First, before anything can print. See `console` for why the ordering
    // matters: Rust resolves the standard handles once, lazily, and if they are
    // still empty at that moment every later write is silently discarded.
    console::attach_to_parent();

    let args: Vec<String> = std::env::args().skip(1).collect();

    // The guard is created first so it drops last, after every COM object.
    // Releasing an interface past CoUninitialize crashes at exit, after
    // printing correct output. See `winaudio::com`.
    let com = match winaudio::com::initialize() {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("COM initialisation failed: {e}");
            std::process::exit(1);
        }
    };

    let code = run(&args);

    // Explicit: process::exit does not run destructors.
    drop(com);
    std::process::exit(code);
}

/// Stop a running instance.
///
/// # Why this asks rather than kills
///
/// This core holds COM interfaces for its whole lifetime, and they must be
/// released before the COM apartment goes (see `winaudio::com`). Terminating it
/// from outside skips that ordering completely.
///
/// It also owns `config.json`. The write is atomic, but there is still a moment
/// between the temporary file and the rename, and asking it to stop lets it
/// finish.
///
/// The uninstaller calls this, because Windows will not delete files that a
/// running process holds open.
fn cmd_quit() -> i32 {
    match api::oneshot::send("quit") {
        Ok(()) => {
            println!("Lanes has stopped.");
            0
        }
        // Nothing running is the state the caller was asking for. An
        // uninstaller failing here would be failing because its job was
        // already done.
        Err(api::oneshot::Error::NotRunning) => {
            println!("Lanes is not running.");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None => lifecycle::run(),
        Some("--help" | "-h") => {
            println!("{USAGE}");
            0
        }
        Some("--snapshot") => cmd_snapshot(args.get(1).map(String::as_str).unwrap_or("manual")),
        Some("--restore") => cmd_restore(),
        Some("--status") => cmd_status(),
        Some("--list-snapshots") => cmd_list_snapshots(),

        Some("--channels") => commands::channels(),
        Some("--unrouted") => commands::unrouted(),
        Some("--apply") => commands::apply(),
        Some("--watch") => commands::watch(),
        Some("--tray") => lifecycle::run(),
        Some("--minimised") => lifecycle::run_with(true),
        Some("--ensure-core") => lifecycle::ensure_core(),
        Some("--quit") => cmd_quit(),
        Some("--autostart") => match args.get(1).map(String::as_str) {
            Some(state) => lifecycle::autostart_command(state),
            None => lifecycle::autostart_command("status"),
        },
        Some("--serve") => {
            let port = args.get(1).and_then(|p| p.parse().ok());
            commands::serve(port)
        }

        Some("--assign") => match (args.get(1), args.get(2)) {
            (Some(exe), Some(channel)) => commands::assign(exe, channel),
            _ => usage_error("--assign needs an executable and a channel"),
        },
        Some("--unassign") => match args.get(1) {
            Some(exe) => commands::unassign(exe),
            None => usage_error("--unassign needs an executable"),
        },
        Some("--set-volume") => match (args.get(1), args.get(2).and_then(|v| v.parse().ok())) {
            (Some(channel), Some(level)) => commands::set_volume(channel, level),
            _ => usage_error("--set-volume needs a channel and a level between 0 and 1"),
        },
        Some("--set-mute") => match (args.get(1), args.get(2)) {
            (Some(channel), Some(state)) => commands::set_mute(channel, state),
            _ => usage_error("--set-mute needs a channel and on/off"),
        },
        Some("--set-device") => match (args.get(1), args.get(2)) {
            (Some(channel), Some(device)) => commands::set_device(channel, Some(device)),
            _ => usage_error("--set-device needs a channel and a device name"),
        },
        Some("--clear-device") => match args.get(1) {
            Some(channel) => commands::set_device(channel, None),
            None => usage_error("--clear-device needs a channel"),
        },

        Some(other) => {
            eprintln!("Unknown option '{other}'.\n{USAGE}");
            2
        }
    }
}

fn usage_error(message: &str) -> i32 {
    eprintln!("Error: {message}\n{USAGE}");
    2
}

fn cmd_snapshot(reason: &str) -> i32 {
    match snapshot::capture(reason) {
        Ok(snap) => match snapshot::save(&snap) {
            Ok(path) => {
                println!(
                    "Captured {} application(s) across {} device(s).",
                    snap.apps.len(),
                    snap.devices.len()
                );
                println!("  {}", path.display());
                0
            }
            Err(e) => {
                eprintln!("Could not save snapshot: {e}");
                1
            }
        },
        Err(e) => {
            eprintln!("Could not capture state: {e}");
            1
        }
    }
}

/// `--restore`: put Windows audio back as it was before Lanes, and pause.
///
/// When Lanes is running the request goes to it, so the running core - which
/// owns the config - is the one that pauses; changing the file underneath it
/// would be overwritten the next time it saved. Otherwise it is done here.
fn cmd_restore() -> i32 {
    if crate::api::oneshot::send("restore_windows_settings").is_ok() {
        println!("Asked the running Lanes to restore Windows audio and pause.");
        println!("Resume it from the tray when you want Lanes managing apps again.");
        return 0;
    }

    let mut config = config::Config::load();
    match restore::reset_windows(&mut config) {
        Ok(report) => {
            report.print();
            if let Err(e) = config.save() {
                eprintln!("Could not save that Lanes is paused: {e}");
                return 1;
            }
            if report.failures.is_empty() { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("Restore failed: {e}");
            1
        }
    }
}

fn cmd_status() -> i32 {
    match snapshot::capture("status") {
        Ok(snap) => {
            println!("Output and input devices:");
            for d in &snap.devices {
                let marker = if d.is_default { "  (default)" } else { "" };
                println!("  [{}] {}{}", d.direction, d.friendly_name, marker);
            }

            println!("\nApplications with audio sessions:");
            if snap.apps.is_empty() {
                println!("  (none)");
            }
            for app in &snap.apps {
                let route = app
                    .endpoint
                    .as_ref()
                    .and_then(|id| snap.devices.iter().find(|d| &d.id == id))
                    .map(|d| d.friendly_name.as_str())
                    .or(app.endpoint.as_deref())
                    .unwrap_or("system default");

                let volume = app
                    .sessions
                    .first()
                    .map(|s| format!("{:.0}%", s.volume * 100.0))
                    .unwrap_or_else(|| "?".into());

                println!(
                    "  {:<26} {:>5}  -> {}  ({} session(s))",
                    app.executable,
                    volume,
                    route,
                    app.sessions.len()
                );
            }

            println!("\nState folder: {}", paths::root().display());
            println!("Audit logs:   {}", paths::logs_dir().display());
            0
        }
        Err(e) => {
            eprintln!("Could not read state: {e}");
            1
        }
    }
}

fn cmd_list_snapshots() -> i32 {
    let dir = paths::snapshots_dir();

    let Ok(entries) = std::fs::read_dir(&dir) else {
        println!("No snapshots yet ({} does not exist).", dir.display());
        return 0;
    };

    let mut found = false;
    let mut paths_list: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    paths_list.sort_by_key(|e| e.file_name());

    for entry in paths_list {
        let name = entry.file_name().to_string_lossy().to_string();
        let marker = if name == "first-run.json" {
            "  <- never overwritten"
        } else {
            ""
        };
        println!("  {name}{marker}");
        found = true;
    }

    if !found {
        println!("No snapshots yet.");
    }

    println!("\nIn: {}", dir.display());
    0
}
