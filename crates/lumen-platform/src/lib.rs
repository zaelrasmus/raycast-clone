//! The operating-system boundary (AD-04).
//!
//! This crate declares [`Platform`] and the small set of value types that cross
//! it. It contains no `#[cfg(windows)]`, no `#[cfg(unix)]` and no system calls:
//! those live in `lumen-platform-windows` and `lumen-platform-linux`.
//!
//! The surface is deliberately small. A need to cross this boundary with
//! something new signals coupling and should be fixed rather than accommodated
//! by widening the trait (AD-04). Changing any signature here requires updating
//! both implementations in the same commit (I-9).

use std::path::PathBuf;

use anyhow::Result;
use raw_window_handle::RawWindowHandle;

pub mod events;
pub mod geometry;
pub mod tiling;

pub use events::{
    ClipboardEvent, ClipboardSink, GestureEvent, GestureSink, HoldPhase, Hotkey, Modifier,
    Modifiers,
};
pub use geometry::{MonitorInfo, Rect};
pub use tiling::WindowLayout;

/// How the launcher window's background is produced.
///
/// Lives on the boundary rather than in a platform crate because it is a
/// *preference* the user configures and the UI must know about -- a translucent
/// surface and an opaque one are painted differently -- while the mechanism
/// behind each variant is entirely platform-specific.
///
/// R-1 anticipates that real transparency may not survive the combination of
/// GPUI's DirectComposition swap chain and DWM's backdrop. Rather than discover
/// that at runtime with no recourse, all three rungs of the ladder exist and are
/// selectable from configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackdropMode {
    /// Windows: `DWMWA_SYSTEMBACKDROP_TYPE` = `DWMSBT_TRANSIENTWINDOW`, the
    /// Acrylic designed for flyouts and launchers -- deliberately not Mica,
    /// which is meant for persistent windows and samples the wallpaper rather
    /// than the content behind. Hyprland: blur declared as a compositor rule
    /// for the window class.
    #[default]
    SystemAcrylic,
    /// Windows: the undocumented `SetWindowCompositionAttribute` acrylic, which
    /// is what GPUI itself drives through `WindowBackgroundAppearance::Blurred`.
    /// Kept as the middle rung: it works on builds where the documented
    /// attribute is ignored.
    LegacyAcrylic,
    /// Plan B from R-1: no blur at all, an opaque surface. Visually defensible,
    /// and it changes nothing else in the design.
    Solid,
}

impl BackdropMode {
    /// Whether the UI should paint a translucent or an opaque surface.
    ///
    /// The renderer must not clear opaquely when a backdrop is active, or the
    /// backdrop is composited behind a wall of pixels and never seen. This is
    /// the "indispensable condition" of section 7.
    pub fn wants_translucent_surface(self) -> bool {
        !matches!(self, BackdropMode::Solid)
    }
}

/// Which of the daemon's two warm windows an operation refers to.
///
/// The overlay is "a new platform surface, not a UI feature" (F3), and the two
/// surfaces differ in ways only the platform can express: the launcher takes
/// focus and the overlay must never take it, the launcher accepts clicks and the
/// overlay is click-through. Rather than adding a parallel set of
/// `show_overlay` / `hide_overlay` methods, the existing window methods take a
/// role. The method count on the boundary stays where AD-04 wants it, and
/// "two warm windows, not one" becomes something the boundary states outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowRole {
    /// The launcher: takes focus, accepts input, restores focus on hide.
    Launcher,
    /// The shortcut overlay: never activates, never takes focus, click-through,
    /// and never consumes the keystroke that follows it.
    Overlay,
    /// The settings window: an ordinary focusable window, created lazily.
    ///
    /// Unlike the other two it is *not* pre-created at startup. I-4 exists to
    /// keep invocation instant, and settings is opened deliberately and rarely;
    /// paying idle memory for a third warm window to save a few hundred
    /// milliseconds twice a month is the wrong side of the budget in section 5.
    /// Once created it is kept and only hidden, exactly like the others.
    Settings,
}

