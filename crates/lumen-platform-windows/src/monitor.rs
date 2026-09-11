//! Monitor discovery.
//!
//! The launcher appears on the monitor holding the cursor, not the primary one:
//! it must show up where the user's attention already is. Per-monitor DPI
//! awareness v2 is declared in the application manifest, so the coordinates
//! below are genuine physical pixels rather than values Windows has quietly
//! scaled on our behalf.

use anyhow::{Context as _, Result};
use lumen_platform::{MonitorInfo, Rect};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// The DPI Windows reports for a 100% scale factor.
const USER_DEFAULT_SCREEN_DPI: u32 = 96;

pub fn cursor_monitor() -> Result<MonitorInfo> {
    unsafe {
        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor).context("GetCursorPos")?;

        // DEFAULTTOPRIMARY rather than DEFAULTTONEAREST: the cursor is always on
        // some monitor, so a miss here means the position was stale, and the
        // primary monitor is a better answer than the nearest one to a bad point.
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY);
        describe(monitor)
    }
}

pub(crate) fn describe(monitor: HMONITOR) -> Result<MonitorInfo> {
    unsafe {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            anyhow::bail!("GetMonitorInfoW failed for monitor {monitor:?}");
        }

        let mut dpi_x = USER_DEFAULT_SCREEN_DPI;
        let mut dpi_y = USER_DEFAULT_SCREEN_DPI;
        // A monitor can legitimately fail to report DPI (it was just
        // disconnected, for instance). 96 is the correct fallback, and a
        // slightly mispositioned window beats no window at all.
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_err() {
            dpi_x = USER_DEFAULT_SCREEN_DPI;
        }

        Ok(MonitorInfo {
            bounds: rect_from(info.rcMonitor),
            work_area: rect_from(info.rcWork),
            scale_factor: dpi_x as f32 / USER_DEFAULT_SCREEN_DPI as f32,
        })
    }
}

fn rect_from(rect: windows::Win32::Foundation::RECT) -> Rect {
    Rect::new(
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    )
}
