//! Running the product: tray, API and watcher together.
//!
//! # Threading
//!
//! The tray needs a Windows message pump, and a message pump owns its thread.
//! So the tray runs on the main thread and the core — COM, audio state, the
//! API — runs on another. They talk over a channel.
//!
//! Same split as the API server, for the same reason: every Windows audio call
//! must stay on the thread that initialised the COM apartment.

use std::sync::mpsc::channel;

use crate::audit;
use crate::autostart;
use crate::config::Config;
use crate::engine;
use crate::instance;
use crate::tray::{self, TrayCommand};

pub fn run() -> i32 {
    run_with(false)
}

/// Run the app.
///
/// `force_minimised` is what `--minimised` passes: start-on-boot registers that
/// flag, and it must win over the saved setting so booting never surprises the
/// user with a window.
pub fn run_with(force_minimised: bool) -> i32 {
    run_full(force_minimised, WhenRunning::ShowWindow)
}

/// What a second copy should do when it finds the first already running.
pub enum WhenRunning {
    /// Ask the running core to show its mixer. What a person clicking a
    /// shortcut means.
    ShowWindow,
    /// Nothing at all. What `--ensure-core` means.
    DoNothing,
}

/// Start the core if it is not already running, and never open a window.
///
/// # Why this exists, and why it is not `--minimised`
///
/// The mixer window is a separate executable and a pure API client, so a
/// person who pins *it* to the taskbar - which is what Windows offers when you
/// pin the window you are looking at - gets a mixer with nothing behind it. It
/// connects to no core, shows no devices, and looks broken.
///
/// The window therefore starts the core itself when it cannot find one. It
/// cannot use `--minimised` to do it: that path answers "already running" by
/// asking the core to **show its window**, so a race between the window's
/// connect attempt and the core coming up would open a second mixer.
///
/// So: start it, or leave it alone. Never open anything.
pub fn ensure_core() -> i32 {
    run_full(true, WhenRunning::DoNothing)
}

