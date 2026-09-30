//! The Lumen daemon and its thin client (AD-02).
//!
//! Running `lumen` with no arguments toggles the launcher. If no daemon is
//! listening, this process becomes it. That single behaviour is what makes the
//! Hyprland binding in section 8 -- `bind = ALT, SPACE, exec, lumen` -- work
//! without the compositor knowing anything about daemons.
//!
//! The window is created once, when the daemon starts, and from then on is only
//! shown and hidden (I-4). Creating it per invocation would give back exactly
//! the GPU initialisation cost that AD-02 exists to eliminate.

// A launcher summoned by a keystroke has no console to print to, and a console
// window flashing on every boot is the most visible possible defect.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::AppContext as _;
use gpui::{
    App, Application, Bounds, Entity, Render, Window, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions, point, px, size,
};
use lumen_core::clipboard::{History, Payload, Retention};
use lumen_core::sheets::{Sheet, SheetLibrary};
use lumen_core::theme::Appearance;
use lumen_core::{Builtin, Config, Paths, Theme};
use lumen_platform::{
    ClipboardSink, FocusOwner, GestureEvent, GestureSink, HoldPhase, MonitorInfo, Platform, Rect,
    WindowRole,
};
use lumen_ui::{Launcher, LauncherEvent, Overlay, OverlayEvent, Settings, SettingsEvent, keymap};

mod clipboard;
mod currency;
mod dictionary;
mod extensions;
mod ipc;
mod snippets;
mod timers;
mod timing;
mod translate;

use ipc::Message;

