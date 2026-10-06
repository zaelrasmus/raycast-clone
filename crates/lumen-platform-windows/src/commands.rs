//! The window commands that are not placements (F17): minimise, restore and
//! focus, and the history *Restore* reads.
//!
//! The decisions -- which windows, which one comes back, which one is next --
//! are made in `lumen_platform::window_commands`, where they are tested. What
//! is here reads the facts they need and makes the calls.
//!
//! # Where the history lives
//!
//! In [`crate::WindowsPlatform`], behind a mutex. It is the system's own
//! placement record (`WINDOWPLACEMENT`) keyed by window handle, meaningless
//! outside this process and to any other platform, so the boundary carries
//! none of it. The mutex is taken only from [`run`], which the daemon calls
//! from its own task, never from an input callback (I-15).
//!
//! # Never waiting on another program
//!
//! Minimising goes through `ShowWindowAsync` and restoring a placement sets
//! `WPF_ASYNCWINDOWPLACEMENT`: both post to the window's thread rather than
//! waiting for it, so one hung application among those on a display cannot
//! hang the daemon with it.

use std::ffi::c_void;

use anyhow::Result;
use lumen_platform::window_commands::{self, Cycle, History, WindowFacts};
use lumen_platform::{FocusOwner, LayoutKind, WindowLayout};
use parking_lot::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, TRUE};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromPoint, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GA_ROOTOWNER, GW_OWNER, GWL_EXSTYLE, GetAncestor, GetCursorPos, GetWindow,
    GetWindowLongPtrW, GetWindowPlacement, GetWindowTextLengthW, GetWindowThreadProcessId,
    IsIconic, IsWindow, IsWindowVisible, IsZoomed, SW_MINIMIZE, SW_RESTORE, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetWindowPlacement, SetWindowPos, ShowWindowAsync, WINDOWPLACEMENT,
    WPF_ASYNCWINDOWPLACEMENT, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use crate::{activate, arrange, foreground, window};

/// What *Restore* goes back to, per window.
pub type Placements = Mutex<History<WINDOWPLACEMENT>>;

/// Apply `layout` to `target`, the window in front or the one captured before
/// the launcher appeared; `None` for the desktop.
pub fn run(
    history: &Placements,
    arranging: &arrange::Arranging,
    target: Option<FocusOwner>,
    layout: WindowLayout,
) -> Result<()> {
    let target = target.map(|FocusOwner(raw)| HWND(raw as *mut c_void));

    match (layout, target) {
        (WindowLayout::MinimizeDisplay, _) => minimize_display(history, target),
        (WindowLayout::Restore, _) => restore(history, target),
        (_, None) => {
            log::debug!("no window for {layout:?}");
            Ok(())
        }
        (WindowLayout::Minimize, Some(hwnd)) => minimize(history, hwnd),
        (WindowLayout::FocusNextAppWindow, Some(hwnd)) => focus(hwnd, Cycle::Next),
        (WindowLayout::FocusPreviousAppWindow, Some(hwnd)) => focus(hwnd, Cycle::Previous),
        (_, Some(hwnd)) => {
            debug_assert_ne!(layout.kind(), LayoutKind::Command);
            if alive(key(hwnd)) {
                record(history, hwnd, false);
            }
            arrange::arrange(arranging, FocusOwner(key(hwnd)), layout)
        }
    }
}

fn key(hwnd: HWND) -> u64 {
    hwnd.0 as u64
}

fn handle(key: u64) -> HWND {
    HWND(key as *mut c_void)
}

fn alive(key: u64) -> bool {
    unsafe { IsWindow(Some(handle(key))).as_bool() }
}

fn minimised(key: u64) -> bool {
    unsafe { IsIconic(handle(key)).as_bool() }
}

/// Remember where `hwnd` is, before Lumen moves or minimises it.
pub(crate) fn record(history: &Placements, hwnd: HWND, minimising: bool) {
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // A read of the system's own record: it sends the window nothing, so a
    // hung window cannot hold it up.
    if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_err() {
        log::debug!("no placement to remember for {hwnd:?}");
        return;
    }
    history
        .lock()
        .record(key(hwnd), placement, minimising, alive);
}

/// Minimise `hwnd` from whatever state it is in.
///
/// `SW_MINIMIZE` straight from maximised or snapped: the system's Win+Down
/// takes two presses from maximised because its first one un-maximises.
fn minimize(history: &Placements, hwnd: HWND) -> Result<()> {
    if !alive(key(hwnd)) {
        anyhow::bail!("the window closed before it could be minimised");
    }
    if minimised(key(hwnd)) {
        return Ok(());
    }
    record(history, hwnd, true);
    // Asynchronous, and nothing to report if it is refused: an elevated
    // window ignores a request from an unelevated process (arrange.rs).
    let _ = unsafe { ShowWindowAsync(hwnd, SW_MINIMIZE) };
    Ok(())
}

/// Minimise every ordinary window on the monitor of `target`, or under the
/// cursor when there is none.
fn minimize_display(history: &Placements, target: Option<HWND>) -> Result<()> {
    let monitor = match target {
        Some(hwnd) => unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) },
        None => cursor_monitor()?,
    };

    let chosen: Vec<HWND> = top_level_windows()
        .into_iter()
        .filter(|&hwnd| window_commands::minimisable(facts(hwnd)))
        .filter(|&hwnd| unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) } == monitor)
        .collect();

    // Recorded back to front, so the frontmost is the most recently
    // minimised and *Restore* brings it back first.
    for &hwnd in chosen.iter().rev() {
        record(history, hwnd, true);
    }
    for &hwnd in &chosen {
        let _ = unsafe { ShowWindowAsync(hwnd, SW_MINIMIZE) };
    }
    log::debug!("minimised {} windows on {monitor:?}", chosen.len());
    Ok(())
}

