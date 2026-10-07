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
//!
//! For the caller, a kept-open paste is over at the read: the launcher is in
//! front again, and the owner puts the clipboard back on its own thread
//! after the grace. The next paste waits for that before it touches the
//! clipboard (`settle`), with the launcher still in front.
//!
//! The chord that asked for a kept-open paste is not waited for: its Shift,
//! Alt and Windows are let go of in the keystroke's own batch and Shift and
//! Control pressed again after it if still held (`Owner::type_kept_open`),
//! so holding `Ctrl+Shift` and tapping Enter pastes on each tap.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, AtomicU8, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
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
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_LWIN, VK_MENU, VK_RWIN,
    VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClassNameW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, HC_ACTION, HHOOK,
    HWND_MESSAGE, KBDLLHOOKSTRUCT, KillTimer, LLKHF_INJECTED, LLKHF_UP, MSG, PostMessageW,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_DESTROYCLIPBOARD, WM_RENDERALLFORMATS, WM_RENDERFORMAT, WM_TIMER, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

use crate::clipboard::{self, ClipboardLock};
use crate::foreground;
use chord::{Key, Mods};

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
/// Posted by the hook when a keep-open keystroke has gone through it: every
/// key the user let go of before then has been seen, so the modifiers still
/// held can be pressed again.
const WM_REPRESS: u32 = WM_APP + 2;

/// Carried by the last key of a keep-open keystroke, for the hook to know it.
const KEYSTROKE_MARK: usize = 0x4C75_6D65;

const READ_TIMER: usize = 1;
const GRACE_TIMER: usize = 2;

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
    // already, unless the read was not seen; the clipboard may still be
    // put back after this returns (see `settle`).
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
    // The clipboard is the last paste's until it has been put back.
    settle();
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
        // The chord that asked for this may still be down: not waited for,
        // but let go of around the keystroke (`Owner::type_kept_open`), so
        // that holding it to paste again does not cost a second a paste.
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
    let (handed, given_back) = mpsc::channel();
    let send = Delivery {
        keys: paste.keys,
        caret_back: paste.caret_back,
        put_back: paste.restore_clipboard,
        keep_out_of_history: paste.keep_out_of_history,
        give_back: launcher.map(|launcher| (launcher.0 as isize, handed)),
    };
    let raw = window.0;
    let owner = std::thread::Builder::new()
        .name("lumen-paste".into())
        .spawn(move || Owner::run(HWND(raw as *mut c_void), formats, send))
        .context("starting the paste")?;
    // Kept open, the paste is over for the caller once the launcher has the
    // foreground back, at the target's read: the owner stays on for the
    // grace and the putting back, and the next paste waits for it then.
    // Disconnected, the owner ended without a read; its outcome says why.
    if keep_open && given_back.recv().is_ok() {
        *LINGERING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(owner);
        return Ok(());
    }
    owner
        .join()
        .map_err(|_| anyhow!("the paste stopped unexpectedly"))?
}

/// The owner of a kept-open paste that was read, still putting the clipboard
/// back. One at most: the next paste settles it before it starts.
static LINGERING: Mutex<Option<JoinHandle<Result<()>>>> = Mutex::new(None);

/// Wait for a kept-open paste still putting the clipboard back: offering the
/// next text before then would save the last one's to put back, and leave
/// it there. What went wrong is only logged: that paste has been told over.
fn settle() {
    let lingering = LINGERING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let Some(owner) = lingering else {
        return;
    };
    let started = Instant::now();
    match owner.join() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => log::warn!("a paste kept open did not finish: {error:#}"),
        Err(_) => log::warn!("a paste kept open stopped unexpectedly"),
    }
    log::debug!("paste: waited {:?} for the last one", started.elapsed());
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
    send(
        &chord::keystrokes(Mods::NONE, keys, caret_back, MAX_CARET_BACK),
        0,
    )
}

