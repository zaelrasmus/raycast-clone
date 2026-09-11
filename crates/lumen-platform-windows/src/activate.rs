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
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::Shell::PropertiesSystem::{
    IPropertyStore, PSGetPropertyKeyFromName, SHGetPropertyStoreForWindow,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SetForegroundWindow,
    ShowWindow, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, Interface as _, PCWSTR};

use crate::com;

/// What identifies the application a target refers to.
#[derive(Debug, Default)]
struct Identity {
    /// The packaged application's model id, when it has one.
    aumid: Option<String>,
    /// Lowercased executable stems any of which counts as a match, best first.
    stems: Vec<String>,
}

impl Identity {
    fn is_empty(&self) -> bool {
        self.aumid.is_none() && self.stems.is_empty()
    }
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

    unsafe { raise(hwnd) };
    Ok(true)
}

/// Work out what to look for.
fn identify(target: &LaunchTarget) -> Identity {
    let parsing = match target {
        LaunchTarget::ShellExecute(name) => name.clone(),
        LaunchTarget::AppUserModelId(aumid) => {
            return Identity {
                aumid: Some(aumid.to_lowercase()),
                stems: Vec::new(),
            };
        }
        LaunchTarget::DesktopEntry(_) => return Identity::default(),
    };

    // `shell:AppsFolder\<something>`. Everything the index produces has this
    // shape; a bare path is also accepted so the function is usable with one.
    let item = parsing
        .rsplit_once('\\')
        .map(|(_, item)| item)
        .unwrap_or(&parsing);

    // A packaged entry is `Publisher.Product_hash!App` -- the `!` is what
    // distinguishes it from a file name, and the whole string is the model id.
    if item.contains('!') {
        return Identity {
            aumid: Some(item.to_lowercase()),
            stems: Vec::new(),
        };
    }

    let mut stems = Vec::new();

    // Rule 2, when the shortcut can be found and read.
    if let Some(resolved) = resolve_shortcut(&parsing)
        && let Some(stem) = stem_of(&resolved)
    {
        stems.push(stem);
    }

    // Rule 3: the shortcut's own name.
    if let Some(stem) = stem_of(item)
        && !stems.contains(&stem)
    {
        stems.push(stem);
    }

    Identity { aumid: None, stems }
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
///
/// Goes through `IShellLink` rather than parsing the file: the format is
/// undocumented in the parts that matter, and the shell resolves relative
/// paths, environment variables and the 32/64-bit Program Files split on the
/// way out.
fn resolve_shortcut(parsing_name: &str) -> Option<String> {
    // The parsing name reaches the shortcut through the AppsFolder namespace;
    // `IShellLink` wants a filesystem path, so ask the shell for one.
    let path = crate::apps::filesystem_path(parsing_name)?;
    if !path.to_lowercase().ends_with(".lnk") {
        return Some(path);
    }

    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;
        file.Load(
            PCWSTR(com::wide(&path).as_ptr()),
            windows::Win32::System::Com::STGM_READ,
        )
        .ok()?;

        let mut buffer = [0u16; MAX_PATH as usize];
        link.GetPath(&mut buffer, std::ptr::null_mut(), 0).ok()?;

        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        (end > 0).then(|| String::from_utf16_lossy(&buffer[..end]))
    }
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

fn matches(hwnd: HWND, identity: &Identity) -> bool {
    if let Some(wanted) = &identity.aumid {
        return window_aumid(hwnd).is_some_and(|found| &found == wanted);
    }

    window_stem(hwnd).is_some_and(|stem| identity.stems.contains(&stem))
}

/// A window's Application User Model ID, when it has one.
fn window_aumid(hwnd: HWND) -> Option<String> {
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
        (!text.is_empty()).then(|| text.to_lowercase())
    }
}

/// The lowercased stem of the executable a window belongs to.
fn window_stem(hwnd: HWND) -> Option<String> {
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

        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        stem_of(&path)
    }
}

/// Bring a window forward.
///
/// Restores it first: a minimised window given the foreground stays minimised,
/// and the user sees the taskbar flash and nothing else.
unsafe fn raise(hwnd: HWND) {
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

    #[test]
    fn a_packaged_target_is_identified_by_its_model_id() {
        let identity = identify(&LaunchTarget::ShellExecute(
            "shell:AppsFolder\\Microsoft.WindowsTerminal_8wekyb3d8bbwe!App".into(),
        ));

        assert_eq!(
            identity.aumid.as_deref(),
            Some("microsoft.windowsterminal_8wekyb3d8bbwe!app")
        );
        assert!(identity.stems.is_empty());
    }

    /// Rule 3 on its own, without the shell: the shortcut's name is always
    /// available even when the `.lnk` cannot be resolved.
    #[test]
    fn a_shortcut_is_identified_by_its_own_name_at_least() {
        let identity = identify(&LaunchTarget::ShellExecute(
            "shell:AppsFolder\\Discord.lnk".into(),
        ));

        assert!(identity.aumid.is_none());
        assert!(
            identity.stems.contains(&"discord".to_string()),
            "{:?}",
            identity.stems
        );
    }

    #[test]
    fn a_desktop_entry_identifies_nothing_here() {
        let identity = identify(&LaunchTarget::DesktopEntry("/usr/share/x.desktop".into()));
        assert!(identity.is_empty());
    }

    /// Whole stems, not substrings. Raising the wrong window is worse than
    /// starting a second copy.
    #[test]
    fn stems_are_compared_whole() {
        let identity = Identity {
            aumid: None,
            stems: vec!["discord".into()],
        };

        assert!(identity.stems.contains(&"discord".to_string()));
        assert!(!identity.stems.contains(&"discordcanary".to_string()));
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
