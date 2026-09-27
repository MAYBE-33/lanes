//! The server: one COM-owning core thread, connections on their own threads.
//!
//! See the module docs in `api/mod.rs` for why it is arranged this way.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use crate::api::protocol::*;
use crate::api::state as state_builder;
use crate::config::{Config, DeviceRef as ConfigDevice};
use crate::engine;
use crate::paths;
use crate::watcher;

/// Default port. If it is taken, the next free one above it is used and
/// written to the port file.
const DEFAULT_PORT: u16 = 8477;

/// How many ports to try before giving up.
const PORT_SCAN_RANGE: u16 = 20;

/// How often meter levels are pushed to subscribed clients.
///
/// **30 per second.** At ten per second a meter does not read as a live level —
/// it reads as a stutter. A tick costs one read per held meter handle (see
/// [`channel_meters`]), so the rate can be what the eye needs.
///
/// Still only while a client asks for it, and it stops the moment the last one
/// disconnects, so an idle core does no metering work at all.
const METER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);

/// Everything the core thread reacts to, merged into one queue.
///
/// `std::sync::mpsc` has no `select`, and adding a crate to get one would be a
/// dependency bought for a single line. Funnelling every source into one enum
/// is simpler and makes the core loop trivially readable.
enum CoreEvent {
    Connected {
        id: u64,
        events: Sender<Event>,
    },
    Disconnected {
        id: u64,
    },
    Request {
        id: u64,
        request: Request,
        reply: Sender<Reply>,
    },
    /// A Windows session-created callback.
    SessionCreated {
        process_id: u32,
    },
    /// An audio endpoint appeared, vanished, or became the default.
    DevicesChanged,
    /// Master's device had its volume or mute changed outside Lanes.
    MasterChanged,
    /// Time to push meter levels.
    MeterTick,
    /// Something the user clicked in the tray.
    ///
    /// Routed through the same queue as API requests so that the core loop
    /// remains the **single owner of the config**. Handling tray clicks on
    /// their own thread would mean two independent load-modify-save cycles
    /// racing, and the loser would silently lose the user's change.
    Tray(crate::tray::TrayCommand),
}

/// Run the API server.
///
/// `control` carries tray commands when the tray is running. They join the same
/// queue as everything else, so there is exactly one owner of the config.
pub fn serve(port: Option<u16>, control: Option<Receiver<crate::tray::TrayCommand>>) -> i32 {
    // One config, loaded once, and the token generated INTO it.
    //
    // Generating the token into a second copy of the config would leave this
    // one without it, and the first save (which happens at startup, when the
    // device list is written) would then write the file back with no token:
    // every client would be refused, on every start.
    let mut config = Config::load();
    let token = ensure_token(&mut config);

    let (listener, port) = match bind(port.unwrap_or(DEFAULT_PORT)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Could not bind a local port: {e}");
            return 1;
        }
    };

    if let Err(e) = write_port_file(port) {
        // Not fatal: clients can still be told the port by other means. But it
        // is the documented discovery mechanism, so say so loudly.
        eprintln!("warning: could not write the port file: {e}");
    }

    println!("Lanes API");
    println!("  ws://127.0.0.1:{port}/     WebSocket (primary)");
    println!("  http://127.0.0.1:{port}/   one-shot HTTP commands");
    println!("  port file: {}", paths::port_file().display());
    println!("  token:     {}", paths::config_file().display());
    println!("\nLoopback only. Ctrl+C to stop.\n");

    let (core_tx, core_rx) = channel::<CoreEvent>();

    // Accept loop, on its own thread. Each connection then gets one more.
    {
        let core_tx = core_tx.clone();
        let token = token.clone();
        std::thread::spawn(move || accept_loop(listener, core_tx, token));
    }

    // Session notifications feed the same queue.
    let windows_events = map_sessions(core_tx.clone());

    let watcher = match watcher::Watcher::register_with(windows_events.clone()) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("warning: not watching for new sessions: {e}");
            None
        }
    };

    // Endpoint notifications feed it too.
    //
    // Without this the core only ever sees the devices that existed when it
    // started: unplugging the DAC would leave every channel pointed at an
    // endpoint that is gone, and plugging it back in would not bring them
    // home. Hot-plug is a first-class feature here, not an edge case.
    let device_watcher = match crate::devicewatch::DeviceWatcher::register(windows_events.clone()) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("warning: not watching for device changes: {e}");
            eprintln!("  channels will not follow a device being unplugged until restart");
            None
        }
    };

    if let Some(control) = control {
        let core_tx = core_tx.clone();
        std::thread::spawn(move || {
            for command in control {
                if core_tx.send(CoreEvent::Tray(command)).is_err() {
                    break;
                }
            }
        });
    }

    let watcher = core_loop(config, core_tx, core_rx, watcher, windows_events);

    drop(watcher);
    drop(device_watcher);
    0
}

/// Adapt the Windows callbacks' event type into the core queue.
///
/// Both watchers send the same type down the same channel, so the core loop
/// keeps one place to wait and stays the single owner of the config.
fn map_sessions(core_tx: Sender<CoreEvent>) -> Sender<watcher::Event> {
    let (tx, rx) = channel::<watcher::Event>();

    std::thread::spawn(move || {
        for event in rx {
            let mapped = match event {
                watcher::Event::SessionCreated { process_id } => {
                    CoreEvent::SessionCreated { process_id }
                }
                watcher::Event::DevicesChanged => CoreEvent::DevicesChanged,
                watcher::Event::MasterChanged => CoreEvent::MasterChanged,
            };

            if core_tx.send(mapped).is_err() {
                break;
            }
        }
    });

    tx
}

// ---------------------------------------------------------------------------
// The core thread
// ---------------------------------------------------------------------------

