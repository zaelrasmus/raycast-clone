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
//!
//! # Reading waits for the clipboard to settle (F12)
//!
//! Not on the notification: [`SETTLE`] after the last one. A program using
//! OLE's clipboard -- every .NET program, among others -- copies in two
//! steps: it announces its formats, which is what notifies this window, and
//! then flushes them, opening the clipboard again to put the data behind the
//! announcement. Reading at the notification asks that program to render a
//! format while it is busy retrying to open the clipboard this window holds;
//! neither moves until .NET gives up, about a second later, and the *copy*
//! fails in the other program. Measured: four .NET copies in six failed with
//! the daemon running, none without it. Clipboard managers that read at once
//! have the same failure; Ditto's answer is this delay.

use anyhow::{Context as _, Result};
use lumen_platform::{ClipboardEvent, ClipboardSink};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, IsClipboardFormatAvailable, RegisterClipboardFormatW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, KillTimer, MSG,
    RegisterClassW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE,
    WM_TIMER, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

use crate::{clipboard, foreground};

/// How long the clipboard must be left alone before it is read. Long enough
/// for a program to finish flushing what it announced, short enough that the
/// entry is there before anybody could open the launcher to look for it.
const SETTLE: u32 = 100;
const SETTLE_TIMER: usize = 1;

/// Per-thread state, reached from the window procedure.
struct WatcherState {
    sink: ClipboardSink,
    /// The exclusion formats, registered once.
    ignore: u32,
    exclude_from_monitors: u32,
    history_flag: u32,
    /// The program in front when the clipboard last changed, kept until it
    /// is read: by then the user may have moved on to another.
    pending_source: Option<Option<String>>,
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
                pending_source: None,
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
        // Who copied is captured now; what they copied, once it settles.
        // Another change before then restarts the wait and names its own
        // source.
        let source = foreground::current()
            .ok()
            .flatten()
            .and_then(|app| app.executable);
        STATE.with(|state| {
            if let Some(state) = state.borrow_mut().as_mut() {
                state.pending_source = Some(source);
            }
        });
        unsafe { SetTimer(Some(hwnd), SETTLE_TIMER, SETTLE, None) };
        return LRESULT(0);
    }
    if message == WM_TIMER && wparam.0 == SETTLE_TIMER {
        unsafe {
            let _ = KillTimer(Some(hwnd), SETTLE_TIMER);
        }
        let source = STATE.with(|state| {
            state
                .borrow_mut()
                .as_mut()
                .and_then(|state| state.pending_source.take())
        });
        if let Some(source) = source {
            handle_update(source);
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// Read the new clipboard contents, if it is files or text and may be
/// recorded.
///
/// This runs on the watcher thread's own message loop, not on a system input
/// callback, so reading the clipboard here is legitimate -- I-15 governs input
/// callbacks specifically, and opening the clipboard from one would be exactly
/// the kind of blocking work it forbids. Nothing here touches UI state.
fn handle_update(source: Option<String>) {
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

    let (content, html) = match clipboard::content() {
        Ok(Some(captured)) => captured,
        // Neither files nor text: an image, or something that vanished
        // between the notification and the read. Both are ordinary.
        Ok(None) => return,
        // Said, because a copy lost here is otherwise lost without a trace.
        Err(error) => {
            log::debug!("could not read a clipboard change: {error:#}");
            return;
        }
    };

    STATE.with(|state| {
        if let Some(state) = state.borrow().as_ref() {
            state.sink.emit(ClipboardEvent {
                content,
                source,
                html,
            });
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
