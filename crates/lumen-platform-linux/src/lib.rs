//! Linux / Wayland implementation of the Lumen platform boundary.
//!
//! # Why this exists before it does anything
//!
//! Invariant I-2: this crate always compiles, even while its methods are
//! `todo!()`, and CI cross-checks it on every commit. That is not ceremony. The
//! failure mode it prevents -- R-4, "Windows assumptions leaking into the core,
//! making porting equivalent to rewriting" -- is invisible until the day the
//! port starts, at which point it is a year of accumulated coupling.
//!
//! Concretely: if someone widens `trait Platform` with an `HWND`-shaped
//! parameter, this file stops compiling the same day (I-9), instead of in F5.
//!
//! # What F5 will fill in
//!
//! - A `wlr-layer-shell` surface on `LAYER_OVERLAY` with exclusive keyboard
//!   interactivity. GPUI does not implement the protocol, so R-6 accepts a
//!   maintained patch to its Wayland backend as a planned task.
//! - Activation delegated to the compositor: `bind = ALT, SPACE, exec, lumen`
//!   in `hyprland.conf`, with a second invocation toggling the resident
//!   instance over the socket. That is AD-02, and it is why the daemon/client
//!   split has to exist from F1 even though Windows would not require it.
//! - Press-and-hold via Hyprland's separate `bind`/`bindr`, which gives the
//!   gesture with no input capture and no special permissions at all.
//! - The index from `.desktop` files per the XDG specification, with icon-theme
//!   resolution and `inotify` watching.

use anyhow::Result;
use lumen_platform::{
    AppEntry, FocusOwner, ForegroundApp, GestureSink, Hotkey, IconImage, IconSource, LaunchTarget,
    MonitorInfo, Platform, Rect, WindowRole,
};
use raw_window_handle::RawWindowHandle;

#[derive(Debug, Default)]
pub struct LinuxPlatform;

impl LinuxPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl Platform for LinuxPlatform {
    fn attach_window(&self, _role: WindowRole, _handle: RawWindowHandle) -> Result<()> {
        // Both roles become layer surfaces, on LAYER_OVERLAY with exclusive
        // keyboard interactivity for the launcher and none for the overlay.
        todo!("F5: wlr-layer-shell surface on LAYER_OVERLAY")
    }

    fn show_window(&self, _role: WindowRole, _bounds: Rect) -> Result<Option<FocusOwner>> {
        // On Wayland a client cannot position itself; the layer-shell surface
        // is anchored by the compositor instead. `bounds` will be advisory here,
        // which is exactly the kind of difference this crate exists to surface
        // early rather than at porting time.
        todo!("F5: map the layer surface")
    }

    fn hide_window(&self, _role: WindowRole, _restore_to: Option<FocusOwner>) -> Result<()> {
        // Focus restoration is the compositor's job under layer-shell:
        // unmapping the surface returns focus on its own, so `restore_to` is
        // expected to stay unused on this platform.
        todo!("F5: unmap the layer surface")
    }

    fn set_window_bounds(&self, _role: WindowRole, _bounds: Rect) -> Result<()> {
        // Under layer-shell the compositor owns placement; the client asks for
        // a size and an anchor rather than a rectangle. Another difference this
        // crate exists to surface before F5 rather than during it.
        todo!("F5: request a layer surface size")
    }

    fn cursor_monitor(&self) -> Result<MonitorInfo> {
        todo!("F5: wl_output enumeration")
    }

    fn foreground_application(&self) -> Result<Option<ForegroundApp>> {
        // Simpler here than on Windows: Hyprland's event socket exposes the
        // active window's class and title directly, with no process handle to
        // open and no frame host to descend through (section 8).
        todo!("F5: read the active window from Hyprland's event socket")
    }

    fn start_gesture_listener(&self, _hotkey: Hotkey, _sink: GestureSink) -> Result<()> {
        // No global shortcut API exists on Wayland. The compositor runs the
        // thin client, which toggles this instance over the socket, so this
        // method stays a no-op rather than growing an input-capture path.
        todo!("F5: compositor-delegated activation")
    }

    fn enumerate_applications(&self) -> Result<Vec<AppEntry>> {
        todo!("F5: XDG desktop entries")
    }

    fn load_icon(&self, _source: &IconSource, _size_px: u32) -> Result<IconImage> {
        todo!("F5: XDG icon theme resolution with size fallbacks")
    }

    fn launch(&self, _target: &LaunchTarget) -> Result<()> {
        todo!("F5: desktop entry activation")
    }

    fn run_script(&self, _path: &std::path::Path) -> Result<String> {
        // Portable in principle -- `std::process` works everywhere -- but the
        // policy is not: Windows needs CREATE_NO_WINDOW, and Linux needs the
        // executable bit honoured and a shebang respected.
        todo!("F5: spawn the script, honouring its shebang")
    }

    fn clipboard_text(&self) -> Result<Option<String>> {
        todo!("F5: wl_data_device")
    }

    fn set_clipboard_text(&self, _text: &str) -> Result<()> {
        todo!("F5: wl_data_device")
    }
}