/// Owns COM, the config, and every connected client.
///
/// Single-threaded by construction. Nothing here is behind a lock because
/// nothing else touches it.
fn core_loop(
    mut config: Config,
    core_tx: Sender<CoreEvent>,
    core_rx: Receiver<CoreEvent>,
    mut watcher: Option<watcher::Watcher>,
    windows_events: Sender<watcher::Event>,
) -> Option<watcher::Watcher> {
    let mut clients: HashMap<u64, Sender<Event>> = HashMap::new();
    let mut meter_subscribers: HashMap<u64, bool> = HashMap::new();
    let mut last_state: Option<State> = None;
    let mut windows = crate::lifecycle::Windows::default();

    // Applications a new-app notice has already been shown for, lower-cased.
    // Once per application per run: a browser opens a session per tab, and a
    // notice per tab would be a notice per click.
    let mut announced: std::collections::HashSet<String> = std::collections::HashSet::new();

    let metering = Arc::new(AtomicBool::new(false));

    // The microphone's capture stream, open only while somebody is watching
    // the meters.
    //
    // This is what lights the Windows microphone-in-use indicator, so its
    // lifetime is the feature's whole privacy story: opened on the first meter
    // tick that needs it, and dropped the moment nothing is subscribed. It
    // lives here, in the loop's own state, because COM is apartment threaded
    // and this is the thread that owns the apartment.
    let mut input_meter: Option<winaudio::capture::InputMeter> = None;

    // What was plugged in when the last device event was handled.
    //
    // Kept so a change can be *described* - which device went, which came
    // back, which channels moved - rather than merely acted on. Without it a
    // device vanishing while nothing was playing left no trace anywhere: no
    // app moved, so the engine logged nothing, and "the watcher never fired"
    // looked identical to "it fired and there was nothing to do".
    let mut last_devices: Vec<winaudio::devices::Device> =
        winaudio::devices::list_all().unwrap_or_default();

    // Every output device present at startup has a place in the fallback
    // order. On the first run this builds the list; afterwards it only ever
    // adds devices it has not met, at the bottom of the enabled ones.
    if crate::routing::maintain(&mut config.device_priority, &last_devices) {
        if let Err(e) = config.save() {
            eprintln!("could not save the device priority list: {e}");
        }
    }

    // Master is Windows' default output (see `master`): read it now, and watch
    // it, so the taskbar slider moving moves Master.
    let mut endpoint_watch = crate::endpointwatch::EndpointWatcher::register(windows_events);
    if crate::master::pull(&mut config) {
        if let Err(e) = config.save() {
            eprintln!("could not save Master's volume from Windows: {e}");
        }
    }

    // What the tray menu was last told. Published before anything else so the
    // Profiles submenu is filled from the start.
    let mut last_tray: Option<crate::tray::TrayView> = None;
    sync_tray(&config, &mut last_tray);

    while let Ok(event) = core_rx.recv() {
        match event {
            CoreEvent::Connected { id, events } => {
                let state = current_state(&config);
                let _ = events.send(Event::Welcome {
                    protocol_version: PROTOCOL_VERSION,
                    authenticated: true,
                });
                let _ = events.send(Event::State {
                    state: state.clone(),
                });
                clients.insert(id, events);
                last_state = Some(state);
            }

            CoreEvent::Disconnected { id } => {
                clients.remove(&id);
                meter_subscribers.remove(&id);
                update_metering(&metering, &meter_subscribers, &core_tx, &mut input_meter);
            }

            CoreEvent::Request { id, request, reply } => {
                let response = handle(&mut config, request, id, &mut meter_subscribers, &core_tx);
                update_metering(&metering, &meter_subscribers, &core_tx, &mut input_meter);
                let _ = reply.send(response);

                // Anything might have changed; tell everyone what actually did.
                broadcast_delta(&config, &clients, &mut last_state);
                sync_tray(&config, &mut last_tray);
            }

            CoreEvent::SessionCreated { process_id } => {
                // A new session means the cached view of the audio graph is
                // genuinely out of date, rather than merely old.
                invalidate_snapshot();

                // Reloaded so hand-edits to config.json take effect without a
                // restart.
                config = Config::load();
                if config.settings.paused {
                    // Paused after a Restore: the only thing Lanes does is
                    // reset an app still waiting for it. See `restore`.
                    crate::restore::reset_if_pending(process_id);
                } else {
                    let _ = engine::apply_to_pid(&config, process_id, "session-created");
                }

                if config.settings.notify_new_apps && !config.settings.paused {
                    if let Some(executable) = unassigned_executable(&config, process_id) {
                        // Counted as announced even when held back, so a game that
                        // started full-screen does not get its notice the moment
                        // the player alt-tabs - the pool's amber count says it
                        // is waiting.
                        if announced.insert(executable.to_lowercase())
                            && !crate::lifecycle::user_is_busy()
                        {
                            crate::lifecycle::open_arrival(&mut windows, &executable);
                        }
                    }
                }

                broadcast_delta(&config, &clients, &mut last_state);
                sync_tray(&config, &mut last_tray);
            }

            CoreEvent::MasterChanged => {
                // Handled first, so a change made while this runs raises a
                // new event rather than being lost.
                endpoint_watch.handled();
                if crate::master::pull(&mut config) {
                    if let Err(e) = config.save() {
                        eprintln!("could not save Master's volume from Windows: {e}");
                    }
                    // Every channel's ceiling moved with the slider.
                    let _ = engine::apply_levels_only(&config, None, "windows:master");
                    broadcast_delta(&config, &clients, &mut last_state);
                    sync_tray(&config, &mut last_tray);
                }
            }

            CoreEvent::DevicesChanged => {
                // Windows sends these in bursts - unplugging one device fires
                // several - so take everything already queued before acting.
                // Re-applying once per event would mean several passes over
                // the audio graph for one physical act, and re-routing a live
                // stream is audible.
                //
                // Everything else that arrived in the meantime is KEPT and put
                // back on the queue. Matching only device events in a
                // `while let Ok(CoreEvent::DevicesChanged) = ...` looks like it
                // skips the others and does not: `try_recv` has already taken
                // an event off the queue by the time the pattern fails, so a
                // request from the window or a new audio session arriving
                // mid-burst would be silently dropped.
                let mut deferred: Vec<CoreEvent> = Vec::new();
                while let Ok(queued) = core_rx.try_recv() {
                    if !matches!(queued, CoreEvent::DevicesChanged) {
                        deferred.push(queued);
                    }
                }

                invalidate_snapshot();

                // Session notifications are per device, so a device that has
                // just appeared needs subscribing to or nothing playing on it
                // will ever be seen.
                if let Some(w) = watcher.as_mut() {
                    w.refresh();
                }

                config = Config::load();

                // The default output may have moved, and Master with it: watch
                // the new one and take its slider.
                endpoint_watch.refresh();
                if crate::master::pull(&mut config) {
                    if let Err(e) = config.save() {
                        eprintln!("could not save Master's device from Windows: {e}");
                    }
                }

                let now = winaudio::devices::list_all().unwrap_or_default();

                // A device plugged in for the first time joins the fallback
                // order now, so an open settings list shows it straight away.
                if crate::routing::maintain(&mut config.device_priority, &now) {
                    if let Err(e) = config.save() {
                        eprintln!("could not save the device priority list: {e}");
                    }
                }

                describe_device_change(&config, &last_devices, &now);
                last_devices = now;

                match engine::apply_all(&config, "devices-changed") {
                    Ok(report) => {
                        if report.devices_changed > 0 || report.volumes_changed > 0 {
                            println!(
                                "  re-applied {} route(s), {} level(s).",
                                report.devices_changed, report.volumes_changed
                            );
                        }
                    }
                    Err(e) => eprintln!("could not re-apply after a device change: {e}"),
                }

                broadcast_delta(&config, &clients, &mut last_state);
                sync_tray(&config, &mut last_tray);

                // Back of the queue. Order relative to later events changes,
                // which is harmless: requests carry their own reply channel,
                // and a session event is handled the same whenever it lands.
                for event in deferred {
                    let _ = core_tx.send(event);
                }
            }

            // The tray thread reporting which hotkeys Windows accepted.
            CoreEvent::Tray(crate::tray::TrayCommand::HotkeysRegistered(results)) => {
                crate::hotkeys::record(results);
                broadcast_delta(&config, &clients, &mut last_state);
            }

            // A hotkey was pressed. Run through the same handler as the API,
            // so it does exactly what the equivalent button does.
            CoreEvent::Tray(crate::tray::TrayCommand::Hotkey(keys)) => {
                if let Some(command) = hotkey_command(&config, &keys) {
                    match command {
                        HotkeyCommand::Api(command) => {
                            let request = Request {
                                id: None,
                                token: None,
                                command,
                            };
                            let reply =
                                handle(&mut config, request, 0, &mut meter_subscribers, &core_tx);
                            if let Some(error) = reply.error {
                                eprintln!("[Hotkey {keys}] {}", error.message);
                            }
                        }
                        HotkeyCommand::Tray(command) => {
                            let _ = core_tx.send(CoreEvent::Tray(command));
                        }
                    }
                }
                broadcast_delta(&config, &clients, &mut last_state);
                sync_tray(&config, &mut last_tray);
            }

            CoreEvent::Tray(command) => {
                if crate::lifecycle::apply_tray_command(&mut config, command, &mut windows) {
                    // Tell every client before going, so a window somebody
                    // started by hand closes itself too. `apply_tray_command`
                    // kills the window this core spawned, which covers the
                    // usual case but not that one.
                    for events in clients.values() {
                        let _ = events.send(Event::Shutdown);
                    }

                    // Long enough for the sends above to reach their sockets.
                    // The clients are local, so this is generous.
                    std::thread::sleep(std::time::Duration::from_millis(250));

                    // Quit: stop the loop so the process can exit. The
                    // watcher goes back to the caller so its registrations are
                    // released in a defined order rather than at process exit.
                    return watcher;
                }
                broadcast_delta(&config, &clients, &mut last_state);
                sync_tray(&config, &mut last_tray);
            }

            CoreEvent::MeterTick => {
                if meter_subscribers.values().any(|on| *on) {
                    let levels = read_meters(&config, &mut input_meter);
                    for (id, on) in &meter_subscribers {
                        if *on {
                            if let Some(tx) = clients.get(id) {
                                let _ = tx.send(Event::Meters {
                                    levels: levels.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Handed back so the caller releases the registrations in a defined
    // order, rather than leaving them to process teardown.
    watcher
}

/// A device some channel already refers to, found by id.
///
/// The config is the one place that knows about devices Windows is not
/// currently offering; see `routing::remembered_absent` for why that list is
/// derived from it rather than from every endpoint Windows has ever seen.
fn remembered(config: &Config, wanted: &str) -> Option<ConfigDevice> {
    config
        .channels
        .iter()
        .filter_map(|c| c.device.clone())
        .chain(config.device_priority.iter().map(|e| ConfigDevice {
            id: e.id.clone(),
            name: e.name.clone(),
        }))
        .find(|d| d.id == wanted)
}

/// Record what a device event actually changed.
///
/// # Why this runs even when nothing needs to move
///
/// The engine only writes, and so only logs, when an application's route or
/// level differs from what it should be. Unplug a device while nothing is
/// playing on it and there is nothing to differ - so without this, a device
/// vanishing would leave no evidence at all, and nobody could tell "the watcher
/// never fired" from "it fired and there was nothing to do". Those need
/// opposite responses.
///
/// The deciding is in [`crate::routing::device_changes`], which is pure and
/// tested; this only prints and records what it returns.
fn describe_device_change(
    config: &Config,
    before: &[winaudio::devices::Device],
    after: &[winaudio::devices::Device],
) {
    let changes = crate::routing::device_changes(config, before, after);

    if changes.is_empty() {
        // Still worth one line: the event did arrive. Silence here would be
        // exactly the ambiguity this function exists to remove.
        println!("Audio devices changed: nothing relevant to any channel.");
        return;
    }

    println!("Audio devices changed:");
    for change in &changes {
        println!("  {}", change.line());
        crate::audit::change(
            &change.target,
            change.field,
            Some(change.from.clone()),
            Some(change.to.clone()),
            "devices-changed",
        );
    }
}

/// Start or stop the meter timer.
///
/// Metering is the one thing in the core that runs on a timer, and it runs
/// **only while a client wants it**. The ticker thread exists only while
/// somebody is subscribed, rather than ticking perpetually and discarding the
/// result, so an idle core does no periodic work at all.
fn update_metering(
    running: &Arc<AtomicBool>,
    subscribers: &HashMap<u64, bool>,
    core_tx: &Sender<CoreEvent>,
    input_meter: &mut Option<winaudio::capture::InputMeter>,
) {
    let wanted = subscribers.values().any(|on| *on);
    let currently = running.load(Ordering::SeqCst);

    if wanted && !currently {
        running.store(true, Ordering::SeqCst);
        let running = Arc::clone(running);
        let core_tx = core_tx.clone();

        std::thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                std::thread::sleep(METER_INTERVAL);
                if core_tx.send(CoreEvent::MeterTick).is_err() {
                    break;
                }
            }
        });
    } else if !wanted && currently {
        running.store(false, Ordering::SeqCst);
    }

    // Close the microphone here, and nowhere else.
    //
    // The obvious place is the meter tick — "if nobody is listening, let go" —
    // and it does not work: stopping the ticker above means **no further ticks
    // arrive**, so that branch would never run and the capture stream would
    // stay open for the life of the process. The Windows microphone-in-use
    // indicator would then sit lit long after the window was closed, which is
    // exactly the thing `winaudio::capture` promises does not happen.
    //
    // This function is the single point where metering turns on and off, so it
    // is the only place that cannot be forgotten.
    if !wanted {
        *input_meter = None;
    }
}

/// The audio graph as last seen, so a reply does not have to re-enumerate it.
struct Snapshot {
    taken: std::time::Instant,
    devices: Vec<winaudio::devices::Device>,
    sessions: Vec<winaudio::sessions::Session>,
}

/// How long a snapshot may be reused.
///
/// A compromise with a measured basis. Enumerating the devices and sessions
/// costs 25-45ms, mostly resolving a process path for every session, and every
/// command reply needs a state. Dragging a fader sends a command per step, so
/// paying that on every reply would put the core seconds behind the hand
/// moving it, and the window would replay the drag afterwards from the
/// backlog.
///
/// Nothing a client needs urgently comes from here: channel volumes, mutes and
/// devices are read straight from the config and are always exact. What ages is
/// the list of running applications, and a session appearing is signalled
/// separately by the watcher, which clears this. So the window is a backstop
/// for a session *ending*, which nothing else reports.
const SNAPSHOT_MAX_AGE: std::time::Duration = std::time::Duration::from_millis(750);

thread_local! {
    /// Single-threaded by construction: the core loop owns COM and is the only
    /// thing that ever builds state.
    static SNAPSHOT: std::cell::RefCell<Option<Snapshot>> =
        const { std::cell::RefCell::new(None) };

    /// Whether per-app routing works on this Windows build.
    ///
    /// Probed once. It is a property of the operating system, not of anything
    /// that changes while the app runs, and the probe is a WinRT activation
    /// that was being done on every single reply.
    static ROUTING_AVAILABLE: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// Drop the snapshot, because the set of sessions has genuinely changed.
fn invalidate_snapshot() {
    SNAPSHOT.with(|cell| *cell.borrow_mut() = None);
    OWNERS.with(|cell| *cell.borrow_mut() = None);
    METERS.with(|cell| *cell.borrow_mut() = None);
}

/// How long the process-to-channel map may be reused.
///
/// A backstop only. It is dropped outright whenever a session appears or the
/// configuration changes, which covers every way the mapping can actually move;
/// this catches a session *ending*, which nothing reports.
const OWNERS_MAX_AGE: std::time::Duration = std::time::Duration::from_millis(1000);

thread_local! {
    static OWNERS: std::cell::RefCell<Option<(std::time::Instant, HashMap<u32, String>)>> =
        const { std::cell::RefCell::new(None) };

    /// The meter interfaces for those processes, held rather than re-found.
    static METERS: std::cell::RefCell<Option<HeldMeters>> =
        const { std::cell::RefCell::new(None) };
}

/// Meter interfaces, with the set of processes they were found for.
///
/// The process list is kept alongside them because it is the only thing that
/// makes the cache answerable: if it still matches, the interfaces are still
/// the right ones.
type HeldMeters = (Vec<u32>, HashMap<u32, Vec<winaudio::sessions::MeterHandle>>);

/// The meter interfaces for a set of processes, found once and kept.
///
/// # Why this is cached
///
/// Finding them means sweeping every endpoint and every session. Reading them
/// is a single call each. Doing the sweep thirty times a second costs about
/// **12% of one core**, measured, where the budget for metering is **1%** — a
/// meter is not supposed to be the most expensive thing the application does.
///
/// Re-found only when the set of processes changes, which is exactly when the
/// answer could differ.
fn channel_meters(
    owner: &HashMap<u32, String>,
) -> HashMap<u32, Vec<winaudio::sessions::MeterHandle>> {
    let mut pids: Vec<u32> = owner.keys().copied().collect();
    pids.sort_unstable();

    METERS.with(|cell| {
        let mut slot = cell.borrow_mut();

        let matches = slot.as_ref().is_some_and(|(known, _)| *known == pids);
        if !matches {
            let found = winaudio::sessions::meters_for(&pids).unwrap_or_default();
            *slot = Some((pids, found));
        }

        slot.as_ref().expect("just populated").1.clone()
    })
}

/// Which channel each process's audio belongs to.
///
/// # Why this is cached, with numbers
///
/// Working it out needs `sessions::list()`, which resolves a **process path for
/// every session** — `OpenProcess` plus `QueryFullProcessImageNameW` each time.
/// Doing that on every meter tick, thirty times a second, costs about **24% of
/// one core** (measured), and the pushes arrive in bursts because ticks queue
/// up faster than they can be served - a meter that visibly lags the audio.
///
/// Nothing in this map changes at audio rate. It changes when an application
/// starts or stops making sound, or when a rule is edited — both of which drop
/// the cache explicitly.
fn channel_owners(config: &Config) -> HashMap<u32, String> {
    use crate::rules::{self, Resolution};

    OWNERS.with(|cell| {
        let mut slot = cell.borrow_mut();

        let fresh = slot
            .as_ref()
            .is_some_and(|(taken, _)| taken.elapsed() < OWNERS_MAX_AGE);

        if !fresh {
            let sessions = winaudio::sessions::list().unwrap_or_default();
            let mut owner: HashMap<u32, String> = HashMap::new();

            for session in &sessions {
                if session.process_id == 0 {
                    continue;
                }
                if let Resolution::Channel { id, .. } =
                    rules::resolve(config, &session.executable, &session.full_path)
                {
                    owner.insert(session.process_id, id.to_string());
                }
            }

            *slot = Some((std::time::Instant::now(), owner));
        }

        slot.as_ref().expect("just populated").1.clone()
    })
}

fn current_state(config: &Config) -> State {
    let (devices, sessions) = SNAPSHOT.with(|cell| {
        let mut slot = cell.borrow_mut();

        let fresh = slot
            .as_ref()
            .is_some_and(|snapshot| snapshot.taken.elapsed() < SNAPSHOT_MAX_AGE);

        if !fresh {
            *slot = Some(Snapshot {
                taken: std::time::Instant::now(),
                devices: winaudio::devices::list_all().unwrap_or_default(),
                sessions: winaudio::sessions::list().unwrap_or_default(),
            });
        }

        let snapshot = slot.as_ref().expect("just populated");
        (snapshot.devices.clone(), snapshot.sessions.clone())
    });

    let routing = ROUTING_AVAILABLE.with(|cell| match cell.get() {
        Some(known) => known,
        None => {
            let available = winaudio::policy::AudioPolicyConfig::connect().is_ok();
            cell.set(Some(available));
            available
        }
    });

    state_builder::build(config, &devices, &sessions, routing)
}

/// What a hotkey turns into: an API command, or something only the tray does.
enum HotkeyCommand {
    Api(Command),
    Tray(crate::tray::TrayCommand),
}

/// The command a pressed hotkey stands for, if it is still bound.
///
/// Looked up by keys rather than by position, so a list changed between the
/// press and its arrival cannot run the wrong binding.
fn hotkey_command(config: &Config, keys: &str) -> Option<HotkeyCommand> {
    use crate::hotkeys::{Action, VOLUME_STEP};
    use crate::tray::TrayCommand;

    let binding = config.hotkeys.iter().find(|b| b.keys == keys)?;

    Some(match &binding.action {
        Action::ToggleMute { channel } => HotkeyCommand::Api(Command::ToggleChannelMute {
            channel: channel.clone(),
        }),
        Action::MuteAll => HotkeyCommand::Api(Command::MuteAll {
            muted: !config.master().is_some_and(|m| m.muted),
        }),
        Action::VolumeUp { channel } => HotkeyCommand::Api(Command::AdjustChannelVolume {
            channel: channel.clone(),
            delta: VOLUME_STEP,
        }),
        Action::VolumeDown { channel } => HotkeyCommand::Api(Command::AdjustChannelVolume {
            channel: channel.clone(),
            delta: -VOLUME_STEP,
        }),
        Action::CycleDevice { channel } => HotkeyCommand::Api(Command::CycleChannelDevice {
            channel: channel.clone(),
        }),
        Action::ActivateProfile { name } => {
            HotkeyCommand::Api(Command::ActivateProfile { name: name.clone() })
        }
        Action::ShowWindow => HotkeyCommand::Tray(TrayCommand::ToggleWindow),
        Action::QuickMixer => HotkeyCommand::Tray(TrayCommand::ToggleFlyout),
    })
}

/// Keep the tray menu in step with the mixer: the Mute all tick, the Profiles
/// submenu, and the hotkeys the tray thread registers. Publishes only when
/// something it shows or holds has changed.
fn sync_tray(config: &Config, last: &mut Option<crate::tray::TrayView>) {
    let view = crate::tray::TrayView {
        muted: config.master().is_some_and(|m| m.muted),
        profiles: crate::profiles::names(config),
        active: crate::profiles::active(config),
        hotkeys: config.hotkeys.iter().map(|b| b.keys.clone()).collect(),
        paused: config.settings.paused,
    };

    if last.as_ref() != Some(&view) {
        crate::tray::publish(view.clone());
        *last = Some(view);
    }
}

fn broadcast_delta(
    config: &Config,
    clients: &HashMap<u64, Sender<Event>>,
    last: &mut Option<State>,
) {
    if clients.is_empty() {
        return;
    }

    let current = current_state(config);

    if let Some(previous) = last {
        if let Some(delta) = state_builder::diff(previous, &current) {
            for tx in clients.values() {
                let _ = tx.send(delta.clone());
            }
        }
    }

    *last = Some(current);
}

/// Per-channel level, taken as the loudest session in that channel.
///
/// A channel's meter should show "is this channel making noise", and the
/// loudest member answers that. Averaging would make a single quiet app drag
/// the needle down and misrepresent a channel that is plainly audible.
///
/// **Input channels are metered differently** — see [`input_level`]. They own
/// no sessions, so the loop below would report a permanent zero for them.
fn read_meters(
    config: &Config,
    input_meter: &mut Option<winaudio::capture::InputMeter>,
) -> Vec<MeterLevel> {
    let mut levels: HashMap<String, f32> = HashMap::new();

    // Which process feeds which channel, and the meter interfaces to read —
    // both cached, because enumerating the audio graph thirty times a second is
    // what a meter costs if you let it. See `channel_owners`.
    let owner = channel_owners(config);
    let meters = channel_meters(&owner);

    for (pid, channel) in &owner {
        let peak = meters
            .get(pid)
            .map(|held| winaudio::sessions::peak_of(held))
            .unwrap_or(0.0);

        let entry = levels.entry(channel.clone()).or_insert(0.0);
        *entry = entry.max(peak);
    }

    config
        .channels
        .iter()
        .map(|c| MeterLevel {
            channel: c.id.clone(),
            level: if c.is_input {
                input_level(c, input_meter)
            } else {
                levels.get(&c.id).copied().unwrap_or(0.0)
            },
        })
        .collect()
}

/// Level for an input channel, from a capture stream held open across ticks.
///
/// # Why not the endpoint meter
///
/// `IAudioMeterInformation` on the capture endpoint is far simpler and needs
/// nothing of ours running — and it reads a flat zero, because Windows only
/// runs the capture path while something has the device open: fifty
/// consecutive reads of 0.000 while speaking. A level that moves when the user
/// talks needs a stream of our own. See
/// [`winaudio::capture`], which is also where the privacy consequences are
/// written down.
///
/// The stream is opened lazily on the first tick that needs it, and reopened if
/// the channel's device changes underneath it. A failure is reported as
/// silence: an unplugged microphone is an ordinary event, and a meter is the
/// wrong place to raise it — the device handling in `routing` is.
fn input_level(
    channel: &crate::config::Channel,
    input_meter: &mut Option<winaudio::capture::InputMeter>,
) -> f32 {
    let wanted = channel.device.as_ref().map(|d| d.id.as_str());

    // Reopen when the configured device no longer matches what is open. `None`
    // means "follow the system default", and the default can move, so that case
    // is left alone rather than being torn down and rebuilt every tick.
    let stale = match (input_meter.as_ref(), wanted) {
        (Some(open), Some(id)) => open.device_id() != id,
        _ => false,
    };
    if stale {
        *input_meter = None;
    }

    if input_meter.is_none() {
        match winaudio::capture::InputMeter::open(wanted) {
            Ok(meter) => *input_meter = Some(meter),
            Err(_) => return 0.0,
        }
    }

    input_meter.as_ref().map(|m| m.peak()).unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn handle(
    config: &mut Config,
    request: Request,
    conn: u64,
    meter_subscribers: &mut HashMap<u64, bool>,
    core_tx: &Sender<CoreEvent>,
) -> Reply {
    let id = request.id;

    macro_rules! channel_or_404 {
        ($name:expr) => {
            match config.channel($name) {
                Some(c) => c.id.clone(),
                None => {
                    return Reply::err(
                        id,
                        ErrorCode::NotFound,
                        format!("no channel called '{}'", $name),
                    )
                }
            }
        };
    }

    match request.command {
        Command::GetState => Reply::ok(id, current_state(config)),

        Command::SetChannelVolume { channel, value } => {
            let key = channel_or_404!(&channel);
            if let Some(c) = config.channel_mut(&key) {
                c.volume = value.clamp(0.0, 1.0);
            }
            commit_levels(config, id, "api:set_channel_volume", scope(&key))
        }

        Command::AdjustChannelVolume { channel, delta } => {
            let key = channel_or_404!(&channel);
            if let Some(c) = config.channel_mut(&key) {
                c.volume = (c.volume + delta).clamp(0.0, 1.0);
            }
            commit_levels(config, id, "api:adjust_channel_volume", scope(&key))
        }

        Command::SetChatMix { value } => {
            config.chat_mix.value = value.clamp(-1.0, 1.0);
            // Two channels, so no single-channel narrowing — but still levels
            // only, which is where nearly all the saving is.
            commit_levels(config, id, "api:set_chat_mix", None)
        }

        Command::SetChatMixChannels { game, chat } => match config.set_mix_channels(&game, &chat) {
            // Levels: the channel that was attenuated is not any more.
            Ok(()) => commit_levels(config, id, "api:set_chat_mix_channels", None),
            Err(refusal) => manage_refused(id, refusal),
        },

        Command::AdjustChatMix { delta } => {
            config.chat_mix.value = (config.chat_mix.value + delta).clamp(-1.0, 1.0);
            commit_levels(config, id, "api:adjust_chat_mix", None)
        }

        Command::SetChannelMute { channel, muted } => {
            let key = channel_or_404!(&channel);
            if let Some(c) = config.channel_mut(&key) {
                c.muted = muted;
            }
            commit_levels(config, id, "api:set_channel_mute", scope(&key))
        }

        Command::ToggleChannelMute { channel } => {
            let key = channel_or_404!(&channel);
            if let Some(c) = config.channel_mut(&key) {
                c.muted = !c.muted;
            }
            commit_levels(config, id, "api:toggle_channel_mute", scope(&key))
        }

        // Both of these are the tray's own commands under another name, and
        // they are deliberately routed back through the same queue rather than
        // acted on here. The tray path is the tested one, it is the single
        // owner of the config, and having two ways to quit would eventually
        // mean two behaviours.
        Command::ShowWindow => {
            let _ = core_tx.send(CoreEvent::Tray(crate::tray::TrayCommand::ShowWindow));
            Reply::ok(id, current_state(config))
        }

        Command::Quit => {
            let _ = core_tx.send(CoreEvent::Tray(crate::tray::TrayCommand::Quit));
            Reply::ok(id, current_state(config))
        }

        Command::SetWindowPlacement {
            x,
            y,
            width,
            height,
            maximised,
        } => {
            // Guard against a zero or negative size being stored.
            //
            // A minimised window reports a placement that is not a usable
            // rectangle, and saving it would mean reopening at nothing. Windows
            // keeps the restore rectangle separately for exactly this reason;
            // if what arrives is not usable, the previous value is kept.
            if width > 0 && height > 0 {
                config.settings.window = Some(crate::config::WindowPlacement {
                    x,
                    y,
                    width,
                    height,
                    maximised,
                });
            }

            if let Err(e) = config.save() {
                return Reply::err(id, ErrorCode::AudioError, format!("could not save: {e}"));
            }
            Reply::ok(id, current_state(config))
        }

        Command::SetChannelCollapsed { channel, collapsed } => {
            let key = channel_or_404!(&channel);
            if let Some(c) = config.channel_mut(&key) {
                c.collapsed = collapsed;
            }

            // Saved, but no audio call: this changes nothing about the sound,
            // and running the engine for it would mean a device write for a
            // disclosure triangle.
            if let Err(e) = config.save() {
                return Reply::err(id, ErrorCode::AudioError, format!("could not save: {e}"));
            }
            Reply::ok(id, current_state(config))
        }

        Command::SetChannelDevice { channel, device_id } => {
            let key = channel_or_404!(&channel);

            let resolved = match &device_id {
                None => None,
                Some(wanted) => match winaudio::devices::list_all() {
                    Ok(devices) => match devices.into_iter().find(|d| &d.id == wanted) {
                        Some(d) => Some(ConfigDevice {
                            id: d.id,
                            name: d.friendly_name,
                        }),
                        // Not plugged in - but if any channel already remembers
                        // it, it is a real device and choosing it is fine. The
                        // picker offers remembered devices precisely so a
                        // channel can be pointed at a DAC that is switched
                        // off; refusing here would make that offer a lie.
                        None => match remembered(config, wanted) {
                            Some(known) => Some(known),
                            None => {
                                return Reply::err(
                                    id,
                                    ErrorCode::NotFound,
                                    format!("no device with id '{wanted}'"),
                                )
                            }
                        },
                    },
                    Err(e) => return Reply::err(id, ErrorCode::AudioError, e.to_string()),
                },
            };

            if let Some(c) = config.channel_mut(&key) {
                if c.is_input {
                    return Reply::err(
                        id,
                        ErrorCode::BadRequest,
                        "the mic channel never routes the microphone signal",
                    );
                }
                // Master's device *is* the Windows default, so "system
                // default" is not a choice for it - it already is one.
                if key == "master" && resolved.is_none() {
                    return Reply::ok(id, current_state(config));
                }
                c.device = resolved;
            }
            commit(config, id, "api:set_channel_device")
        }

        Command::SetDevicePriority { entries } => {
            let wanted: Vec<(String, bool)> =
                entries.into_iter().map(|e| (e.id, e.enabled)).collect();
            config.device_priority = crate::routing::reorder(&config.device_priority, &wanted);

            // A full apply, not a save: changing the order can change where a
            // channel whose own device is unplugged should be right now.
            commit(config, id, "api:set_device_priority")
        }

        Command::CycleChannelDevice { channel } => {
            let key = channel_or_404!(&channel);

            let outputs: Vec<_> = match winaudio::devices::list_all() {
                Ok(d) => d.into_iter().filter(|d| d.direction == "output").collect(),
                Err(e) => return Reply::err(id, ErrorCode::AudioError, e.to_string()),
            };

            if outputs.is_empty() {
                return Reply::err(id, ErrorCode::NotFound, "no output devices");
            }

            // Refused for the same reason `set_channel_device` refuses it. The
            // cycle had no such check, so a Stream Deck key could route the
            // Mic channel where the device picker never would.
            if config.channel(&key).is_some_and(|c| c.is_input) {
                return Reply::err(
                    id,
                    ErrorCode::BadRequest,
                    "the mic channel never routes the microphone signal",
                );
            }

            // Master's next device is the next Windows default, round and
            // round; "system default" is not a position when Master is it.
            if key == "master" {
                let current = winaudio::endpoint::default_output_id().ok().flatten();
                if let Some(next) = crate::master::next_output(current.as_deref()) {
                    if let Some(c) = config.channel_mut("master") {
                        c.device = Some(next);
                    }
                }
                return commit(config, id, "api:cycle_channel_device");
            }

            if let Some(c) = config.channel_mut(&key) {
                // The cycle includes "system default" as a position, so a
                // single button can reach every state rather than being unable
                // to get back to the default once it has left it.
                let next = match &c.device {
                    None => Some(0),
                    Some(current) => outputs
                        .iter()
                        .position(|d| d.id == current.id)
                        .map(|i| i + 1)
                        .filter(|i| *i < outputs.len()),
                };

                c.device = next.map(|i| ConfigDevice {
                    id: outputs[i].id.clone(),
                    name: outputs[i].friendly_name.clone(),
                });
            }
            commit(config, id, "api:cycle_channel_device")
        }

        Command::AssignApp {
            executable,
            channel,
        } => {
            let key = channel_or_404!(&channel);
            // The user is putting it somewhere on purpose, so an application
            // that refused a device before gets one more try.
            crate::engine::forget_refusal(&executable);
            config
                .rules
                .retain(|r| !r.pattern.eq_ignore_ascii_case(&executable));
            config.rules.push(crate::config::Rule {
                pattern: executable,
                channel: key,
                path_contains: None,
                trim: None,
            });
            commit(config, id, "api:assign_app")
        }

        Command::UnassignApp { executable } => {
            config
                .rules
                .retain(|r| !r.pattern.eq_ignore_ascii_case(&executable));
            commit(config, id, "api:unassign_app")
        }

        Command::MuteAll { muted } => {
            // Mute-all is Master's mute, not every channel's. Muting each
            // channel individually would destroy the user's per-channel mute
            // states, and unmuting afterwards could not restore them.
            if let Some(master) = config.channel_mut("master") {
                master.muted = muted;
            }
            commit_levels(config, id, "api:mute_all", None)
        }

        Command::RestoreWindowsSettings => match crate::restore::reset_windows(config) {
            Ok(report) => {
                report.print();
                // Windows' default device and volume may have moved back, and
                // Master is them.
                crate::master::pull(config);
                if let Err(e) = config.save() {
                    return Reply::err(id, ErrorCode::AudioError, format!("could not save: {e}"));
                }
                invalidate_snapshot();
                Reply::ok(id, current_state(config))
            }
            Err(e) => Reply::err(id, ErrorCode::AudioError, e.to_string()),
        },

        Command::Resume => {
            crate::restore::resume(config);
            commit(config, id, "api:resume")
        }

        Command::SubscribeMeters { enabled } => {
            meter_subscribers.insert(conn, enabled);
            Reply::ok(id, current_state(config))
        }

        // A full apply rather than levels only: a profile can move a channel
        // to another device, and that is the one thing `commit_levels` skips.
        Command::ActivateProfile { name } => match crate::profiles::activate(config, &name) {
            Ok(_) => commit(config, id, "api:activate_profile"),
            Err(refusal) => profile_refused(id, refusal),
        },

        // Saving and deleting change no audio setting, so they save and do
        // nothing else - running the engine to write down what is already
        // true would be a device write for a list entry.
        Command::SaveProfile { name } => match crate::profiles::save(config, &name) {
            Ok(_) => save_only(config, id),
            Err(refusal) => profile_refused(id, refusal),
        },

        Command::DeleteProfile { name } => match crate::profiles::delete(config, &name) {
            Ok(_) => save_only(config, id),
            Err(refusal) => profile_refused(id, refusal),
        },

        Command::SetHotkeys { bindings } => {
            let mut accepted: Vec<crate::hotkeys::Binding> = Vec::new();

            for binding in bindings {
                let keys = match crate::hotkeys::parse(&binding.keys) {
                    Ok((_, keys)) => keys,
                    Err(why) => {
                        return Reply::err(id, ErrorCode::BadRequest, format!("{}: {why}", binding.keys))
                    }
                };

                if accepted.iter().any(|b| b.keys == keys) {
                    return Reply::err(
                        id,
                        ErrorCode::BadRequest,
                        format!("{keys} is bound twice; one key combination can do one thing"),
                    );
                }

                if let Some(channel) = binding.action.channel() {
                    let Some(found) = config.channel(channel) else {
                        return Reply::err(
                            id,
                            ErrorCode::NotFound,
                            format!("no channel called '{channel}'"),
                        );
                    };
                    if found.is_input
                        && matches!(binding.action, crate::hotkeys::Action::CycleDevice { .. })
                    {
                        return Reply::err(
                            id,
                            ErrorCode::BadRequest,
                            "the mic channel never routes the microphone signal",
                        );
                    }
                }

                accepted.push(crate::hotkeys::Binding {
                    keys,
                    action: binding.action,
                });
            }

            // Saved and nothing else. The tray thread picks the new list up
            // from `sync_tray` after this reply, registers it, and reports back
            // which keys Windows refused.
            config.hotkeys = accepted;
            save_only(config, id)
        }

        Command::RenameChannel { channel, name } => match config.rename_channel(&channel, &name) {
            Ok(_) => save_only(config, id),
            Err(refusal) => manage_refused(id, refusal),
        },

        // Ignoring changes which applications are managed, so the engine runs:
        // not to move the ignored app - it is left exactly where it is - but
        // because un-ignoring puts an app back under its rule, and that has to
        // be applied now rather than the next time it makes a sound.
        Command::IgnoreApp { executable } => {
            config.ignore(&executable);
            commit(config, id, "api:ignore_app")
        }

        Command::UnignoreApp { executable } => {
            if !config.unignore(&executable) {
                return Reply::err(
                    id,
                    ErrorCode::NotFound,
                    format!("{executable} is not on the ignore list"),
                );
            }
            commit(config, id, "api:unignore_app")
        }

        Command::SetAppTrim { executable, trim } => match config.set_trim(&executable, trim) {
            Ok(channel) => commit_levels(config, id, "api:set_app_trim", scope(&channel)),
            Err(refusal) => manage_refused(id, refusal),
        },

        Command::GetDiagnostics => {
            let mut reply = Reply::ok(id, current_state(config));
            reply.diagnostics = Some(diagnostics(config));
            reply
        }

        Command::ExportConfig => match serde_json::to_value(config.exported()) {
            Ok(value) => {
                let mut reply = Reply::ok(id, current_state(config));
                reply.config = Some(value);
                reply
            }
            Err(e) => Reply::err(id, ErrorCode::AudioError, format!("could not export: {e}")),
        },

        Command::ImportConfig { config: incoming } => {
            let imported: Config = match serde_json::from_value(incoming) {
                Ok(parsed) => parsed,
                Err(e) => {
                    return Reply::err(
                        id,
                        ErrorCode::BadRequest,
                        format!("that file is not a Lanes configuration: {e}"),
                    )
                }
            };

            // Two nets before anything changes. The audio as it is now, so
            // `--restore` can undo what the import is about to apply; and the
            // config as it is now, beside the real one, so the import itself
            // can be undone by hand.
            match crate::snapshot::capture("before-import") {
                Ok(snapshot) => {
                    if let Err(e) = crate::snapshot::save(&snapshot) {
                        return Reply::err(
                            id,
                            ErrorCode::AudioError,
                            format!("not imported: could not save a snapshot first: {e}"),
                        );
                    }
                }
                Err(e) => {
                    return Reply::err(
                        id,
                        ErrorCode::AudioError,
                        format!("not imported: could not snapshot the audio first: {e}"),
                    )
                }
            }
            let backup = crate::paths::config_file().with_extension("json.before-import");
            if let Err(e) = std::fs::copy(crate::paths::config_file(), &backup) {
                return Reply::err(
                    id,
                    ErrorCode::AudioError,
                    format!("not imported: could not keep a copy of the current config: {e}"),
                );
            }

            if let Err(refusal) = config.import(imported) {
                return manage_refused(id, refusal);
            }
            crate::audit::change(
                "<config>",
                "import",
                None,
                Some(backup.display().to_string()),
                "api:import_config",
            );
            commit(config, id, "api:import_config")
        }

        Command::SetNotifyNewApps { enabled } => {
            config.settings.notify_new_apps = enabled;
            save_only(config, id)
        }

        Command::SetTheme { theme } => {
            config.settings.theme = theme;
            save_only(config, id)
        }

        // Neither changes any audio: a new channel holds no apps, and a
        // removed one's apps are left where they are, as unassigning does.
        Command::AddChannel { name } => match config.add_channel(&name) {
            Ok(_) => save_only(config, id),
            Err(refusal) => manage_refused(id, refusal),
        },

        Command::RemoveChannel { channel } => match config.remove_channel(&channel) {
            Ok(_) => save_only(config, id),
            Err(refusal) => manage_refused(id, refusal),
        },
    }
}

/// Save a change that has no audio consequence, and answer with the state.
fn save_only(config: &Config, id: Option<u64>) -> Reply {
    if let Err(e) = config.save() {
        return Reply::err(id, ErrorCode::AudioError, format!("could not save: {e}"));
    }
    Reply::ok(id, current_state(config))
}

/// The executable behind a new session, if it is one no rule places and the
/// ignore list does not hide - the ones a new-app notice is for.
fn unassigned_executable(config: &Config, process_id: u32) -> Option<String> {
    let session = winaudio::sessions::list()
        .ok()?
        .into_iter()
        .find(|s| s.process_id == process_id && s.full_path != "<system>")?;

    matches!(
        crate::rules::resolve(config, &session.executable, &session.full_path),
        crate::rules::Resolution::Unassigned
    )
    .then_some(session.executable)
}

fn manage_refused(id: Option<u64>, refusal: crate::manage::Refusal) -> Reply {
    let code = match refusal {
        crate::manage::Refusal::NotFound(_) => ErrorCode::NotFound,
        crate::manage::Refusal::Invalid(_) => ErrorCode::BadRequest,
    };
    Reply::err(id, code, refusal.to_string())
}

/// Everything `get_diagnostics` reports.
///
/// Built fresh rather than from the state's snapshot: diagnostics is asked for
/// when something looks wrong, and a view three-quarters of a second old is
/// the wrong thing to hand someone working out why.
fn diagnostics(config: &Config) -> Diagnostics {
    let sessions = winaudio::sessions::list()
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.process_id != 0)
        .map(|s| {
            let ignored = crate::rules::is_ignored(config, &s.executable);
            let rule = (!ignored)
                .then(|| crate::rules::best_rule(config, &s.executable, &s.full_path))
                .flatten();
            DiagnosticSession {
                channel: rule
                    .and_then(|r| config.channel(&r.channel))
                    .map(|c| c.id.clone()),
                rule: rule.map(|r| r.pattern.clone()),
                ignored,
                executable: s.executable,
                path: s.full_path,
                process_id: s.process_id,
                state: s.state,
                volume: s.volume,
                muted: s.muted,
            }
        })
        .collect();

    Diagnostics {
        state_folder: paths::root().display().to_string(),
        log_folder: paths::logs_dir().display().to_string(),
        portable: paths::portable_dir().is_some(),
        routing_available: winaudio::policy::AudioPolicyConfig::connect().is_ok(),
        sessions,
        recent: recent_audit(40),
    }
}

/// The last `count` audit entries, from the newest two days' files.
fn recent_audit(count: usize) -> Vec<serde_json::Value> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(paths::logs_dir())
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("audit-") && n.ends_with(".jsonl"))
                })
                .collect()
        })
        .unwrap_or_default();

    // The date is in the name, so the names sort in time order.
    files.sort();
    let newest: Vec<_> = files.iter().rev().take(2).rev().collect();

    let mut lines: Vec<serde_json::Value> = newest
        .into_iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .flat_map(|text| {
            text.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect::<Vec<_>>()
        })
        .collect();

    let skip = lines.len().saturating_sub(count);
    lines.drain(..skip);
    lines
}

fn profile_refused(id: Option<u64>, refusal: crate::profiles::Refusal) -> Reply {
    let code = match refusal {
        crate::profiles::Refusal::NotFound(_) => ErrorCode::NotFound,
        crate::profiles::Refusal::BadName(_) => ErrorCode::BadRequest,
    };
    Reply::err(id, code, refusal.to_string())
}

/// Which channels a change to `channel_id` can possibly affect.
///
/// Master is a ceiling over every other channel, so a change to it reaches
/// everything.
/// Any other channel reaches only the applications assigned to it.
fn scope(channel_id: &str) -> Option<&str> {
    if channel_id == "master" {
        None
    } else {
        Some(channel_id)
    }
}

/// Save and apply, for a change that moved a level rather than a device.
///
/// Splitting this from [`commit`] is what makes a fader usable. The full apply
/// costs roughly 100ms, measured, and almost none of it is the volume: it is one
/// WinRT activation for the undocumented routing interface plus an endpoint
/// read for every process, none of which can have changed because someone moved
/// a slider. See [`engine::apply_levels_only`].
fn commit_levels(
    config: &Config,
    id: Option<u64>,
    reason: &str,
    only_channel: Option<&str>,
) -> Reply {
    if let Err(e) = config.save() {
        return Reply::err(id, ErrorCode::AudioError, format!("could not save: {e}"));
    }

    // Master's volume and mute go to Windows first, so the engine sets every
    // channel against the slider as it now is. A Master change moves every
    // channel's ceiling, so it is never narrowed to one channel.
    crate::master::push(config, reason);
    let only_channel = only_channel.filter(|c| *c != "master");

    match engine::apply_levels_only(config, only_channel, reason) {
        Ok(_) => Reply::ok(id, current_state(config)),
        Err(e) => Reply::err(id, ErrorCode::AudioError, e.to_string()),
    }
}

/// Save the config and apply it, then answer with the resulting state.
fn commit(config: &Config, id: Option<u64>, reason: &str) -> Reply {
    if let Err(e) = config.save() {
        return Reply::err(id, ErrorCode::AudioError, format!("could not save: {e}"));
    }

    // Master's device, volume and mute to Windows first - see `commit_levels`.
    // A new default device changes what every client should be shown, so the
    // cached view of the audio graph goes.
    crate::master::push(config, reason);
    invalidate_snapshot();

    match engine::apply_all(config, reason) {
        Ok(_) => Reply::ok(id, current_state(config)),
        Err(e) => Reply::err(id, ErrorCode::AudioError, e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

fn bind(preferred: u16) -> std::io::Result<(TcpListener, u16)> {
    let mut last = None;

    for offset in 0..PORT_SCAN_RANGE {
        let port = preferred.saturating_add(offset);
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));

        match TcpListener::bind(addr) {
            Ok(listener) => return Ok((listener, port)),
            Err(e) => last = Some(e),
        }
    }

    Err(last.unwrap_or_else(|| std::io::Error::other("no ports tried")))
}

/// Publish the chosen port so clients can find it without configuration.
fn write_port_file(port: u16) -> std::io::Result<()> {
    paths::ensure_dir(&paths::root())?;
    std::fs::write(paths::port_file(), port.to_string())
}

/// The API token, generating and saving one if the config has none.
///
/// Takes the config the core will go on to use, rather than loading its own,
/// so that the token it generates is in the copy that gets saved from then on.
fn ensure_token(config: &mut Config) -> String {
    if let Some(existing) = &config.api_token {
        if !existing.is_empty() {
            return existing.clone();
        }
    }

    // Local-only guard, not a secret worth protecting against a determined
    // attacker with code execution on this computer — at which point the game
    // is already lost. 128 bits of OS randomness is ample for "stop other local
    // processes poking it casually", which is all it is for.
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let token: String = (0..32)
        .map(|_| {
            let index = rand::random::<u32>() as usize % ALPHABET.len();
            ALPHABET[index] as char
        })
        .collect();

    config.api_token = Some(token.clone());
    if let Err(e) = config.save() {
        eprintln!("warning: could not persist the API token: {e}");
    }

    token
}

fn accept_loop(listener: TcpListener, core_tx: Sender<CoreEvent>, token: String) {
    let mut next_id = 1u64;

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };

        let id = next_id;
        next_id += 1;

        let core_tx = core_tx.clone();
        let token = token.clone();

        std::thread::spawn(move || {
            if let Err(e) = handle_connection(stream, id, core_tx, token) {
                // Clients disconnecting abruptly is normal, not noteworthy.
                let _ = e;
            }
        });
    }
}

