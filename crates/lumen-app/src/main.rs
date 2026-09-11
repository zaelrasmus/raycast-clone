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
use lumen_core::sheets::{Sheet, SheetLibrary};
use lumen_core::theme::Appearance;
use lumen_core::{Builtin, Config, Paths, Theme};
use lumen_platform::{
    FocusOwner, GestureEvent, GestureSink, HoldPhase, MonitorInfo, Platform, Rect, WindowRole,
};
use lumen_ui::{Launcher, LauncherEvent, Overlay, OverlayEvent, Settings, SettingsEvent, keymap};

mod ipc;

use ipc::Message;

fn main() -> Result<()> {
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
        Some("reload-sheets") => forward_only(Message::ReloadSheets),
        Some("quit") => forward_only(Message::Quit),
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
         reload-sheets      Re-read the shortcut sheets from disk\n    \
         quit               Stop the running daemon\n\n\
         With no running daemon, `toggle`, `show` and `settings` start one."
    );
}

/// Send `message`; if nothing is listening, become the daemon.
fn dispatch(message: Message) -> Result<()> {
    if ipc::send(message)? {
        log::debug!("handed {message:?} to the running daemon");
        return Ok(());
    }
    run_daemon()
}

/// Send `message` and do nothing if there is no daemon.
///
/// Starting a whole launcher because someone released a key would be absurd.
fn forward_only(message: Message) -> Result<()> {
    if !ipc::send(message)? {
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

    Application::new().run(move |cx: &mut App| {
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
        GestureSink::new(move |event| {
            let message = match event {
                GestureEvent::Invoke => Message::Toggle,
                GestureEvent::Hold(HoldPhase::Began) => Message::OverlayPress,
                GestureEvent::Hold(HoldPhase::Ended) => Message::OverlayRelease,
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

    let ipc_tx = tx.clone();
    ipc::serve(move |message| {
        let _ = ipc_tx.unbounded_send(message);
    })
    .context("starting the IPC listener")?;

    subscribe_to_launcher(
        cx,
        &window,
        platform.clone(),
        config.clone(),
        shell.clone(),
        tx.clone(),
    );
    subscribe_to_overlay(cx, &overlay, platform.clone(), shell.clone());
    watch_theme(cx, &window, &overlay, &paths, &config);

    let surfaces = Surfaces {
        launcher: window,
        overlay,
        settings: Rc::new(RefCell::new(None)),
        sheets: Rc::new(RefCell::new(sheets)),
        fallback: Rc::new(fallback),
        paths: paths.clone(),
        theme: Rc::new(RefCell::new(theme.clone())),
    };

    watch_sheets(cx, &surfaces);

    cx.spawn({
        let platform = platform.clone();
        let config = config.clone();
        async move |cx| {
            while let Some(message) = rx.next().await {
                let outcome = handle(message, &platform, &config, &shell, &surfaces, cx);
                if let Err(error) = outcome {
                    log::error!("could not handle {message:?}: {error:#}");
                }
            }
        }
    })
    .detach();

    Ok(())
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

    cx.subscribe(&entity, move |_entity: Entity<Overlay>, event, _cx| {
        let OverlayEvent::FadedOut = event;

        // Hiding at the start of the fade would mean there was no fade; hiding
        // here, after it, is what lets the window stop compositing once it is
        // genuinely invisible.
        let mut shell = shell.borrow_mut();
        if !shell.overlay_shown {
            return;
        }
        shell.overlay_shown = false;

        if let Err(error) = platform.hide_window(WindowRole::Overlay, None) {
            log::error!("could not hide the overlay: {error:#}");
        }
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

    let watcher = match DirectoryWatcher::start(&surfaces.paths.sheets_dir()) {
        Ok(watcher) => watcher,
        Err(error) => {
            log::warn!("shortcut sheets will not reload automatically: {error:#}");
            return;
        }
    };

    let sheets = surfaces.sheets.clone();
    let paths = surfaces.paths.clone();

    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(400))
                .await;

            if !watcher.changed() {
                continue;
            }

            let mut library = SheetLibrary::load(&paths.sheets_dir());
            library.add(keymap::sheet());
            log::info!("reloaded {} shortcut sheets", library.len());
            *sheets.borrow_mut() = library;
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
            settings.focus(window);
        })
        .ok();

    let monitor = platform.cursor_monitor()?;
    let bounds = monitor.place(
        Rect::size(
            lumen_core::layout::settings::WIDTH as i32,
            lumen_core::layout::settings::HEIGHT as i32,
        ),
        // Centred rather than biased upward: this is a window to work in, not a
        // flyout to glance at.
        0.5,
    );
    platform.show_window(WindowRole::Settings, bounds)?;

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

    let window = open_hidden_window(
        cx,
        platform,
        WindowRole::Settings,
        config,
        size(
            px(lumen_core::layout::settings::WIDTH),
            px(lumen_core::layout::settings::HEIGHT),
        ),
        move |_window, cx| cx.new(|cx| Settings::new(theme, sheets_dir, cx)),
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

    cx.subscribe(&entity, move |_entity: Entity<Settings>, event, _cx| {
        match event {
            // A save or a delete changes what the overlay should show, so the
            // library is rebuilt immediately rather than at the next restart.
            SettingsEvent::Saved { path } | SettingsEvent::Deleted { path } => {
                log::debug!("sheets changed at {}", path.display());
                let mut library = SheetLibrary::load(&paths.sheets_dir());
                library.add(keymap::sheet());
                *sheets.borrow_mut() = library;
            }
            SettingsEvent::Closed => {
                if let Err(error) = platform.hide_window(WindowRole::Settings, None) {
                    log::error!("could not hide the settings window: {error:#}");
                }
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
    config: Config,
    shell: Rc<RefCell<Shell>>,
    commands: mpsc::UnboundedSender<Message>,
) {
    let Ok(entity) = window.entity(cx) else {
        return;
    };

    cx.subscribe(&entity, move |_entity: Entity<Launcher>, event, _cx| {
        let outcome = match event {
            LauncherEvent::Dismissed => hide(&platform, &shell),
            LauncherEvent::RowsChanged(rows) => resize(&platform, &config, &shell, *rows),
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
        if let Err(error) = outcome {
            log::error!("could not apply {event:?}: {error:#}");
        }
    })
    .detach();
}

/// Re-read the theme when its file changes (AD-05).
fn watch_theme(
    cx: &mut App,
    window: &gpui::WindowHandle<Launcher>,
    overlay: &gpui::WindowHandle<Overlay>,
    paths: &Paths,
    config: &Config,
) {
    use lumen_core::watcher::ThemeWatcher;

    let watcher = match ThemeWatcher::start(&paths.themes_dir(), &config.theme) {
        Ok(watcher) => watcher,
        Err(error) => {
            // Live reload is a development convenience. Losing it must not stop
            // the launcher from running.
            log::warn!("theme live reload is unavailable: {error:#}");
            return;
        }
    };

    let window = *window;
    let overlay = *overlay;
    cx.spawn(async move |cx| {
        loop {
            // Polled rather than pushed because `notify` hands back a std
            // channel, and blocking a background thread on it for the life of
            // the process to save a poll would be the more expensive choice.
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;

            let Some(theme) = watcher.latest() else {
                continue;
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
}

fn handle(
    message: Message,
    platform: &Arc<dyn Platform>,
    config: &Config,
    shell: &Rc<RefCell<Shell>>,
    surfaces: &Surfaces,
    cx: &mut gpui::AsyncApp,
) -> Result<()> {
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
        Message::ReloadSheets => {
            surfaces.reload_sheets();
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
    // Reset the view before the window is on screen, so the first frame the user
    // sees is already the cleared state rather than the previous search.
    let rows = window
        .update(cx, |launcher, window, cx| {
            launcher.on_shown(window, cx);
            launcher.visible_rows()
        })
        .unwrap_or(config.window.visible_rows);

    let monitor = platform.cursor_monitor()?;
    let bounds = placement(&monitor, config, rows);

    let restore_to = platform.show_window(WindowRole::Launcher, bounds)?;

    let mut shell = shell.borrow_mut();
    shell.visible = true;
    shell.restore_to = restore_to;
    shell.monitor = Some(monitor);

    Ok(())
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

fn resize(
    platform: &Arc<dyn Platform>,
    config: &Config,
    shell: &Rc<RefCell<Shell>>,
    rows: usize,
) -> Result<()> {
    let shell = shell.borrow();
    if !shell.visible {
        return Ok(());
    }
    let Some(monitor) = shell.monitor else {
        return Ok(());
    };

    platform.set_window_bounds(WindowRole::Launcher, placement(&monitor, config, rows))
}

/// Where the window goes, in physical pixels.
fn placement(monitor: &MonitorInfo, config: &Config, rows: usize) -> Rect {
    monitor.place(
        Rect::size(
            config.window.width as i32,
            config.window_height(rows) as i32,
        ),
        config.window.vertical_bias,
    )
}
