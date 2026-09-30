//! Pasting into another application (F11).
//!
//! Put the text on the clipboard, type the keystroke that pastes -- `Ctrl+V`,
//! or what the target pastes on instead -- into the window the launcher was
//! opened over, and -- unless the user keeps pastes -- put back what was
//! on the clipboard before.
//!
//! # When to put it back
//!
//! The target reads the clipboard whenever it gets round to handling the
//! keystroke. Restoring before that pastes the old contents; a fixed delay is
//! a guess that is wrong on a slow machine and slow on a fast one. So the text
//! is offered by *delayed rendering*: a window of ours owns the clipboard and
//! announces text without supplying it, and Windows asks that window for it
//! -- `WM_RENDERFORMAT` -- at the moment somebody reads it. The window that
//! has the clipboard open then is the reader; when it belongs to the target's
//! program (a browser reads from a process other than the window's), or is
//! unnamed, since `OpenClipboard(NULL)` is legal, the paste has been taken,
//! and the clipboard is restored a moment later, which is how long an editor
//! takes to read the other formats it asks for.
//!
//! Anything else that reads it first is given the text and not counted. Well
//! behaved clipboard managers do not read it at all: the text carries
//! `ExcludeClipboardContentFromMonitorProcessing` and a zero
//! `CanIncludeInClipboardHistory`, so it reaches neither Lumen's history
//! (7.13) nor Windows'.
//!
//! When something else read the text first, the target's read cannot be seen
//! -- the text is already supplied, and reading it asks this window nothing
//! -- so the clipboard is restored after [`READ_WAIT`] instead, the paste
//! taken to have gone through.
//!
//! When nothing asks within [`READ_WAIT`], the paste was dropped: the window
//! runs at a higher integrity level and Windows discarded the keystrokes
//! without a word (UIPI), or it does not paste on `Ctrl+V`. The text is then
//! left on the clipboard, where the user can paste it by hand, and the caller
//! is told.
//!
//! If somebody else replaces the clipboard in the meantime, nothing is put
//! back: what they copied is newer than what was there.

use std::cell::RefCell;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use lumen_platform::{FocusOwner, Paste, PasteKeys};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardFormatNameW,
    GetClipboardOwner, GetOpenClipboardWindow, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_INSERT, VK_LWIN,
    VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClassNameW,
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, HWND_MESSAGE, KillTimer, MSG,
    PostQuitMessage, RegisterClassW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_DESTROYCLIPBOARD, WM_RENDERALLFORMATS, WM_RENDERFORMAT, WM_TIMER, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

use crate::clipboard::{self, ClipboardLock};
use crate::foreground;

/// How long the target has to come to the front after the launcher hides.
const FOREGROUND_WAIT: Duration = Duration::from_millis(1000);
/// How long to wait for Shift, Alt and the Windows key to be let go, so the
/// keystroke is `Ctrl+V` and not `Ctrl+Alt+V`.
const MODIFIER_WAIT: Duration = Duration::from_millis(1000);
/// How long the target has to read the text before the paste is taken to
/// have been dropped.
const READ_WAIT: Duration = Duration::from_millis(2000);
/// How long after the first read the text stays: an editor asks for more than
/// one format, one after another.
const READ_GRACE: Duration = Duration::from_millis(250);

const READ_TIMER: usize = 1;
const GRACE_TIMER: usize = 2;

/// The virtual key for V, which is `'V'` on every layout: applications match
/// `Ctrl+V` on the virtual key, not on the character.
const VK_V: VIRTUAL_KEY = VIRTUAL_KEY(0x56);

pub fn paste(window: FocusOwner, paste: &Paste) -> Result<()> {
    let target = HWND(window.0 as *mut c_void);

    if !wait_for_foreground(target) {
        // Typing into whatever is in front instead is the race 7.15 exists
        // to avoid, so nothing is typed; the text is not lost either.
        clipboard::set_text(&paste.text)?;
        bail!("the window did not come back to the front, so the snippet was copied instead");
    }
    wait_for_modifiers();

    if !paste.restore_clipboard {
        clipboard::set_text(&paste.text)?;
        return send_keys(paste.keys);
    }

    // A thread of its own: the window that owns the clipboard needs a message
    // loop, and the caller's thread is not one to borrow.
    let text = paste.text.clone();
    let keys = paste.keys;
    let raw = window.0;
    std::thread::Builder::new()
        .name("lumen-paste".into())
        .spawn(move || Owner::run(HWND(raw as *mut c_void), &text, keys))
        .context("starting the paste")?
        .join()
        .map_err(|_| anyhow!("the paste stopped unexpectedly"))?
}

