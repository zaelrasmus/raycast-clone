//! Window chrome, visibility and focus on Windows 11.
//!
//! GPUI creates the window; everything the launcher needs beyond that is
//! applied here to the raw `HWND`. In particular GPUI's `PlatformWindow` has no
//! `hide()`, which is why invariant I-4 -- create once, then only show and hide,
//! never destroy -- is implemented at this layer rather than in the UI.

use std::ffi::c_void;

use anyhow::{Context as _, Result};
use lumen_platform::{BackdropMode, FocusOwner, Rect};
use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE, DWMSBT_TRANSIENTWINDOW,
    DWMWA_BORDER_COLOR, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetShellWindow, GetWindowLongPtrW,
    GetWindowThreadProcessId, HWND_TOPMOST, IsWindowVisible, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT,
};
use windows::core::{BOOL, w};

/// Apply the launcher treatment to a window GPUI has already created.
///
/// Called exactly once per process. Everything here is idempotent, but there is
/// only ever one window to apply it to (I-4).
pub fn attach(hwnd: HWND, backdrop: BackdropMode, dark: bool) -> Result<()> {
    unsafe {
        // GPUI already applies WS_EX_TOOLWINDOW for `WindowKind::PopUp`, which
        // is what keeps the window out of Alt+Tab and the taskbar. We assert it
        // rather than assume it, because that is an F0 acceptance criterion and
        // it would otherwise regress silently on a GPUI upgrade (R-2).
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let mut ex_style = current as u32;
        ex_style |= WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0;
        ex_style &= !WS_EX_APPWINDOW.0;
        // Never WS_EX_NOACTIVATE: the launcher is a text field and must take
        // focus. Section 7 offers it only for the opposite decision.
        ex_style &= !WS_EX_NOACTIVATE.0;
        if ex_style != current as u32 {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);
        }

        // Dark mode first: it changes how DWM tints the backdrop, so setting it
        // after the backdrop produces a visible one-frame flash of light tint.
        let dark_mode = windows::core::BOOL::from(dark);
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_mode as *const _ as *const c_void,
            size_of_attr::<windows::core::BOOL>(),
        )
        .context("DWMWA_USE_IMMERSIVE_DARK_MODE")?;

        let corner = DWMWCP_ROUND;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const c_void,
            size_of_attr::<DWM_WINDOW_CORNER_PREFERENCE>(),
        )
        .context("DWMWA_WINDOW_CORNER_PREFERENCE")?;

        match backdrop {
            BackdropMode::SystemAcrylic => {
                // Extend the frame across the whole client area *before* asking
                // for a backdrop. DWM only composites a system backdrop behind
                // frame it owns; a window that has not extended its frame gets
                // the attribute accepted and silently ignored, which is exactly
                // the "accepted but nothing happens" failure R-1 anticipates.
                let margins = MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                };
                DwmExtendFrameIntoClientArea(hwnd, &margins)
                    .context("DwmExtendFrameIntoClientArea")?;

                let kind = DWMSBT_TRANSIENTWINDOW;
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_SYSTEMBACKDROP_TYPE,
                    &kind as *const _ as *const c_void,
                    size_of_attr::<DWM_SYSTEMBACKDROP_TYPE>(),
                )
                .context("DWMWA_SYSTEMBACKDROP_TYPE")?;

                // A single-pixel border is what separates the window from a
                // background of a similar tone. Without it an acrylic surface
                // over a dark wallpaper has no visible edge at all.
                let border = COLORREF(0x0033_3333);
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_BORDER_COLOR,
                    &border as *const _ as *const c_void,
                    size_of_attr::<COLORREF>(),
                )
                .context("DWMWA_BORDER_COLOR")?;
            }
            BackdropMode::LegacyAcrylic => {
                composition::set_accent_acrylic(hwnd)?;
            }
            BackdropMode::Solid => {}
        }
    }

    Ok(())
}

