//! Moving somebody else's window (F3, window management).
//!
//! # The invisible border
//!
//! Since Windows 10 a window's resize border is drawn *outside* what the user
//! perceives as its edge, and `GetWindowRect` reports the outer rectangle while
//! DWM composites the inner one. Placing a window at exactly half the work area
//! therefore leaves a visible strip of desktop down the middle and lets the
//! window overhang the screen edges by the same amount -- around seven physical
//! pixels at 100%, more when scaled.
//!
//! `DWMWA_EXTENDED_FRAME_BOUNDS` reports the rectangle that is actually drawn.
//! The difference between the two is the padding to expand a target by, and
//! honouring it is the difference between a tiling feature that looks
//! deliberate and one that looks approximate.
//!
//! # What cannot be done, and is not attempted
//!
//! A process cannot move the window of a process running at a higher integrity
//! level. Windows does not report this as an error -- `SetWindowPos` simply
//! returns success and nothing moves -- so there is nothing to detect and
//! nothing to report. Running Lumen elevated to arrange elevated windows is a
//! decision for whoever installs it, not a workaround to build in.
//!
//! # Gaps and cycling (7.58)
//!
//! The configured gaps are logical pixels; each arrangement scales them by
//! the scale factor of the monitor the window lands on. With a screen gap,
//! *Maximise* places the window on the inset work area instead of
//! maximising it, because a maximised window cannot have a margin; with
//! none it is a real `SW_MAXIMIZE`, as before. Which size or monitor a
//! repeated half goes to is `window_commands::half_step`'s choice; what is
//! here is the half Lumen last placed, which it needs.

use std::ffi::c_void;

use anyhow::{Context as _, Result};
use lumen_platform::tiling::{self, Share, WindowLayout};
use lumen_platform::window_commands::{self, HalfStep, Placed};
use lumen_platform::{FocusOwner, Rect, WindowSettings};
use parking_lot::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsIconic, IsWindow, IsZoomed, SW_MAXIMIZE, SW_RESTORE, SWP_NOACTIVATE,
    SWP_NOZORDER, SetWindowPos, ShowWindow,
};

use crate::monitor;

/// What arranging reads and remembers between calls.
///
/// Both locks are taken only from [`arrange`], which the daemon calls from
/// its own task, and the settings once more on a configuration reload; never
/// from an input callback (I-15).
#[derive(Default)]
pub struct Arranging {
    pub settings: Mutex<WindowSettings>,
    /// The half Lumen placed last, under the layout that was *pressed*: a
    /// half carried to another monitor is recorded as the half asked for, so
    /// pressing it again carries it on.
    last_half: Mutex<Option<Placed>>,
}

/// Apply `layout` to `window`.
pub fn arrange(arranging: &Arranging, window: FocusOwner, layout: WindowLayout) -> Result<()> {
    let hwnd = HWND(window.0 as *mut c_void);
    let settings = *arranging.settings.lock();

    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            anyhow::bail!("the window closed before it could be arranged");
        }

        // A minimised window has no meaningful rectangle to work from, and a
        // maximised one ignores `SetWindowPos` entirely. Both are restored
        // first, which is also what the user means: picking "left half" for a
        // maximised window is a request to unmaximise it.
        if IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }

        // Anything but a half ends a cycle; a half records its own below.
        let last = arranging.last_half.lock().take();

        if layout.changes_monitor() {
            return place(hwnd, move_to_adjacent_monitor(hwnd, layout)?);
        }

        let monitor = monitor::describe(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST))?;
        let gaps = settings.gaps(monitor.scale_factor);

        if layout == WindowLayout::Maximize && gaps.screen == 0 {
            // Genuinely maximised rather than a window the size of the screen.
            // The two differ when the taskbar auto-hides, when the title bar
            // is double-clicked, and in what the window itself is told about
            // its own state.
            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
            return Ok(());
        }

        // The visible rectangle, read after the restore above, so that the
        // layouts which keep the window's size or one of its coordinates
        // keep what the user sees rather than the invisible border or the
        // maximised state being left.
        let current = match frame_bounds(hwnd) {
            Ok(drawn) => drawn,
            Err(_) => outer_bounds(hwnd)?,
        };

        if !layout.is_half() {
            let target = layout
                .place(monitor.work_area, current, monitor.scale_factor, gaps)
                .expect("only the monitor moves have no place, and they are handled above");
            return place(hwnd, target);
        }

        let step =
            window_commands::half_step(settings.cycling, last.as_ref(), window.0, layout, current);
        let (share, target) = match step {
            HalfStep::Here(share) => {
                let target = layout
                    .place_at(
                        share,
                        monitor.work_area,
                        current,
                        monitor.scale_factor,
                        gaps,
                    )
                    .expect("a half always has a place");
                (share, target)
            }
            HalfStep::Across(facing) => {
                match half_across(&settings, &monitor, facing, layout, current)? {
                    Some(target) => (Share::Half, target),
                    // No monitor that way: the window stays, and so does the
                    // record, so the next press asks again.
                    None => {
                        *arranging.last_half.lock() = last;
                        return Ok(());
                    }
                }
            }
        };

        place(hwnd, target)?;
        *arranging.last_half.lock() = Some(Placed {
            window: window.0,
            layout,
            share,
            rect: target,
        });
        Ok(())
    }
}