/// Whether `target` is in front, or comes to the front soon.
fn wait_for_foreground(target: HWND) -> bool {
    let started = Instant::now();
    loop {
        if unsafe { GetForegroundWindow() } == target {
            return true;
        }
        if started.elapsed() > FOREGROUND_WAIT {
            let front = unsafe { GetForegroundWindow() };
            log::debug!(
                "waited for {:?} ({}) to come to the front; {:?} ({}) is there instead",
                target.0,
                class_of(target),
                front.0,
                class_of(front)
            );
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A window's class, for the log: what kind of window it is, never what it
/// shows.
fn class_of(hwnd: HWND) -> String {
    let mut name = [0u16; 128];
    let length = unsafe { GetClassNameW(hwnd, &mut name) };
    String::from_utf16_lossy(&name[..length.max(0) as usize])
}

/// Wait, briefly, for the keys that would change what the paste keystroke
/// means. Control is not among them: `Ctrl+V` with Control already down is
/// `Ctrl+V`.
fn wait_for_modifiers() {
    let started = Instant::now();
    let held = || {
        [VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            // The high bit is "down now".
            .any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) } < 0)
    };
    while held() && started.elapsed() < MODIFIER_WAIT {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Type the keystroke that pastes: the modifiers down, the key down and up,
/// the modifiers up in reverse.
fn send_keys(keys: PasteKeys) -> Result<()> {
    let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let none = KEYBD_EVENT_FLAGS(0);
    let (modifiers, main, main_flags): (&[VIRTUAL_KEY], VIRTUAL_KEY, KEYBD_EVENT_FLAGS) = match keys
    {
        PasteKeys::CtrlV => (&[VK_CONTROL], VK_V, none),
        PasteKeys::CtrlShiftV => (&[VK_CONTROL, VK_SHIFT], VK_V, none),
        // Insert is on the extended keypad: without the flag some
        // programs read it as the number pad's 0.
        PasteKeys::ShiftInsert => (&[VK_SHIFT], VK_INSERT, KEYEVENTF_EXTENDEDKEY),
    };

    let mut inputs: Vec<INPUT> = modifiers.iter().map(|vk| key(*vk, none)).collect();
    inputs.push(key(main, main_flags));
    inputs.push(key(main, main_flags | KEYEVENTF_KEYUP));
    inputs.extend(modifiers.iter().rev().map(|vk| key(*vk, KEYEVENTF_KEYUP)));

    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        bail!(
            "Windows took {sent} of the {} keystrokes: {}",
            inputs.len(),
            windows::core::Error::from_win32()
        );
    }
    Ok(())
}

/// The window that owns the clipboard while a paste is read, and what it
/// knows. Reached from the window procedure, one paste per thread.
struct Owner {
    hwnd: HWND,
    target_process: u32,
    /// The target's program. A browser or an Electron application is several
    /// processes, and the one that reads the clipboard is not the one that
    /// owns the window: the same program reading counts as the target.
    target_executable: Option<String>,
    text: Vec<u8>,
    /// Everything that was on the clipboard, to put back.
    saved: Vec<(u32, Vec<u8>)>,
    exclude: u32,
    history: u32,
    /// The target has asked for the text.
    read: bool,
    /// Something other than the target asked first. The target then reads
    /// text already supplied, which asks this window nothing, so its read
    /// cannot be seen; the paste most likely went through.
    read_by_other: bool,
    /// The text is announced on the clipboard and not yet supplied. Only
    /// then may this window write it: once it has been supplied, restored
    /// over or given up on, writing it again would put the snippet back over
    /// whatever is there now -- which is how a restored clipboard was first
    /// found overwritten, by the `WM_RENDERALLFORMATS` sent as this window
    /// closes.
    owed: bool,
    outcome: Option<Result<()>>,
}

thread_local! {
    static OWNER: RefCell<Option<Owner>> = const { RefCell::new(None) };
}

impl Owner {
    fn run(target: HWND, text: &str, keys: PasteKeys) -> Result<()> {
        let hwnd = unsafe { create_window()? };
        let mut target_process = 0u32;
        unsafe { GetWindowThreadProcessId(target, Some(&mut target_process)) };

        let owner = Owner {
            hwnd,
            target_process,
            target_executable: foreground::executable_of(target),
            text: clipboard::utf16(text),
            saved: Vec::new(),
            exclude: unsafe {
                RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"))
            },
            history: unsafe { RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")) },
            read: false,
            read_by_other: false,
            owed: false,
            outcome: None,
        };
        OWNER.with(|cell| *cell.borrow_mut() = Some(owner));

        let offered = with_owner(|owner| owner.offer()).unwrap_or(Ok(()));
        let result = offered.and_then(|()| {
            send_keys(keys)?;
            unsafe { SetTimer(Some(hwnd), READ_TIMER, READ_WAIT.as_millis() as u32, None) };
            pump();
            with_owner(|owner| owner.outcome.take())
                .flatten()
                .unwrap_or(Ok(()))
        });

        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        OWNER.with(|cell| cell.borrow_mut().take());
        result
    }

    /// Keep what is on the clipboard, then take it with the text announced
    /// and not yet supplied.
    fn offer(&mut self) -> Result<()> {
        let _lock = ClipboardLock::acquire_for(self.hwnd)?;
        self.saved = unsafe { saved_formats() };
        log::debug!(
            "paste: kept {} clipboard formats to put back",
            self.saved.len()
        );
        unsafe { EmptyClipboard() }.context("EmptyClipboard")?;

        unsafe {
            // Delayed rendering: a null handle. The call returns null for it
            // too, which the binding reads as an error, so its result says
            // nothing and is not looked at.
            let _ = SetClipboardData(CF_UNICODETEXT.0 as u32, None);
            self.mark()?;
        }
        self.owed = true;
        Ok(())
    }

    /// The formats that keep a clipboard write out of history, Lumen's and
    /// Windows'. The clipboard must be open.
    unsafe fn mark(&self) -> Result<()> {
        unsafe {
            if self.exclude != 0 {
                let marker = clipboard::global(&[0])?;
                SetClipboardData(self.exclude, Some(HANDLE(marker.0)))
                    .context("marking the paste as not for history")?;
            }
            if self.history != 0 {
                let never = clipboard::global(&0u32.to_le_bytes())?;
                SetClipboardData(self.history, Some(HANDLE(never.0)))
                    .context("keeping the paste out of Windows' clipboard history")?;
            }
        }
        Ok(())
    }

    /// Somebody asked for the text: supply it, and note whether it was the
    /// target. The reader has the clipboard open, so it is not opened here.
    fn render(&mut self, format: u32) {
        if format != u32::from(CF_UNICODETEXT.0) || !self.owed {
            return;
        }
        self.owed = false;
        match clipboard::global(&self.text) {
            Ok(global) => unsafe {
                let _ = SetClipboardData(format, Some(HANDLE(global.0)));
            },
            Err(error) => log::warn!("could not supply the snippet: {error:#}"),
        }

        // `Err` is a null window: `OpenClipboard(NULL)`, which names nobody.
        let by_target = match unsafe { GetOpenClipboardWindow() } {
            Err(_) => {
                log::debug!("paste: read by an unnamed window");
                true
            }
            Ok(reader) => {
                let mut process = 0u32;
                unsafe { GetWindowThreadProcessId(reader, Some(&mut process)) };
                let same_program = process != self.target_process
                    && self.target_executable.is_some()
                    && foreground::executable_of(reader) == self.target_executable;
                log::debug!(
                    "paste: read by process {process} ({}), the target is {}{}",
                    class_of(reader),
                    self.target_process,
                    if same_program {
                        ", the same program"
                    } else {
                        ""
                    }
                );
                process == self.target_process || same_program
            }
        };
        if !by_target {
            self.read_by_other = true;
        }
        if by_target && !self.read {
            self.read = true;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), READ_TIMER);
                SetTimer(
                    Some(self.hwnd),
                    GRACE_TIMER,
                    READ_GRACE.as_millis() as u32,
                    None,
                );
            }
        }
    }

    /// Put back what was on the clipboard.
    fn restore(&mut self) -> Result<()> {
        let _lock = ClipboardLock::acquire_for(self.hwnd)?;
        // Sends this window `WM_DESTROYCLIPBOARD`, re-entrantly; see
        // `with_owner`.
        unsafe { EmptyClipboard() }.context("EmptyClipboard")?;

        self.owed = false;
        log::debug!("paste: putting back {} formats", self.saved.len());
        if self.saved.is_empty() {
            return Ok(());
        }
        for (format, bytes) in &self.saved {
            let global = clipboard::global(bytes)?;
            unsafe {
                // One format that will not go back is not a reason to lose
                // the others.
                if SetClipboardData(*format, Some(HANDLE(global.0))).is_err() {
                    log::debug!("could not put back clipboard format {format}");
                }
            }
        }
        // Marked, so the history does not record again what it already has.
        unsafe { self.mark() }
    }

    /// Nobody took the paste: leave the text where it can be pasted by hand,
    /// supplied now, since this window is about to go.
    fn give_up(&mut self) -> Result<()> {
        if !self.owed {
            // Somebody else read it, so it is there already.
            return Ok(());
        }
        self.owed = false;
        let _lock = ClipboardLock::acquire_for(self.hwnd)?;
        if unsafe { GetClipboardOwner() }.ok() != Some(self.hwnd) {
            return Ok(());
        }
        let global = clipboard::global(&self.text)?;
        unsafe {
            SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(global.0)))
                .context("SetClipboardData")?;
        }
        Ok(())
    }

    fn finish(&mut self, outcome: Result<()>) {
        if self.outcome.is_none() {
            self.outcome = Some(outcome);
        }
        unsafe { PostQuitMessage(0) };
    }
}