/// The application currently in the foreground.
///
/// This is the context the shortcut overlay is keyed on. The plan notes that it
/// "is already captured for focus restoration, so the incremental cost is
/// minimal" -- which is true of the window handle, though the executable name
/// behind it still has to be resolved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ForegroundApp {
    /// The executable's file name, lowercased, without a path: `"code.exe"`,
    /// `"firefox.exe"`. `None` when the process could not be opened, which
    /// happens for elevated processes when Lumen is not elevated.
    pub executable: Option<String>,
    /// The window title, used for the second rung of the fallback chain.
    pub title: String,
}

/// A handle to the window whose foreground state we took, so it can be given
/// back verbatim when the launcher hides (F0 acceptance criterion).
///
/// Opaque by construction: only the platform implementation that produced one
/// knows how to interpret it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusOwner(pub u64);

/// Where an application's icon can be found. Resolving it to pixels is the
/// platform's job, because the mechanism differs entirely between
/// `IShellItemImageFactory` and the XDG icon theme specification.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum IconSource {
    /// A shell parsing name whose associated icon should be extracted.
    ///
    /// Either a filesystem path (`.lnk`, `.exe`) or a namespace path such as
    /// `shell:AppsFolder\<app user model id>`, which is how a packaged
    /// application's icon is reached. A plain path is a valid parsing name, so
    /// one variant covers both.
    ShellItem(String),
    /// A named icon to be resolved through the platform's icon theme.
    Named(String),
    /// An image file to be decoded directly.
    File(PathBuf),
}

/// Decoded icon pixels: straight (non-premultiplied) RGBA8, `width * height * 4`.
#[derive(Debug, Clone)]
pub struct IconImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One launchable application, as discovered by the platform's index.
///
/// `lumen-core` ranks and matches these without knowing where they came from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AppEntry {
    /// Stable identity across reindexes. Used as the frecency key, so it must
    /// not change when the index is rebuilt.
    pub id: String,
    pub name: String,
    /// Publisher, package family name, or the target executable: whatever
    /// disambiguates two entries sharing a name.
    pub detail: Option<String>,
    /// What to hand back to the platform to launch this entry.
    pub target: LaunchTarget,
    pub icon: Option<IconSource>,
    /// Extra strings the matcher should consider, such as `.lnk` comments or
    /// XDG `Keywords=`. Never shown in the UI.
    pub keywords: Vec<String>,
}

/// How an [`AppEntry`] is started.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LaunchTarget {
    /// Hand a shell parsing name to the shell and let it decide.
    ///
    /// Covers `.lnk` shortcuts, documents, URLs, and packaged applications via
    /// `shell:AppsFolder\<app user model id>`. Going through the shell rather
    /// than spawning the target directly preserves the arguments, working
    /// directory and elevation the shortcut declares.
    ShellExecute(String),
    /// A packaged application, activated through its application user model ID.
    AppUserModelId(String),
    /// A desktop entry to be activated per the XDG specification.
    DesktopEntry(PathBuf),
}

/// The operating-system boundary.
///
/// Implementations are held behind an `Arc` and touched from the UI thread, so
/// methods take `&self`.
pub trait Platform: Send + Sync {
    /// Apply the platform's treatment for `role` to an already-created window:
    /// extended styles, backdrop, corner shape, dark mode.
    ///
    /// Called exactly once per role, when the daemon creates its two windows.
    /// From then on they are only shown and hidden, never destroyed (I-4).
    fn attach_window(&self, role: WindowRole, handle: RawWindowHandle) -> Result<()>;

    /// Show a window at `bounds`, on the monitor holding the cursor.
    ///
    /// [`WindowRole::Launcher`] takes focus, and the outgoing foreground owner
    /// is captured first and returned so the caller can hand it back on hide.
    /// [`WindowRole::Overlay`] never activates and always returns `None`: it
    /// documents the shortcuts of the application underneath, so taking that
    /// application's focus would defeat the entire feature.
    fn show_window(&self, role: WindowRole, bounds: Rect) -> Result<Option<FocusOwner>>;