fn main() -> Result<()> {
    // First, before anything else: the cold-start budget is measured from here,
    // and initialising the logger is already part of what the user waits for.
    timing::mark_process_start();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("lumen=info,warn"))
        .init();

    match std::env::args().nth(1).as_deref() {
        None | Some("toggle") => dispatch(Message::Toggle),
        Some("show") => dispatch(Message::Show),
        Some("hide") => forward_only(Message::Hide),
        // Hyprland's `bind` and `bindr` pair (section 8). Two thin-client
        // invocations, two socket messages, and the daemon measures the
        // duration -- no input capture and no special permissions.
        Some("overlay-press") => forward_only(Message::OverlayPress),
        Some("overlay-release") => forward_only(Message::OverlayRelease),
        Some("settings") => dispatch(Message::Settings),
        Some("timers") => dispatch(Message::ShowTimers),
        Some("reload" | "reload-sheets") => forward_only(Message::ReloadSheets),
        Some("quit") => forward_only(Message::Quit),
        Some("timer") => start_timer(),
        Some("--version" | "-V") => {
            println!("lumen {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some(unknown) => {
            eprintln!("lumen: unrecognised argument {unknown:?}\n");
            print_help();
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "lumen - a native command palette\n\n\
         USAGE:\n    \
         lumen [COMMAND]\n\n\
         COMMANDS:\n    \
         toggle             Show or hide the launcher (the default)\n    \
         show               Show the launcher\n    \
         hide               Hide the launcher\n    \
         overlay-press      Report that the hold gesture began\n    \
         overlay-release    Report that the hold gesture ended\n    \
         settings           Open the settings window\n    \
         timers             Show the running timers, or hide them\n    \
         reload             Re-read the configuration, sheets, scripts, snippets and quicklinks\n    \
         timer DURATION [NAME]  Start a timer in the running daemon: `lumen timer 25m tea`\n    \
         quit               Stop the running daemon\n\n\
         With no running daemon, `toggle`, `show` and `settings` start one."
    );
}

/// `lumen timer 25m tea`: the same grammar as the launcher, from a terminal.
///
/// Forwarded only. Starting a daemon here would drop the timer on the floor,
/// since a new daemon does not replay the message that started it -- so
/// having nothing to send to is said, not swallowed.
fn start_timer() -> Result<()> {
    let typed: Vec<String> = std::env::args().skip(2).collect();
    let Some(typed) = lumen_core::timers::duration::parse(&typed.join(" ")) else {
        eprintln!("lumen: expected a duration such as 25m, 1h30 or 1:30, then an optional name");
        std::process::exit(2);
    };
    let command = lumen_core::timers::Command::Start {
        seconds: typed.duration.as_secs(),
        name: typed.name,
        focus: false,
    };
    if !ipc::send(Message::Timer(command))? {
        eprintln!("lumen: no daemon is running; start one with `lumen`");
        std::process::exit(1);
    }
    Ok(())
}

/// Send `message`; if nothing is listening, become the daemon.
fn dispatch(message: Message) -> Result<()> {
    if ipc::send(message.clone())? {
        log::debug!("handed {message:?} to the running daemon");
        return Ok(());
    }
    run_daemon()
}

/// Send `message` and do nothing if there is no daemon.
///
/// Starting a whole launcher because someone released a key would be absurd.
fn forward_only(message: Message) -> Result<()> {
    if !ipc::send(message.clone())? {
        log::debug!("no daemon is running; {message:?} dropped");
    }
    Ok(())
}

/// Everything the shell needs to remember between events.
///
/// Lives on the UI thread inside an `Rc<RefCell<_>>`: both the gesture task and
/// the launcher's event subscription run there, so no lock is involved.
struct Shell {
    visible: bool,
    /// The window that held the foreground when we appeared, so it can be given
    /// back exactly (an F0 acceptance criterion).
    restore_to: Option<FocusOwner>,
    /// The monitor the launcher was shown on. Remembered rather than re-queried,
    /// so a resize mid-search cannot teleport the window to wherever the mouse
    /// has since wandered.
    monitor: Option<MonitorInfo>,
    /// Whether the overlay's window is currently mapped.
    ///
    /// Distinct from whether it is *visible*: it stays mapped throughout the
    /// fade-out, and is hidden only once the fade reports it has finished.
    overlay_shown: bool,
}

fn run_daemon() -> Result<()> {
    let paths = Paths::discover()?;
    paths.ensure_exists()?;
    lumen_core::install_builtin_themes(&paths.themes_dir())?;
    lumen_core::install_builtin_sheets(&paths.sheets_dir())?;
    lumen_core::install_example_script(&paths.scripts_dir())?;
    lumen_core::install_example_extensions(&paths.extensions_dir())?;
    lumen_core::snippets::install_example(&paths.snippets_file())?;
    lumen_core::providers::quicklinks::install_example(&paths.quicklinks_file())?;
    Config::write_default(&paths.config_file())?;

    let config = Config::load(&paths.config_file());
    let theme = Theme::load(&paths.theme_file(&config.theme)).unwrap_or_else(|error| {
        log::warn!("could not load theme {:?}: {error:#}", config.theme);
        Theme::default()
    });

    log::info!(
        "starting the daemon; hotkey {}, theme {:?}",
        config.hotkey,
        theme.name
    );

    // The icon set and the font are compiled in, so there is no folder to
    // find at run time and nothing to go missing between install and use.
    Application::new()
        .with_assets(lumen_ui::Assets)
        .run(move |cx: &mut App| {
            if let Err(error) = start(cx, paths, config, theme) {
                log::error!("could not start: {error:#}");
                cx.quit();
            }
        });

    Ok(())
}

fn start(cx: &mut App, paths: Paths, config: Config, theme: Theme) -> Result<()> {
    lumen_ui::fonts::register(cx);
    keymap::install(cx);
    keymap::verify_against_registry(cx);

    let platform = build_platform(&config, &theme);

    let window = open_hidden_window(
        cx,
        &platform,
        WindowRole::Launcher,
        &config,
        size(
            px(config.window.width),
            px(config.window_height(config.window.visible_rows)),
        ),
        {
            let platform = platform.clone();
            let paths = paths.clone();
            let config = config.clone();
            let theme = theme.clone();
            move |_window, cx| cx.new(|cx| Launcher::new(platform, paths, config, theme, cx))
        },
    )?;

    // The sheets the overlay will draw from, assembled once at startup.
    //
    // Lumen's own sheet is generated from the keymap rather than written by
    // hand (F3), and the built-in global sheet is the last rung of the fallback
    // chain, compiled in so the guarantee that the overlay is never empty does
    // not depend on a file existing.
    let mut sheets = SheetLibrary::load(&paths.sheets_dir());
    sheets.add(keymap::sheet());
    let fallback = lumen_core::sheets::builtin_global();
    log::info!("loaded {} shortcut sheets", sheets.len());

    let overlay = open_hidden_window(
        cx,
        &platform,
        WindowRole::Overlay,
        &config,
        size(
            px(fallback.measured_width(lumen_core::layout::overlay::COLUMNS)),
            px(fallback.measured_height(lumen_core::layout::overlay::COLUMNS)),
        ),
        {
            let theme = theme.clone();
            let fallback = fallback.clone();
            move |_window, cx| cx.new(|_cx| Overlay::new(theme, fallback))
        },
    )?;

    // Copied out before `Surfaces` takes ownership, for the warm-up below.
    let overlay_handle = overlay;

    let shell = Rc::new(RefCell::new(Shell {
        visible: false,
        restore_to: None,
        monitor: None,
        overlay_shown: false,
    }));

    // One channel for both sources. The gesture listener and the IPC listener
    // are different threads, and both must do nothing but push (I-15).
    let (tx, mut rx) = mpsc::unbounded::<Message>();

    let gesture_tx = tx.clone();
    let listening = platform.start_gesture_listener(
        config.hotkey.clone(),
        config.timers.hotkey.clone(),
        GestureSink::new(move |event| {
            let message = match event {
                GestureEvent::Invoke => Message::Toggle,
                GestureEvent::Hold(HoldPhase::Began) => Message::OverlayPress,
                GestureEvent::Hold(HoldPhase::Ended) => Message::OverlayRelease,
                GestureEvent::Timers => Message::ShowTimers,
            };
            // Unbounded and non-blocking: this runs on the input thread.
            let _ = gesture_tx.unbounded_send(message);
        }),
    );

    // A hotkey conflict must not stop the daemon. Global hotkeys are first come,
    // first served on Windows, so the key being taken says nothing about whether
    // this launcher works -- it is still reachable over the socket, which is how
    // the Wayland path drives it anyway. Refusing to start would turn "pick a
    // different key" into "the application is broken", with the explanation only
    // visible to someone who thought to run it from a terminal.
    if let Err(error) = listening {
        log::error!(
            "could not register {}: {error:#}\n         \
             Another application already holds that combination. Lumen is running \
             and can still be summoned with `lumen toggle`; change \"hotkey\" in {} \
             and restart to bind a different one.",
            config.hotkey,
            paths.config_file().display()
        );
    }

    if config.clipboard.enabled {
        start_clipboard_history(&platform, &paths, config.clipboard.retention, &window, cx);
    } else {
        log::info!("clipboard history is turned off in the configuration");
    }

    // The one place a deliberately invisible process admits to running.
    //
    // Onto the same queue as everything else: the tray menu, `lumen settings`
    // in a terminal and "Configure Shortcuts" in the list all arrive at the
    // same code by the same route, in one order.
    let tray_tx = tx.clone();
    let tray = platform.install_tray(
        lumen_platform::TrayInfo {
            // The hotkey, because "is it running and what do I press" is one
            // question and this is the only place to answer it. Taken from the
            // configuration rather than written out, so it stays true when the
            // user changes it.
            tooltip: format!("Lumen — press {} to open", config.hotkey),
        },
        lumen_platform::TraySink::new(move |command| {
            let message = match command {
                lumen_platform::TrayCommand::Toggle => Message::Toggle,
                lumen_platform::TrayCommand::Settings => Message::Settings,
                lumen_platform::TrayCommand::Reload => Message::ReloadSheets,
                lumen_platform::TrayCommand::Quit => Message::Quit,
            };
            let _ = tray_tx.unbounded_send(message);
        }),
    );

    // Not fatal. A daemon without a tray icon is harder to find and works
    // exactly as well, and refusing to start over a missing icon would be the
    // tail wagging the dog.
    if let Err(error) = tray {
        log::warn!("no notification area icon: {error:#}");
    }

    let ipc_tx = tx.clone();
    ipc::serve(move |message| {
        let _ = ipc_tx.unbounded_send(message);
    })
    .context("starting the IPC listener")?;

    // Installing or removing an application re-indexes, rather than waiting
    // for the next restart. Through the queue like everything else: the
    // platform's watch thread only pushes.
    let apps_tx = tx.clone();
    let watching = platform.watch_applications(lumen_platform::ApplicationsSink::new(move || {
        let _ = apps_tx.unbounded_send(Message::ApplicationsChanged);
    }));
    if let Err(error) = watching {
        log::warn!(
            "newly installed applications will appear after `lumen reload` or a restart: {error:#}"
        );
    }

    // The configuration as it is *now*. Everything that runs after startup
    // reads it from here rather than holding the copy taken at launch, which
    // is what lets an edit to config.json apply without a restart.
    let live = Rc::new(RefCell::new(config.clone()));
    watch_config(cx, &paths, tx.clone());

    // Before the launcher's events are heard: a refusal (F8) is the
    // timers' to show. A timer that ended while the daemon was not running
    // is reported now, not when the next one ends.
    let timers = timers::Timers::start(
        platform.clone(),
        window,
        paths.timers_file(),
        paths.config_file(),
        &config.timers,
        &cx.to_async(),
    );

    subscribe_to_launcher(
        cx,
        &window,
        platform.clone(),
        live.clone(),
        shell.clone(),
        tx.clone(),
        timers.clone(),
    );
    subscribe_to_overlay(cx, &overlay, platform.clone(), shell.clone());
    let reload_theme = watch_theme(cx, &window, &overlay, &paths, live.clone());

    let surfaces = Surfaces {
        launcher: window,
        overlay,
        settings: Rc::new(RefCell::new(None)),
        sheets: Rc::new(RefCell::new(sheets)),
        fallback: Rc::new(fallback),
        paths: paths.clone(),
        theme: Rc::new(RefCell::new(theme.clone())),
        config: live,
        reload_theme,
        timers,
    };

    watch_sheets(cx, &surfaces);
    watch_quicklinks(cx, &surfaces);

    cx.spawn({
        let platform = platform.clone();
        async move |cx| {
            while let Some(message) = rx.next().await {
                let described = format!("{message:?}");
                let outcome = handle(message, &platform, &shell, &surfaces, cx);
                if let Err(error) = outcome {
                    log::error!("could not handle {described}: {error:#}");
                }
            }
        }
    })
    .detach();

    // Everything a summons needs now exists: the window, the keymap, the
    // gesture listener and the loop that reads from both.
    timing::report_ready();

    warm_the_renderer(platform, overlay_handle, cx);

    Ok(())
}

/// Draw one frame nobody sees, so the first frame somebody does see is fast.
///
/// The first frame a GPUI process paints costs far more than every frame after
/// it: pipelines are created, shaders compiled, and the glyph atlas filled with
/// the first run of text. Measured on this machine the first summons took
/// 101 ms and the second 24 ms, against what was then an 80 ms p95; from the third
/// onwards it is 8.
///
/// So the overlay -- which never activates and never takes focus, which is what
/// makes it usable for this -- is shown once far outside the virtual screen and
/// hidden again. GPUI's renderer state is per process, so the launcher inherits
/// what this frame builds.
///
/// Deliberately *after* `report_ready`, and deliberately not awaited. The
/// cold-start budget is "until the first search responds", the daemon already
/// answers, and paying 100 ms of startup to save 100 ms on a keystroke that may
/// not come for an hour is the wrong trade. A user quick enough to beat it gets
/// exactly what they would have got anyway.
fn warm_the_renderer(
    platform: Arc<dyn Platform>,
    overlay: gpui::WindowHandle<Overlay>,
    cx: &mut App,
) {
    // Far outside any plausible virtual screen, so there is nothing to see even
    // for the one frame it is mapped.
    const OFF_SCREEN: Rect = Rect::new(-30_000, -30_000, 640, 480);

    cx.spawn(async move |cx| {
        if let Err(error) = platform.show_window(WindowRole::Overlay, OFF_SCREEN) {
            log::debug!("could not warm the renderer: {error:#}");
            return;
        }

        // Waited on rather than slept through: a timer would be too short on a
        // slow machine and wasteful on a fast one.
        let (tx, rx) = futures::channel::oneshot::channel();
        let asked = overlay
            .update(cx, |_overlay, window, _cx| {
                window.on_next_frame(move |_window, _cx| {
                    let _ = tx.send(());
                });
                window.refresh();
            })
            .is_ok();

        if asked {
            let _ = rx.await;
        }

        if let Err(error) = platform.hide_window(WindowRole::Overlay, None) {
            log::debug!("could not put the warm-up window away: {error:#}");
        }
        log::debug!("renderer warmed");
    })
    .detach();
}

/// The daemon's windows and the sheets they draw from.
///
/// Grouped rather than threaded through every function as six more parameters,
/// which is what the event loop would otherwise become.
///
/// The sheet library sits behind `RefCell` because saving from the settings
/// window replaces it while the overlay may be holding a borrow of the sheet it
/// is currently drawing.
struct Surfaces {
    launcher: gpui::WindowHandle<Launcher>,
    overlay: gpui::WindowHandle<Overlay>,
    /// Created the first time settings is opened, then kept and hidden.
    settings: Rc<RefCell<Option<gpui::WindowHandle<Settings>>>>,
    sheets: Rc<RefCell<SheetLibrary>>,
    fallback: Rc<Sheet>,
    paths: Paths,
    theme: Rc<RefCell<Theme>>,
    /// The current configuration, replaced when the file changes.
    config: Rc<RefCell<Config>>,
    /// Asks the theme watch to reload, for when `theme` names another file.
    reload_theme: mpsc::UnboundedSender<()>,
    /// The running timers (F8).
    timers: timers::Timers,
}

impl Surfaces {
    /// Re-read the sheets folder and put the generated Lumen sheet back.
    ///
    /// Called after a save, a delete, and whenever the watcher notices someone
    /// editing a file by hand. Rebuilt from scratch rather than patched: the
    /// library is small, and reconciling an in-place edit with a file that may
    /// have been renamed underneath is more ways to be wrong than it is worth.
    fn reload_sheets(&self) {
        let mut library = SheetLibrary::load(&self.paths.sheets_dir());
        library.add(keymap::sheet());
        log::info!("reloaded {} shortcut sheets", library.len());
        *self.sheets.borrow_mut() = library;
    }
}

/// Hide the overlay's window once its fade has actually finished.
fn subscribe_to_overlay(
    cx: &mut App,
    overlay: &gpui::WindowHandle<Overlay>,
    platform: Arc<dyn Platform>,
    shell: Rc<RefCell<Shell>>,
) {
    let Ok(entity) = overlay.entity(cx) else {
        return;
    };

    cx.subscribe(&entity, move |_entity: Entity<Overlay>, event, cx| {
        let OverlayEvent::FadedOut = event;

        let platform = platform.clone();
        let shell = shell.clone();

        // Off the callback before touching a window. See `off_the_callback`.
        off_the_callback(cx, move || {
            // Hiding at the start of the fade would mean there was no fade;
            // hiding here, after it, is what lets the window stop compositing
            // once it is genuinely invisible.
            let mut shell = shell.borrow_mut();
            if !shell.overlay_shown {
                return Ok(());
            }
            shell.overlay_shown = false;

            platform.hide_window(WindowRole::Overlay, None)
        });
    })
    .detach();
}

/// Re-read the sheets folder when anything in it changes.
///
/// Themes have had live reload since F1 (AD-05); sheets get the same treatment
/// for the same reason. Editing a sheet by hand and having to restart the daemon
/// to see it is exactly the friction that stops people iterating.
fn watch_sheets(cx: &mut App, surfaces: &Surfaces) {
    use lumen_core::watcher::DirectoryWatcher;

    let (sink, mut changes) = change_channel();
    let watcher = match DirectoryWatcher::start(&surfaces.paths.sheets_dir(), sink) {
        Ok(watcher) => watcher,
        Err(error) => {
            log::warn!("shortcut sheets will not reload automatically: {error:#}");
            return;
        }
    };

    let sheets = surfaces.sheets.clone();
    let paths = surfaces.paths.clone();

    cx.spawn(async move |cx| {
        // Owned by the task: the watch lasts exactly as long as this loop.
        let _watcher = watcher;
        while next_change(&mut changes, cx).await {
            let mut library = SheetLibrary::load(&paths.sheets_dir());
            library.add(keymap::sheet());
            log::info!("reloaded {} shortcut sheets", library.len());
            *sheets.borrow_mut() = library;
        }
    })
    .detach();
}

/// Re-read the quicklinks when `quicklinks.json` is saved (F6).
///
/// Live, unlike snippets: a quicklink is most often added the moment you want
/// it, and making that wait on `lumen reload` is the friction live reload
/// exists to remove.
fn watch_quicklinks(cx: &mut App, surfaces: &Surfaces) {
    use lumen_core::watcher::FileWatcher;

    let (sink, mut changes) = change_channel();
    let watcher = match FileWatcher::start(&surfaces.paths.quicklinks_file(), sink) {
        Ok(watcher) => watcher,
        Err(error) => {
            log::warn!("quicklinks will need `lumen reload` to change: {error:#}");
            return;
        }
    };

    let launcher = surfaces.launcher;
    let settings = surfaces.settings.clone();
    cx.spawn(async move |cx| {
        let _watcher = watcher;
        while next_change(&mut changes, cx).await {
            let reloaded = launcher.update(cx, |launcher, _window, cx| {
                launcher.reload_quicklinks(cx);
            });
            if reloaded.is_err() {
                break;
            }
            // The settings page too, once it exists: a link saved from the
            // launcher, or by hand, appears in its list.
            let window = *settings.borrow();
            if let Some(window) = window {
                window
                    .update(cx, |settings, _window, cx| settings.reload_quicklinks(cx))
                    .ok();
            }
        }
    })
    .detach();
}

/// Show the settings window, creating it the first time.
///
/// Lazy on purpose: see the note on [`WindowRole::Settings`]. Once created it is
/// kept and only hidden, so the second open is as instant as the launcher's.
fn open_settings(
    platform: &Arc<dyn Platform>,
    config: &Config,
    surfaces: &Surfaces,
    cx: &mut gpui::AsyncApp,
) -> Result<()> {
    // Captured before the window appears, so the offer to "make a sheet for
    // what I was just using" names the application the user actually came from.
    let foreground = platform.foreground_application().unwrap_or(None);

    let existing = *surfaces.settings.borrow();
    let window = match existing {
        Some(window) => window,
        None => {
            let created =
                cx.update(|cx| create_settings_window(cx, platform, config, surfaces))??;
            *surfaces.settings.borrow_mut() = Some(created);
            created
        }
    };

    // Always reloaded from disk on open. The user may have edited a file by
    // hand since last time, and showing them a stale editor would invite them
    // to save over their own work.
    let entries = surfaces
        .sheets
        .borrow()
        .entries()
        .iter()
        .map(|entry| (entry.sheet.clone(), entry.path.clone()))
        .collect::<Vec<_>>();

    window
        .update(cx, |settings, window, cx| {
            settings.load(entries, cx);
            if let Some(app) = foreground {
                settings.set_foreground(app.executable, app.title);
            }
            settings.reload_quicklinks(cx);
            settings.reload_translation(cx);
            settings.reload_timers(cx);
            settings.reload_currency(cx);
            settings.focus(window, cx);
        })
        .ok();

    let monitor = platform.cursor_monitor()?;
    let bounds = monitor.place(
        Rect::size(
            lumen_core::layout::settings::WIDTH as i32,
            lumen_core::layout::settings::HEIGHT as i32,
        ),
        // Centred both ways rather than biased upward: this is a window to
        // work in, not a flyout to glance at, and it does not follow the
        // launcher wherever the user has put that.
        0.5,
        0.5,
    );
    platform.show_window(WindowRole::Settings, bounds)?;

    // The browsers, for the quicklink form's hint: a registry walk, so off the
    // UI thread and after the window is already up.
    let browsers = cx.background_executor().spawn({
        let platform = platform.clone();
        async move {
            platform
                .installed_browsers()
                .unwrap_or_default()
                .into_iter()
                .map(|browser| browser.name)
                .collect::<Vec<_>>()
        }
    });
    // The alert sounds, for the Timers page: a folder listing, likewise.
    let sounds = cx.background_executor().spawn({
        let platform = platform.clone();
        async move { platform.alert_sounds().unwrap_or_default() }
    });
    cx.spawn(async move |cx| {
        let browsers = browsers.await;
        let sounds = sounds.await;
        window
            .update(cx, |settings, _window, cx| {
                settings.set_browsers(browsers, cx);
                settings.set_alert_sounds(sounds, cx);
            })
            .ok();
    })
    .detach();

    Ok(())
}

fn create_settings_window(
    cx: &mut App,
    platform: &Arc<dyn Platform>,
    config: &Config,
    surfaces: &Surfaces,
) -> Result<gpui::WindowHandle<Settings>> {
    let theme = surfaces.theme.borrow().clone();
    let sheets_dir = surfaces.paths.sheets_dir();
    let quicklinks_file = surfaces.paths.quicklinks_file();
    let favicons = surfaces.paths.favicons_dir();
    let config_file = surfaces.paths.config_file();

    let window = open_hidden_window(
        cx,
        platform,
        WindowRole::Settings,
        config,
        size(
            px(lumen_core::layout::settings::WIDTH),
            px(lumen_core::layout::settings::HEIGHT),
        ),
        move |_window, cx| {
            cx.new(|cx| {
                Settings::new(
                    theme,
                    sheets_dir,
                    quicklinks_file,
                    favicons,
                    config_file,
                    cx,
                )
            })
        },
    )?;

    subscribe_to_settings(cx, &window, platform.clone(), surfaces);

    Ok(window)
}

/// React to saves, deletions and closing.
fn subscribe_to_settings(
    cx: &mut App,
    window: &gpui::WindowHandle<Settings>,
    platform: Arc<dyn Platform>,
    surfaces: &Surfaces,
) {
    let Ok(entity) = window.entity(cx) else {
        return;
    };

    let sheets = surfaces.sheets.clone();
    let paths = surfaces.paths.clone();
    let launcher = surfaces.launcher;

    cx.subscribe(&entity, move |_entity: Entity<Settings>, event, cx| {
        match event {
            // A save or a delete changes what the overlay should show, so the
            // library is rebuilt immediately rather than at the next restart.
            // No window is touched, so this stays on the callback.
            SettingsEvent::Saved { path } | SettingsEvent::Deleted { path } => {
                log::debug!("sheets changed at {}", path.display());
                let mut library = SheetLibrary::load(&paths.sheets_dir());
                library.add(keymap::sheet());
                *sheets.borrow_mut() = library;
            }
            SettingsEvent::QuicklinkIconsChanged => {
                launcher
                    .update(cx, |launcher, _window, cx| launcher.reindex(cx))
                    .ok();
            }
            SettingsEvent::PreviewSound(name) => {
                let platform = platform.clone();
                let name = name.clone();
                std::thread::spawn(move || {
                    if let Err(error) = platform.play_alert(&name, false) {
                        log::warn!("could not preview {name:?}: {error:#}");
                    }
                });
            }
            SettingsEvent::Closed => {
                let platform = platform.clone();
                off_the_callback(cx, move || platform.hide_window(WindowRole::Settings, None));
            }
        }
    })
    .detach();
}

#[cfg(target_os = "windows")]
fn build_platform(config: &Config, theme: &Theme) -> Arc<dyn Platform> {
    use lumen_platform_windows::{WindowsPlatform, WindowsPlatformOptions};

    Arc::new(WindowsPlatform::new(WindowsPlatformOptions {
        backdrop: config.backdrop,
        dark: theme.appearance == Appearance::Dark,
    }))
}

#[cfg(target_os = "linux")]
fn build_platform(_config: &Config, _theme: &Theme) -> Arc<dyn Platform> {
    Arc::new(lumen_platform_linux::LinuxPlatform::new())
}

/// Create the one window this process will ever have (I-4).
/// Create one of the daemon's two warm windows, hidden, and hand its raw handle
/// to the platform for the treatment GPUI does not expose.
///
/// "Pre-created hidden window, like the launcher's. That is two warm windows in
/// memory, not one" (F3). The overlay pays its GPU initialisation once at
/// startup for the same reason the launcher does: a surface that is created when
/// the gesture fires cannot also appear within the gesture's own duration.
fn open_hidden_window<V: 'static + Render>(
    cx: &mut App,
    platform: &Arc<dyn Platform>,
    role: WindowRole,
    config: &Config,
    initial: gpui::Size<gpui::Pixels>,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Result<gpui::WindowHandle<V>> {
    let window = cx
        .open_window(
            WindowOptions {
                // A plausible starting rectangle. The real position is chosen on
                // every show, against the monitor holding the cursor.
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: initial,
                })),
                // No system decoration: the border, the shadow and the corners
                // are the application's, on both platforms.
                titlebar: None,
                focus: false,
                // Created hidden. The whole point of a resident daemon is that
                // the first invocation is not the first frame.
                show: false,
                // PopUp is what gets WS_EX_TOOLWINDOW on Windows, keeping both
                // surfaces out of Alt+Tab and the taskbar.
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                display_id: None,
                // The indispensable condition of section 7: without a genuinely
                // transparent client area, the system backdrop is composited
                // behind an opaque surface and never seen.
                window_background: if config.backdrop.wants_translucent_surface() {
                    WindowBackgroundAppearance::Transparent
                } else {
                    WindowBackgroundAppearance::Opaque
                },
                app_id: Some("lumen".into()),
                window_min_size: None,
                window_decorations: None,
                tabbing_identifier: None,
            },
            build,
        )
        .with_context(|| format!("creating the {role:?} window"))?;

    window
        .update(cx, |_view, window, _cx| -> Result<()> {
            use raw_window_handle::HasWindowHandle;

            let handle = window
                .window_handle()
                .map_err(|error| anyhow::anyhow!("no raw window handle: {error}"))?;
            platform.attach_window(role, handle.as_raw())
        })
        .context("reaching the new window")?
        .with_context(|| format!("applying the {role:?} window treatment"))?;

    Ok(window)
}