fn cursor_monitor() -> Result<HMONITOR> {
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor) }?;
    Ok(unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) })
}

/// Put a window back where it was before Lumen last arranged or minimised it.
///
/// In order:
///
/// 1. `target` has a history: that.
/// 2. Otherwise the window Lumen minimised most recently that still exists
///    and is still minimised. After a minimise the window in front is some
///    other one, so this is what makes Minimise then Restore a round trip
///    from a hotkey, and from the launcher opened over the desktop.
/// 3. Otherwise a `target` minimised or maximised is restored as the system
///    would; one in its normal state is left alone.
fn restore(history: &Placements, target: Option<HWND>) -> Result<()> {
    let mut history = history.lock();

    if let Some(hwnd) = target
        && let Some(recorded) = history.pop(key(hwnd), alive)
    {
        drop(history);
        if recorded.minimised && minimised(key(hwnd)) {
            unminimise(hwnd);
        } else {
            apply(hwnd, recorded.saved);
        }
        return Ok(());
    }

    let minimised_last = history.latest_minimised(|window| alive(window) && minimised(window));
    if let Some(window) = minimised_last {
        history.pop(window, alive);
        drop(history);
        unminimise(handle(window));
        return Ok(());
    }
    drop(history);

    if let Some(hwnd) = target
        && unsafe { IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() }
    {
        let _ = unsafe { ShowWindowAsync(hwnd, SW_RESTORE) };
    }
    Ok(())
}

/// Bring back a window Lumen minimised, in front.
///
/// `SW_RESTORE` rather than the saved placement: it returns the window to
/// the state it was minimised from, which keeps a snapped window snapped. A
/// snapped window's saved placement is its rectangle from *before* it was
/// snapped, so re-applying it would unsnap it.
fn unminimise(hwnd: HWND) {
    // Restores, then takes the foreground the way the launcher hands it back.
    unsafe { activate::raise(hwnd) };
}

/// Put a window back on the placement saved for it.
fn apply(hwnd: HWND, mut placement: WINDOWPLACEMENT) {
    placement.flags |= WPF_ASYNCWINDOWPLACEMENT;
    if let Err(error) = unsafe { SetWindowPlacement(hwnd, &placement) } {
        log::debug!("could not restore {hwnd:?}: {error}");
    }
}

/// Move focus to the next or previous window of the application `current`
/// belongs to.
///
/// The application is the executable, with a packaged application's frame
/// looked through to the program inside (R-14), so two windows of one
/// program in two processes are one application. When the executable cannot
/// be read -- an elevated window -- the process stands in for it.
fn focus(current: HWND, direction: Cycle) -> Result<()> {
    // A dialog in front cycles its owner's application, from its owner.
    let current = match unsafe { GetAncestor(current, GA_ROOTOWNER) } {
        root if !root.0.is_null() => root,
        _ => current,
    };
    let app = application(current);

    let windows: Vec<HWND> = top_level_windows()
        .into_iter()
        .filter(|&hwnd| window_commands::switchable(facts(hwnd)))
        .filter(|&hwnd| hwnd == current || application(hwnd) == app)
        .collect();

    let Some(step) = window_commands::cycle(&windows, current, direction) else {
        log::debug!("no other window of {app:?} to focus");
        return Ok(());
    };

    unsafe {
        activate::raise(step.activate);
        if let Some(behind) = step.sink_behind {
            // Behind the application's backmost window, not to the bottom of
            // everything: the other applications keep their places.
            let _ = SetWindowPos(
                current,
                Some(behind),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
    Ok(())
}

/// Which application a window belongs to, for [`focus`].
#[derive(Debug, PartialEq, Eq)]
enum Application {
    Executable(String),
    Process(u32),
}

fn application(hwnd: HWND) -> Application {
    match foreground::of(hwnd).and_then(|app| app.executable) {
        Some(executable) => Application::Executable(executable),
        None => {
            let mut process = 0;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
            Application::Process(process)
        }
    }
}

/// Every top-level window, in z-order, front first.
pub(crate) fn top_level_windows() -> Vec<HWND> {
    let mut found: Vec<HWND> = Vec::new();

    unsafe extern "system" fn visit(hwnd: HWND, state: LPARAM) -> BOOL {
        // Collect and return, as every enumeration here does.
        let found = unsafe { &mut *(state.0 as *mut Vec<HWND>) };
        found.push(hwnd);
        TRUE
    }

    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

/// What the choices in `window_commands` need to know about `hwnd`.
pub(crate) fn facts(hwnd: HWND) -> WindowFacts {
    unsafe {
        let visible = IsWindowVisible(hwnd).as_bool();
        if !visible {
            // The common case by far; nothing else is worth reading.
            return WindowFacts::default();
        }

        let mut cloaked = 0u32;
        let cloaked = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0;
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;

        WindowFacts {
            visible,
            cloaked,
            tool_window: ex_style & WS_EX_TOOLWINDOW.0 != 0,
            owned: GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.0.is_null()),
            ours: window::is_ours(hwnd),
            shell: window::is_shell(hwnd),
            titled: GetWindowTextLengthW(hwnd) > 0,
            minimised: IsIconic(hwnd).as_bool(),
        }
    }
}
