//! Finding an application that is already running, and bringing it forward.
//!
//! # Why a launcher has to do this
//!
//! Handing a shortcut to the shell starts a *new* copy. For an editor that is
//! a second window on the same project; for a chat client that refuses to run
//! twice it is nothing at all, which is worse, because the user pressed Enter
//! and the machine did not react. Either way it is not what "open Zed" means
//! when Zed is already open.
//!
//! So: look for a window that belongs to the thing being launched, and if
//! there is one, raise it instead.
//!
//! # Matching is three rules, tried in order
//!
//! There is no single identifier that covers every application on Windows, so
//! this uses the strongest one available and falls back:
//!
//! 1. **The Application User Model ID.** Exact, and the only thing that works
//!    for packaged applications, whose windows are hosted by
//!    `ApplicationFrameHost.exe` and whose process tells you nothing.
//! 2. **The executable a shortcut points at.** Covers ordinary Win32
//!    applications: `Zed.lnk` points at `Zed.exe`, and the window belongs to a
//!    process running `Zed.exe`.
//! 3. **The shortcut's own name.** Covers the Electron-shaped case, where the
//!    Start Menu entry points at an updater -- Discord's shortcut runs
//!    `Update.exe`, and the window belongs to `Discord.exe`. Rule 2 cannot
//!    match that and rule 3 can, because the shortcut is called `Discord`.
//!
//! Rule 3 is a heuristic and is deliberately last. It compares whole stems,
//! not substrings, so `Discord` does not match `DiscordCanary` — being wrong
//! here means raising the wrong window, which is more annoying than starting a
//! second copy.

use std::path::Path;

use anyhow::Result;
use lumen_platform::LaunchTarget;
use windows::Win32::Foundation::{HWND, LPARAM, MAX_PATH, TRUE};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::Shell::PropertiesSystem::{
    IPropertyStore, PSGetPropertyKeyFromName, SHGetPropertyStoreForWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SetForegroundWindow,
    ShowWindow, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PCWSTR};

use crate::com;

/// What identifies the application a target refers to.
///
/// Several candidates rather than one, because the AppsFolder does not have a
/// single naming convention. Real parsing names from one machine:
///
/// | Parsing name | What it is |
/// | ------------ | ---------- |
/// | `Microsoft.WindowsTerminal_8wekyb3d8bbwe!App` | a packaged app's model id |
/// | `ZedIndustries.Zed` | a model id with no `!` at all |
/// | `Firefox.lnk` | a shortcut file |
///
/// The middle row is the one that broke the first version of this: it has no
/// `!`, so it was not recognised as a model id, and `file_stem` then treated
/// `.Zed` as an extension and produced `zedindustries` -- which matches no
/// process on earth. A match is now anything in either list.
#[derive(Debug, Default)]
pub(crate) struct Identity {
    /// Model ids to compare against a window's own, exactly.
    aumids: Vec<String>,
    /// Lowercased executable stems, any of which counts as a match.
    stems: Vec<String>,
}

impl Identity {
    fn is_empty(&self) -> bool {
        self.aumids.is_empty() && self.stems.is_empty()
    }

    fn push_stem(&mut self, stem: String) {
        if !stem.is_empty() && !is_too_generic(&stem) && !self.stems.contains(&stem) {
            self.stems.push(stem);
        }
    }
}

/// Words that identify nothing, and would match the wrong window if trusted.
///
/// The last dot-separated segment of a model id is usually the application --
/// `ZedIndustries.Zed` -- but sometimes it is a role. Raising the wrong window
/// is worse than starting a second copy, so anything this generic is dropped
/// rather than guessed with.
fn is_too_generic(stem: &str) -> bool {
    const GENERIC: &[&str] = &[
        "app",
        "exe",
        "lnk",
        "url",
        "main",
        "desktop",
        "launcher",
        "client",
        "start",
        "run",
        "shell",
        "host",
        "application",
    ];

    stem.len() < 3 || GENERIC.contains(&stem)
}

/// Raise an already-running instance, if there is one.
///
/// Returns whether it found one. `false` is an ordinary answer, not a failure:
/// it means the caller should start the application.
pub fn raise_existing(target: &LaunchTarget) -> Result<bool> {
    let identity = identify(target);
    if identity.is_empty() {
        return Ok(false);
    }

    let Some(hwnd) = find_window(&identity) else {
        return Ok(false);
    };
    log::debug!("raising {hwnd:?} for {identity:?}");

    unsafe { raise(hwnd) };
    Ok(true)
}