/// React to what the view asks for.
fn subscribe_to_launcher(
    cx: &mut App,
    window: &gpui::WindowHandle<Launcher>,
    platform: Arc<dyn Platform>,
    config: Rc<RefCell<Config>>,
    shell: Rc<RefCell<Shell>>,
    commands: mpsc::UnboundedSender<Message>,
    timers: timers::Timers,
) {
    let Ok(entity) = window.entity(cx) else {
        return;
    };

    cx.subscribe(&entity, move |_entity: Entity<Launcher>, event, cx| {
        // Focus refused something (F8): the timers say why, in the launcher.
        if let LauncherEvent::Blocked { what, action } = event {
            let timers = timers.clone();
            let (what, action) = (what.clone(), action.clone());
            cx.spawn(async move |cx| timers.open_blocked(what, action, cx))
                .detach();
            return;
        }

        let platform = platform.clone();
        let shell = shell.clone();
        // Read at the moment of the event, so a resize after an edit to
        // config.json lands where the new configuration says.
        let config = config.borrow().clone();
        let commands = commands.clone();
        let event = event.clone();

        // Starting a process needs the `App` this callback is holding, so it
        // goes back through the same queue the hotkey and the socket use
        // rather than being done here. One queue, one order of events.
        if let LauncherEvent::RunExtension { extension, command } = &event {
            let _ = commands.unbounded_send(Message::RunExtension {
                extension: extension.clone(),
                command: command.clone(),
            });
            return;
        }
        if let LauncherEvent::RunTranslate { preset } = &event {
            let _ = commands.unbounded_send(Message::RunTranslate {
                preset: preset.clone(),
            });
            return;
        }
        // The timers are the event loop's (F8), so these go on its queue:
        // behind the dismissal that came first, in order.
        match &event {
            LauncherEvent::RunTimers => {
                let _ = commands.unbounded_send(Message::RunTimers);
                return;
            }
            LauncherEvent::RunAlertSounds => {
                let _ = commands.unbounded_send(Message::RunAlertSounds);
                return;
            }
            LauncherEvent::RunCurrency => {
                let _ = commands.unbounded_send(Message::RunCurrency);
                return;
            }
            LauncherEvent::RunDictionary => {
                let _ = commands.unbounded_send(Message::RunDictionary);
                return;
            }
            LauncherEvent::RunSnippets { query } => {
                let _ = commands.unbounded_send(Message::RunSnippets {
                    query: query.clone(),
                });
                return;
            }
            LauncherEvent::RunClipboard => {
                let _ = commands.unbounded_send(Message::RunClipboard);
                return;
            }
            LauncherEvent::RefreshRates => {
                let _ = commands.unbounded_send(Message::RefreshRates);
                return;
            }
            LauncherEvent::Timer(command) => {
                let _ = commands.unbounded_send(Message::Timer(command.clone()));
                return;
            }
            _ => {}
        }

        off_the_callback(cx, move || {
            let outcome = match event {
                LauncherEvent::Dismissed => hide(&platform, &shell),
                LauncherEvent::Resized(height) => resize(&platform, &config, &shell, height),
                LauncherEvent::ArrangeWindow(layout) => arrange(&platform, &shell, layout),
                LauncherEvent::Paste(ref paste) => {
                    paste_into(&platform, &config, &shell, paste.clone())
                }
                // Handled above, before this task was spawned.
                LauncherEvent::RunExtension { .. }
                | LauncherEvent::RunTranslate { .. }
                | LauncherEvent::RunTimers
                | LauncherEvent::RunAlertSounds
                | LauncherEvent::RunCurrency
                | LauncherEvent::RunDictionary
                | LauncherEvent::RunSnippets { .. }
                | LauncherEvent::RunClipboard
                | LauncherEvent::RefreshRates
                | LauncherEvent::Blocked { .. }
                | LauncherEvent::Timer(_) => Ok(()),
                // Sent back through the same channel the hotkey and the socket use,
                // rather than acted on here. One queue means one order of events,
                // and it means `lumen settings` from a terminal and "Configure
                // Shortcuts" from the launcher take the identical path.
                LauncherEvent::Builtin(command) => {
                    let _ = commands.unbounded_send(match command {
                        Builtin::OpenSettings => Message::Settings,
                        Builtin::ReloadSheets => Message::ReloadSheets,
                        Builtin::Quit => Message::Quit,
                    });
                    Ok(())
                }
            };
            outcome.with_context(|| format!("could not apply {event:?}"))
        });
    })
    .detach();
}

