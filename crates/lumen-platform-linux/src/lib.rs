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
//! The full plan, method by method, is in `docs/F5-LINUX.md`.
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
    AppEntry, ApplicationsSink, Browser, ClipboardSink, FocusOwner, ForegroundApp, GestureSink,
    Hotkey, IconImage, IconSource, LaunchMode, LaunchTarget, MonitorInfo, Platform, Rect, TrayInfo,
    TraySink, WindowLayout, WindowRole,
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

    fn watch_applications(&self, _sink: ApplicationsSink) -> Result<()> {
        // inotify on every `applications/` folder under `` and
        // ``. Package managers write `.desktop` files there and
        // nowhere else, so the watch is simpler than Windows', which needs a
        // registry key as well as the Start Menu for packaged applications.
        todo!("F5: inotify on the XDG applications folders")
    }

    fn load_icon(&self, _source: &IconSource, _size_px: u32) -> Result<IconImage> {
        todo!("F5: XDG icon theme resolution with size fallbacks")
    }

    fn launch(&self, _target: &LaunchTarget, _mode: LaunchMode) -> Result<()> {
        // Reuse is the compositor's business here rather than the shell's:
        // Hyprland can be asked for the client list over the same IPC socket
        // AD-02 already uses, and told to focus one.
        todo!("F5: desktop entry activation, and `hyprctl dispatch focuswindow`")
    }

    fn start_clipboard_listener(&self, _sink: ClipboardSink) -> Result<()> {
        // Wayland has no clipboard-change notification for a client that does
        // not own the selection. The established route is the
        // `wlr-data-control` protocol, which Hyprland supports and which exists
        // precisely for clipboard managers.
        todo!("F5: observe the selection through wlr-data-control")
    }

    fn run_script(&self, _path: &std::path::Path) -> Result<String> {
        // Portable in principle -- `std::process` works everywhere -- but the
        // policy is not: Windows needs CREATE_NO_WINDOW, and Linux needs the
        // executable bit honoured and a shebang respected.
        todo!("F5: spawn the script, honouring its shebang")
    }

    fn arrange_window(&self, _window: FocusOwner, _layout: WindowLayout) -> Result<()> {
        // The one method here that gets *easier* on this side. Wayland gives a
        // client no way to move another client's window, and it should not:
        // the compositor owns layout. Hyprland exposes exactly this over its
        // IPC socket (`dispatch movewindow`, `dispatch resizeactive`), which is
        // the same socket AD-02 already uses for activation.
        //
        // `lumen_platform::tiling` does the arithmetic for both platforms, so
        // what lands here is the dispatch and nothing else.
        todo!("F5: arrange through Hyprland's IPC socket")
    }

    fn spawn_extension(
        &self,
        _program: &str,
        _arguments: &[String],
        _working_directory: &std::path::Path,
    ) -> Result<std::process::Child> {
        // Nearly portable: `std::process` does the work on both sides. What
        // differs is the policy around it -- no console window to suppress
        // here, but the executable bit and the shebang matter, and a manifest
        // naming `./main.sh` has to be resolved against the extension's folder
        // rather than against `PATH`.
        todo!("F5: spawn with the executable bit and shebang honoured")
    }

    fn install_tray(&self, _info: TrayInfo, _sink: TraySink) -> Result<()> {
        // No notification area on Wayland: the equivalent is the
        // StatusNotifierItem D-Bus interface, which Waybar and the other
        // Hyprland status bars implement. A different mechanism for the same
        // idea, which is why it is behind the same method.
        todo!("F5: StatusNotifierItem over D-Bus")
    }

    fn clipboard_text(&self) -> Result<Option<String>> {
        todo!("F5: wl_data_device")
    }

    fn set_clipboard_text(&self, _text: &str) -> Result<()> {
        todo!("F5: wl_data_device")
    }

    fn installed_browsers(&self) -> Result<Vec<Browser>> {
        todo!("F5: desktop entries whose MimeType lists x-scheme-handler/https")
    }

    fn open_url(&self, _url: &str, _browser: Option<&Browser>, _private: bool) -> Result<()> {
        todo!("F5: `xdg-open`, or the chosen desktop entry's Exec with %u")
    }
}