/// Type `typed` in one call, its last key marked with `mark` for the hook
/// that watches for it (see [`ModifierWatch`]); zero marks nothing.
fn send(typed: &[Key], mark: usize) -> Result<()> {
    let mut inputs: Vec<INPUT> = typed
        .iter()
        .map(|key| {
            let mut flags = KEYBD_EVENT_FLAGS(0);
            if key.extended {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            if key.up {
                flags |= KEYEVENTF_KEYUP;
            }
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(key.vk),
                        dwFlags: flags,
                        ..Default::default()
                    },
                },
            }
        })
        .collect();
    if let Some(last) = inputs.last_mut() {
        last.Anonymous.ki.dwExtraInfo = mark;
    }
    if inputs.is_empty() {
        return Ok(());
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
    /// target reads, so what the user types next is not the target's; and
    /// where to say it has been.
    give_back: Option<(isize, mpsc::Sender<()>)>,
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
    give_back: Option<(HWND, mpsc::Sender<()>)>,
    /// Kept open: the modifiers held when the keystroke was typed, while it
    /// is let go of and some pressed again.
    watch: Option<ModifierWatch>,
    /// Let go of for the keystroke and to be pressed again if still held;
    /// emptied once that is decided.
    candidates: Mods,
    /// Pressed again, to be let go of if the user had already.
    repressed: Mods,
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

        let kept_open = send.give_back.is_some();
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
            kept_open,
            give_back: send
                .give_back
                .map(|(raw, handed)| (HWND(raw as *mut c_void), handed)),
            watch: None,
            candidates: Mods::NONE,
            repressed: Mods::NONE,
            owed: Vec::new(),
            outcome: None,
        };
        OWNER.with(|cell| *cell.borrow_mut() = Some(owner));

        let offered = with_owner(|owner| owner.offer()).unwrap_or(Ok(()));
        let result = offered.and_then(|()| {
            if kept_open {
                with_owner(|owner| owner.type_kept_open(send.keys, send.caret_back))
                    .unwrap_or(Ok(()))?;
            } else {
                send_keys(send.keys, send.caret_back)?;
            }
            unsafe { SetTimer(Some(hwnd), READ_TIMER, READ_WAIT.as_millis() as u32, None) };
            pump();
            with_owner(|owner| owner.outcome.take())
                .flatten()
                .unwrap_or(Ok(()))
        });
        with_owner(|owner| owner.stop_watching());

        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        OWNER.with(|cell| cell.borrow_mut().take());
        result
    }

    /// Type the keystroke with the launcher kept open, the chord that asked
    /// for it possibly still down.
    ///
    /// Shift, Alt and Windows held would turn `Ctrl+V` into something else,
    /// so they are let go of in the same batch, just before it; Alt or
    /// Windows after an unassigned key, so that the release is not a lone
    /// one, which opens a menu or Start. Shift and Control are pressed again
    /// afterwards if the user is still holding them, so that the chord
    /// pastes again without being pressed anew. Which ones are still held
    /// `GetAsyncKeyState` cannot say once they have been let go of here, so
    /// a low-level hook watches the real keys meanwhile ([`ModifierWatch`]).
    fn type_kept_open(&mut self, keys: PasteKeys, caret_back: usize) -> Result<()> {
        let held = logical_modifiers();
        let candidates = chord::candidates(held, keys);
        if !candidates.is_empty() {
            self.watch = ModifierWatch::start(self.hwnd, held);
            if self.watch.is_some() {
                self.candidates = candidates;
            } else {
                // Left up: a key pressed again blindly could stay down.
                log::debug!("paste: could not watch the modifiers");
            }
        }
        let mark = if self.watch.is_some() {
            KEYSTROKE_MARK
        } else {
            0
        };
        send(
            &chord::keystrokes(held, keys, caret_back, MAX_CARET_BACK),
            mark,
        )
    }

    /// The keystroke has gone through the hook: press again the modifiers
    /// let go of for it that the user is still holding.
    fn repress(&mut self) {
        let again = chord::repress(self.candidates, ModifierWatch::physical());
        self.candidates = Mods::NONE;
        if again.is_empty() {
            return;
        }
        match send(&chord::presses(again, false), 0) {
            Ok(()) => self.repressed = again,
            Err(error) => log::debug!("could not press the modifiers again: {error:#}"),
        }
    }

    /// The paste is over: decide what is still undecided, let go of a key
    /// pressed again after the user had, and stop watching.
    fn stop_watching(&mut self) {
        if self.watch.is_none() {
            return;
        }
        if !self.candidates.is_empty() {
            self.repress();
        }
        let stuck = chord::stuck(
            self.repressed,
            ModifierWatch::physical(),
            logical_modifiers(),
        );
        self.watch = None;
        if !stuck.is_empty()
            && let Err(error) = send(&chord::presses(stuck, true), 0)
        {
            log::debug!("could not let go of a modifier: {error:#}");
        }
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

/// The modifiers down as far as Windows knows, left and right apart.
fn logical_modifiers() -> Mods {
    chord::KEYS
        .iter()
        .filter(|(_, vk, _)| unsafe { GetAsyncKeyState(i32::from(*vk)) } < 0)
        .fold(Mods::NONE, |held, (bit, _, _)| held.or(*bit))
}

/// The window told when the keystroke has gone through [`watch_modifiers`];
/// zero when nothing is watched.
static WATCHER: AtomicIsize = AtomicIsize::new(0);
/// The modifiers physically down, as the hook has seen the real keys.
static PHYSICAL: AtomicU8 = AtomicU8::new(0);

/// A low-level keyboard hook on the paste's thread, which pumps the messages
/// it is called through, for as long as the paste lasts. Removed on drop.
struct ModifierWatch(HHOOK);

impl ModifierWatch {
    /// Watch from `held`, which the user is taken to be holding now.
    fn start(owner: HWND, held: Mods) -> Option<Self> {
        PHYSICAL.store(held.0, Ordering::SeqCst);
        WATCHER.store(owner.0 as isize, Ordering::SeqCst);
        let instance = unsafe { GetModuleHandleW(None) }.ok()?;
        match unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(watch_modifiers),
                Some(instance.into()),
                0,
            )
        } {
            Ok(hook) => Some(Self(hook)),
            Err(error) => {
                WATCHER.store(0, Ordering::SeqCst);
                log::debug!("could not hook the keyboard: {error}");
                None
            }
        }
    }

    fn physical() -> Mods {
        Mods(PHYSICAL.load(Ordering::SeqCst))
    }
}