/// Run `work` on this thread's paste, if there is one and it is not already
/// being worked on.
///
/// The second case is re-entrancy, not a race: emptying the clipboard while
/// this window owns it sends it `WM_DESTROYCLIPBOARD` synchronously, from
/// inside the handler that emptied it. A message that arrives then was caused
/// by this code's own clipboard calls, and is not somebody else's copy.
fn with_owner<T>(work: impl FnOnce(&mut Owner) -> T) -> Option<T> {
    OWNER.with(|cell| {
        let mut owner = cell.try_borrow_mut().ok()?;
        owner.as_mut().map(work)
    })
}

/// Everything on the clipboard that can be copied as bytes. The clipboard
/// must be open.
///
/// Left out: formats that are handles to GDI objects rather than memory
/// (bitmaps, metafiles, palettes; the DIB beside a bitmap is kept, and the
/// bitmap comes back from it), and OLE's own, which point into the process
/// that copied and mean nothing once it has let go.
unsafe fn saved_formats() -> Vec<(u32, Vec<u8>)> {
    const GDI: [u32; 8] = [2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E];
    let mut saved = Vec::new();
    let mut format = 0;
    loop {
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        if GDI.contains(&format) || (0x300..=0x3FF).contains(&format) || is_ole(format) {
            continue;
        }
        let Ok(handle) = (unsafe { GetClipboardData(format) }) else {
            continue;
        };
        if let Some(bytes) = unsafe { bytes_of(HGLOBAL(handle.0)) } {
            saved.push((format, bytes));
        }
    }
    saved
}

