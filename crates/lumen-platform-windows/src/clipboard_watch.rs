//! Watching the clipboard (F3).
//!
//! `AddClipboardFormatListener` on a message-only window, which is the modern
//! replacement for the clipboard-viewer chain: no linked list to join, nothing
//! to forward, and no way for one badly behaved application to break every
//! other listener.
//!
//! # The exclusion formats
//!
//! This is the platform half of the privacy layer. Applications that handle
//! secrets mark a clipboard write as not-for-history, and there are three
//! conventions rather than one, because the ecosystem grew them separately:
//!
//! - `Clipboard Viewer Ignore` -- the oldest, set by clipboard managers
//!   themselves so they do not observe each other.
//! - `ExcludeClipboardContentFromMonitorProcessing` -- what password managers
//!   set, and the one that matters most here.
//! - `CanIncludeInClipboardHistory` -- Windows 10's own, a DWORD where zero
//!   means no.
//!
//! Honouring the presence of any of the first two, and a zero in the third, is
//! what the plan means by "respect for the 'do not include in history'
//! clipboard flags on Windows". Missing one of them means silently recording a
//! password, so all three are checked and any of them is enough to decline.

use anyhow::{Context as _, Result};
use lumen_platform::{ClipboardEvent, ClipboardSink};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, IsClipboardFormatAvailable, RegisterClipboardFormatW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG,
    RegisterClassW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

use crate::{clipboard, foreground};

/// Per-thread state, reached from the window procedure.
struct WatcherState {
    sink: ClipboardSink,
    /// The exclusion formats, registered once.
    ignore: u32,
    exclude_from_monitors: u32,
    history_flag: u32,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<WatcherState>> = const { std::cell::RefCell::new(None) };
}

/// Start watching on a dedicated thread.
pub fn start(sink: ClipboardSink) -> Result<()> {
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<()>>(1);

    std::thread::Builder::new()
        .name("lumen-clipboard".into())
        .spawn(move || match run(sink) {
            Ok(pump) => {
                let _ = ready_tx.send(Ok(()));
                pump();
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error));
            }
        })
        .context("spawning the clipboard watcher thread")?;

    ready_rx
        .recv()
        .context("the clipboard watcher stopped before reporting readiness")?
}

fn run(sink: ClipboardSink) -> Result<impl FnOnce()> {
    unsafe {
        let instance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        let class_name = w!("LumenClipboardWatcher");

        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&class);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR::null(),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
        .context("creating the clipboard watcher window")?;

        STATE.with(|state| {
            *state.borrow_mut() = Some(WatcherState {
                sink,
                ignore: RegisterClipboardFormatW(w!("Clipboard Viewer Ignore")),
                exclude_from_monitors: RegisterClipboardFormatW(w!(
                    "ExcludeClipboardContentFromMonitorProcessing"
                )),
                history_flag: RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")),
            });
        });

        AddClipboardFormatListener(hwnd).context("registering as a clipboard listener")?;
    }

    Ok(pump_messages)
}

fn pump_messages() {
    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CLIPBOARDUPDATE {
        handle_update();
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// Read the new clipboard contents, if it is text and may be recorded.
///
/// This runs on the watcher thread's own message loop, not on a system input
/// callback, so reading the clipboard here is legitimate -- I-15 governs input
/// callbacks specifically, and opening the clipboard from one would be exactly
/// the kind of blocking work it forbids. Nothing here touches UI state.
fn handle_update() {
    let excluded = STATE.with(|state| {
        let state = state.borrow();
        let Some(state) = state.as_ref() else {
            return true;
        };
        is_excluded(state)
    });

    if excluded {
        log::debug!("clipboard entry marked not-for-history; ignoring it");
        return;
    }

    let Ok(Some(text)) = clipboard::text() else {
        // Not text: an image, a file list, or something that vanished between
        // the notification and the read. Both are ordinary.
        return;
    };

    // Captured now, because the foreground application is the one that did the
    // copying, and by the time the daemon sees this event it may not be.
    let source = foreground::current()
        .ok()
        .flatten()
        .and_then(|app| app.executable);

    STATE.with(|state| {
        if let Some(state) = state.borrow().as_ref() {
            state.sink.emit(ClipboardEvent { text, source });
        }
    });
}

/// Whether the current clipboard contents are marked not-for-history.
fn is_excluded(state: &WatcherState) -> bool {
    unsafe {
        if state.ignore != 0 && IsClipboardFormatAvailable(state.ignore).is_ok() {
            return true;
        }
        if state.exclude_from_monitors != 0
            && IsClipboardFormatAvailable(state.exclude_from_monitors).is_ok()
        {
            return true;
        }

        // The Windows 10 flag is a DWORD rather than a marker: present and zero
        // means no, present and non-zero means yes, absent means no opinion.
        // Treating its mere presence as a refusal would drop entries the source
        // explicitly allowed.
        if state.history_flag != 0 && IsClipboardFormatAvailable(state.history_flag).is_ok() {
            return matches!(clipboard::dword(state.history_flag), Ok(Some(0)));
        }
    }

    false
}
