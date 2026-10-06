//! Custom window layouts (F17, 7.60): several applications placed at once.
//!
//! Which running window an entry takes, and which windows are "the others", is
//! decided in `lumen_platform::layouts`, where it is tested. What is here
//! reads the windows, launches what is missing, waits for it and makes the
//! calls.
//!
//! # Launch and wait
//!
//! Every missing application is launched first, then all of them are waited
//! for together: one look at the windows every 50 ms, five seconds in all. A
//! launched entry takes a window of its application that was not there before
//! the launch. An entry with an argument is launched even when its
//! application is running, and a browser handed a URL opens a tab rather than
//! a window; after 1.5 s with no new window, such an entry takes its
//! application's frontmost window that no other entry has. An application
//! that allows one instance hands a second launch to the first, so a second
//! entry of it never gets a window of its own and is reported missing.
//!
//! # Never terminating anything
//!
//! *Close Others* posts `WM_CLOSE`, the message a window's own close button
//! sends: an editor with unsaved work still asks. Minimising is
//! `ShowWindowAsync`. Neither waits on the window's thread, so a hung
//! application cannot hang the daemon (7.57).

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use lumen_platform::layouts::{self, MAX_PER_DISPLAY, Seen};
use lumen_platform::{
    AppTarget, Argument, LaunchTarget, LayoutEntry, LayoutReport, MonitorInfo, OtherWindows,
    Placement, WindowLayoutSpec, WindowSettings, window_commands,
};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint,
    MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOP, IsIconic, IsZoomed, PostMessageW, SW_MINIMIZE, SW_RESTORE, SWP_ASYNCWINDOWPOS,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow, ShowWindowAsync, WM_CLOSE,
};

use crate::activate::{self, Identity};
use crate::arrange::{self, Arranging};
use crate::commands::{self, Placements};
use crate::{com, foreground, monitor};

const POLL: Duration = Duration::from_millis(50);
const TIMEOUT: Duration = Duration::from_secs(5);
/// How long an entry launched with an argument waits for a window of its own
/// before taking its application's frontmost one.
const HANDED_OVER: Duration = Duration::from_millis(1500);

/// An entry whose application was launched, waiting for its window.
struct Waiting {
    index: usize,
    identity: Identity,
    /// The application's windows before the launch.
    before: Vec<HWND>,
    with_argument: bool,
}

/// Apply `spec`. Blocks while launched applications start, five seconds at
/// most; the daemon calls it on a background task.
pub fn apply(
    history: &Placements,
    arranging: &Arranging,
    spec: &WindowLayoutSpec,
) -> Result<LayoutReport> {
    spec.validate().map_err(anyhow::Error::msg)?;
    let _apartment = com::Apartment::enter().ok();
    let monitors = monitors()?;
    let primary = primary(&monitors);
    let settings = *arranging.settings.lock();

    let windows = commands::top_level_windows();
    let mut chosen = layouts::assign(&spec.entries, |app| matching(&windows, &identity(app)));

    let mut report = LayoutReport::default();
    let mut waiting = Vec::new();
    for (index, entry) in spec.entries.iter().enumerate() {
        if chosen[index].is_some() {
            continue;
        }
        let identity = identity(&entry.app);
        let before = matching(&windows, &identity);
        let argument = entry.argument.command_line();
        match crate::shell_execute(&entry.app.path, argument.as_deref()) {
            Ok(()) => {
                report.launched += 1;
                waiting.push(Waiting {
                    index,
                    identity,
                    before,
                    with_argument: argument.is_some(),
                });
            }
            Err(error) => {
                log::warn!("could not launch {:?}: {error:#}", entry.app.path);
                report.missing.push(entry.app.name.clone());
            }
        }
    }

    let started = Instant::now();
    while !waiting.is_empty() && started.elapsed() < TIMEOUT {
        std::thread::sleep(POLL);
        let windows = commands::top_level_windows();
        let late = started.elapsed() >= HANDED_OVER;
        waiting.retain(|wait| {
            let taken: Vec<HWND> = chosen.iter().flatten().copied().collect();
            let free: Vec<HWND> = matching(&windows, &wait.identity)
                .into_iter()
                .filter(|hwnd| !taken.contains(hwnd))
                .collect();
            let found = match free.iter().find(|hwnd| !wait.before.contains(hwnd)) {
                Some(&new) => Some(new),
                None if wait.with_argument && late => free.first().copied(),
                None => None,
            };
            chosen[wait.index] = found;
            found.is_none()
        });
    }
    for wait in waiting {
        let app = &spec.entries[wait.index].app;
        log::warn!("no window of {:?} after {TIMEOUT:?}", app.path);
        report.missing.push(app.name.clone());
    }

    let placed: Vec<(usize, HWND)> = chosen
        .iter()
        .enumerate()
        .filter_map(|(index, hwnd)| hwnd.map(|hwnd| (index, hwnd)))
        .collect();
    for &(index, hwnd) in &placed {
        let entry = &spec.entries[index];
        let display = layouts::display_index(entry.display, monitors.len(), primary);
        match unsafe {
            place(
                history,
                &settings,
                &monitors[display],
                spec.gaps,
                entry,
                hwnd,
            )
        } {
            Ok(()) => report.placed += 1,
            Err(error) => log::warn!("could not place {:?}: {error:#}", entry.app.path),
        }
    }

    if spec.other_windows != OtherWindows::None {
        let displays: Vec<usize> = spec
            .entries
            .iter()
            .map(|entry| layouts::display_index(entry.display, monitors.len(), primary))
            .collect();
        let placed: Vec<HWND> = placed.iter().map(|&(_, hwnd)| hwnd).collect();
        deal_with_others(history, spec.other_windows, &monitors, &displays, &placed);
    }

    // The layout's windows in front, in its order, and the first one active.
    unsafe {
        for &(_, hwnd) in placed.iter().rev() {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
            );
        }
        if let Some(&(_, first)) = placed.first() {
            activate::raise(first);
        }
    }
    Ok(report)
}