/// The `facing` half of the monitor next to `from` toward `toward`, with that
/// monitor's gaps; `None` when there is no monitor that way.
unsafe fn half_across(
    settings: &WindowSettings,
    from: &lumen_platform::MonitorInfo,
    facing: WindowLayout,
    toward: WindowLayout,
    current: Rect,
) -> Result<Option<Rect>> {
    let monitors = enumerate()?
        .into_iter()
        .map(monitor::describe)
        .collect::<Result<Vec<_>>>()?;
    let bounds: Vec<Rect> = monitors.iter().map(|m| m.bounds).collect();
    let Some(index) = tiling::monitor_toward(&bounds, from.bounds, toward) else {
        return Ok(None);
    };
    let to = &monitors[index];
    let gaps = settings.gaps(to.scale_factor);
    Ok(facing.place(to.work_area, current, to.scale_factor, gaps))
}

/// Move `hwnd` to the monitor next to the one it is on, keeping its share of
/// the screen.
unsafe fn move_to_adjacent_monitor(hwnd: HWND, layout: WindowLayout) -> Result<Rect> {
    let monitors = enumerate()?;
    if monitors.len() < 2 {
        anyhow::bail!("there is only one monitor");
    }

    let current = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let index = monitors
        .iter()
        .position(|&handle| handle == current)
        .context("the window is on a monitor that is no longer attached")?;

    // Wrapping, so that "next" on the last monitor returns to the first. A
    // two-monitor setup is the common case and wrapping makes one key do both
    // directions there, which is what people actually do with it.
    let destination = match layout {
        WindowLayout::NextMonitor => (index + 1) % monitors.len(),
        _ => (index + monitors.len() - 1) % monitors.len(),
    };

    let from = monitor::describe(current)?;
    let to = monitor::describe(monitors[destination])?;

    Ok(tiling::carry_between(
        unsafe { frame_bounds(hwnd)? },
        from.work_area,
        to.work_area,
    ))
}

/// Every monitor, in a stable left-to-right, top-to-bottom order.
///
/// Ordered by position rather than by whatever `EnumDisplayMonitors` happens to
/// return, because "next monitor" has to mean the one to the right every time.
/// The enumeration order follows the display adapter and can change when a
/// monitor is unplugged and plugged back in.
pub(crate) fn enumerate() -> Result<Vec<HMONITOR>> {
    let mut found: Vec<(HMONITOR, i32, i32)> = Vec::new();

    unsafe extern "system" fn callback(
        handle: HMONITOR,
        _hdc: HDC,
        rect: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        // I-15 does not apply here -- this is not an input callback -- but the
        // same discipline does: collect and return, do no work inside it.
        let found = unsafe { &mut *(data.0 as *mut Vec<(HMONITOR, i32, i32)>) };
        let rect = unsafe { &*rect };
        found.push((handle, rect.left, rect.top));
        true.into()
    }

    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(callback),
            LPARAM(&mut found as *mut _ as isize),
        )
        .ok()
        .context("EnumDisplayMonitors")?;
    }

    found.sort_by_key(|&(_, left, top)| (left, top));
    Ok(found.into_iter().map(|(handle, _, _)| handle).collect())
}

/// The rectangle DWM actually draws, which is what the user sees.
pub(crate) unsafe fn frame_bounds(hwnd: HWND) -> Result<Rect> {
    unsafe {
        let mut rect = RECT::default();
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .context("DWMWA_EXTENDED_FRAME_BOUNDS")?;

        Ok(Rect::new(
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        ))
    }
}

/// The window's outer rectangle, invisible border included.
pub(crate) unsafe fn outer_bounds(hwnd: HWND) -> Result<Rect> {
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).context("GetWindowRect")?;
        Ok(Rect::new(
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        ))
    }
}

/// Put the window's *visible* edges on `target`.
pub(crate) unsafe fn place(hwnd: HWND, target: Rect) -> Result<()> {
    unsafe {
        let mut outer = RECT::default();
        GetWindowRect(hwnd, &mut outer).context("GetWindowRect")?;

        // The padding is the invisible border: how far the outer rectangle
        // extends past the drawn one on each side. Usually zero at the top,
        // where there is no resize border to hide.
        let (left, top, right, bottom) = match frame_bounds(hwnd) {
            Ok(drawn) => (
                (drawn.x - outer.left).max(0),
                (drawn.y - outer.top).max(0),
                (outer.right - (drawn.x + drawn.width)).max(0),
                (outer.bottom - (drawn.y + drawn.height)).max(0),
            ),
            // A window DWM will not describe -- a console host, something
            // drawing its own frame -- is placed as asked. Slightly wrong beats
            // not moving.
            Err(_) => (0, 0, 0, 0),
        };

        SetWindowPos(
            hwnd,
            None,
            target.x - left,
            target.y - top,
            target.width + left + right,
            target.height + top + bottom,
            // Never SWP_SHOWWINDOW and never an activation: the launcher has
            // already handed the foreground back, and re-activating here would
            // take it a second time from a window that already has it.
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .context("SetWindowPos")
    }
}