/// One connection. Sniffs whether it is a WebSocket upgrade or a plain HTTP
/// request, because both are served on the same port.
fn handle_connection(
    mut stream: TcpStream,
    id: u64,
    core_tx: Sender<CoreEvent>,
    token: String,
) -> std::io::Result<()> {
    let mut peek = [0u8; 1024];
    let read = stream.peek(&mut peek)?;
    let head = String::from_utf8_lossy(&peek[..read]);

    if let Some(why) = browser_refusal(&head) {
        let body = format!("{why}\n");
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        );
        return Ok(());
    }

    if head.to_lowercase().contains("upgrade: websocket") {
        websocket_connection(stream, id, core_tx, token)
    } else {
        http_request(&mut stream, &head, core_tx, token)
    }
}

/// Why a request that came from a web page is refused, or `None` for one that
/// did not.
///
/// # Why
///
/// The API is on 127.0.0.1, which keeps other machines out and does nothing
/// about the browser on this one. **A browser does not apply its cross-origin
/// rules to WebSockets**, so any page open in any tab could connect to
/// `ws://127.0.0.1:8477/` and be handed the full state on connect - every
/// application playing sound, every device, every profile name - without a
/// token, since the token guards commands and not the state push.
///
/// Every client this API has - the window, the plugin, curl, a script - is a
/// program, and programs send no `Origin`. Browsers always send one on a
/// WebSocket handshake and on any cross-site request, so refusing any request
/// that carries one shuts browsers out and nothing else.
///
/// `Host` closes the other door, DNS rebinding: a page on a hostile domain that
/// re-points its own name at 127.0.0.1 makes *same-origin* requests, which a
/// browser sends without an `Origin` - but with the hostile name in `Host`.
/// Programs connecting to 127.0.0.1 send 127.0.0.1 or localhost, or nothing.
fn browser_refusal(head: &str) -> Option<&'static str> {
    for line in head.split("\r\n").skip(1).take_while(|line| !line.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();

        if name.eq_ignore_ascii_case("origin") {
            return Some("Lanes does not accept requests from web pages");
        }

        if name.eq_ignore_ascii_case("host") {
            // Drop the port: "127.0.0.1:8477", "[::1]:8477", "localhost".
            let host = match value.rsplit_once(':') {
                Some((h, port)) if !h.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
                _ => value,
            };
            let local = host.eq_ignore_ascii_case("localhost")
                || host == "127.0.0.1"
                || host == "[::1]";
            if !local {
                return Some("Lanes only answers requests addressed to 127.0.0.1");
            }
        }
    }
    None
}