/// Run `work` on the next turn of the run loop rather than here.
///
/// Every caller is inside a GPUI subscription callback, which runs while the
/// `App` is mutably borrowed. That is fine until the work shows, hides, moves
/// or resizes a window: those calls send window messages *synchronously* to our
/// own window procedure on this same thread, GPUI tries to take the borrow it
/// cannot have, and the message is dropped -- silently, apart from a
/// `RefCell already borrowed` line in the log with no indication of what it
/// was about.
///
/// What gets dropped is size and position notifications for a window that was
/// being resized at that moment, which is not the kind of thing to leave to
/// luck. Spawning puts the work on the foreground executor, where it runs on
/// this thread, in order, with no borrow held.
fn off_the_callback(cx: &mut App, work: impl FnOnce() -> Result<()> + 'static) {
    cx.spawn(async move |_cx| {
        if let Err(error) = work() {
            log::error!("{error:#}");
        }
    })
    .detach();
}

/// Re-read the theme when its file changes, or when the configuration names a
/// different one (AD-05).
///
/// Returns the sender that asks for a reload, which the configuration reload
/// uses when `theme` names a different file. It works even if the folder
/// could not be watched.
fn watch_theme(
    cx: &mut App,
    window: &gpui::WindowHandle<Launcher>,
    overlay: &gpui::WindowHandle<Overlay>,
    paths: &Paths,
    config: Rc<RefCell<Config>>,
) -> mpsc::UnboundedSender<()> {
    use lumen_core::watcher::DirectoryWatcher;

    let (reload, mut changes) = mpsc::unbounded::<()>();

    // The folder rather than one file: which file is "the theme" is whatever
    // config.json says right now, and that can change while this runs.
    let sink = reload.clone();
    let watcher = DirectoryWatcher::start(&paths.themes_dir(), move || {
        let _ = sink.unbounded_send(());
    })
    // Live reload is a convenience. Losing it must not stop the launcher
    // from running.
    .inspect_err(|error| log::warn!("theme live reload is unavailable: {error:#}"))
    .ok();

    let window = *window;
    let overlay = *overlay;
    let paths = paths.clone();

    cx.spawn(async move |cx| {
        let _watcher = watcher;
        while next_change(&mut changes, cx).await {
            let wanted = config.borrow().theme.clone();
            let theme = match Theme::load(&paths.theme_file(&wanted)) {
                Ok(theme) => theme,
                // Keep the current theme. A syntax error mid-edit is the
                // normal case, and dropping to the base theme on every stray
                // keystroke would make editing unusable.
                Err(error) => {
                    log::warn!("theme reload skipped: {error:#}");
                    continue;
                }
            };
            log::info!("reloaded theme {:?}", theme.name);

            // Both surfaces follow the same theme file. Reloading one and not
            // the other would leave two differently coloured windows one
            // keystroke apart.
            overlay
                .update(cx, |overlay, _window, cx| {
                    overlay.apply_theme(theme.clone(), cx)
                })
                .ok();

            let updated = window.update(cx, |launcher, _window, cx| {
                launcher.apply_theme(theme, cx);
            });
            if updated.is_err() {
                break;
            }
        }
    })
    .detach();

    reload
}