/// Work out what to look for.
pub(crate) fn identify(target: &LaunchTarget) -> Identity {
    let parsing = match target {
        LaunchTarget::ShellExecute(name) => name.clone(),
        LaunchTarget::AppUserModelId(aumid) => {
            let mut identity = Identity {
                aumids: vec![aumid.to_lowercase()],
                stems: Vec::new(),
            };
            identity.push_stem(last_segment(aumid));
            return identity;
        }
        LaunchTarget::DesktopEntry(_) => return Identity::default(),
    };

    // `shell:AppsFolder\<something>`. Everything the index produces has this
    // shape; a bare path is also accepted so the function is usable with one.
    let item = parsing
        .rsplit_once('\\')
        .map(|(_, item)| item)
        .unwrap_or(&parsing);

    let mut identity = Identity::default();

    // Every AppsFolder item is *some* kind of model id, whether or not it has
    // the `!` that a packaged one carries. Comparing it against a window's own
    // is exact when it works and costs nothing when it does not.
    identity.aumids.push(item.to_lowercase());

    // A packaged entry is `Publisher.Product_hash!App`. Only the model id can
    // identify it -- its windows belong to `ApplicationFrameHost.exe`, whose
    // executable name says nothing about which application is inside.
    if item.contains('!') {
        return identity;
    }

    // What a shortcut points at, when the shell will say.
    if let Some(resolved) = resolve_shortcut(&parsing)
        && let Some(stem) = stem_of(&resolved)
    {
        identity.push_stem(stem);
    }

    // The item's own name, with any launcher extension removed. `Firefox.lnk`
    // is a file and `ZedIndustries.Zed` is not, so the extension is stripped
    // only when it is one -- otherwise `file_stem` eats `.Zed` and leaves the
    // publisher.
    let base = strip_launcher_extension(item);
    identity.push_stem(base.to_lowercase());

    // And the last dot-separated segment, which for a model id is the
    // application: `ZedIndustries.Zed` -> `zed`, matching a process running
    // `zed.exe`.
    identity.push_stem(last_segment(base));

    identity
}

/// The item's name without a trailing `.lnk`, `.exe` or `.url`.
///
/// Only those three. A dot in an AppsFolder item is far more often a namespace
/// separator than a file extension, and treating it as one is what turned
/// `ZedIndustries.Zed` into `zedindustries`.
fn strip_launcher_extension(item: &str) -> &str {
    const LAUNCHER: &[&str] = &[".lnk", ".exe", ".url"];

    for extension in LAUNCHER {
        if item.len() > extension.len()
            && item[item.len() - extension.len()..].eq_ignore_ascii_case(extension)
        {
            return &item[..item.len() - extension.len()];
        }
    }

    item
}

/// The last dot-separated segment, lowercased.
fn last_segment(value: &str) -> String {
    // The `!` half of a packaged id is a role, not a name, so it is dropped
    // before looking for the application.
    let value = value.split('!').next().unwrap_or(value);
    // As is the `_hash` a packaged family name carries.
    let value = value.split('_').next().unwrap_or(value);

    value.rsplit('.').next().unwrap_or(value).to_lowercase()
}

/// The lowercased file stem of a path or file name.
fn stem_of(path: &str) -> Option<String> {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_lowercase)
        .filter(|stem| !stem.is_empty())
}

/// What a `.lnk` points at.
fn resolve_shortcut(parsing_name: &str) -> Option<String> {
    // The parsing name reaches the shortcut through the AppsFolder namespace;
    // `IShellLink` wants a filesystem path, so ask the shell for one.
    let path = crate::apps::filesystem_path(parsing_name)?;
    if !path.to_lowercase().ends_with(".lnk") {
        return Some(path);
    }
    crate::apps::shortcut_target(&path)
}

/// Every candidate window, newest first is not available, so: the first match
/// in z-order, which is the one the user saw most recently.
fn find_window(identity: &Identity) -> Option<HWND> {
    let mut found: Vec<HWND> = Vec::new();

    unsafe extern "system" fn visit(hwnd: HWND, state: LPARAM) -> BOOL {
        let windows = unsafe { &mut *(state.0 as *mut Vec<HWND>) };
        windows.push(hwnd);
        TRUE
    }

    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut found as *mut _ as isize));
    }

    // `EnumWindows` walks in z-order, so the first acceptable match is the
    // most recently active window of that application -- which is the one to
    // come back to.
    found
        .into_iter()
        .find(|hwnd| is_candidate(*hwnd) && matches(*hwnd, identity))
}

/// Whether a window is the kind a user would switch to.
///
/// The same test the task switcher applies: visible, titled, and not a tool
/// window. Without it this matches the invisible message-only windows almost
/// every application keeps around, and raises one of those instead.
fn is_candidate(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        if GetWindowTextLengthW(hwnd) == 0 {
            return false;
        }

        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        ex_style & WS_EX_TOOLWINDOW.0 == 0
    }
}

pub(crate) fn matches(hwnd: HWND, identity: &Identity) -> bool {
    // The model id first: it is exact, and for a packaged application it is the
    // only thing that can work.
    if !identity.aumids.is_empty()
        && let Some(found) = window_aumid(hwnd)
        && identity.aumids.contains(&found)
    {
        return true;
    }

    window_stem(hwnd).is_some_and(|stem| identity.stems.contains(&stem))
}

/// A window's Application User Model ID, when it has one.
fn window_aumid(hwnd: HWND) -> Option<String> {
    window_aumid_as_written(hwnd).map(|aumid| aumid.to_lowercase())
}