fn websocket_connection(
    stream: TcpStream,
    id: u64,
    core_tx: Sender<CoreEvent>,
    token: String,
) -> std::io::Result<()> {
    // One WebSocket, one thread, both directions.
    //
    // Not a cloned TCP stream with a second `tungstenite::WebSocket` on a
    // writer thread: two independent protocol state machines writing frames to
    // the same socket can interleave them, producing a stream that is valid to
    // neither side. That appears to work, and shows up only as an unexplained
    // error on close.
    //
    // Instead the socket stays on one thread, with a read timeout so the thread
    // can alternate between serving requests and flushing queued pushes.
    let mut socket = match tungstenite::accept(stream) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };

    // How long to sit in a socket read before going to look for pushes.
    //
    // **Short, and the waiting is done elsewhere.** A long read timeout would
    // hold back everything the core had queued until it expired - meter levels
    // generated every 33ms would go out in bursts, and the meter would freeze
    // and jump. Instead the loop blocks on the *event channel* for up to
    // `IDLE_WAIT`, which wakes instantly when there is something to send, so
    // an idle connection still costs nothing.
    //
    // Set on the socket tungstenite actually holds, via `get_ref`. Setting it
    // on a `try_clone`d handle beforehand silently does nothing: the read
    // blocks forever, queued pushes are never flushed, and the connection
    // simply goes quiet with no error anywhere.
    const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(5);
    const IDLE_WAIT: std::time::Duration = std::time::Duration::from_millis(100);
    socket.get_ref().set_read_timeout(Some(READ_TIMEOUT))?;

    let (events_tx, events_rx) = channel::<Event>();

    let _ = core_tx.send(CoreEvent::Connected {
        id,
        events: events_tx,
    });

    let mut closed = false;

    while !closed {
        // 1. Anything the client sent.
        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                let reply = dispatch(text.as_ref(), id, &core_tx, &token);
                if let Ok(json) = serde_json::to_string(&reply) {
                    if socket
                        .send(tungstenite::Message::Text(json.into()))
                        .is_err()
                    {
                        closed = true;
                    }
                }
            }
            Ok(tungstenite::Message::Close(_)) => {
                // Complete the closing handshake rather than just dropping the
                // socket. tungstenite queues the close reply when it reads the
                // frame, but it has to be flushed — without this the client
                // sees an abnormal closure and reports an error for what was a
                // perfectly orderly disconnect.
                let _ = socket.close(None);
                let _ = socket.flush();
                closed = true;
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Expected: the read timeout elapsed with nothing to read.
            }
            Err(_) => closed = true,
        }

        // 2. Anything the core wants to push, right now.
        while let Ok(event) = events_rx.try_recv() {
            if !push(&mut socket, &event) {
                closed = true;
                break;
            }
        }

        // 3. Then wait on the CHANNEL rather than on the socket.
        //
        // This is what keeps a push prompt. Blocking on the socket read meant
        // everything queued behind a timeout that had nothing to do with when
        // the core had something to say; blocking here wakes the moment it
        // does, and otherwise sleeps just as long as before.
        if !closed {
            match events_rx.recv_timeout(IDLE_WAIT) {
                Ok(event) => {
                    if !push(&mut socket, &event) {
                        closed = true;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => closed = true,
            }
        }
    }

    let _ = core_tx.send(CoreEvent::Disconnected { id });
    Ok(())
}