/// Tell the event loop when config.json changes.
///
/// Only a message: the reload itself happens in [`reload_config`], on the same
/// queue as a summons, so a window cannot be re-placed halfway through being
/// shown.
fn watch_config(cx: &mut App, paths: &Paths, commands: mpsc::UnboundedSender<Message>) {
    use lumen_core::watcher::FileWatcher;

    let (sink, mut changes) = change_channel();
    let watcher = match FileWatcher::start(&paths.config_file(), sink) {
        Ok(watcher) => watcher,
        Err(error) => {
            log::warn!("configuration changes will need `lumen reload` or a restart: {error:#}");
            return;
        }
    };

    cx.spawn(async move |cx| {
        let _watcher = watcher;
        while next_change(&mut changes, cx).await {
            if commands.unbounded_send(Message::ReloadConfig).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// How long a burst of filesystem events is given to finish.
///
/// An editor's save is a truncate, a write and a rename, reported as several
/// events within a few milliseconds. Waiting this long after the first turns
/// them into one reload. It is a one-shot timer started by a change, not a
/// poll: with nothing being edited, nothing wakes.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(100);

/// A sink for a watcher to push into, and the receiving end to await.
fn change_channel() -> (impl Fn() + Send + 'static, mpsc::UnboundedReceiver<()>) {
    let (sender, receiver) = mpsc::unbounded();
    let sink = move || {
        let _ = sender.unbounded_send(());
    };
    (sink, receiver)
}

/// Wait for the next change, then let the burst it belongs to settle.
///
/// Returns `false` once nothing can report a change any more.
async fn next_change(changes: &mut mpsc::UnboundedReceiver<()>, cx: &gpui::AsyncApp) -> bool {
    if changes.next().await.is_none() {
        return false;
    }
    cx.background_executor().timer(SETTLE).await;
    while changes.try_recv().is_ok() {}
    true
}

/// Re-read config.json and apply what can be applied without a restart.
///
/// Position, size, row count, theme and file search take effect at once --
/// including on a launcher that is open while the file is saved. The hotkey,
/// the backdrop and clipboard history are wired into things made once at
/// startup, and are reported as waiting rather than half-applied.
fn reload_config(
    platform: &Arc<dyn Platform>,
    shell: &Rc<RefCell<Shell>>,
    surfaces: &Surfaces,
    cx: &mut gpui::AsyncApp,
) -> Result<()> {
    let newer = match Config::try_load(&surfaces.paths.config_file()) {
        Ok(config) => config,
        // Most often a save caught halfway. The write that completes it
        // arrives as another change and another reload.
        Err(error) => {
            log::warn!("kept the running configuration: {error:#}");
            return Ok(());
        }
    };

    let older = surfaces.config.borrow().clone();
    let waiting = older.restart_required(&newer);
    if !waiting.is_empty() {
        log::warn!(
            "{} changed in the configuration and will apply after a restart \
             (`lumen quit`, then `lumen`)",
            waiting.join(", ")
        );
    }

    // The translation and timers pages follow the file whoever changed it,
    // the page itself included. Before the check below: a change the running config
    // already holds is still one the page may not have read.
    let settings = *surfaces.settings.borrow();
    if let Some(settings) = settings {
        settings
            .update(cx, |settings, _window, cx| {
                settings.reload_translation(cx);
                settings.reload_timers(cx);
                settings.reload_currency(cx);
            })
            .ok();
    }

    let applied = older.merge_live(&newer);
    if applied == older {
        return Ok(());
    }
    surfaces.timers.configure(&applied.timers, cx);
    *surfaces.config.borrow_mut() = applied.clone();
    log::info!("reloaded the configuration");

    if applied.theme != older.theme {
        let _ = surfaces.reload_theme.unbounded_send(());
    }

    // The launcher first, so the height used to place the window is the one
    // for the new row count.
    let height = surfaces
        .launcher
        .update(cx, |launcher, _window, cx| {
            launcher.apply_config(applied.clone(), cx);
            launcher.content_height()
        })
        .ok();

    // Already open: move it now rather than on the next summons, so the edit
    // can be seen while it is being made.
    let shell = shell.borrow();
    if shell.visible
        && let (Some(monitor), Some(height)) = (shell.monitor, height)
    {
        platform.set_window_bounds(WindowRole::Launcher, placement(&monitor, &applied, height))?;
    }

    Ok(())
}

fn handle(
    message: Message,
    platform: &Arc<dyn Platform>,
    shell: &Rc<RefCell<Shell>>,
    surfaces: &Surfaces,
    cx: &mut gpui::AsyncApp,
) -> Result<()> {
    // Cloned per message rather than borrowed: `show` and `open_settings` call
    // into GPUI, and a borrow held across that is a borrow a reload could
    // collide with.
    let config = &surfaces.config.borrow().clone();

    // A sleep may have woken late -- the machine slept through a deadline --
    // so a summons reports what has ended before anything is drawn (F8).
    if matches!(
        message,
        Message::Toggle | Message::Show | Message::ShowTimers
    ) {
        surfaces.timers.catch_up(cx);
    }

    match message {
        Message::Toggle => {
            let visible = shell.borrow().visible;
            if visible {
                hide(platform, shell)
            } else {
                show(platform, config, shell, &surfaces.launcher, cx)
            }
        }
        Message::Show => show(platform, config, shell, &surfaces.launcher, cx),
        Message::Hide => hide(platform, shell),
        Message::OverlayPress => show_overlay(platform, shell, surfaces, cx),
        Message::OverlayRelease => hide_overlay(surfaces, cx),
        Message::Settings => open_settings(platform, config, surfaces, cx),
        Message::ReloadConfig => reload_config(platform, shell, surfaces, cx),
        Message::ApplicationsChanged => {
            log::info!("applications changed; re-indexing");
            surfaces
                .launcher
                .update(cx, |launcher, _window, cx| {
                    launcher.reindex_applications(cx)
                })
                .ok();
            Ok(())
        }
        Message::ReloadSheets => {
            // The watcher normally has this covered; asked for explicitly,
            // the answer should not depend on whether it noticed.
            reload_config(platform, shell, surfaces, cx)?;
            surfaces.reload_sheets();
            // Scripts, snippets and quicklinks live in the launcher's index,
            // so reloading is two halves: the sheet library here, and a
            // reindex there. The settings page keeps its own copy of the
            // quicklinks, so it re-reads too. One command reloads everything
            // the user can edit by hand.
            surfaces
                .launcher
                .update(cx, |launcher, _window, cx| launcher.reindex(cx))
                .ok();
            let settings = *surfaces.settings.borrow();
            if let Some(settings) = settings {
                settings
                    .update(cx, |settings, _window, cx| settings.reload_quicklinks(cx))
                    .ok();
            }
            Ok(())
        }
        Message::RunExtension { extension, command } => {
            run_extension(platform, surfaces, extension, command, cx)
        }
        Message::RunTranslate { preset } => {
            let platform = platform.clone();
            let setup = dictionary_setup(config, &surfaces.paths);
            let config = config.translate.clone();
            let window = surfaces.launcher;
            cx.update(move |cx| translate::run(platform, config, preset, None, setup, window, cx))
                .context("reaching the app to open the translator")
        }
        Message::RunTimers => {
            surfaces.timers.open_view(surfaces.launcher, cx);
            Ok(())
        }
        // A toggle, like the launcher's own key: pressed on the Timers view,
        // it goes away; anywhere else, it brings the view.
        Message::ShowTimers => {
            let visible = shell.borrow().visible;
            if visible && surfaces.timers.is_viewing() {
                return hide(platform, shell);
            }
            if !visible {
                show(platform, config, shell, &surfaces.launcher, cx)?;
            }
            surfaces.timers.open_view(surfaces.launcher, cx);
            Ok(())
        }
        Message::RunCurrency => {
            let platform = platform.clone();
            let config = config.currency.clone();
            let path = surfaces.paths.currency_file();
            let window = surfaces.launcher;
            cx.update(move |cx| currency::run(platform, config, path, window, cx))
                .context("reaching the app to open the currency view")
        }
        Message::RunDictionary => {
            let platform = platform.clone();
            let setup = dictionary_setup(config, &surfaces.paths);
            let translate = config.translate.clone();
            let window = surfaces.launcher;
            cx.update(move |cx| dictionary::run(platform, setup, translate, None, window, cx))
                .context("reaching the app to open the dictionary")
        }
        Message::RunSnippets { query } => {
            let platform = platform.clone();
            let path = surfaces.paths.snippets_file();
            let paste = config.snippets.paste;
            let window = surfaces.launcher;
            cx.update(move |cx| snippets::run(platform, path, paste, query, window, cx))
                .context("reaching the app to open the snippets")
        }
        Message::RunClipboard => {
            let platform = platform.clone();
            let window = surfaces.launcher;
            cx.update(move |cx| clipboard::run(platform, window, cx))
                .context("reaching the app to open the clipboard history")
        }
        Message::RefreshRates => {
            let config = config.currency.clone();
            let path = surfaces.paths.currency_file();
            let window = surfaces.launcher;
            cx.update(move |cx| currency::refresh(config, path, window, cx))
                .context("reaching the app to refresh exchange rates")
        }
        Message::RunAlertSounds => {
            surfaces.timers.open_sounds(surfaces.launcher, cx);
            Ok(())
        }
        Message::Timer(command) => {
            surfaces.timers.command(command, cx);
            Ok(())
        }
        Message::Quit => {
            log::info!("stopping the daemon");
            cx.update(|cx| cx.quit()).ok();
            Ok(())
        }
        Message::Ping => Ok(()),
    }
}

/// Raise the shortcut sheet for whatever is in front.
fn show_overlay(
    platform: &Arc<dyn Platform>,
    shell: &Rc<RefCell<Shell>>,
    surfaces: &Surfaces,
    cx: &mut gpui::AsyncApp,
) -> Result<()> {
    // Asked before the overlay is mapped, so the answer is the application the
    // user is actually looking at rather than Lumen. The overlay never takes
    // focus, so this stays true for as long as it is up.
    let foreground = platform.foreground_application().unwrap_or_else(|error| {
        log::debug!("could not identify the foreground application: {error:#}");
        None
    });

    // Cloned out of the library rather than borrowed across the rest of this
    // function: the settings window and the watcher can both replace the
    // library while the overlay is up, and a live borrow would make that a
    // panic instead of a reload.
    let sheet = surfaces
        .sheets
        .borrow()
        .resolve(foreground.as_ref(), &surfaces.fallback)
        .clone();

    log::debug!(
        "overlay: {:?} -> sheet {:?}",
        foreground
            .as_ref()
            .and_then(|app| app.executable.as_deref()),
        sheet.name
    );

    let columns = lumen_core::layout::overlay::COLUMNS;
    let height = sheet.measured_height(columns);
    let width = sheet.measured_width(columns);

    // The window is positioned and mapped while its content is still at zero
    // opacity, and only then told to fade in. That ordering is what F3 means by
    // "the animation starts at zero opacity": a first frame that arrives late
    // is invisible, so the hitch hides itself instead of flashing.
    let already_shown = shell.borrow().overlay_shown;
    if !already_shown {
        let monitor = platform.cursor_monitor()?;
        let bounds = monitor.place(
            Rect::size(width as i32, height as i32),
            // The overlay is always centred: it documents the application under
            // it, so it belongs where the eye already is rather than wherever the
            // launcher was moved to.
            0.5,
            lumen_core::layout::overlay::VERTICAL_BIAS,
        );
        platform.show_window(WindowRole::Overlay, bounds)?;
        shell.borrow_mut().overlay_shown = true;
    } else {
        // Already up, but a different application may now be in front, so the
        // panel has to be resized to the new sheet. A position change, not a
        // visibility change: the overlay keeps its z-order and its passivity.
        if let Some(monitor) = shell
            .borrow()
            .monitor
            .or_else(|| platform.cursor_monitor().ok())
        {
            let bounds = monitor.place(
                Rect::size(width as i32, height as i32),
                // The overlay is always centred: it documents the application under
                // it, so it belongs where the eye already is rather than wherever the
                // launcher was moved to.
                0.5,
                lumen_core::layout::overlay::VERTICAL_BIAS,
            );
            platform.set_window_bounds(WindowRole::Overlay, bounds).ok();
        }
    }

    surfaces
        .overlay
        .update(cx, |overlay, _window, cx| overlay.present(sheet, cx))
        .ok();

    Ok(())
}

/// Begin fading the sheet out. The window is hidden later, by the subscription,
/// once the fade reports that it has finished.
fn hide_overlay(surfaces: &Surfaces, cx: &mut gpui::AsyncApp) -> Result<()> {
    surfaces
        .overlay
        .update(cx, |overlay, _window, cx| overlay.dismiss(cx))
        .ok();
    Ok(())
}

fn show(
    platform: &Arc<dyn Platform>,
    config: &Config,
    shell: &Rc<RefCell<Shell>>,
    window: &gpui::WindowHandle<Launcher>,
    cx: &mut gpui::AsyncApp,
) -> Result<()> {
    let summoned = std::time::Instant::now();

    // Reset the view before the window is on screen, so the first frame the user
    // sees is already the cleared state rather than the previous search.
    let height = window
        .update(cx, |launcher, window, cx| {
            launcher.on_shown(window, cx);
            // Registered here rather than after `show_window`, because the
            // frame this reports on is the one `on_shown` just asked for.
            timing::report_first_frame(window, summoned);
            launcher.content_height()
        })
        .unwrap_or_else(|_| config.window_height(config.window.visible_rows));

    let monitor = platform.cursor_monitor()?;
    let bounds = placement(&monitor, config, height);

    let restore_to = platform.show_window(WindowRole::Launcher, bounds)?;

    let mut shell = shell.borrow_mut();
    shell.visible = true;
    shell.restore_to = restore_to;
    shell.monitor = Some(monitor);

    Ok(())
}

/// What the Dictionary needs from the configuration (F10), for either view
/// to open it.
fn dictionary_setup(config: &Config, paths: &Paths) -> dictionary::Setup {
    dictionary::Setup {
        browser: config.preferred_browser.clone(),
        dictionary: config.dictionary.clone(),
        file: paths.dictionary_file(),
    }
}

fn hide(platform: &Arc<dyn Platform>, shell: &Rc<RefCell<Shell>>) -> Result<()> {
    let restore_to = {
        let mut shell = shell.borrow_mut();
        if !shell.visible {
            return Ok(());
        }
        shell.visible = false;
        shell.restore_to.take()
    };

    platform.hide_window(WindowRole::Launcher, restore_to)
}

/// Start an extension's command (F4, step 3).
///
/// The manifest is re-read here rather than taken from the index, for two
/// reasons. The index holds what a command *is called*, which is all the root
/// list needs; and a manifest edited since the daemon started should be the one
/// that runs, because the alternative is telling an extension author to restart
/// the launcher after every change.
fn run_extension(
    platform: &Arc<dyn Platform>,
    surfaces: &Surfaces,
    directory: std::path::PathBuf,
    command: String,
    cx: &mut gpui::AsyncApp,
) -> Result<()> {
    let manifest = directory.join(lumen_core::extension::MANIFEST);
    let source = std::fs::read_to_string(&manifest)
        .with_context(|| format!("reading {}", manifest.display()))?;
    let manifest: lumen_core::extension::Manifest =
        serde_json::from_str(&source).with_context(|| format!("parsing {}", manifest.display()))?;

    let extension = lumen_core::extension::Extension {
        directory,
        manifest,
    };

    let window = surfaces.launcher;
    let platform = platform.clone();
    cx.update(move |cx| extensions::run(platform, extension, command, window, cx))
        .context("reaching the app to start an extension")?
}

/// Hide, give the foreground back, and then move that window.
///
/// The order matters in both directions. The handle is read before hiding,
/// because hiding consumes it; the arrangement happens after, because the
/// window should be the one the user is looking at by the time it moves rather
/// than jumping about behind a launcher that is still on screen.
fn arrange(
    platform: &Arc<dyn Platform>,
    shell: &Rc<RefCell<Shell>>,
    layout: lumen_platform::WindowLayout,
) -> Result<()> {
    let target = shell.borrow().restore_to;
    hide(platform, shell)?;

    let Some(target) = target else {
        // Nothing had the foreground when the launcher appeared: the user came
        // from the desktop. There is no window to arrange and nothing has gone
        // wrong.
        log::debug!("no window to arrange");
        return Ok(());
    };

    platform.arrange_window(target, layout)
}

/// Hide, then paste into the window the launcher was opened over (F11).
///
/// Arranging's shape (7.15): the handle is taken before hiding consumes it.
/// The paste itself waits for the window to read the clipboard, which can
/// take a second or two, so it runs on a thread of its own and says so if it
/// fails -- by then the launcher is gone and there is nowhere else to say it.
fn paste_into(
    platform: &Arc<dyn Platform>,
    config: &Config,
    shell: &Rc<RefCell<Shell>>,
    mut paste: lumen_platform::Paste,
) -> Result<()> {
    let target = shell.borrow().restore_to;
    hide(platform, shell)?;

    let Some(target) = target else {
        // Opened from the desktop: there is no window to paste into, so the
        // snippet is copied, which is the most a paste could have done.
        log::info!("no window to paste into; copied the snippet instead");
        return platform.set_clipboard_text(&paste.text);
    };

    let platform = platform.clone();
    let snippets = config.snippets.clone();
    std::thread::Builder::new()
        .name("lumen-paste-request".into())
        .spawn(move || {
            // Which program it is decides the keystroke: some terminals pass
            // `Ctrl+V` on rather than pasting. Asked here, off the UI thread.
            let executable = platform
                .window_application(target)
                .ok()
                .flatten()
                .and_then(|app| app.executable);
            paste.keys = snippets.keys_for(executable.as_deref());
            log::debug!("pasting into {executable:?} with {:?}", paste.keys);
            if let Err(error) = platform.paste(target, &paste) {
                log::warn!("could not paste: {error:#}");
                if let Err(error) = platform.notify("Couldn't paste", &format!("{error:#}")) {
                    log::warn!("could not say so: {error:#}");
                }
            }
        })
        .context("starting the paste")?;
    Ok(())
}

fn resize(
    platform: &Arc<dyn Platform>,
    config: &Config,
    shell: &Rc<RefCell<Shell>>,
    height: f32,
) -> Result<()> {
    let shell = shell.borrow();
    if !shell.visible {
        return Ok(());
    }
    let Some(monitor) = shell.monitor else {
        return Ok(());
    };

    platform.set_window_bounds(WindowRole::Launcher, placement(&monitor, config, height))
}

/// Where the window goes, in physical pixels.
///
/// The top edge is where a full root list would put it, so the search field
/// stays still while the result count changes the height.
fn placement(monitor: &MonitorInfo, config: &Config, height: f32) -> Rect {
    let anchor = config.window_height(config.window.visible_rows);
    monitor.place_anchored(
        Rect::size(config.window.width as i32, height as i32),
        anchor as i32,
        config.window.horizontal_bias,
        config.window.vertical_bias,
    )
}

/// Open the clipboard history and begin recording into it (F3).
///
/// The privacy decision and the storage are deliberately apart: the platform
/// declines entries the *system* marked not-for-history, `privacy::may_record`
/// declines the ones Lumen can recognise as secrets, and only what survives
/// both reaches the database.
///
/// A failure to open the database is not fatal. Clipboard history is one
/// provider among several, and a launcher that refuses to start because a
/// SQLite file is unreadable would be a far worse trade than one that runs
/// without it.
fn start_clipboard_history(
    platform: &Arc<dyn Platform>,
    paths: &Paths,
    retention: Retention,
    window: &gpui::WindowHandle<Launcher>,
    cx: &mut App,
) {
    let history = match History::open(&paths.clipboard_file(), retention) {
        Ok(history) => Arc::new(parking_lot::Mutex::new(history)),
        Err(error) => {
            log::error!("clipboard history is unavailable: {error:#}");
            return;
        }
    };

    // The view reads the history through the same handle the recorder
    // writes through.
    window
        .update(cx, |launcher, _window, _cx| {
            launcher.use_clipboard_history(history.clone());
        })
        .ok();

    let (tx, mut rx) = mpsc::unbounded::<lumen_platform::ClipboardEvent>();

    let listening = platform.start_clipboard_listener(ClipboardSink::new(move |event| {
        // On the watcher thread: push and return.
        let _ = tx.unbounded_send(event);
    }));

    if let Err(error) = listening {
        log::error!("clipboard history will not record: {error:#}");
        return;
    }

    cx.spawn(async move |cx| {
        while let Some(event) = rx.next().await {
            // Files are kept as their paths, one per line (F12), and pass the
            // same checks: a path can name a password manager's vault.
            let (payload, text) = match event.content {
                lumen_platform::ClipboardContent::Text(text) => (Payload::Text, text),
                lumen_platform::ClipboardContent::Files(files) => (
                    Payload::Files,
                    files
                        .iter()
                        .map(|file| file.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(
                            "
",
                        ),
                ),
            };
            let verdict = lumen_core::clipboard::may_record(&text, event.source.as_deref());

            if let Err(reason) = verdict {
                // Logged at debug and without the text, which is the entire
                // point: a rejected entry must not reach the log either.
                log::debug!("clipboard entry not recorded: {reason:?}");
                continue;
            }

            // The write is small but it is still disk I/O, so it goes to a
            // background thread rather than blocking whatever frame the UI
            // thread is in the middle of.
            let history = history.clone();
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) =
                        history
                            .lock()
                            .record(payload, &text, event.source.as_deref())
                    {
                        log::warn!("could not record a clipboard entry: {error:#}");
                    }
                })
                .await;
        }
    })
    .detach();
}
