//! The command-line surface for channels, rules and the watcher.
//!
//! Kept apart from `main.rs` so that argument parsing and dispatch stay legible.
//! These are the same operations the local API offers, for scripting and for
//! checking behaviour from a terminal without the window.

use crate::api;
use crate::config::{self, Config, DeviceRef, Rule};
use crate::engine;
use crate::instance;
use crate::rules::{self, Resolution};
use crate::watcher;

pub fn channels() -> i32 {
    let config = Config::load();
    let sessions = winaudio::sessions::list().unwrap_or_default();
    let devices = winaudio::devices::list_all().unwrap_or_default();

    println!("Channels\n");

    for channel in &config.channels {
        let device = match &channel.device {
            Some(d) => d.name.clone(),
            None if channel.is_input => "system default input".into(),
            None => "system default".into(),
        };

        let kind = if channel.is_input {
            "  [input only]"
        } else {
            ""
        };
        let mute = if channel.muted { "MUTED" } else { "     " };

        println!(
            "  {:<8} {:<8} {:>3}%  {}  -> {}{}",
            channel.id,
            channel.name,
            (channel.volume * 100.0).round(),
            mute,
            device,
            kind
        );

        let mut members: Vec<String> = sessions
            .iter()
            .filter(|s| s.process_id != 0 && s.full_path != "<system>")
            .filter(|s| {
                matches!(
                    rules::resolve(&config, &s.executable, &s.full_path),
                    Resolution::Channel { id, .. } if id == channel.id
                )
            })
            .map(|s| s.executable.clone())
            .collect();
        members.sort();
        members.dedup();

        if members.is_empty() {
            println!("           (no apps)");
        } else {
            for member in members {
                println!("           - {member}");
            }
        }
        println!();
    }

    if !config.rules.is_empty() {
        println!("Rules");
        for rule in &config.rules {
            let qualifier = rule
                .path_contains
                .as_ref()
                .map(|p| format!("  (path contains '{p}')"))
                .unwrap_or_default();
            println!("  {:<28} -> {}{}", rule.pattern, rule.channel, qualifier);
        }
        println!();
    }

    println!("Available output devices");
    for d in devices.iter().filter(|d| d.direction == "output") {
        let marker = if d.is_default { "  (default)" } else { "" };
        println!("  {}{}", d.friendly_name, marker);
    }

    0
}

pub fn unrouted() -> i32 {
    let config = Config::load();

    let sessions = match winaudio::sessions::list() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not read sessions: {e}");
            return 1;
        }
    };

    let mut list: Vec<(String, String)> = sessions
        .iter()
        .filter(|s| s.process_id != 0 && s.full_path != "<system>")
        .filter(|s| {
            matches!(
                rules::resolve(&config, &s.executable, &s.full_path),
                Resolution::Unassigned
            )
        })
        .map(|s| (s.executable.clone(), s.full_path.clone()))
        .collect();

    list.sort();
    list.dedup();

    println!("To be routed\n");

    if list.is_empty() {
        println!("  (nothing — every playing application has a channel)");
        return 0;
    }

    for (exe, path) in &list {
        println!("  {exe}");
        println!("      {path}");
    }

    // An unrouted app plays at full volume outside the user's control, so these
    // should be noticeable rather than merely listed.
    println!(
        "\n{} application(s) playing outside any channel, at their own volume.",
        list.len()
    );
    println!("Assign one with:  Lanes --assign <exe> <channel>");
    0
}