/// Put `hwnd` where `entry` says on `monitor`, remembering where it was for
/// *Restore* as an arrangement does (7.57).
unsafe fn place(
    history: &Placements,
    settings: &WindowSettings,
    (handle, monitor): &(HMONITOR, MonitorInfo),
    gaps: bool,
    entry: &LayoutEntry,
    hwnd: HWND,
) -> Result<()> {
    unsafe {
        commands::record(history, hwnd, false);
        if IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let current = arrange::frame_bounds(hwnd).or_else(|_| arrange::outer_bounds(hwnd))?;
        let target = Placement {
            gaps,
            ..entry.placement
        }
        .place(
            monitor.work_area,
            current,
            monitor.scale_factor,
            settings.gaps(monitor.scale_factor),
        );
        let crossing = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) != *handle;
        arrange::place(hwnd, target)?;
        if crossing {
            // A window arriving on a monitor of another scale resizes itself
            // for it; placed again, it keeps the size it was given.
            arrange::place(hwnd, target)?;
        }
        Ok(())
    }
}

/// Minimise or close the windows on `displays` the layout did not place.
fn deal_with_others(
    history: &Placements,
    other_windows: OtherWindows,
    monitors: &[(HMONITOR, MonitorInfo)],
    displays: &[usize],
    placed: &[HWND],
) {
    let seen: Vec<Seen<HWND>> = commands::top_level_windows()
        .into_iter()
        .map(|hwnd| {
            let eligible = window_commands::minimisable(commands::facts(hwnd));
            Seen {
                window: hwnd,
                display: match eligible {
                    true => display_of(monitors, hwnd).unwrap_or(usize::MAX),
                    false => usize::MAX,
                },
                eligible,
            }
        })
        .collect();
    for hwnd in layouts::others(&seen, displays, placed) {
        unsafe {
            match other_windows {
                OtherWindows::Minimize => {
                    commands::record(history, hwnd, true);
                    let _ = ShowWindowAsync(hwnd, SW_MINIMIZE);
                }
                OtherWindows::Close => {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                OtherWindows::None => {}
            }
        }
    }
}

/// The windows on screen as entries, front first (*Save Current Layout*).
pub fn current() -> Result<Vec<LayoutEntry>> {
    let _apartment = com::Apartment::enter().ok();
    let monitors = monitors()?;
    let mut per_display = vec![0; monitors.len()];
    let mut entries = Vec::new();
    for hwnd in commands::top_level_windows() {
        if !window_commands::minimisable(commands::facts(hwnd)) {
            continue;
        }
        let Some(display) = display_of(&monitors, hwnd) else {
            continue;
        };
        if per_display[display] >= MAX_PER_DISPLAY {
            continue;
        }
        let Some(app) = app_of(hwnd) else {
            continue;
        };
        let Ok(bounds) =
            (unsafe { arrange::frame_bounds(hwnd).or_else(|_| arrange::outer_bounds(hwnd)) })
        else {
            continue;
        };
        per_display[display] += 1;
        entries.push(LayoutEntry {
            app,
            argument: Argument::None,
            display: display as u32,
            placement: layouts::percent_placement(bounds, monitors[display].1.work_area),
        });
    }
    Ok(entries)
}

/// What launches the application `hwnd` belongs to: a packaged app by its
/// model id, the frame host looked through (R-14), anything else by its
/// executable.
fn app_of(hwnd: HWND) -> Option<AppTarget> {
    if let Some(aumid) = activate::window_aumid_as_written(hwnd)
        && aumid.contains('!')
    {
        let title = foreground::of(hwnd)
            .map(|app| app.title)
            .unwrap_or_default();
        return Some(AppTarget {
            path: format!(r"shell:AppsFolder\{aumid}"),
            name: if title.is_empty() { aumid } else { title },
        });
    }
    let path = activate::image_path(hwnd)?;
    if path.to_lowercase().ends_with(r"\applicationframehost.exe") {
        return None;
    }
    let name = Path::new(&path).file_stem()?.to_string_lossy().into_owned();
    Some(AppTarget { path, name })
}

fn identity(app: &AppTarget) -> Identity {
    activate::identify(&LaunchTarget::ShellExecute(app.path.clone()))
}

/// The windows in `windows` a user could switch to that belong to the
/// application `identity` names, in z-order.
fn matching(windows: &[HWND], identity: &Identity) -> Vec<HWND> {
    windows
        .iter()
        .copied()
        .filter(|&hwnd| window_commands::switchable(commands::facts(hwnd)))
        .filter(|&hwnd| activate::matches(hwnd, identity))
        .collect()
}

/// Every monitor in the platform's order, left to right.
fn monitors() -> Result<Vec<(HMONITOR, MonitorInfo)>> {
    arrange::enumerate()?
        .into_iter()
        .map(|handle| Ok((handle, monitor::describe(handle)?)))
        .collect()
}

fn primary(monitors: &[(HMONITOR, MonitorInfo)]) -> usize {
    let handle = unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) };
    monitors.iter().position(|(h, _)| *h == handle).unwrap_or(0)
}

fn display_of(monitors: &[(HMONITOR, MonitorInfo)], hwnd: HWND) -> Option<usize> {
    let handle = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    monitors.iter().position(|(h, _)| *h == handle)
}