fn run_full(force_minimised: bool, when_running: WhenRunning) -> i32 {
    let _lock = match instance::acquire() {
        instance::Acquired::Yes(lock) => lock,
        instance::Acquired::AlreadyRunning => {
            // Relaunching shows the running instance's window rather than
            // refusing, which is what somebody who just clicked a Start Menu
            // shortcut expects. The core is already listening on loopback, so
            // the second copy asks it and leaves.
            //
            // Success, not an error: the intent was "show me the mixer", and
            // that is what happened.
            if matches!(when_running, WhenRunning::DoNothing) {
                return 0;
            }

            if crate::api::oneshot::show_window() {
                return 0;
            }

            // Running but not answering. Separating the two cases is the whole
            // point of having a message at all.
            eprintln!("Lanes is already running, but did not respond.");
            eprintln!("Open it from the tray icon, or quit it there and start again.");
            return 1;
        }
    };

    // The safety snapshot comes before everything: nothing that changes a
    // persisted audio setting is reachable except through this path.
    match crate::snapshot::ensure_first_run() {
        Ok(Some(path)) => {
            println!("First run. Captured this machine's audio settings before any change:");
            println!("  {}\n", path.display());
        }
        Ok(None) => {}
        Err(e) => {
            eprintln!("Could not capture the first-run snapshot: {e}");
            eprintln!("Refusing to continue: this app must be able to record what it");
            eprintln!("found before it is allowed to change anything.");
            return 1;
        }
    }

    let config = Config::load();

    // Refuse rather than run empty.
    //
    // Starting here would mean a tray icon, a mixer window and an API that all
    // work perfectly and apply none of the user's rules - the most convincing
    // possible way to look fine while doing nothing. `Config::load` has already
    // explained what it could not open.
    if !config.readable {
        eprintln!();
        eprintln!("Lanes has not started.");
        eprintln!("Close anything that might be holding the file open and try again.");
        return 1;
    }

    let settings = config.settings.clone();

    // Correct a stale start-on-boot path before anything else.
    //
    // This is a portable executable people are expected to move. Once moved,
    // the registered path points at nothing and start-on-boot silently stops
    // working — which the user would discover weeks later, if ever.
    if let Some(old) = autostart::reconcile(settings.start_minimised) {
        println!("Corrected a stale start-on-boot path.\n  was: {old}");
        audit::change(
            "<app>",
            "autostart-path",
            Some(old),
            std::env::current_exe()
                .ok()
                .map(|p| p.display().to_string()),
            "stale-path-correction",
        );
    }

    let toggles = tray::Toggles {
        // The registry is the source of truth for whether this is actually on;
        // the config records what the user last asked for.
        start_with_windows: autostart::is_enabled(),
        start_minimised: settings.start_minimised,
        close_to_tray: settings.close_to_tray,
    };

    let muted = config.channel("master").map(|m| m.muted).unwrap_or(false);

    let handles = match tray::build(
        muted,
        toggles.start_with_windows,
        toggles.start_minimised,
        toggles.close_to_tray,
    ) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Could not create the tray icon: {e}");
            return 1;
        }
    };

    // Remember which thread the message loop will run on, before anything
    // else is started. A quit arriving over the API is handled on the core
    // thread and has to be able to wake this one. See `shutdown`.
    crate::shutdown::remember_main_thread();

    let (commands_tx, commands_rx) = channel::<TrayCommand>();
    let startup_tx = commands_tx.clone();

    // The core, on its own thread with its own COM apartment.
    //
    // This runs the API SERVER, which owns the config, the session watcher and
    // every client. Tray clicks are handed to it over `commands_rx` rather than
    // being handled here, so there is exactly one owner of the config — two
    // independent load-modify-save cycles would race and silently lose changes.
    let core = std::thread::spawn(move || {
        let _com = match winaudio::com::initialize() {
            Ok(guard) => guard,
            Err(e) => {
                eprintln!("COM initialisation failed on the core thread: {e}");
                return;
            }
        };
        crate::api::serve(None, Some(commands_rx));
    });

    // Open the window unless the user asked to start minimised.
    //
    // Double-clicking the executable should give you the app, not a command
    // line. The flags are for scripting and for what start-on-boot registers.
    if !settings.start_minimised && !force_minimised {
        // Asked for through the core rather than spawned here.
        //
        // This looks like a detour and is not. The core keeps the window's
        // `Child` so it can refuse to stack a second one and can close it on
        // quit; a window started behind its back is invisible to both, and a
        // relaunch would then open a second window onto the same mixer. There
        // is one path that opens a window, and everything uses it.
        let _ = startup_tx.send(TrayCommand::ShowWindow);
    }

    println!("Lanes is running in the tray.\n");
    println!("  Click          the quick mixer");
    println!("  Double-click   the mixer window");
    println!("  Right-click    menu\n");
    println!("Closing the window does NOT quit. Quit lives in the tray Settings menu.");

    // Blocks until Quit.
    tray::run_message_loop(handles, commands_tx, toggles);

    let _ = core.join();
    println!("Lanes has exited.");
    0
}

/// Launch the mixer window as a child process.
///
/// # Why a process and not a thread
///
/// The window is a pure client of the local API with no privileged knowledge.
/// Inside one process that is a rule kept by discipline, and a single
/// `use crate::engine` for something the API does not expose yet would quietly
/// end it. Across a process boundary it is a fact: if the window needs
/// something, the only way to give it that is to add it to the API.
///
/// It also means closing the window destroys it and returns its memory, rather
/// than hiding it. A process that exits does that completely.
fn open_window(existing: &mut Option<std::process::Child>) {
    // Do not stack windows. A window already open is the one the user wants;
    // spawning a second would give two live clients of the same core, both
    // correct and both confusing.
    // `try_wait` returning Ok(None) means it is still running. Anything else
    // means it has gone, so fall through and start a new one.
    if let Some(Ok(None)) = existing.as_mut().map(|c| c.try_wait()) {
        println!("[Open] the window is already open.");
        return;
    }

    match spawn_window(&[]) {
        Ok(child) => {
            *existing = Some(child);
            println!("[Open] window opened.");
        }
        Err(e) => eprintln!("[Open] could not open the window: {e}"),
    }
}

/// Ask every top-level window of a process to close, as its X button would.
fn close_windows_of(process_id: u32) {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
    };

    unsafe extern "system" fn each(hwnd: HWND, wanted: LPARAM) -> BOOL {
        let mut owner = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut owner));
            if owner == wanted.0 as u32 && IsWindowVisible(hwnd).as_bool() {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(process_id as isize));
    }
}

/// The windows this core has opened, kept so neither is ever stacked.
#[derive(Default)]
pub struct Windows {
    /// The full mixer.
    pub mixer: Option<std::process::Child>,
    /// The tray's quick mixer.
    pub flyout: Option<std::process::Child>,
    /// The settings window.
    pub settings: Option<std::process::Child>,
    /// The new-application notice, if one is showing.
    pub arrival: Option<std::process::Child>,
}

