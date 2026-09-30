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
pub mod tray;

pub use events::{
    ApplicationsSink, ClipboardEvent, ClipboardSink, GestureEvent, GestureSink, HoldPhase, Hotkey,
    Modifier, Modifiers,
};
pub use geometry::{MonitorInfo, Rect};
pub use tiling::WindowLayout;
pub use tray::{TrayCommand, TrayInfo, TraySink};

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

/// Text to paste into another application (F11).
///
/// A struct rather than arguments, so that what a paste carries can grow --
/// a caret to move back, an HTML form of the text -- without the method's
/// signature changing under both implementations each time.
#[derive(Clone, PartialEq, Eq)]
pub struct Paste {
    pub text: String,
    /// What the paste is typed with in the target.
    pub keys: PasteKeys,
    /// How many times to press Left once pasted, to leave the caret where
    /// the snippet's `{cursor}` was. Zero leaves it at the end.
    pub caret_back: usize,
    /// Put back what was on the clipboard once the window has read the
    /// text, and keep the text out of clipboard history. `false` leaves the
    /// text on the clipboard, like any copy.
    pub restore_clipboard: bool,
}

/// Without the text: a snippet can be a password or an address, and an event
/// that is logged when it fails must not carry it into the log (7.13).
impl std::fmt::Debug for Paste {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Paste")
            .field(
                "text",
                &format_args!("<{} characters>", self.text.chars().count()),
            )
            .field("keys", &self.keys)
            .field("caret_back", &self.caret_back)
            .field("restore_clipboard", &self.restore_clipboard)
            .finish()
    }
}

/// The keystroke that pastes, which is not the same everywhere (F11).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum PasteKeys {
    /// Nearly every program.
    #[default]
    #[serde(rename = "ctrl+v")]
    CtrlV,
    /// Terminals that send `Ctrl+V` on to the program running in them.
    #[serde(rename = "ctrl+shift+v")]
    CtrlShiftV,
    /// The oldest convention, which every edit control and console honours.
    #[serde(rename = "shift+insert")]
    ShiftInsert,
}

/// A moment on the user's wall clock, already in their time zone (F11).
///
/// Plain fields rather than a date library's type, so the boundary does not
/// tie both implementations to one crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i16,
    /// 1 to 12.
    pub month: u8,
    pub day: u8,
    /// 0 is Sunday, as the system's calendars count.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// Which part of a [`LocalTime`] to write (F11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStyle {
    /// The short date: `30/09/2026`, or `9/30/2026`.
    Date,
    /// Hours and minutes.
    Time,
    /// Both, the date first.
    DateTime,
    /// The day of the week, in full.
    Weekday,
}

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

/// Whether opening something may reuse a copy that is already running.
///
/// Handing a shortcut to the shell always starts a *new* copy, and for a
/// launcher that is the wrong default. "Open Zed" when Zed is already open
/// means the window you already have; a second one is a thing you ask for.
/// Worse, an application that refuses to run twice does nothing at all, so the
/// user presses Enter and the machine appears not to have noticed.
///
/// A parameter rather than two methods because it is one decision with two
/// answers, and because the call site then has to say which it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LaunchMode {
    /// Raise a running instance if there is one, and start it otherwise.
    #[default]
    Reuse,
    /// Always start another copy, whatever is already running.
    Fresh,
}

/// A web browser the system knows about (F6).
///
/// Found through the system's own registration rather than a list of known
/// browsers, so one Lumen has never heard of is offered like any other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Browser {
    /// What the system calls it -- "Zen Browser", "Brave" -- and what the user
    /// writes to choose it.
    ///
    /// The display name rather than the registration's own key, because the key
    /// is not a name anybody knows: Zen registers as `Firefox-<hash>`.
    pub name: String,
    /// The program a URL is handed to.
    pub executable: PathBuf,
}

