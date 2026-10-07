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
//!
//! # Keeping the launcher open
//!
//! *Paste and Keep Window Open* (F18) does not hide the launcher: the
//! foreground is given to the target and taken back once the paste is over
//! -- read, given up on, or the clipboard replaced -- and the paste is over
//! only when the launcher is in front again. Given back at the target's
//! first read, not after the grace that follows it, so that what the user
//! types next goes to the launcher; never after a delay of its own: the
//! keystroke waits in the target's queue until it is handled, and taking
//! the foreground back first can leave it unread. Both handoffs are made
//! from the paste's thread, which has to earn the right first (see
//! `hand_foreground`). A paste not taken, kept open, leaves the clipboard
//! as it was, unless pastes are kept: the launcher is still there to paste
//! from again. The text is offered by delayed rendering whether or not the clipboard is
//! put back, so that the read is seen.

use std::cell::RefCell;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use lumen_platform::{ClipboardContent, FocusOwner, Paste, PasteKeys};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardFormatNameW,
    GetClipboardOwner, GetOpenClipboardWindow, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_INSERT, VK_LEFT,
    VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClassNameW,
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, HWND_MESSAGE, KillTimer, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_DESTROYCLIPBOARD, WM_RENDERALLFORMATS,
    WM_RENDERFORMAT, WM_TIMER, WNDCLASSW,
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

/// Posted to the paste's own window when the target has read, to give the
/// foreground back to a launcher kept open.
const WM_GIVE_BACK: u32 = WM_APP + 1;

const READ_TIMER: usize = 1;
const GRACE_TIMER: usize = 2;

/// The virtual key for V, which is `'V'` on every layout: applications match
/// `Ctrl+V` on the virtual key, not on the character.
const VK_V: VIRTUAL_KEY = VIRTUAL_KEY(0x56);

/// The most Left presses a `{cursor}` is worth.
const MAX_CARET_BACK: usize = 2000;

pub fn paste(window: FocusOwner, paste: &Paste) -> Result<()> {
    deliver(window, paste, None)
}

/// [`paste`] with the launcher, `launcher`, left showing: the foreground is
/// lent to the target and given back once the paste is over.
pub fn paste_keep_open(window: FocusOwner, paste: &Paste, launcher: HWND) -> Result<()> {
    let outcome = deliver(window, paste, Some(launcher));
    // Read, given up on, or the clipboard replaced: `deliver` returns only
    // then, so the target has had the keystroke. Given back at the read
    // already, unless the read was not seen.
    let started = Instant::now();
    hand_foreground(launcher);
    // Not over until the launcher is in front again: a paste after this one
    // hands the foreground over from there.
    if wait_for_foreground(launcher) {
        log::debug!(
            "paste: the launcher is in front again after {:?}",
            started.elapsed()
        );
    } else {
        log::debug!("could not take the foreground back to the launcher");
    }
    outcome
}

/// Type `paste` into `window`. With `keep_open` the launcher is still in
/// front, so the foreground is handed to `window` here rather than by
/// hiding, and the read is always watched for.
fn deliver(window: FocusOwner, paste: &Paste, launcher: Option<HWND>) -> Result<()> {
    let keep_open = launcher.is_some();
    let target = HWND(window.0 as *mut c_void);
    // The window that owns the clipboard watches for the read: needed to
    // put the clipboard back, and to know when the launcher may have the
    // foreground again.
    let watched = paste.restore_clipboard || keep_open;

    // The text always, or the files and their names (F12); the HTML beside
    // it when there is some, which a document or a mail reads in preference.
    let mut formats = if let Some(image) = &paste.image {
        clipboard::formats_of(&ClipboardContent::Image(image.clone()))
    } else if !paste.files.is_empty() {
        clipboard::formats_of(&ClipboardContent::Files(paste.files.clone()))
    } else {
        vec![(u32::from(CF_UNICODETEXT.0), clipboard::utf16(&paste.text))]
    };
    // Only for a paste nobody watches: a watched one is marked by its owner.
    if paste.keep_out_of_history && !watched {
        let exclude = clipboard::exclude_format();
        if exclude != 0 {
            formats.push((exclude, vec![0]));
        }
    }
    if let Some(html) = &paste.html {
        let format = clipboard::html_format();
        if format != 0 {
            formats.push((format, clipboard::cf_html(html)));
        }
    }

    let started = if keep_open {
        // The chord that asked for this is still down. Let go of in front of
        // the launcher, the target never sees Shift's release, nor a
        // `Ctrl+Shift+V` made of it.
        wait_for_modifiers();
        let started = Instant::now();
        hand_foreground(target);
        started
    } else {
        Instant::now()
    };
    if !wait_for_foreground(target) {
        // Typing into whatever is in front instead is the race 7.15 exists
        // to avoid, so nothing is typed.
        if keep_open {
            // The launcher is still open to paste from again, so the
            // clipboard is left as the user had it.
            bail!("the window did not come to the front, so nothing was pasted");
        }
        // Hidden, the launcher is gone: the text is not lost either.
        clipboard::set_formats(&formats)?;
        bail!("the window did not come back to the front, so the snippet was copied instead");
    }
    if keep_open {
        log::debug!(
            "paste: the target is in front after {:?}",
            started.elapsed()
        );
    } else {
        wait_for_modifiers();
    }

    if !watched {
        clipboard::set_formats(&formats)?;
        return send_keys(paste.keys, paste.caret_back);
    }

    // A thread of its own: the window that owns the clipboard needs a message
    // loop, and the caller's thread is not one to borrow.
    let send = Delivery {
        keys: paste.keys,
        caret_back: paste.caret_back,
        put_back: paste.restore_clipboard,
        keep_out_of_history: paste.keep_out_of_history,
        give_back: launcher.map(|launcher| launcher.0 as isize),
    };
    let raw = window.0;
    std::thread::Builder::new()
        .name("lumen-paste".into())
        .spawn(move || Owner::run(HWND(raw as *mut c_void), formats, send))
        .context("starting the paste")?
        .join()
        .map_err(|_| anyhow!("the paste stopped unexpectedly"))?
}