/// Whether the user is in something a notice must not interrupt: a full-screen
/// game or video, a presentation, Focus / quiet hours, or a locked screen.
///
/// Windows' own answer (`SHQueryUserNotificationState`), the one its toasts
/// respect. Anything but "accepts notifications" holds the notice back, because
/// a window appearing over a game is an interruption, however small. If Windows
/// cannot say, the notice is shown.
pub fn user_is_busy() -> bool {
    use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_ACCEPTS_NOTIFICATIONS};
    match unsafe { SHQueryUserNotificationState() } {
        Ok(state) => state != QUNS_ACCEPTS_NOTIFICATIONS,
        Err(_) => false,
    }
}

/// Show the notice that an application with no channel has started playing.
///
/// One at a time: a newer arrival replaces an older notice rather than
/// stacking beside it, because two notices in the corner is clutter and the
/// older one is still waiting in "To be routed" anyway.
pub fn open_arrival(windows: &mut Windows, executable: &str) {
    if let Some(child) = windows.arrival.as_mut() {
        if let Ok(None) = child.try_wait() {
            close_windows_of(child.id());
        }
    }

    match spawn_window(&["--arrival", executable]) {
        Ok(child) => windows.arrival = Some(child),
        Err(e) => eprintln!("[New app] could not show the notice: {e}"),
    }
}

/// Open the quick mixer beside the tray.
///
/// # Why a toggle needs to know when the flyout closed
///
/// A flyout closes itself the moment it loses focus, as every notification-area
/// flyout does. Clicking the tray icon to close it therefore closes it *before*
/// the click reaches us - and then the click arrives, sees no flyout, and opens
/// a new one. The icon could never close what it opened.
///
/// The click is also late on purpose: the tray holds a single click for the
/// system double-click interval, to see whether it becomes a double-click (see
/// `tray::IconClick`). So "the flyout exited within that interval, plus a
/// margin" is exactly the signature of "this click is what closed it", and the
/// right response is to do nothing.
///
/// The exit time comes from Windows (`GetProcessTimes`) rather than from when
/// we happened to notice, because nothing here waits on the child.
fn open_flyout(existing: &mut Option<std::process::Child>, toggle: bool) {
    if let Some(child) = existing.as_mut() {
        match child.try_wait() {
            // Still open. A toggle closes it; the menu item leaves it be.
            Ok(None) => {
                if toggle {
                    let _ = child.kill();
                    let _ = child.wait();
                    *existing = None;
                }
                return;
            }
            Ok(Some(_)) => {
                let closed_by_this_click = toggle && exited_within(child, toggle_grace());
                *existing = None;
                if closed_by_this_click {
                    return;
                }
            }
            Err(_) => *existing = None,
        }
    }

    match spawn_window(&["--flyout"]) {
        Ok(child) => *existing = Some(child),
        Err(e) => eprintln!("[Quick mixer] could not open it: {e}"),
    }
}

/// How recently the flyout must have closed for a click to count as the one
/// that closed it. The double-click interval the click was held for, plus
/// enough for the flyout's process to exit after losing focus.
fn toggle_grace() -> std::time::Duration {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;

    std::time::Duration::from_millis(unsafe { GetDoubleClickTime() } as u64 + 400)
}

/// Whether an exited child exited within `within` of now.
fn exited_within(child: &std::process::Child, within: std::time::Duration) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{FILETIME, HANDLE};
    use windows::Win32::System::Threading::GetProcessTimes;

    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );

    let ok = unsafe {
        GetProcessTimes(
            HANDLE(child.as_raw_handle()),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    }
    .is_ok();

    if !ok {
        return false;
    }

    // FILETIME counts 100ns intervals since 1601; the Unix epoch is this many
    // of them later.
    const UNIX_EPOCH_AS_FILETIME: u64 = 116_444_736_000_000_000;
    let ticks = ((exited.dwHighDateTime as u64) << 32) | exited.dwLowDateTime as u64;
    let Some(since_unix) = ticks.checked_sub(UNIX_EPOCH_AS_FILETIME) else {
        return false;
    };
    let exited_at = std::time::UNIX_EPOCH + std::time::Duration::from_nanos(since_unix * 100);

    std::time::SystemTime::now()
        .duration_since(exited_at)
        .is_ok_and(|age| age <= within)
}

