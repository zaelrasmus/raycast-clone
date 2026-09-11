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
    App, Application, Bounds, Entity, WindowBackgroundAppearance, WindowBounds, WindowKind,
    WindowOptions, point, px, size,
};
use lumen_core::theme::Appearance;
use lumen_core::{Config, Paths, Theme};
use lumen_platform::{
    FocusOwner, GestureEvent, GestureSink, HoldPhase, MonitorInfo, Platform, Rect,
};
use lumen_ui::{Launcher, LauncherEvent, keymap};

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
         overlay-release    Report that the hold gesture ended\n\n\
         With no running daemon, `toggle` and `show` start one."
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
    hold_active: bool,
}

fn run_daemon() -> Result<()> {
    let paths = Paths::discover()?;
    paths.ensure_exists()?;
    lumen_core::install_builtin_themes(&paths.themes_dir())?;
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
    keymap::install(cx);

    let platform = build_platform(&config, &theme);
    let window = open_hidden_window(cx, &platform, &paths, &config, theme)?;

    let shell = Rc::new(RefCell::new(Shell {
        visible: false,
        restore_to: None,
        monitor: None,
        hold_active: false,
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

    subscribe_to_launcher(cx, &window, platform.clone(), config.clone(), shell.clone());
    watch_theme(cx, &window, &paths, &config);

    cx.spawn({
        let platform = platform.clone();
        let config = config.clone();
        async move |cx| {
            while let Some(message) = rx.next().await {
                let outcome = handle(message, &platform, &config, &shell, &window, cx);
                if let Err(error) = outcome {
                    log::error!("could not handle {message:?}: {error:#}");
                }
            }
        }
    })
    .detach();

    Ok(())
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
fn open_hidden_window(
    cx: &mut App,
    platform: &Arc<dyn Platform>,
    paths: &Paths,
    config: &Config,
    theme: Theme,
) -> Result<gpui::WindowHandle<Launcher>> {
    // A plausible starting rectangle. The real position is chosen on every show,
    // against the monitor holding the cursor.
    let initial = size(
        px(config.window.width),
        px(config.window_height(config.window.visible_rows)),
    );

    let window = cx
        .open_window(
            WindowOptions {
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
                // PopUp is what gets WS_EX_TOOLWINDOW on Windows, keeping the
                // launcher out of Alt+Tab and the taskbar.
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
            {
                let platform = platform.clone();
                let paths = paths.clone();
                let config = config.clone();
                move |_window, cx| cx.new(|cx| Launcher::new(platform, paths, config, theme, cx))
            },
        )
        .context("creating the launcher window")?;

    // Hand the raw handle to the platform so it can apply the styles, the
    // backdrop and the corner shape GPUI does not expose.
    window
        .update(cx, |_view, window, _cx| -> Result<()> {
            use raw_window_handle::HasWindowHandle;

            let handle = window
                .window_handle()
                .map_err(|error| anyhow::anyhow!("no raw window handle: {error}"))?;
            platform.attach_window(handle.as_raw())
        })
        .context("reaching the new window")?
        .context("applying the launcher's window treatment")?;

    Ok(window)
}

/// React to what the view asks for.
fn subscribe_to_launcher(
    cx: &mut App,
    window: &gpui::WindowHandle<Launcher>,
    platform: Arc<dyn Platform>,
    config: Config,
    shell: Rc<RefCell<Shell>>,
) {
    let Ok(entity) = window.entity(cx) else {
        return;
    };

    cx.subscribe(&entity, move |_entity: Entity<Launcher>, event, _cx| {
        let outcome = match event {
            LauncherEvent::Dismissed => hide(&platform, &shell),
            LauncherEvent::RowsChanged(rows) => resize(&platform, &config, &shell, *rows),
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
    window: &gpui::WindowHandle<Launcher>,
    cx: &mut gpui::AsyncApp,
) -> Result<()> {
    match message {
        Message::Toggle => {
            let visible = shell.borrow().visible;
            if visible {
                hide(platform, shell)
            } else {
                show(platform, config, shell, window, cx)
            }
        }
        Message::Show => show(platform, config, shell, window, cx),
        Message::Hide => hide(platform, shell),
        Message::OverlayPress => {
            // F3's context-aware shortcut overlay is not built yet. The gesture
            // is already plumbed end to end, so what is missing is the overlay's
            // own window -- a new platform surface, not a UI feature.
            shell.borrow_mut().hold_active = true;
            log::debug!("hold gesture began (the overlay arrives in F3)");
            Ok(())
        }
        Message::OverlayRelease => {
            shell.borrow_mut().hold_active = false;
            Ok(())
        }
        Message::Ping => Ok(()),
    }
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

    let restore_to = platform.show_window(bounds)?;

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

    platform.hide_window(restore_to)
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

    platform.set_window_bounds(placement(&monitor, config, rows))
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