/// [`window_aumid`] in the case the application gave it, to launch it by.
pub(crate) fn window_aumid_as_written(hwnd: HWND) -> Option<String> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd).ok()?;

        // Named rather than declared as a constant: `PKEY_AppUserModel_ID` is
        // absent from the Windows metadata and so from the `windows` crate,
        // and looking it up by its canonical name is both shorter and harder
        // to get silently wrong than transcribing a GUID.
        let mut key = windows::Win32::Foundation::PROPERTYKEY::default();
        PSGetPropertyKeyFromName(
            PCWSTR(com::wide("System.AppUserModel.ID").as_ptr()),
            &mut key,
        )
        .ok()?;

        let value = store.GetValue(&key).ok()?;
        let text = value.to_string();
        (!text.is_empty()).then_some(text)
    }
}

/// The lowercased stem of the executable a window belongs to.
fn window_stem(hwnd: HWND) -> Option<String> {
    stem_of(&image_path(hwnd)?)
}

/// The full path of the executable a window belongs to.
pub(crate) fn image_path(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }

        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buffer = [0u16; MAX_PATH as usize];
        let mut length = buffer.len() as u32;
        let queried = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );

        let _ = windows::Win32::Foundation::CloseHandle(process);
        queried.ok()?;

        Some(String::from_utf16_lossy(&buffer[..length as usize]))
    }
}

/// Bring a window forward.
///
/// Restores it first: a minimised window given the foreground stays minimised,
/// and the user sees the taskbar flash and nothing else.
pub(crate) unsafe fn raise(hwnd: HWND) {
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }

        // The same attach-and-set dance the launcher uses on itself: Windows
        // refuses `SetForegroundWindow` from a process that does not own the
        // foreground, unless its input queue is attached to the one that does.
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

        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));

        if attached {
            let _ = AttachThreadInput(this_thread, foreground_thread, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three shapes an AppsFolder item really takes, taken from a running
    /// machine rather than imagined. The middle one is the regression: it has
    /// no `!`, and the first version of this read `ZedIndustries.Zed` as a file
    /// called `ZedIndustries` with a `.Zed` extension.
    #[test]
    fn a_model_id_without_an_exclamation_mark_still_finds_the_application() {
        let identity = identify(&LaunchTarget::ShellExecute(
            "shell:AppsFolder\\ZedIndustries.Zed".into(),
        ));

        assert!(
            identity.stems.contains(&"zed".to_string()),
            "a window owned by zed.exe has to match: {:?}",
            identity.stems
        );
        assert!(!identity.stems.contains(&"zedindustries".to_string()));
    }

    #[test]
    fn a_packaged_target_is_identified_by_its_model_id() {
        let identity = identify(&LaunchTarget::ShellExecute(
            "shell:AppsFolder\\Microsoft.WindowsTerminal_8wekyb3d8bbwe!App".into(),
        ));

        assert!(
            identity
                .aumids
                .contains(&"microsoft.windowsterminal_8wekyb3d8bbwe!app".to_string())
        );
        // `App` is a role, not a name, and `windowsterminal` is what the
        // process would be called if it had one of its own.
        assert!(!identity.stems.contains(&"app".to_string()));
    }

    #[test]
    fn a_shortcut_file_loses_only_a_real_extension() {
        let identity = identify(&LaunchTarget::ShellExecute(
            "shell:AppsFolder\\Firefox.lnk".into(),
        ));

        assert!(identity.stems.contains(&"firefox".to_string()));
        assert!(!identity.stems.contains(&"lnk".to_string()));
    }

    /// A segment that names a role rather than an application must not be
    /// trusted: raising the wrong window is worse than starting a second copy.
    #[test]
    fn generic_segments_are_not_matched_on() {
        assert!(is_too_generic("app"));
        assert!(is_too_generic("exe"));
        assert!(is_too_generic("ui"));
        assert!(!is_too_generic("zed"));
        assert!(!is_too_generic("firefox"));
    }

    #[test]
    fn the_last_segment_ignores_the_packaged_suffixes() {
        assert_eq!(last_segment("ZedIndustries.Zed"), "zed");
        assert_eq!(
            last_segment("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"),
            "windowsterminal"
        );
        assert_eq!(last_segment("Firefox"), "firefox");
    }

    #[test]
    fn only_launcher_extensions_are_stripped() {
        assert_eq!(strip_launcher_extension("Firefox.lnk"), "Firefox");
        assert_eq!(strip_launcher_extension("Tool.EXE"), "Tool");
        // Not an extension: a namespace separator.
        assert_eq!(
            strip_launcher_extension("ZedIndustries.Zed"),
            "ZedIndustries.Zed"
        );
    }

    #[test]
    fn a_desktop_entry_identifies_nothing_here() {
        let identity = identify(&LaunchTarget::DesktopEntry("/usr/share/x.desktop".into()));
        assert!(identity.is_empty());
    }

    #[test]
    fn a_stem_is_the_file_name_without_its_extension() {
        assert_eq!(
            stem_of("C:/Program Files/Zed/Zed.exe").as_deref(),
            Some("zed")
        );
        assert_eq!(stem_of("Discord.lnk").as_deref(), Some("discord"));
        assert_eq!(stem_of(""), None);
    }
}