pub fn assign(executable: &str, channel_id: &str) -> i32 {
    let mut config = Config::load();

    if config.channel(channel_id).is_none() {
        eprintln!("No channel called '{channel_id}'.");
        eprintln!(
            "Known channels: {}",
            config
                .channels
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return 1;
    }

    // Replace rather than append. Two rules for one executable would resolve by
    // tie-break rules the user cannot see, which makes assignment feel
    // unpredictable. One app, one rule — unless someone hand-edits the file to
    // say otherwise, which is their business.
    config
        .rules
        .retain(|r| !r.pattern.eq_ignore_ascii_case(executable));

    config.rules.push(Rule {
        pattern: executable.to_string(),
        channel: channel_id.to_string(),
        path_contains: None,
        trim: None,
    });

    if let Err(e) = config.save() {
        eprintln!("Could not save config: {e}");
        return 1;
    }

    println!("{executable} -> {channel_id}");
    apply_now(&config, "assign")
}

pub fn unassign(executable: &str) -> i32 {
    let mut config = Config::load();
    let before = config.rules.len();

    config
        .rules
        .retain(|r| !r.pattern.eq_ignore_ascii_case(executable));

    if config.rules.len() == before {
        println!("{executable} had no rule; nothing to remove.");
        return 0;
    }

    if let Err(e) = config.save() {
        eprintln!("Could not save config: {e}");
        return 1;
    }

    println!("{executable} is now unrouted.");
    // Deliberately does not undo the device assignment. Removing a rule says
    // "stop managing this", which is not the same as "put it back" — and
    // silently moving audio on an unassign would be exactly the kind of
    // unrequested change this app avoids. Restore is the way back.
    println!("Its current device assignment is left as it is. Use --restore to undo changes.");
    0
}

pub fn set_volume(channel_id: &str, level: f32) -> i32 {
    if !(0.0..=1.0).contains(&level) {
        eprintln!("Volume must be between 0.0 and 1.0.");
        return 2;
    }

    let mut config = Config::load();

    let Some(channel) = config.channel_mut(channel_id) else {
        eprintln!("No channel called '{channel_id}'.");
        return 1;
    };

    let old = channel.volume;
    channel.volume = level;
    let name = channel.name.clone();

    if let Err(e) = config.save() {
        eprintln!("Could not save config: {e}");
        return 1;
    }

    println!("{name}: {:.0}% -> {:.0}%", old * 100.0, level * 100.0);
    apply_now(&config, "set-volume")
}

pub fn set_mute(channel_id: &str, state: &str) -> i32 {
    let muted = match state.to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" | "mute" => true,
        "off" | "false" | "no" | "0" | "unmute" => false,
        _ => {
            eprintln!("Expected on or off, got '{state}'.");
            return 2;
        }
    };

    let mut config = Config::load();

    let Some(channel) = config.channel_mut(channel_id) else {
        eprintln!("No channel called '{channel_id}'.");
        return 1;
    };

    channel.muted = muted;
    let name = channel.name.clone();

    if let Err(e) = config.save() {
        eprintln!("Could not save config: {e}");
        return 1;
    }

    println!("{name}: {}", if muted { "muted" } else { "unmuted" });
    apply_now(&config, "set-mute")
}

pub fn set_device(channel_id: &str, device_name: Option<&str>) -> i32 {
    let mut config = Config::load();

    match config.channel(channel_id) {
        None => {
            eprintln!("No channel called '{channel_id}'.");
            return 1;
        }
        Some(channel) if channel.is_input => {
            eprintln!(
                "'{channel_id}' is the input channel. This app never routes the \
                 microphone signal — that is out of scope by design."
            );
            return 1;
        }
        Some(_) => {}
    }

    let resolved = match device_name {
        None => None,
        Some(name) => match winaudio::devices::resolve(name) {
            Ok(Some(d)) => Some(DeviceRef {
                id: d.id,
                name: d.friendly_name,
            }),
            Ok(None) => {
                eprintln!("No single output device matches '{name}'. Run --channels for the list.");
                return 1;
            }
            Err(e) => {
                eprintln!("Could not read devices: {e}");
                return 1;
            }
        },
    };

    let Some(channel) = config.channel_mut(channel_id) else {
        return 1;
    };
    channel.device = resolved.clone();
    let name = channel.name.clone();

    if let Err(e) = config.save() {
        eprintln!("Could not save config: {e}");
        return 1;
    }

    match &resolved {
        Some(d) => println!("{name} -> {}", d.name),
        None => println!("{name} -> system default"),
    }

    apply_now(&config, "set-device")
}