const fn size_of_attr<T>() -> u32 {
    std::mem::size_of::<T>() as u32
}

/// Show the window at `bounds` (physical pixels) and take focus.
///
/// Returns the window that held the foreground, so it can be given back on
/// hide. Capturing it *before* showing is the whole trick: once our window is
/// up, `GetForegroundWindow` returns us.
pub fn show(hwnd: HWND, bounds: Rect) -> Result<Option<FocusOwner>> {
    unsafe {
        let previous = GetForegroundWindow();

        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .context("positioning the launcher window")?;

        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        take_foreground(hwnd);

        Ok(non_null(previous).map(|hwnd| FocusOwner(hwnd.0 as u64)))
    }
}

/// Hide the window and give the foreground back to `restore_to`.
///
/// The window is hidden, never destroyed (I-4): destroying it would give back
/// the GPU initialisation cost that AD-02 exists to eliminate.
pub fn hide(hwnd: HWND, restore_to: Option<FocusOwner>) -> Result<()> {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);

        if let Some(FocusOwner(raw)) = restore_to {
            let previous = HWND(raw as *mut c_void);
            if IsWindowVisible(previous).as_bool() {
                take_foreground(previous);
            }
        }
    }
    Ok(())
}

/// Move `target` to the foreground reliably.
///
/// `SetForegroundWindow` on its own fails intermittently: Windows refuses the
/// call unless the calling thread owns the foreground or shares an input queue
/// with it. Attaching to the foreground thread's input queue for the duration
/// of the call satisfies that rule, which is why this is the documented recipe
/// in section 7 and not a superstition.
unsafe fn take_foreground(target: HWND) {
    unsafe {
        let foreground = GetForegroundWindow();
        let foreground_thread = if foreground.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(foreground, None)
        };
        let this_thread = GetCurrentThreadId();

        let attached = foreground_thread != 0
            && foreground_thread != this_thread
            && AttachThreadInput(this_thread, foreground_thread, true).as_bool();

        let _ = SetForegroundWindow(target);
        let _ = SetFocus(Some(target));

        if attached {
            let _ = AttachThreadInput(this_thread, foreground_thread, false);
        }
    }
}

/// The window in front now, as [`show`] captures it, unless it is not one to
/// arrange: Lumen's own -- any window of this process, the launcher, the
/// settings or a dialog of theirs -- or the shell's desktop or taskbar.
pub fn foreground() -> Option<FocusOwner> {
    let hwnd = non_null(unsafe { GetForegroundWindow() })?;

    let mut process = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
    if process == unsafe { GetCurrentProcessId() } {
        return None;
    }

    // The desktop is `Progman`, or the `WorkerW` that hosts the wallpaper
    // once the desktop has been shown; moving either moves the icons.
    let mut name = [0u16; 32];
    let length = unsafe { GetClassNameW(hwnd, &mut name) };
    let class = String::from_utf16_lossy(&name[..length.max(0) as usize]);
    if hwnd == unsafe { GetShellWindow() }
        || matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        )
    {
        return None;
    }

    Some(FocusOwner(hwnd.0 as u64))
}

fn non_null(hwnd: HWND) -> Option<HWND> {
    (!hwnd.0.is_null()).then_some(hwnd)
}