/// Start the mixer window.
///
/// The window is a separate program, `Lanes.Window.exe`, written in C# and WPF
/// and installed beside this one. The same executable serves the mixer, the
/// quick mixer (`--flyout`), Settings (`--settings`) and the new-app notice
/// (`--arrival <exe>`).
///
/// **This is the only place that knows where the window is**, so there is one
/// thing to change if that ever moves.
///
/// A missing window is reported rather than swallowed. It means a broken
/// install — the two files ship together — and the tray's "Open" doing nothing
/// at all, with no explanation, is the worst way to find that out.
fn spawn_window(args: &[&str]) -> std::io::Result<std::process::Child> {
    let exe = std::env::current_exe()?;

    let window = exe
        .parent()
        .map(|dir| dir.join(WINDOW_EXE))
        .filter(|path| path.is_file())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{WINDOW_EXE} is not beside {}", exe.display()),
            )
        })?;

    // Let whatever we start take the foreground. We are allowed to, because
    // the tray icon was just clicked; the window we start is not, until we say
    // so. Without this the flyout can open behind other windows and without
    // focus - and a flyout that never had focus never closes on losing it.
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }

    std::process::Command::new(window).args(args).spawn()
}

/// The mixer window's file name, which ships beside the core.
pub const WINDOW_EXE: &str = "Lanes.Window.exe";

/// Apply one tray command. Called by the API core loop, which owns the config.
///
/// Returns `true` when the user chose Quit.
pub fn apply_tray_command(
    config: &mut Config,
    command: TrayCommand,
    windows: &mut Windows,
) -> bool {
    match command {
        TrayCommand::ShowWindow => open_window(&mut windows.mixer),

        // A hotkey: open the mixer, or close it if it is open. Closed by asking
        // it to close rather than by ending the process, so it saves where it
        // was, exactly as clicking its X does.
        TrayCommand::ToggleWindow => match windows.mixer.as_mut().map(|c| c.try_wait()) {
            Some(Ok(None)) => {
                if let Some(child) = windows.mixer.as_ref() {
                    close_windows_of(child.id());
                }
            }
            _ => open_window(&mut windows.mixer),
        },

        // Handled by the core loop before it gets here; listed so the match
        // stays exhaustive and says so.
        TrayCommand::Hotkey(_) | TrayCommand::HotkeysRegistered(_) => {}

        TrayCommand::ShowFlyout => open_flyout(&mut windows.flyout, false),

        // Not stacked, for the same reason the mixer is not: two settings
        // windows editing one list would each overwrite the other.
        TrayCommand::ShowSettings => {
            if !matches!(windows.settings.as_mut().map(|c| c.try_wait()), Some(Ok(None))) {
                match spawn_window(&["--settings"]) {
                    Ok(child) => windows.settings = Some(child),
                    Err(e) => eprintln!("[Settings] could not open it: {e}"),
                }
            }
        }
        TrayCommand::ToggleFlyout => open_flyout(&mut windows.flyout, true),

        TrayCommand::ToggleMuteAll => {
            // Mute-all is Master's mute, not every channel's. Muting each
            // channel individually would destroy the per-channel mute states
            // the user set, and unmuting could not restore them.
            let Some(master) = config.channel_mut("master") else {
                return false;
            };
            master.muted = !master.muted;
            let now = master.muted;

            if let Err(e) = config.save() {
                eprintln!("Could not save: {e}");
                return false;
            }

            crate::master::push(config, "tray:mute_all");
            let _ = engine::apply_all(config, "tray:mute_all");
            println!("[Mute all] {}", if now { "muted" } else { "unmuted" });
        }

        TrayCommand::ActivateProfile(name) => match crate::profiles::activate(config, &name) {
            Ok(name) => {
                if let Err(e) = config.save() {
                    eprintln!("Could not save: {e}");
                    return false;
                }

                // A full apply: a profile can move channels between devices.
                crate::master::push(config, "tray:activate_profile");
                let _ = engine::apply_all(config, "tray:activate_profile");
                println!("[Profile] {name}");
            }
            Err(refusal) => eprintln!("Could not switch profile: {refusal}"),
        },

        TrayCommand::SetStartWithWindows(on) => {
            let minimised = config.settings.start_minimised;

            let result = if on {
                autostart::enable(minimised)
            } else {
                autostart::disable()
            };

            match result {
                Ok(()) => {
                    config.settings.start_with_windows = on;
                    let _ = config.save();
                    audit::change(
                        "<app>",
                        "start_with_windows",
                        Some((!on).to_string()),
                        Some(on.to_string()),
                        "tray",
                    );
                    println!("[Start with Windows] {}", if on { "on" } else { "off" });
                }
                Err(e) => eprintln!("Could not change start-on-boot: {e}"),
            }
        }

        TrayCommand::SetStartMinimised(on) => {
            config.settings.start_minimised = on;
            let _ = config.save();

            // The registered command line embeds this flag, so it must be
            // rewritten when the setting changes. Without this the toggle
            // appears to work and does nothing at the next boot.
            if autostart::is_enabled() {
                let _ = autostart::enable(on);
            }

            println!(
                "[Start minimised] {} — launching will {}",
                if on { "on" } else { "off" },
                if on {
                    "go straight to the tray"
                } else {
                    "open the window"
                }
            );
        }

        TrayCommand::SetCloseToTray(on) => {
            config.settings.close_to_tray = on;
            let _ = config.save();
            println!(
                "[Close to tray] {} — X will {}",
                if on { "on" } else { "off" },
                if on { "close the window only" } else { "quit" }
            );
        }

        // Confirmed on the tray thread before it got here (`tray::confirm_restore`).
        TrayCommand::RestoreWindowsSettings => match crate::restore::reset_windows(config) {
            Ok(report) => {
                report.print();
                crate::master::pull(config);
                if let Err(e) = config.save() {
                    eprintln!("Could not save: {e}");
                }
            }
            Err(e) => eprintln!("Restore failed: {e}"),
        },

        TrayCommand::Resume => {
            crate::restore::resume(config);
            if let Err(e) = config.save() {
                eprintln!("Could not save: {e}");
                return false;
            }
            crate::master::push(config, "tray:resume");
            let _ = engine::apply_all(config, "tray:resume");
            println!("[Resume] Lanes is managing applications again.");
        }

        TrayCommand::Quit => {
            println!("[Quit] shutting down.");

            // Close the window we opened. It is a pure client with no state of
            // its own, so there is nothing to lose - but its executable stays
            // locked while it runs, and an uninstaller cannot delete a locked
            // file. Clients also get a `shutdown` event, which is the polite
            // path; this is the backstop for a window that did not act on it,
            // or one somebody started by hand.
            for child in [
                windows.mixer.as_mut(),
                windows.flyout.as_mut(),
                windows.settings.as_mut(),
                windows.arrival.as_mut(),
            ]
                .into_iter()
                .flatten()
            {
                let _ = child.kill();
                let _ = child.wait();
            }

            // Wake the message loop, which is on another thread and is what is
            // actually holding the process open.
            crate::shutdown::request();
            return true;
        }
    }

    false
}

