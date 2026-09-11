//! Windows 11 implementation of the Lumen platform boundary.
//!
//! Everything `#[cfg(windows)]` in the project lives in this crate, so that
//! `lumen-core` can hold invariant I-1: no operating-system calls, no
//! conditional compilation, nothing that would have to be rewritten for the
//! Wayland port in F5.

#![cfg(target_os = "windows")]

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use lumen_platform::{
    AppEntry, ClipboardSink, FocusOwner, ForegroundApp, GestureSink, Hotkey, IconImage, IconSource,
    LaunchTarget, MonitorInfo, Platform, Rect, WindowLayout, WindowRole,
};
use raw_window_handle::RawWindowHandle;
use windows::Win32::Foundation::HWND;

mod apps;
mod arrange;
mod clipboard;
mod clipboard_watch;
mod com;
mod foreground;
mod icons;
mod input;
mod monitor;
mod scripts;
mod window;

pub use lumen_platform::BackdropMode;

/// Construction-time settings that do not belong on the platform trait.
///
/// AD-04 keeps `trait Platform` deliberately small. A knob that only Windows has
/// is configured on the Windows implementation, not added to the boundary every
/// platform has to honour.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsPlatformOptions {
    pub backdrop: BackdropMode,
    pub dark: bool,
}

pub struct WindowsPlatform {
    options: WindowsPlatformOptions,
    /// The launcher window's handle, published once `attach_window` runs.
    ///
    /// Atomics rather than a lock because they are read on every show and hide
    /// and written exactly once each, and because `HWND` is a raw pointer that
    /// would otherwise make this type `!Send`.
    launcher: AtomicU64,
    /// The overlay window's handle. Two warm windows in memory, not one (F3).
    overlay: AtomicU64,
    /// The settings window's handle, published only once it has been opened.
    settings: AtomicU64,
}

impl WindowsPlatform {
    pub fn new(options: WindowsPlatformOptions) -> Self {
        Self {
            options,
            launcher: AtomicU64::new(0),
            overlay: AtomicU64::new(0),
            settings: AtomicU64::new(0),
        }
    }

    fn slot(&self, role: WindowRole) -> &AtomicU64 {
        match role {
            WindowRole::Launcher => &self.launcher,
            WindowRole::Overlay => &self.overlay,
            WindowRole::Settings => &self.settings,
        }
    }

    fn window(&self, role: WindowRole) -> Result<HWND> {
        let raw = self.slot(role).load(Ordering::Acquire);
        anyhow::ensure!(raw != 0, "the {role:?} window has not been attached yet");
        Ok(HWND(raw as *mut c_void))
    }
}

impl Platform for WindowsPlatform {
    fn attach_window(&self, role: WindowRole, handle: RawWindowHandle) -> Result<()> {
        let RawWindowHandle::Win32(handle) = handle else {
            anyhow::bail!("expected a Win32 window handle, got {handle:?}");
        };

        let hwnd = HWND(handle.hwnd.get() as *mut c_void);
        match role {
            WindowRole::Launcher => window::attach(hwnd, self.options.backdrop, self.options.dark)?,
            WindowRole::Overlay => {
                window::attach_overlay(hwnd, self.options.backdrop, self.options.dark)?
            }
            // Settings gets the launcher's chrome: rounded, dark-aware, out of
            // Alt+Tab. It differs from the launcher only in that it is opened
            // deliberately rather than summoned, which is a question of when it
            // is created, not of how it looks.
            WindowRole::Settings => window::attach(hwnd, self.options.backdrop, self.options.dark)?,
        }
        self.slot(role).store(hwnd.0 as u64, Ordering::Release);
        Ok(())
    }

    fn show_window(&self, role: WindowRole, bounds: Rect) -> Result<Option<FocusOwner>> {
        let hwnd = self.window(role)?;
        match role {
            WindowRole::Launcher => window::show(hwnd, bounds),
            // Shown without activating and without being raised into the
            // foreground: the overlay annotates whatever is underneath it, and
            // stealing that window's focus would break the shortcuts it is
            // describing.
            WindowRole::Overlay => window::show_passive(hwnd, bounds).map(|()| None),
            WindowRole::Settings => window::show(hwnd, bounds),
        }
    }

    fn hide_window(&self, role: WindowRole, restore_to: Option<FocusOwner>) -> Result<()> {
        window::hide(self.window(role)?, restore_to)
    }

    fn set_window_bounds(&self, role: WindowRole, bounds: Rect) -> Result<()> {
        window::set_bounds(self.window(role)?, bounds)
    }

    fn cursor_monitor(&self) -> Result<MonitorInfo> {
        monitor::cursor_monitor()
    }

    fn foreground_application(&self) -> Result<Option<ForegroundApp>> {
        foreground::current()
    }

    fn start_gesture_listener(&self, hotkey: Hotkey, sink: GestureSink) -> Result<()> {
        input::start(hotkey, sink)
    }

    fn enumerate_applications(&self) -> Result<Vec<AppEntry>> {
        apps::enumerate()
    }

    fn load_icon(&self, source: &IconSource, size_px: u32) -> Result<IconImage> {
        icons::load(source, size_px)
    }

    fn launch(&self, target: &LaunchTarget) -> Result<()> {
        let parsing_name = match target {
            LaunchTarget::ShellExecute(name) => name.clone(),
            LaunchTarget::AppUserModelId(aumid) => format!("shell:AppsFolder\\{aumid}"),
            LaunchTarget::DesktopEntry(path) => {
                anyhow::bail!("desktop entries are a Linux concept: {}", path.display())
            }
        };
        shell_execute(&parsing_name)
    }

    fn start_clipboard_listener(&self, sink: ClipboardSink) -> Result<()> {
        clipboard_watch::start(sink)
    }

    fn run_script(&self, path: &std::path::Path) -> Result<String> {
        scripts::run(path)
    }

    fn arrange_window(&self, window: FocusOwner, layout: WindowLayout) -> Result<()> {
        arrange::arrange(window, layout)
    }

    fn clipboard_text(&self) -> Result<Option<String>> {
        clipboard::text()
    }

    fn set_clipboard_text(&self, text: &str) -> Result<()> {
        clipboard::set_text(text)
    }
}

/// Hand a parsing name to the shell.
///
/// Deliberately not `Command::spawn`: the shell is what knows a shortcut's
/// arguments, working directory and elevation requirements, and what can
/// activate a packaged application at all.
fn shell_execute(parsing_name: &str) -> Result<()> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let result = unsafe {
        ShellExecuteW(
            None,
            &com::wide("open"),
            &com::wide(parsing_name),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW reports failure as a "HINSTANCE" of 32 or less. This is one
    // of the oldest and worst calling conventions in the API, and it is the
    // reason a plain `.ok()?` here would silently succeed on every failure.
    let code = result.0 as usize;
    anyhow::ensure!(
        code > 32,
        "the shell refused to launch {parsing_name:?} (code {code})"
    );
    Ok(())
}