impl Browser {
    /// The argument that opens a URL in a private window, if this browser
    /// has one Lumen knows.
    ///
    /// Known by the program's name rather than the browser's, because that is
    /// what decides the flag: Helium's program is `chrome.exe` and takes
    /// Chromium's. A browser missing here gets no private option at all --
    /// guessing a flag would open the link in an ordinary window, which is
    /// exactly what asking for a private one is meant to prevent.
    pub fn private_flag(&self) -> Option<&'static str> {
        // Split by hand on both separators: `Path` only knows the host's, and
        // a Windows path read on Linux, or the other way round, must still
        // name its program.
        let path = self.executable.to_str()?.to_ascii_lowercase();
        let file = path.rsplit(['\\', '/']).next()?;
        let program = file.strip_suffix(".exe").unwrap_or(file);
        match program {
            "chrome" | "chromium" | "brave" | "vivaldi" | "thorium" | "helium" => {
                Some("--incognito")
            }
            "msedge" => Some("--inprivate"),
            "firefox" | "zen" | "librewolf" | "waterfox" | "floorp" | "mullvadbrowser" => {
                Some("--private-window")
            }
            "opera" => Some("--private"),
            // Opera starts through a program called `launcher`, which is too
            // common a name to trust on its own.
            "launcher" if self.name.to_ascii_lowercase().contains("opera") => Some("--private"),
            _ => None,
        }
    }
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

    /// The application that owns `window` (F11), found the same way as
    /// [`Platform::foreground_application`] -- packaged applications
    /// included -- for a window that need not be in front.
    ///
    /// What a paste is typed with depends on it: most programs paste on
    /// `Ctrl+V`, some terminals only on `Ctrl+Shift+V`.
    fn window_application(&self, window: FocusOwner) -> Result<Option<ForegroundApp>>;

    /// Begin delivering global gesture events to `sink`: `hotkey` as
    /// [`GestureEvent::Invoke`] and its hold, and `timers`, when there is one,
    /// as [`GestureEvent::Timers`] (F8).
    ///
    /// A `timers` key that cannot be bound is logged, not returned: the
    /// launcher's own hotkey is what the result reports, and one taken key
    /// must not cost the other.
    ///
    /// The implementation must not do work on the system's input callback:
    /// it pushes to the sink and returns (I-15).
    fn start_gesture_listener(
        &self,
        hotkey: Hotkey,
        timers: Option<Hotkey>,
        sink: GestureSink,
    ) -> Result<()>;

    /// Enumerate installed applications. Called off the UI thread.
    fn enumerate_applications(&self) -> Result<Vec<AppEntry>>;

    /// Report when the set of installed applications may have changed.
    ///
    /// Only a signal: the answer to "something was installed" is to call
    /// [`Platform::enumerate_applications`] again, off the UI thread, and let
    /// the result replace the index. Watching the places applications register
    /// -- rather than re-enumerating on a timer -- is what keeps idle CPU at
    /// zero: a full enumeration costs hundreds of milliseconds.
    ///
    /// Implementations should coalesce: an installer writes dozens of files,
    /// and one install should be one signal.
    fn watch_applications(&self, sink: ApplicationsSink) -> Result<()>;

    /// Resolve an icon to pixels at the requested size. Called off the UI thread.
    fn load_icon(&self, source: &IconSource, size_px: u32) -> Result<IconImage>;

    /// Open an entry. The launcher hides first, so this runs with the
    /// previous application already restored to the foreground.
    ///
    /// See [`LaunchMode`] for why this takes one.
    fn launch(&self, target: &LaunchTarget, mode: LaunchMode) -> Result<()>;

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

    /// Paste text into a window that is not ours (F11).
    ///
    /// `window` is the foreground owner captured before the launcher
    /// appeared, for [`Platform::arrange_window`]'s reason, and the launcher
    /// has already handed the foreground back to it. Nothing is typed until
    /// that window is in front: a paste into whatever happens to be there
    /// instead is the race 7.15 exists to avoid.
    ///
    /// Blocking: with [`Paste::restore_clipboard`] it returns only once the
    /// clipboard has been put back, which is when the window has read the
    /// text or has been given up on. Call it on a thread of its own.
    fn paste(&self, window: FocusOwner, paste: &Paste) -> Result<()>;

    /// A date or time written the way the user's regional settings write it,
    /// or `locale`'s (`"fr-FR"`) when one is named (F11).
    ///
    /// The system's, not Lumen's: the system already knows every locale's
    /// month names and orders, and "the way my computer writes dates" is the
    /// system's to define. An unknown locale is an error.
    fn format_date(
        &self,
        when: &LocalTime,
        style: DateStyle,
        locale: Option<&str>,
    ) -> Result<String>;

    /// Start a long-lived extension process with its input and output piped
    /// (F4, step 3).
    ///
    /// Separate from [`Platform::run_script`] because the two are different
    /// shapes, not different flavours: a script command is started, waited for
    /// and read once, while an extension is a conversation that lasts as long
    /// as its view is open.
    ///
    /// Returns `std::process::Child`, which is portable. What is *not*
    /// portable, and is why this crosses the boundary at all, is how it has to
    /// be configured: Windows needs `CREATE_NO_WINDOW` or every extension
    /// flashes a console, and Linux needs the executable bit honoured and a
    /// shebang respected.
    ///
    /// `working_directory` is the extension's own folder, so that a manifest
    /// can name `main.ps1` rather than an absolute path that only exists on the
    /// machine it was written on.
    fn spawn_extension(
        &self,
        program: &str,
        arguments: &[String],
        working_directory: &std::path::Path,
    ) -> Result<std::process::Child>;

    /// Put an icon in the notification area and deliver its commands.
    ///
    /// A launcher is deliberately invisible -- no taskbar button, no Alt+Tab
    /// entry -- and this is the one place it admits to running. Failure is not
    /// fatal: a daemon with no tray icon is harder to find and works exactly
    /// as well.
    fn install_tray(&self, info: TrayInfo, sink: TraySink) -> Result<()>;

    /// Tell the user something went wrong, in a system notification.
    ///
    /// For failures that happen after the launcher has closed -- a launch, a
    /// private window refused -- when there is no window of Lumen's left to
    /// say it in. Raised from the tray icon, so it fails if there is none.
    fn notify(&self, title: &str, body: &str) -> Result<()>;

    /// Read the system clipboard as text, if it currently holds text.
    fn clipboard_text(&self) -> Result<Option<String>>;

    /// Replace the system clipboard contents with `text`.
    fn set_clipboard_text(&self, text: &str) -> Result<()>;

    /// The browsers installed for this user, in no particular order (F6).
    ///
    /// Read from the system each time rather than cached: it is asked when a
    /// link is opened, off the UI thread, and a cache would be one more thing
    /// to invalidate when a browser is installed.
    fn installed_browsers(&self) -> Result<Vec<Browser>>;

    /// Open `url` in `browser`, or in the system's default when `None`; in a
    /// private window when `private`.
    ///
    /// `url` has already been checked to start with a scheme, so it cannot
    /// reach a browser's command line as a flag. A private window the browser
    /// cannot be asked for ([`Browser::private_flag`]) is an error, never an
    /// ordinary window.
    fn open_url(&self, url: &str, browser: Option<&Browser>, private: bool) -> Result<()>;

    /// The alert sounds the system ships, by name, sorted (F8).
    ///
    /// The system's rather than bundled ones: Windows has twenty alarms and
    /// rings in its media folder, and a desktop on Linux has a sound theme.
    /// That is no installed size and no audio decoder in the binary.
    fn alert_sounds(&self) -> Result<Vec<String>>;

    /// Play the alert sound called `name`, once, or over and over until
    /// [`Platform::stop_alert`] when `repeat`.
    ///
    /// Returns at once; the sound plays on the system's own time. One alert
    /// at a time: playing another replaces it.
    fn play_alert(&self, name: &str, repeat: bool) -> Result<()>;

    /// Stop the alert that is playing, if one is. Stopping silence is fine.
    fn stop_alert(&self) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser(name: &str, executable: &str) -> Browser {
        Browser {
            name: name.into(),
            executable: PathBuf::from(executable),
        }
    }

    #[test]
    fn each_family_gets_its_own_private_flag() {
        let cases = [
            (
                "Google Chrome",
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                "--incognito",
            ),
            (
                "Brave",
                r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
                "--incognito",
            ),
            (
                "Microsoft Edge",
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                "--inprivate",
            ),
            (
                "Zen Browser",
                r"C:\Program Files\Zen Browser\zen.exe",
                "--private-window",
            ),
            ("Firefox", "/usr/lib/firefox/firefox", "--private-window"),
            ("Chromium", "/usr/bin/chromium", "--incognito"),
            (
                "Opera GX",
                r"C:\Users\x\AppData\Local\Programs\Opera GX\opera.exe",
                "--private",
            ),
            (
                "Opera",
                r"C:\Users\x\AppData\Local\Programs\Opera\launcher.exe",
                "--private",
            ),
        ];
        for (name, executable, flag) in cases {
            assert_eq!(
                browser(name, executable).private_flag(),
                Some(flag),
                "{name}"
            );
        }
    }

    /// Helium's program is `chrome.exe`: the program decides, not the name.
    #[test]
    fn the_program_decides_not_the_name() {
        let helium = browser(
            "Helium",
            r"C:\Users\x\AppData\Local\imput\Helium\Application\chrome.exe",
        );
        assert_eq!(helium.private_flag(), Some("--incognito"));
    }

    /// No guessing: an unknown browser is offered no private window at all.
    #[test]
    fn a_paste_is_logged_without_its_text() {
        let paste = Paste {
            text: "hunter2".into(),
            keys: PasteKeys::CtrlV,
            caret_back: 0,
            restore_clipboard: true,
        };
        let logged = format!("{paste:?}");
        assert!(!logged.contains("hunter2"));
        assert!(logged.contains("<7 characters>"));
    }

    #[test]
    fn a_browser_without_a_known_flag_gets_none() {
        assert_eq!(
            browser(
                "Internet Explorer",
                r"C:\Program Files\Internet Explorer\iexplore.exe"
            )
            .private_flag(),
            None
        );
        assert_eq!(
            browser(
                "Razer AppEngine",
                r"C:\Program Files\Razer\RzAppEngine\rzappengine.exe"
            )
            .private_flag(),
            None
        );
        // `launcher.exe` is only Opera when it says so.
        assert_eq!(
            browser("Something", r"C:\x\launcher.exe").private_flag(),
            None
        );
    }
}