/// Inspect or change start-on-boot from the command line.
///
/// The toggle is in the tray's Settings menu. A command-line path exists as well
/// so the registry behaviour can be checked, or set up by a script, without
/// clicking a menu.
pub fn autostart_command(state: &str) -> i32 {
    let mut config = Config::load();

    match state.to_lowercase().as_str() {
        "status" => {
            match autostart::current() {
                Some(command) => {
                    println!("Start with Windows: ON");
                    println!("  registered: {command}");
                }
                None => println!("Start with Windows: OFF"),
            }
            println!();
            println!("This is the only registry value this app writes:");
            println!(r"  HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Lanes");
            0
        }

        "on" | "true" | "yes" | "1" => match autostart::enable(config.settings.start_minimised) {
            Ok(()) => {
                config.settings.start_with_windows = true;
                let _ = config.save();
                audit::change(
                    "<app>",
                    "start_with_windows",
                    Some("false".into()),
                    Some("true".into()),
                    "cli",
                );
                println!("Start with Windows: ON");
                println!("  registered: {}", autostart::current().unwrap_or_default());
                0
            }
            Err(e) => {
                eprintln!("Could not enable: {e}");
                1
            }
        },

        "off" | "false" | "no" | "0" => match autostart::disable() {
            Ok(()) => {
                config.settings.start_with_windows = false;
                let _ = config.save();
                audit::change(
                    "<app>",
                    "start_with_windows",
                    Some("true".into()),
                    Some("false".into()),
                    "cli",
                );
                println!("Start with Windows: OFF");
                0
            }
            Err(e) => {
                eprintln!("Could not disable: {e}");
                1
            }
        },

        other => {
            eprintln!("Expected on, off or status; got '{other}'.");
            2
        }
    }
}
