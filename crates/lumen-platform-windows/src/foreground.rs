//! Which application is in front.
//!
//! This is the context the shortcut overlay is keyed on. Section 8 notes that
//! Hyprland hands this over directly from its event socket, while Windows needs
//! `GetForegroundWindow` chained into `GetWindowThreadProcessId` and
//! `QueryFullProcessImageNameW`. That chain is here.
//!
//! # R-14, and the half of it that is solvable
//!
//! A packaged application does not own its own top-level window: the window
//! belongs to `ApplicationFrameHost.exe`, and the real process is behind a child
//! window. Reporting `applicationframehost.exe` would mean every packaged
//! application shares one sheet, which is the same as having no sheets. The
//! descent below fixes that.
//!
//! The other half stays unsolved and is declared out of scope: a browser or an
//! Electron application reports its container without distinguishing what is
//! open inside it. Sheets for those are keyed on the window title instead, which
//! is exactly why the fallback chain has a title rung.

use std::path::Path;

use anyhow::Result;
use lumen_platform::ForegroundApp;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, MAX_PATH};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::BOOL;

/// The host process that owns the top-level window of every packaged
/// application. Seeing this name means the real application is a child window.
const FRAME_HOST: &str = "applicationframehost.exe";

pub fn current() -> Result<Option<ForegroundApp>> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            // Briefly true while the desktop switches. Not an error.
            return Ok(None);
        }

        let title = window_title(hwnd);
        let mut executable = executable_of(hwnd);

        if executable.as_deref() == Some(FRAME_HOST) {
            // The frame host owns the window but not the application. Descend.
            if let Some(real) = packaged_child(hwnd) {
                executable = real;
            }
        }

        Ok(Some(ForegroundApp { executable, title }))
    }
}

fn window_title(hwnd: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return String::new();
        }
        // +1 for the terminating NUL, which GetWindowTextW writes but does not
        // count in its return value.
        let mut buffer = vec![0u16; length as usize + 1];
        let written = GetWindowTextW(hwnd, &mut buffer);
        String::from_utf16_lossy(&buffer[..written as usize])
    }
}

/// The lowercased file name of the process owning `hwnd`.
fn executable_of(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut process_id = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == 0 {
            return None;
        }

        // QUERY_LIMITED_INFORMATION rather than QUERY_INFORMATION: it is the
        // right that still works against processes at a higher integrity level,
        // and asking for more than is needed would fail on every elevated
        // window.
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id).ok()?;

        let mut buffer = [0u16; MAX_PATH as usize];
        let mut length = buffer.len() as u32;
        let queried = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );

        let _ = CloseHandle(process);

        queried.ok()?;

        let full = String::from_utf16_lossy(&buffer[..length as usize]);
        file_name_of(&full)
    }
}

/// Reduce a full image path to the lowercased file name sheets match on.
fn file_name_of(full_path: &str) -> Option<String> {
    let name = Path::new(full_path).file_name()?.to_string_lossy();
    (!name.is_empty()).then(|| name.to_lowercase())
}

/// State threaded through `EnumChildWindows` while looking for the real process
/// behind a frame host window.
struct Descent {
    /// The frame host's own process id, so its own child windows are skipped.
    host_process: u32,
    found: Option<String>,
}

/// Find the packaged application hiding behind an `ApplicationFrameHost` window.
fn packaged_child(host: HWND) -> Option<Option<String>> {
    unsafe {
        let mut host_process = 0u32;
        GetWindowThreadProcessId(host, Some(&mut host_process));

        let mut descent = Descent {
            host_process,
            found: None,
        };

        let _ = EnumChildWindows(
            Some(host),
            Some(visit_child),
            LPARAM(&mut descent as *mut Descent as isize),
        );

        descent.found.map(Some)
    }
}

extern "system" fn visit_child(child: HWND, state: LPARAM) -> BOOL {
    unsafe {
        let descent = &mut *(state.0 as *mut Descent);

        if !IsWindowVisible(child).as_bool() {
            return true.into();
        }

        let mut process_id = 0u32;
        GetWindowThreadProcessId(child, Some(&mut process_id));
        // The frame host's own child windows are chrome, not the application.
        if process_id == 0 || process_id == descent.host_process {
            return true.into();
        }

        if let Some(executable) = executable_of(child) {
            descent.found = Some(executable);
            // Stop: the first foreign visible child is the application.
            return false.into();
        }

        true.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduces_an_image_path_to_a_lowercased_file_name() {
        assert_eq!(
            file_name_of(r"C:\Program Files\Microsoft VS Code\Code.exe").as_deref(),
            Some("code.exe")
        );
    }

    /// Sheets are keyed on the lowercased name, so matching must not depend on
    /// how the vendor happened to capitalise the binary.
    #[test]
    fn matching_is_case_insensitive_by_construction() {
        assert_eq!(
            file_name_of(r"C:\WINDOWS\SYSTEM32\NOTEPAD.EXE"),
            file_name_of(r"C:\Windows\System32\notepad.exe")
        );
    }

    #[test]
    fn rejects_a_path_with_no_file_name() {
        assert_eq!(file_name_of(""), None);
        assert_eq!(file_name_of(r"C:\"), None);
    }
}
