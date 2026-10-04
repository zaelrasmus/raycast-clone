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
    AppEntry, ApplicationsSink, Browser, ClipboardContent, ClipboardSink, DateStyle, FocusOwner,
    ForegroundApp, GestureSink, Hotkey, IconImage, IconSource, LaunchMode, LaunchTarget, LocalTime,
    MonitorInfo, Paste, Platform, Rect, TrayInfo, TraySink, WindowLayout, WindowRole,
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

    fn start_gesture_listener(
        &self,
        _hotkey: Hotkey,
        _timers: Option<Hotkey>,
        _sink: GestureSink,
    ) -> Result<()> {
        // The timers key is a compositor bind too: `bind = ALT SHIFT, SPACE,
        // exec, lumen timers`.
        // No global shortcut API exists on Wayland. The compositor runs the
        // thin client, which toggles this instance over the socket, so this
        // method stays a no-op rather than growing an input-capture path.
        todo!("F5: compositor-delegated activation")
    }

    fn enumerate_applications(
        &self,
        _extra_folders: &[std::path::PathBuf],
    ) -> Result<Vec<AppEntry>> {
        todo!("F5: XDG desktop entries")
    }

    fn watch_applications(
        &self,
        _sink: ApplicationsSink,
        _extra_folders: &[std::path::PathBuf],
    ) -> Result<()> {
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

    fn notify(&self, _title: &str, _body: &str) -> Result<()> {
        todo!("F5: org.freedesktop.Notifications over D-Bus")
    }

    fn clipboard_text(&self) -> Result<Option<String>> {
        todo!("F5: wl_data_device")
    }

    fn set_clipboard_text(&self, _text: &str) -> Result<()> {
        todo!("F5: wl_data_device")
    }

    fn set_clipboard(&self, _content: &ClipboardContent, _keep_out_of_history: bool) -> Result<()> {
        todo!("F5: wl_data_device, text/uri-list for files")
    }

    fn window_application(&self, _window: FocusOwner) -> Result<Option<ForegroundApp>> {
        todo!("F5: Hyprland's `clients` over its IPC socket, by window address")
    }

    fn format_date(
        &self,
        _when: &LocalTime,
        _style: DateStyle,
        _locale: Option<&str>,
    ) -> Result<String> {
        todo!("F5: the locale's patterns from glibc (nl_langinfo) or ICU")
    }

    /// Read at start, so not `todo!()`: a stub that panicked here would stop
    /// the daemon before F5 has a chance to replace it.
    fn number_separators(&self) -> Result<lumen_platform::NumberSeparators> {
        // F5: RADIXCHAR and THOUSEP from glibc's nl_langinfo.
        Ok(lumen_platform::NumberSeparators::PLAIN)
    }

    fn paste(&self, _window: FocusOwner, _paste: &Paste) -> Result<()> {
        todo!(
            "F5: wl_data_device for the text, and a virtual keyboard (zwp_virtual_keyboard_v1) for Ctrl+V"
        )
    }

    fn installed_browsers(&self) -> Result<Vec<Browser>> {
        todo!("F5: desktop entries whose MimeType lists x-scheme-handler/https")
    }

    fn open_url(&self, _url: &str, _browser: Option<&Browser>, _private: bool) -> Result<()> {
        todo!("F5: `xdg-open`, or the chosen desktop entry's Exec with %u")
    }

    fn alert_sounds(&self) -> Result<Vec<String>> {
        todo!("F5: the XDG sound theme's alarm-clock-elapsed and friends")
    }

    fn play_alert(&self, _name: &str, _repeat: bool) -> Result<()> {
        todo!("F5: play through the sound theme, with canberra or pw-play")
    }

    fn stop_alert(&self) -> Result<()> {
        todo!("F5: stop the player started by play_alert")
    }

    /// Empty until F5: *Search Files* then shows Lumen's own recents only.
    /// F5 reads `recently-used.xbel` in `$XDG_DATA_HOME`.
    fn recent_files(&self) -> Result<Vec<std::path::PathBuf>> {
        Ok(Vec::new())
    }

    /// F5: `org.freedesktop.FileManager1.ShowItems` over D-Bus.
    fn show_in_folder(&self, path: &std::path::Path) -> Result<()> {
        anyhow::bail!(
            "showing {} in its folder is not built on Linux yet (F5)",
            path.display()
        )
    }

    /// F5: the XDG desktop portal's `OpenURI` with `ask` set.
    fn open_with(&self, path: &std::path::Path) -> Result<()> {
        anyhow::bail!(
            "choosing an application for {} is not built on Linux yet (F5)",
            path.display()
        )
    }

    /// F5: the desktop portal's `FileChooser.OpenFile` with `directory: true`.
    /// Until then nothing is picked, and the page says so by adding nothing.
    fn pick_folder(
        &self,
        _title: &str,
        _start: Option<&std::path::Path>,
    ) -> Result<Option<std::path::PathBuf>> {
        Ok(None)
    }

    /// F5: the block devices `udisks2` reports as not removable, by mount
    /// point. Until then none, so *Search Files* offers no other drive.
    fn internal_drives(&self) -> Result<Vec<lumen_platform::Drive>> {
        Ok(Vec::new())
    }
}