/// Give `hwnd` the foreground, from this thread, which is not the window's.
///
/// Being the foreground process is not enough: `SetForegroundWindow` was
/// refused, now and then, until this process had sent an input of its own
/// -- which is why every paste after a first one worked: its `Ctrl+V`.
/// Once the target is in front and the user's next key is the target's,
/// the right is lost again. So an input that does nothing goes first, a
/// mouse event with no movement and no buttons, as PowerToys does; then a
/// plain call.
///
/// Not attached to the foreground's input: that measured 22-27 ms a hand-off,
/// synchronous with gpui's deactivation, against 1-5 ms plain, and lost more
/// keys in a burst. Not `activate::raise` either: its `SetFocus`, made
/// attached to the launcher's input for a window that is not, left nothing
/// in front on every paste.
fn hand_foreground(hwnd: HWND) {
    let input = [INPUT::default()];
    if unsafe { SendInput(&input, std::mem::size_of::<INPUT>() as i32) } != 1 {
        log::debug!("could not send the input that lets the foreground move");
    }
    if !unsafe { SetForegroundWindow(hwnd) }.as_bool() {
        log::debug!(
            "SetForegroundWindow refused {:?} ({})",
            hwnd.0,
            class_of(hwnd)
        );
    }
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

/// Type the keystroke that pastes -- the modifiers down, the key down and up,
/// the modifiers up in reverse -- then Left `caret_back` times.
///
/// One call, so nothing typed by anyone else falls between the paste and the
/// caret's move: the target reads the clipboard while it handles the paste,
/// before it takes the first Left from its queue.
fn send_keys(keys: PasteKeys, caret_back: usize) -> Result<()> {
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
    // Past this, the caret is left at the end: a `{cursor}` that far from it
    // is more keystrokes than moving it is worth.
    if caret_back <= MAX_CARET_BACK {
        for _ in 0..caret_back {
            inputs.push(key(VK_LEFT, KEYEVENTF_EXTENDEDKEY));
            inputs.push(key(VK_LEFT, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP));
        }
    }

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

/// How a watched paste is typed, and what becomes of the clipboard after.
struct Delivery {
    keys: PasteKeys,
    caret_back: usize,
    /// Put back what was there once the text is read; else leave the text.
    put_back: bool,
    /// For a text that is left: keep it out of the history all the same.
    keep_out_of_history: bool,
    /// The launcher, kept open: given the foreground back the moment the
    /// target reads, so what the user types next is not the target's.
    give_back: Option<isize>,
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
    /// What is offered: each format and its bytes.
    formats: Vec<(u32, Vec<u8>)>,
    /// Everything that was on the clipboard, to put back.
    saved: Vec<(u32, Vec<u8>)>,
    /// Put it back once read; `false` leaves the text, supplied.
    put_back: bool,
    /// A text that is left is kept out of the history all the same.
    keep_out_of_history: bool,
    exclude: u32,
    history: u32,
    /// The target has asked for the text.
    read: bool,
    /// Something other than the target asked first. The target then reads
    /// text already supplied, which asks this window nothing, so its read
    /// cannot be seen; the paste most likely went through.
    read_by_other: bool,
    /// See [`Delivery::give_back`]; taken when it is done.
    give_back: Option<HWND>,
    /// The launcher is still open: a paste the target did not take leaves
    /// the clipboard as it was, to paste again from there.
    kept_open: bool,
    /// The formats announced on the clipboard and not yet supplied. Only
    /// those may this window write: once supplied, restored over or given up
    /// on, writing one again would put the snippet back over whatever is
    /// there now -- which is how a restored clipboard was first found
    /// overwritten, by the `WM_RENDERALLFORMATS` sent as this window closes.
    owed: Vec<u32>,
    outcome: Option<Result<()>>,
}

thread_local! {
    static OWNER: RefCell<Option<Owner>> = const { RefCell::new(None) };
}

impl Owner {
    fn run(target: HWND, formats: Vec<(u32, Vec<u8>)>, send: Delivery) -> Result<()> {
        let hwnd = unsafe { create_window()? };
        let mut target_process = 0u32;
        unsafe { GetWindowThreadProcessId(target, Some(&mut target_process)) };

        let owner = Owner {
            hwnd,
            target_process,
            target_executable: foreground::executable_of(target),
            formats,
            saved: Vec::new(),
            put_back: send.put_back,
            keep_out_of_history: send.keep_out_of_history,
            exclude: unsafe {
                RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"))
            },
            history: unsafe { RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")) },
            read: false,
            read_by_other: false,
            give_back: send.give_back.map(|raw| HWND(raw as *mut c_void)),
            kept_open: send.give_back.is_some(),
            owed: Vec::new(),
            outcome: None,
        };
        OWNER.with(|cell| *cell.borrow_mut() = Some(owner));

        let offered = with_owner(|owner| owner.offer()).unwrap_or(Ok(()));
        let result = offered.and_then(|()| {
            send_keys(send.keys, send.caret_back)?;
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
        if self.put_back {
            self.saved = unsafe { saved_formats() };
            log::debug!(
                "paste: kept {} clipboard formats to put back",
                self.saved.len()
            );
        }
        unsafe { EmptyClipboard() }.context("EmptyClipboard")?;

        unsafe {
            // Delayed rendering: a null handle. The call returns null for it
            // too, which the binding reads as an error, so its result says
            // nothing and is not looked at.
            for (format, _) in &self.formats {
                let _ = SetClipboardData(*format, None);
            }
            if self.put_back {
                self.mark()?;
            } else if self.keep_out_of_history && self.exclude != 0 {
                // As an unwatched paste that stays is marked: out of Lumen's
                // history only.
                let marker = clipboard::global(&[0])?;
                SetClipboardData(self.exclude, Some(HANDLE(marker.0)))
                    .context("marking the paste as not for history")?;
            }
        }
        self.owed = self.formats.iter().map(|(format, _)| *format).collect();
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

    /// Put `format` on the clipboard now, if it is still owed. The clipboard
    /// must be open.
    fn supply(&mut self, format: u32) -> bool {
        let Some(at) = self.owed.iter().position(|owed| *owed == format) else {
            return false;
        };
        self.owed.remove(at);
        let Some((_, bytes)) = self.formats.iter().find(|(offered, _)| *offered == format) else {
            return false;
        };
        match clipboard::global(bytes) {
            Ok(global) => unsafe {
                let _ = SetClipboardData(format, Some(HANDLE(global.0)));
            },
            Err(error) => log::warn!("could not supply the snippet: {error:#}"),
        }
        true
    }

    /// Somebody asked for a format: supply it, and note whether it was the
    /// target. The reader has the clipboard open, so it is not opened here.
    fn render(&mut self, format: u32) {
        if !self.supply(format) {
            return;
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
            if self.give_back.is_some() {
                // Once the target has its text, not from inside its read.
                unsafe {
                    let _ = PostMessageW(Some(self.hwnd), WM_GIVE_BACK, WPARAM(0), LPARAM(0));
                }
            }
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

        self.owed.clear();
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
        if self.owed.is_empty() {
            // Somebody else read it, so it is there already.
            return Ok(());
        }
        let _lock = ClipboardLock::acquire_for(self.hwnd)?;
        if unsafe { GetClipboardOwner() }.ok() != Some(self.hwnd) {
            self.owed.clear();
            return Ok(());
        }
        self.supply_all();
        Ok(())
    }

    /// Everything still owed, now. The clipboard must be open.
    fn supply_all(&mut self) {
        for format in self.owed.clone() {
            self.supply(format);
        }
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
        if let Some(bytes) = unsafe { clipboard::bytes_of(HGLOBAL(handle.0)) } {
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
                    // Supplied, not read: nobody is reading it.
                    owner.supply_all();
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
        // The target has read: the launcher may have the foreground again.
        // The clipboard stays this window's until the grace is over.
        WM_GIVE_BACK => {
            if let Some(Some(launcher)) = with_owner(|owner| owner.give_back.take()) {
                hand_foreground(launcher);
            }
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
                    if owner.put_back {
                        owner.restore()
                    } else {
                        // Taken, and the text stays: supplied, as it goes.
                        owner.give_up()
                    }
                } else if owner.kept_open && owner.put_back {
                    owner.restore().and_then(|()| {
                        Err(anyhow!(
                            "the window did not take the paste -- it may be running as                              administrator -- so nothing was pasted"
                        ))
                    })
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