fn is_ole(format: u32) -> bool {
    if format < 0xC000 {
        return false;
    }
    let mut name = [0u16; 64];
    let length = unsafe { GetClipboardFormatNameW(format, &mut name) };
    let name = String::from_utf16_lossy(&name[..length.max(0) as usize]);
    matches!(name.as_str(), "DataObject" | "Ole Private Data")
}

unsafe fn bytes_of(global: HGLOBAL) -> Option<Vec<u8>> {
    unsafe {
        let size = GlobalSize(global);
        if size == 0 {
            return None;
        }
        let pointer = GlobalLock(global) as *const u8;
        if pointer.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(pointer, size).to_vec();
        let _ = GlobalUnlock(global);
        Some(bytes)
    }
}

unsafe fn create_window() -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        let class_name = w!("LumenPaste");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        // Fails harmlessly on every paste after the first: the class exists.
        RegisterClassW(&class);

        CreateWindowExW(
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
        .context("creating the window that owns the paste")
    }
}

fn pump() {
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
    match message {
        WM_RENDERFORMAT => {
            with_owner(|owner| owner.render(wparam.0 as u32));
            LRESULT(0)
        }
        // This window is going while it still owes the text: supply it.
        WM_RENDERALLFORMATS => {
            with_owner(|owner| {
                if let Ok(_lock) = ClipboardLock::acquire_for(owner.hwnd)
                    && unsafe { GetClipboardOwner() }.ok() == Some(owner.hwnd)
                {
                    owner.render(u32::from(CF_UNICODETEXT.0));
                }
            });
            LRESULT(0)
        }
        WM_DESTROYCLIPBOARD => {
            // Somebody copied something else: that is newer than anything
            // this could put back. (This window's own emptying never gets
            // here: see `with_owner`.)
            with_owner(|owner| {
                log::debug!("the clipboard was replaced during a paste; not restoring it");
                owner.finish(Ok(()));
            });
            LRESULT(0)
        }
        WM_TIMER => {
            let _ = unsafe { KillTimer(Some(hwnd), wparam.0) };
            log::debug!(
                "paste: {}",
                if wparam.0 == GRACE_TIMER {
                    "read; restoring"
                } else {
                    "the target was not seen reading in time"
                }
            );
            with_owner(|owner| {
                let outcome = if wparam.0 == GRACE_TIMER || owner.read_by_other {
                    owner.restore()
                } else {
                    owner.give_up().and_then(|()| {
                        Err(anyhow!(
                            "the window did not take the paste -- it may be running as \
                             administrator -- so the snippet was left on the clipboard"
                        ))
                    })
                };
                owner.finish(outcome);
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