pub fn apply() -> i32 {
    let config = Config::load();
    apply_now(&config, "apply")
}

/// Run until interrupted, applying settings as applications appear.
///
/// Event-driven: this blocks on a channel fed by a Windows callback and does
/// nothing at all while no sessions are being created. No poll loop, no timer —
/// an idle Lanes should cost ~0% CPU with no measurable wakeups.
pub fn watch() -> i32 {
    // Taken before anything else. Two cores do not duplicate work, they fight:
    // each applies its own view of the config and they take turns overwriting
    // each other, re-routing live streams continuously and audibly.
    let _lock = match instance::acquire() {
        instance::Acquired::Yes(lock) => lock,
        instance::Acquired::AlreadyRunning => {
            eprintln!("Another Lanes instance is already running.");
            eprintln!();
            eprintln!("Only one may watch for sessions at a time. Two would apply their own");
            eprintln!("view of the config and fight over it, switching devices continuously.");
            eprintln!();
            eprintln!("Stop the other one first, or use the one-shot commands (--status,");
            eprintln!("--apply, --channels), which are safe to run alongside it.");
            return 1;
        }
    };

    let config = Config::load();

    println!("Applying current configuration...");
    if apply_now(&config, "watch-start") != 0 {
        return 1;
    }

    let watcher = match watcher::Watcher::register() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("\nCould not watch for new sessions: {e}");
            return 1;
        }
    };

    println!(
        "\nWatching {} output device(s) for new audio sessions. Ctrl+C to stop.\n",
        watcher.device_count()
    );

    // Blocks until an event arrives. This is the whole loop.
    for event in watcher.events.iter() {
        let watcher::Event::SessionCreated { process_id } = event else {
            // --watch is a diagnostic; device changes are the core's business.
            continue;
        };

        // Reloaded each time, so hand-edits to config.json take effect without
        // a restart.
        let config = Config::load();

        match engine::apply_to_pid(&config, process_id, "session-created") {
            Ok(report) => {
                if let Some(name) = report.unassigned.first() {
                    println!("  new: {name} (pid {process_id}) — unrouted");
                } else if report.devices_changed + report.volumes_changed + report.mutes_changed > 0
                {
                    println!(
                        "  new: pid {process_id} — applied ({} device, {} volume, {} mute)",
                        report.devices_changed, report.volumes_changed, report.mutes_changed
                    );
                }
                for failure in &report.failures {
                    eprintln!("  ! {failure}");
                }
            }
            Err(e) => eprintln!("  ! pid {process_id}: {e}"),
        }
    }

    0
}

fn apply_now(config: &config::Config, reason: &str) -> i32 {
    match engine::apply_all(config, reason) {
        Ok(report) => {
            println!(
                "  {} device, {} volume, {} mute change(s); {} already correct",
                report.devices_changed,
                report.volumes_changed,
                report.mutes_changed,
                report.unchanged
            );

            if !report.unassigned.is_empty() {
                println!(
                    "  {} unrouted: {}",
                    report.unassigned.len(),
                    report.unassigned.join(", ")
                );
            }

            for failure in &report.failures {
                eprintln!("  ! {failure}");
            }

            if report.failures.is_empty() {
                0
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("Could not apply: {e}");
            1
        }
    }
}

/// Run the local API server.
///
/// Takes the same single-instance lock as `--watch`: the server also applies
/// config and watches sessions, so two of them would fight in exactly the same
/// way.
pub fn serve(port: Option<u16>) -> i32 {
    let _lock = match instance::acquire() {
        instance::Acquired::Yes(lock) => lock,
        instance::Acquired::AlreadyRunning => {
            eprintln!("Another Lanes instance is already running.");
            eprintln!("Stop it first — two cores would fight over the config.");
            return 1;
        }
    };

    api::serve(port, None)
}