/// The largest request body this surface will read, in bytes.
///
/// Commands are tens of bytes. This is a sanity limit so a confused or hostile
/// local client cannot make the server allocate without bound; it is not a
/// meaningful constraint on any real request.
const MAX_HTTP_BODY: usize = 64 * 1024;

/// Index of the first occurrence of `needle` in `haystack`.
///
/// Twenty characters rather than a dependency, and it only ever looks for the
/// four bytes that end an HTTP header block.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Read `Content-Length` from a set of HTTP headers.
///
/// Case-insensitive, because the header name is, and a client that sends
/// `content-length` is not wrong.
fn content_length(head: &str) -> Option<usize> {
    head.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim())
        })
        .and_then(|value| value.parse().ok())
}

/// A small HTTP surface, so a Stream Deck button can be a plain POST.
///
/// # Reading the body properly
///
/// TCP is a stream, not a sequence of messages, and there is no guarantee that
/// a request arrives in one segment — .NET's HTTP client routinely sends the
/// headers and the body as separate writes. A single `stream.read()` would see
/// only the headers, find an empty body, treat the request as `get_state`, and
/// reply **`ok: true` with a full, correct-looking state** while the client's
/// actual command was discarded without a trace.
///
/// So the body is read to the length the headers declare, and a request that
/// declares a body we cannot read is an **error** rather than a silent state
/// dump.
fn http_request(
    stream: &mut TcpStream,
    head: &str,
    core_tx: Sender<CoreEvent>,
    token: String,
) -> std::io::Result<()> {
    // `head` is a PEEK, so every byte it shows is still sitting in the socket.
    // Reading the body out of it would count the same bytes twice. It is useful
    // only for the caller's "is this a WebSocket upgrade" test.
    let _ = head;

    // Nothing here should be able to hold a thread open. A client that promises
    // a body and never sends it is bounded by this rather than by patience.
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));

    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];

    // 1. Read until the headers are complete.
    let header_end = loop {
        if let Some(position) = find(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        if buffer.len() > MAX_HTTP_BODY {
            break buffer.len();
        }
        match stream.read(&mut chunk) {
            Ok(0) => break buffer.len(),
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            Err(_) => break buffer.len(),
        }
    };

    let headers = String::from_utf8_lossy(&buffer[..header_end.min(buffer.len())]).to_string();
    let declared = content_length(&headers).unwrap_or(0).min(MAX_HTTP_BODY);

    // 2. Read until the declared body has arrived. This is the whole fix: a
    //    single read is not enough, because TCP is a stream and clients
    //    routinely write the headers and the body separately.
    while buffer.len().saturating_sub(header_end) < declared {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }

    let raw = String::from_utf8_lossy(&buffer[header_end.min(buffer.len())..]);
    let body = raw.trim();

    let reply = if declared > 0 && body.is_empty() {
        // A body was promised and did not arrive. Saying so is the point of
        // this rewrite: answering `get_state` here is what made a dropped
        // command look like a success.
        Reply::err(
            None,
            ErrorCode::BadRequest,
            "Content-Length promised a body that did not arrive",
        )
    } else if body.is_empty() {
        // A GET with no body is treated as "tell me everything", which makes
        // the server pleasant to poke at with a browser or curl while
        // developing.
        let probe = format!(r#"{{"command":"get_state","token":"{token}"}}"#);
        dispatch(&probe, 0, &core_tx, &token)
    } else {
        dispatch(body, 0, &core_tx, &token)
    };

    let json = serde_json::to_string(&reply).unwrap_or_else(|_| "{}".into());
    let status = if reply.ok {
        "200 OK"
    } else {
        "400 Bad Request"
    };

    write!(
        stream,
        "HTTP/1.1 {status}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{json}",
        json.len()
    )
}

/// Write one event to a client. `false` means the connection is gone.
fn push(socket: &mut tungstenite::WebSocket<TcpStream>, event: &Event) -> bool {
    let Ok(text) = serde_json::to_string(event) else {
        // A message that will not serialise is this side's bug, and dropping
        // the connection over it would turn a cosmetic fault into an outage.
        return true;
    };

    socket.send(tungstenite::Message::Text(text.into())).is_ok()
}

/// Parse, authenticate, and route one request to the core.
fn dispatch(text: &str, conn: u64, core_tx: &Sender<CoreEvent>, token: &str) -> Reply {
    let request: Request = match serde_json::from_str(text) {
        Ok(r) => r,
        Err(e) => return Reply::err(None, ErrorCode::BadRequest, e.to_string()),
    };

    // Checked on every request, not only at connect: it is cheap, and it means
    // a client cannot hold a connection open across a token change.
    if request.token.as_deref() != Some(token) {
        return Reply::err(
            request.id,
            ErrorCode::Unauthorised,
            "missing or incorrect token; it is in config.json",
        );
    }

    let (reply_tx, reply_rx) = channel();

    if core_tx
        .send(CoreEvent::Request {
            id: conn,
            request,
            reply: reply_tx,
        })
        .is_err()
    {
        return Reply::err(None, ErrorCode::AudioError, "core is shutting down");
    }

    reply_rx
        .recv()
        .unwrap_or_else(|_| Reply::err(None, ErrorCode::AudioError, "core did not answer"))
}

#[cfg(test)]
mod tests {
    use super::browser_refusal;

    fn request(headers: &str) -> String {
        format!("GET / HTTP/1.1\r\n{headers}\r\n\r\n")
    }

    #[test]
    fn programs_are_let_in() {
        // What the window, the Stream Deck plugin and curl send.
        assert_eq!(browser_refusal(&request("Host: 127.0.0.1:8477\r\nUpgrade: websocket")), None);
        assert_eq!(browser_refusal(&request("Host: localhost:8477")), None);
        assert_eq!(browser_refusal(&request("Host: [::1]:8477")), None);
        assert_eq!(browser_refusal(&request("Content-Length: 0")), None);
    }

    #[test]
    fn web_pages_are_refused() {
        // Any Origin at all, including a page's own "null".
        assert!(browser_refusal(&request("Host: 127.0.0.1:8477\r\nOrigin: https://example.com")).is_some());
        assert!(browser_refusal(&request("Host: 127.0.0.1:8477\r\norigin: null")).is_some());
        // DNS rebinding: same-origin, so no Origin, but a hostile Host.
        assert!(browser_refusal(&request("Host: evil.example:8477")).is_some());
        assert!(browser_refusal(&request("Host: 127.0.0.1.evil.example")).is_some());
    }

    #[test]
    fn only_the_headers_are_read() {
        // "Origin:" in a body is a body, not a header.
        let post = "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n{\"note\":\"Origin: x\"}";
        assert_eq!(browser_refusal(post), None);
    }
}