impl Drop for ModifierWatch {
    fn drop(&mut self) {
        WATCHER.store(0, Ordering::SeqCst);
        let _ = unsafe { UnhookWindowsHookEx(self.0) };
    }
}

/// The hook: a real modifier going up or down is noted, the keystroke's
/// last key passing is posted, and every key goes on. Nothing else (I-15).
extern "system" fn watch_modifiers(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // For `HC_ACTION`, `lparam` is the event, valid for this call.
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if (event.flags & LLKHF_INJECTED).0 != 0 {
            let watcher = WATCHER.load(Ordering::SeqCst);
            if event.dwExtraInfo == KEYSTROKE_MARK && watcher != 0 {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(watcher as *mut c_void)),
                        WM_REPRESS,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        } else {
            let bit = chord::bit_of(event.vkCode);
            if (event.flags & LLKHF_UP).0 != 0 {
                PHYSICAL.fetch_and(!bit.0, Ordering::SeqCst);
            } else {
                PHYSICAL.fetch_or(bit.0, Ordering::SeqCst);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
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
            if let Some(Some((launcher, handed))) = with_owner(|owner| owner.give_back.take()) {
                hand_foreground(launcher);
                let _ = handed.send(());
            }
            LRESULT(0)
        }
        WM_REPRESS => {
            with_owner(|owner| {
                if !owner.candidates.is_empty() {
                    owner.repress();
                }
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
                    if owner.put_back {
                        owner.restore()
                    } else {
                        // Taken, and the text stays: supplied, as it goes.
                        owner.give_up()
                    }
                } else if owner.kept_open && owner.put_back {
                    owner.restore().and_then(|()| {
                        Err(anyhow!(
                            "the window did not take the paste -- it may be running as \n                             administrator -- so nothing was pasted"
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

/// What a keep-open paste types, worked out without Windows: which held
/// modifiers are let go of before the keystroke, which are pressed again
/// after it, and the order of it all. Pure, so it is tested here; the
/// keyboard itself is checked by hand.
mod chord {
    use lumen_platform::PasteKeys;

    /// A set of modifier keys, left and right apart: letting go of `VK_SHIFT`
    /// lets go of the left one only, and a right Shift left down would make
    /// the paste `Ctrl+Shift+V`.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Mods(pub u8);

    pub const LSHIFT: Mods = Mods(1);
    pub const RSHIFT: Mods = Mods(1 << 1);
    pub const LCTRL: Mods = Mods(1 << 2);
    pub const RCTRL: Mods = Mods(1 << 3);
    pub const LALT: Mods = Mods(1 << 4);
    pub const RALT: Mods = Mods(1 << 5);
    pub const LWIN: Mods = Mods(1 << 6);
    pub const RWIN: Mods = Mods(1 << 7);

    impl Mods {
        pub const NONE: Mods = Mods(0);

        pub const fn or(self, other: Mods) -> Mods {
            Mods(self.0 | other.0)
        }
        pub const fn and(self, other: Mods) -> Mods {
            Mods(self.0 & other.0)
        }
        pub const fn without(self, other: Mods) -> Mods {
            Mods(self.0 & !other.0)
        }
        pub const fn is_empty(self) -> bool {
            self.0 == 0
        }
    }

    /// One key going down or up.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Key {
        pub vk: u16,
        pub extended: bool,
        pub up: bool,
    }

    pub const VK_SHIFT: u16 = 0x10;
    pub const VK_CONTROL: u16 = 0x11;
    /// `'V'` on every layout: applications match `Ctrl+V` on the virtual
    /// key, not on the character.
    pub const VK_V: u16 = 0x56;
    pub const VK_INSERT: u16 = 0x2D;
    pub const VK_LEFT: u16 = 0x25;
    /// Unassigned: typed between a held Alt or Windows key and its release,
    /// so the release is not a lone one, which opens a window's menu (Alt)
    /// or Start (Windows).
    pub const VK_DUMMY: u16 = 0xE8;

    /// Every modifier with its key, in the order they are let go of.
    pub const KEYS: [(Mods, u16, bool); 8] = [
        (LSHIFT, 0xA0, false),
        (RSHIFT, 0xA1, false),
        (LCTRL, 0xA2, false),
        (RCTRL, 0xA3, true),
        (LALT, 0xA4, false),
        (RALT, 0xA5, true),
        (LWIN, 0x5B, true),
        (RWIN, 0x5C, true),
    ];

    /// The modifier a key is, as a low-level hook reports it; none for any
    /// other key. Allocates nothing: the hook calls it (I-15).
    pub const fn bit_of(vk: u32) -> Mods {
        let mut at = 0;
        while at < KEYS.len() {
            if KEYS[at].1 as u32 == vk {
                return KEYS[at].0;
            }
            at += 1;
        }
        Mods::NONE
    }

    /// The modifiers that change what `Ctrl+V` means, let go of before it.
    const CHANGES_THE_KEYSTROKE: Mods = LSHIFT.or(RSHIFT).or(LALT).or(RALT).or(LWIN).or(RWIN);

    /// The keystroke, with `held` down when it is typed: the held keys that
    /// would change it let go of first, then the keys `send_keys` has always
    /// typed. With nothing held, exactly those.
    pub fn keystrokes(
        held: Mods,
        keys: PasteKeys,
        caret_back: usize,
        max_caret_back: usize,
    ) -> Vec<Key> {
        let key = |vk, extended, up| Key { vk, extended, up };
        let release = held.and(CHANGES_THE_KEYSTROKE);
        let mut typed = Vec::new();
        if !held.and(LALT.or(RALT).or(LWIN).or(RWIN)).is_empty() {
            typed.push(key(VK_DUMMY, false, false));
            typed.push(key(VK_DUMMY, false, true));
        }
        typed.extend(presses(release, true));

        let (modifiers, main, extended): (&[u16], u16, bool) = match keys {
            PasteKeys::CtrlV => (&[VK_CONTROL], VK_V, false),
            PasteKeys::CtrlShiftV => (&[VK_CONTROL, VK_SHIFT], VK_V, false),
            // Insert is on the extended keypad: without the flag some
            // programs read it as the number pad's 0.
            PasteKeys::ShiftInsert => (&[VK_SHIFT], VK_INSERT, true),
        };
        typed.extend(modifiers.iter().map(|vk| key(*vk, false, false)));
        typed.push(key(main, extended, false));
        typed.push(key(main, extended, true));
        typed.extend(modifiers.iter().rev().map(|vk| key(*vk, false, true)));
        // Past this, the caret is left at the end: a `{cursor}` that far from
        // it is more keystrokes than moving it is worth.
        if caret_back <= max_caret_back {
            for _ in 0..caret_back {
                typed.push(key(VK_LEFT, true, false));
                typed.push(key(VK_LEFT, true, true));
            }
        }
        typed
    }

    /// The modifiers the keystroke leaves up that may have to go down again:
    /// Shift and Control, the chord a user holds to paste again. Never Alt
    /// or Windows: pressed again here, their release by the user would be a
    /// lone one, with a menu or Start to show for it.
    pub fn candidates(held: Mods, keys: PasteKeys) -> Mods {
        // `VK_SHIFT` and `VK_CONTROL` are the left keys when typed.
        let typed = match keys {
            PasteKeys::CtrlV => LCTRL,
            PasteKeys::CtrlShiftV => LCTRL.or(LSHIFT),
            PasteKeys::ShiftInsert => LSHIFT,
        };
        held.and(CHANGES_THE_KEYSTROKE.or(typed))
            .and(LSHIFT.or(RSHIFT).or(LCTRL).or(RCTRL))
    }

    /// Of `candidates`, those to press again: the ones still physically down.
    /// One let go of meanwhile stays up.
    pub fn repress(candidates: Mods, physical: Mods) -> Mods {
        candidates.and(physical)
    }

    /// Of the keys pressed again, those let go of by the user before the
    /// press landed, and so left down: to be let go of.
    pub fn stuck(repressed: Mods, physical: Mods, logical: Mods) -> Mods {
        repressed.without(physical).and(logical)
    }

    /// `mods` as key presses (`up` false) or releases.
    pub fn presses(mods: Mods, up: bool) -> Vec<Key> {
        KEYS.iter()
            .filter(|(bit, _, _)| !mods.and(*bit).is_empty())
            .map(|&(_, vk, extended)| Key { vk, extended, up })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn key(vk: u16, up: bool) -> Key {
            Key {
                vk,
                extended: false,
                up,
            }
        }

        #[test]
        fn nothing_held_types_exactly_what_a_plain_paste_types() {
            assert_eq!(
                keystrokes(Mods::NONE, PasteKeys::CtrlV, 0, 10),
                vec![
                    key(VK_CONTROL, false),
                    key(VK_V, false),
                    key(VK_V, true),
                    key(VK_CONTROL, true),
                ]
            );
            let shift_insert = keystrokes(Mods::NONE, PasteKeys::ShiftInsert, 1, 10);
            assert_eq!(shift_insert.len(), 6);
            assert_eq!(shift_insert[0], key(VK_SHIFT, false));
            assert!(shift_insert[1].extended && shift_insert[1].vk == VK_INSERT);
            assert!(shift_insert[4].extended && shift_insert[4].vk == VK_LEFT);
            // Past the most, the caret is left at the end.
            assert_eq!(keystrokes(Mods::NONE, PasteKeys::CtrlV, 11, 10).len(), 4);
        }

        #[test]
        fn held_shift_is_let_go_of_before_the_keystroke() {
            let typed = keystrokes(LSHIFT.or(RSHIFT).or(LCTRL), PasteKeys::CtrlV, 0, 10);
            assert_eq!(&typed[..2], &[key(0xA0, true), key(0xA1, true)]);
            // Control is not let go of: Ctrl+V with Control down is Ctrl+V.
            assert!(!typed.iter().any(|k| k.vk == 0xA2));
            assert_eq!(typed[2], key(VK_CONTROL, false));
            assert!(!typed.iter().any(|k| k.vk == VK_DUMMY));
        }

        /// Alt: the dummy key while Alt is still down, then Alt up, then the
        /// paste, in one batch, so Alt's release is not a lone one.
        #[test]
        fn alt_is_let_go_of_after_a_dummy_key_and_before_the_paste() {
            let typed = keystrokes(LALT.or(LSHIFT), PasteKeys::CtrlV, 0, 10);
            assert_eq!(
                typed,
                vec![
                    key(VK_DUMMY, false),
                    key(VK_DUMMY, true),
                    key(0xA0, true),
                    key(0xA4, true),
                    key(VK_CONTROL, false),
                    key(VK_V, false),
                    key(VK_V, true),
                    key(VK_CONTROL, true),
                ]
            );
            let right = keystrokes(RALT.or(RWIN), PasteKeys::CtrlV, 0, 10);
            assert_eq!(right[0], key(VK_DUMMY, false));
            assert_eq!(
                &right[2..4],
                &[
                    Key {
                        vk: 0xA5,
                        extended: true,
                        up: true
                    },
                    Key {
                        vk: 0x5C,
                        extended: true,
                        up: true
                    },
                ]
            );
        }

        #[test]
        fn only_shift_and_control_are_pressed_again() {
            let held = LSHIFT.or(LCTRL).or(LALT).or(RWIN);
            assert_eq!(candidates(held, PasteKeys::CtrlV), LSHIFT.or(LCTRL));
            // A right Control is never let go of, so not pressed again.
            assert_eq!(candidates(RCTRL.or(RSHIFT), PasteKeys::CtrlV), RSHIFT);
            // Shift+Insert lets go of the left Shift it presses.
            assert_eq!(candidates(LSHIFT, PasteKeys::ShiftInsert), LSHIFT);
            assert_eq!(candidates(LCTRL, PasteKeys::ShiftInsert), Mods::NONE);
            assert_eq!(candidates(Mods::NONE, PasteKeys::CtrlShiftV), Mods::NONE);
        }

        #[test]
        fn a_modifier_let_go_of_during_the_paste_is_not_pressed_again() {
            let candidates = LSHIFT.or(LCTRL);
            assert_eq!(repress(candidates, LSHIFT.or(LCTRL)), candidates);
            // Shift let go of physically: Control only.
            assert_eq!(repress(candidates, LCTRL), LCTRL);
            assert_eq!(repress(candidates, Mods::NONE), Mods::NONE);
            // Held but never let go of by us: not ours to press.
            assert_eq!(repress(LCTRL, LCTRL.or(LALT)), LCTRL);
        }

        #[test]
        fn a_press_that_lands_after_the_release_is_undone() {
            // Shift pressed again, then let go of before the press landed.
            assert_eq!(stuck(LSHIFT.or(LCTRL), LCTRL, LSHIFT.or(LCTRL)), LSHIFT);
            // Let go of after it landed: Windows saw the release.
            assert_eq!(stuck(LSHIFT, Mods::NONE, Mods::NONE), Mods::NONE);
            // Still held: left down.
            assert_eq!(stuck(LSHIFT, LSHIFT, LSHIFT), Mods::NONE);
        }

        #[test]
        fn the_hook_knows_the_modifiers_left_and_right() {
            assert_eq!(bit_of(0xA0), LSHIFT);
            assert_eq!(bit_of(0xA3), RCTRL);
            assert_eq!(bit_of(0xA5), RALT);
            assert_eq!(bit_of(0x5B), LWIN);
            assert_eq!(bit_of(0x5C), RWIN);
            assert_eq!(bit_of(0x56), Mods::NONE);
            // The generic keys are never what a low-level hook reports.
            assert_eq!(bit_of(0x10), Mods::NONE);
        }
    }
}