    /// Hide a window and return focus to `restore_to`, if given.
    ///
    /// The window is hidden, never destroyed (I-4).
    fn hide_window(&self, role: WindowRole, restore_to: Option<FocusOwner>) -> Result<()>;

    /// Move and resize a window without touching focus or z-order.
    ///
    /// The launcher hugs its content: the window is exactly as tall as the rows
    /// it is showing, so it grows and shrinks as the result count changes. That
    /// is a position change, not a visibility change, and conflating the two
    /// would re-take the foreground on every keystroke.
    fn set_window_bounds(&self, role: WindowRole, bounds: Rect) -> Result<()>;

    /// The monitor containing the cursor. The launcher appears where the user's
    /// attention is, not on the primary monitor.
    fn cursor_monitor(&self) -> Result<MonitorInfo>;

    /// The application currently in the foreground.
    ///
    /// Must be called *before* either of Lumen's own windows is shown, or the
    /// answer is Lumen. Returns `None` when there is no foreground window at
    /// all, which is briefly true during a desktop switch.
    fn foreground_application(&self) -> Result<Option<ForegroundApp>>;

    /// Begin delivering global gesture events to `sink`.
    ///
    /// The implementation must not do work on the system's input callback:
    /// it pushes to the sink and returns (I-15).
    fn start_gesture_listener(&self, hotkey: Hotkey, sink: GestureSink) -> Result<()>;

    /// Enumerate installed applications. Called off the UI thread.
    fn enumerate_applications(&self) -> Result<Vec<AppEntry>>;

    /// Resolve an icon to pixels at the requested size. Called off the UI thread.
    fn load_icon(&self, source: &IconSource, size_px: u32) -> Result<IconImage>;

    /// Launch an entry. The launcher hides first, so this runs with the
    /// previous application already restored to the foreground.
    fn launch(&self, target: &LaunchTarget) -> Result<()>;

    /// Begin delivering clipboard changes to `sink`.
    ///
    /// The listener is responsible for honouring whatever "do not record this"
    /// convention the platform has -- on Windows a set of registered clipboard
    /// formats that password managers use -- because only it can see them. The
    /// rest of the privacy layer is in `lumen_core::clipboard::privacy`.
    fn start_clipboard_listener(&self, sink: ClipboardSink) -> Result<()>;

    /// Run a script command and return what it printed.
    ///
    /// On the boundary rather than reached for directly with `std::process`,
    /// because AD-09 puts every capability behind the host: "Network,
    /// filesystem, execution, clipboard and notifications pass through the host
    /// even where the subprocess could technically perform them directly…
    /// what matters is that the boundary is drawn, so that introducing a
    /// sandbox later means changing the runtime rather than redesigning the
    /// system." Tier T1 is the first thing to need execution, so this is where
    /// the line gets drawn.
    ///
    /// Called off the UI thread: a script may take as long as it likes, and
    /// nothing in the launcher waits for it.
    fn run_script(&self, path: &std::path::Path) -> Result<String>;

    /// Arrange a window that is not ours.
    ///
    /// `window` is the foreground owner captured before the launcher appeared,
    /// rather than "whatever is in front now", and that distinction is the
    /// whole of the design. Activating a row hides the launcher and hands the
    /// foreground back; asking the system afterwards which window is in front
    /// is a race the user loses by having a window resized that they did not
    /// pick. The handle names the window the user was looking at when they
    /// chose the layout.
    ///
    /// Failure is ordinary: the window may have closed, or belong to a process
    /// at a higher integrity level than ours, in which case Windows refuses
    /// silently and there is nothing to be done about it from here.
    fn arrange_window(&self, window: FocusOwner, layout: WindowLayout) -> Result<()>;

    /// Read the system clipboard as text, if it currently holds text.
    fn clipboard_text(&self) -> Result<Option<String>>;

    /// Replace the system clipboard contents with `text`.
    fn set_clipboard_text(&self, text: &str) -> Result<()>;
}