/// The undocumented composition-attribute acrylic.
///
/// Declared by hand because it is absent from the Windows metadata, and
/// therefore from the `windows` crate. It is the mechanism GPUI itself uses.
mod composition {
    use super::*;

    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }

    #[repr(C)]
    struct WindowCompositionAttributeData {
        attribute: u32,
        data: *mut c_void,
        size: usize,
    }

    const WCA_ACCENT_POLICY: u32 = 19;
    const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
    /// Draw the acrylic on all four edges rather than leaving hairlines.
    const ACCENT_FLAG_DRAW_ALL_BORDERS: u32 = 0x20 | 0x40 | 0x80 | 0x100;

    type SetWindowCompositionAttribute =
        unsafe extern "system" fn(HWND, *mut WindowCompositionAttributeData) -> BOOL;

    /// Resolve the undocumented entry point at run time.
    ///
    /// It cannot be linked: `user32.dll` exports it, but `user32.lib` does not
    /// declare it, because it has never been part of the public SDK. Looking it
    /// up by name is therefore not a workaround but the only way to call it --
    /// and it has the pleasant side effect that a Windows build without the
    /// export degrades to an error we can report rather than a binary that
    /// refuses to load.
    fn entry_point() -> Result<SetWindowCompositionAttribute> {
        unsafe {
            let module = GetModuleHandleW(w!("user32.dll")).context("user32.dll is not loaded")?;
            let address =
                GetProcAddress(module, windows::core::s!("SetWindowCompositionAttribute"))
                    .context(
                        "this build of Windows does not export SetWindowCompositionAttribute",
                    )?;

            Ok(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                SetWindowCompositionAttribute,
            >(address))
        }
    }

    pub fn set_accent_acrylic(hwnd: HWND) -> Result<()> {
        let set_attribute = entry_point()?;

        // AABBGGRR. The alpha here is the tint strength, not the window's
        // opacity: a low value leaves the blur legible instead of milky.
        let mut policy = AccentPolicy {
            accent_state: ACCENT_ENABLE_ACRYLICBLURBEHIND,
            accent_flags: ACCENT_FLAG_DRAW_ALL_BORDERS,
            gradient_color: 0x2800_0000,
            animation_id: 0,
        };
        let mut data = WindowCompositionAttributeData {
            attribute: WCA_ACCENT_POLICY,
            data: &mut policy as *mut _ as *mut c_void,
            size: std::mem::size_of::<AccentPolicy>(),
        };

        unsafe {
            if !set_attribute(hwnd, &mut data).as_bool() {
                anyhow::bail!("SetWindowCompositionAttribute rejected the accent policy");
            }
        }
        Ok(())
    }
}

/// Move and resize without activating or changing z-order.
///
/// `SWP_NOACTIVATE` and `SWP_NOZORDER` together are what make this safe to call
/// on every keystroke: the window keeps its topmost position and its focus, and
/// only its rectangle changes.
pub fn set_bounds(hwnd: HWND, bounds: Rect) -> Result<()> {
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
        .context("resizing the launcher window")
    }
}

/// Apply the overlay treatment: visible, on top, and untouchable.
///
/// Three extended styles carry the whole "passive" requirement of F3:
///
/// - `WS_EX_NOACTIVATE` -- clicking or showing it never makes it the foreground
///   window, so the application it is documenting keeps its focus.
/// - `WS_EX_TRANSPARENT` -- it is click-through: hit testing falls past it to
///   whatever is underneath, so it cannot swallow a click either.
/// - `WS_EX_TOOLWINDOW` -- out of Alt+Tab and the taskbar, like the launcher.
///
/// Together with never registering a keyboard hook, this is what guarantees the
/// overlay "never consumes the following keystroke": it is not in the input
/// path at all, rather than being in it and politely forwarding.
pub fn attach_overlay(hwnd: HWND, backdrop: BackdropMode, dark: bool) -> Result<()> {
    attach(hwnd, backdrop, dark)?;

    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let ex_style = current | WS_EX_NOACTIVATE.0 | WS_EX_TRANSPARENT.0 | WS_EX_TOOLWINDOW.0;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);
    }

    Ok(())
}

/// Show a window at `bounds` without activating it or taking focus.
///
/// `SWP_NOACTIVATE` plus `SW_SHOWNOACTIVATE`, and deliberately no
/// `take_foreground`: that omission is the entire difference between this and
/// [`show`], and it is what lets the overlay appear over an application without
/// interrupting it.
pub fn show_passive(hwnd: HWND, bounds: Rect) -> Result<()> {
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .context("positioning the overlay window")?;

        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok(())
}
