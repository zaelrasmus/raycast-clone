//! Global keyboard gestures on Windows.
//!
//! # Deviation from section 7, and why
//!
//! The plan proposes Raw Input as the *single* event source, deriving both
//! invocation and the hold gesture from it. The reasoning -- one source of
//! truth rather than two event loops -- is right, but Raw Input cannot suppress
//! a keystroke. A launcher bound to `Alt+Space` that does not suppress leaves
//! the keystroke to reach the focused application, where `Alt+Space` opens the
//! system menu. The launcher would appear with a window menu open behind it.
//!
//! What is actually load-bearing in AD-16 is narrower than "one mechanism": a
//! lone held modifier with a measured duration and a release event does not fit
//! the accelerator model. Invocation fits it perfectly.
//!
//! So the split here is by *concern*, not by mechanism:
//!
//! - `RegisterHotKey` for invocation -- suppresses, no hook, no timeout risk.
//! - Raw Input (`RIDEV_INPUTSINK`) for the hold gesture -- reports release.
//!
//! Both are owned by one message-only window on one thread with one message
//! loop, so there are still not two event loops coexisting, which is the
//! property section 7 was protecting. `WH_KEYBOARD_LL` is still not used, and
//! the hazards described around it therefore still do not apply.

use std::ffi::c_void;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use lumen_platform::{
    Chord, GestureEvent, GestureSink, HoldPhase, Hotkey, HotkeyError, Modifier, Modifiers,
    canonical_key,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HID_USAGE_GENERIC_KEYBOARD, HID_USAGE_PAGE_GENERIC,
};
use windows::Win32::Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    RIDEV_INPUTSINK, RIM_TYPEKEYBOARD, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, KillTimer, MSG,
    PostMessageW, RegisterClassW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_APP, WM_HOTKEY, WM_INPUT, WM_TIMER, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

/// How long the modifier must stay down before the overlay is raised.
///
/// Section F3: "The activation threshold sits around 300-500 ms. Three seconds
/// reads as a broken feature." 350 ms is deliberately at the fast end -- the
/// gesture is deliberate, and a threshold the user can feel waiting is the
/// thing that makes an overlay feel cheap.
const HOLD_THRESHOLD: Duration = Duration::from_millis(350);

const HOTKEY_ID: i32 = 1;
/// The timers hotkey (F8), in the same listener: one thread, one window.
const TIMERS_HOTKEY_ID: i32 = 2;
const HOLD_TIMER_ID: usize = 1;
/// Command hotkeys (F17) take ids from here up, slot `i` at `BASE + i`, clear
/// of the launcher's and the timers' fixed ids.
const COMMAND_HOTKEY_BASE: usize = 0x100;
/// The last id `RegisterHotKey` accepts from an application.
const LAST_HOTKEY_ID: usize = 0xBFFF;
/// Posted to the listener window: a new set of command hotkeys is waiting in
/// its request channel (7.53).
const APPLY_COMMAND_HOTKEYS: u32 = WM_APP + 1;
/// How long a caller waits for the listener to answer before giving up. It
/// answers in well under a millisecond unless its thread is wedged.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

type SlotResults = Vec<Result<(), HotkeyError>>;

/// A new set of command hotkeys, and where to send what became of each.
struct Request {
    chords: Vec<Chord>,
    reply: mpsc::SyncSender<SlotResults>,
}

/// Per-thread state for the listener window, reached from the window procedure.
///
/// Thread-local rather than behind a lock: the window procedure is the only
/// reader and writer, it runs on exactly one thread, and taking a lock in an
/// input path is precisely what I-15 forbids.
struct ListenerState {
    sink: GestureSink,
    hold_key: Option<Modifier>,
    /// Whether the hold gesture has already crossed its threshold, so that
    /// auto-repeat does not raise the overlay a second time and release knows
    /// whether it has anything to close.
    hold_active: bool,
    /// Whether the hold key is physically down, so auto-repeat presses are not
    /// treated as fresh presses that restart the timer.
    hold_down: bool,
    hwnd: HWND,
    /// The launcher and timers hotkeys, which no command may take.
    reserved: Vec<Hotkey>,
    /// New command hotkey sets, from [`Listener::set_command_hotkeys`].
    requests: mpsc::Receiver<Request>,
    /// How many command slots the current set registered ids for.
    commands: usize,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<ListenerState>> = const { std::cell::RefCell::new(None) };
}

/// Start the listener on a dedicated thread and return once it is registered.
///
/// The thread owns a message-only window for its whole life. It is never joined:
/// it lives exactly as long as the daemon does.
///
/// Returns the listener and, separately, whether the launcher hotkey could be
/// bound: a taken launcher key leaves the listener running all the same.
pub fn start(
    hotkey: Hotkey,
    timers: Option<Hotkey>,
    sink: GestureSink,
) -> Result<(Listener, Result<()>)> {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(isize, Result<()>)>>(1);
    let (requests_tx, requests_rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("lumen-gestures".into())
        .spawn(move || {
            match run(hotkey, timers, sink, requests_rx) {
                // The listener is up either way. `accelerator` reports only
                // whether the launcher hotkey could be bound, and the pump runs
                // regardless so the hold gesture keeps working.
                Ok((pump, hwnd, accelerator)) => {
                    let _ = ready_tx.send(Ok((hwnd, accelerator)));
                    pump();
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            }
        })
        .context("spawning the gesture listener thread")?;

    let (hwnd, bound) = ready_rx
        .recv()
        .context("the gesture listener thread stopped before reporting readiness")??;
    let listener = Listener {
        hwnd,
        requests: requests_tx,
    };
    Ok((listener, bound))
}

/// A running listener, as the rest of the process reaches it.
///
/// The window handle is kept as an integer because `HWND` is a raw pointer
/// and would make the platform `!Send`. It is only ever posted to, which any
/// thread may do.
pub struct Listener {
    hwnd: isize,
    requests: mpsc::Sender<Request>,
}

impl Listener {
    /// Replace the command hotkeys and wait for what became of each (7.53).
    ///
    /// `RegisterHotKey` binds a key to a window, from the thread that owns
    /// it, so the set is handed to the listener's thread and this waits for
    /// the answer.
    pub fn set_command_hotkeys(&self, chords: &[Chord]) -> Result<SlotResults> {
        let stopped = "the gesture listener has stopped";
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.requests
            .send(Request {
                chords: chords.to_vec(),
                reply: reply_tx,
            })
            .context(stopped)?;
        unsafe {
            PostMessageW(
                Some(HWND(self.hwnd as *mut c_void)),
                APPLY_COMMAND_HOTKEYS,
                WPARAM(0),
                LPARAM(0),
            )
        }
        .context(stopped)?;

        Ok(reply_rx.recv_timeout(REPLY_TIMEOUT).unwrap_or_else(|_| {
            chords
                .iter()
                .map(|_| {
                    Err(HotkeyError::Failed(
                        "the gesture listener did not answer".into(),
                    ))
                })
                .collect()
        }))
    }
}

/// Register everything, then hand back the message pump plus the hotkey outcome.
///
/// Split this way so registration failures surface to the caller instead of
/// disappearing into a thread that simply never delivers events -- and so that a
/// hotkey that is already taken is reported without tearing down a listener that
/// is otherwise working.
fn run(
    hotkey: Hotkey,
    timers: Option<Hotkey>,
    sink: GestureSink,
    requests: mpsc::Receiver<Request>,
) -> Result<(impl FnOnce(), isize, Result<()>)> {
    unsafe {
        let instance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        let class_name = w!("LumenGestureListener");

        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        // A duplicate class registration is expected if the daemon ever
        // restarts the listener in-process, and is not an error.
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
            // HWND_MESSAGE: invisible, never enumerated, receives no broadcast
            // messages. It exists only as an address for input to arrive at.
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
        .context("creating the message-only listener window")?;

        STATE.with(|state| {
            *state.borrow_mut() = Some(ListenerState {
                sink,
                hold_key: hotkey.hold,
                hold_active: false,
                hold_down: false,
                hwnd,
                reserved: std::iter::once(hotkey.clone())
                    .chain(timers.clone())
                    .collect(),
                requests,
                commands: 0,
            });
        });

        // Raw Input first, and deliberately so.
        //
        // The hold gesture and the launcher hotkey are independent concerns
        // that happen to share a message loop -- that is the whole basis of the
        // split described above. Registering the accelerator first and using
        // `?` made a taken hotkey return before this ran, which silently
        // disabled the overlay as well. One unavailable key must not cost the
        // other feature.
        if hotkey.hold.is_some() {
            let device = RAWINPUTDEVICE {
                usUsagePage: HID_USAGE_PAGE_GENERIC,
                usUsage: HID_USAGE_GENERIC_KEYBOARD,
                // INPUTSINK: deliver events even though this window is never in
                // the foreground, which is the entire point.
                dwFlags: RIDEV_INPUTSINK,
                hwndTarget: hwnd,
            };
            RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32)
                .context("registering for raw keyboard input")?;
        }

        let accelerator = accelerator_for(&hotkey).and_then(|(modifiers, vk)| {
            // MOD_NOREPEAT: holding the hotkey must summon the launcher once,
            // not toggle it at the keyboard's repeat rate.
            RegisterHotKey(Some(hwnd), HOTKEY_ID, modifiers | MOD_NOREPEAT, vk).with_context(|| {
                format!("registering the global hotkey {hotkey}; another application may hold it")
            })
        });

        // The timers key is a convenience: taken or unreadable, it is said in
        // the log and everything else carries on.
        if let Some(timers) = timers {
            let bound = accelerator_for(&timers).and_then(|(modifiers, vk)| {
                RegisterHotKey(Some(hwnd), TIMERS_HOTKEY_ID, modifiers | MOD_NOREPEAT, vk)
                    .with_context(|| format!("registering the timers hotkey {timers}"))
            });
            if let Err(error) = bound {
                log::warn!("{error:#}; open Timers from the launcher or with `lumen timers`");
            }
        }

        Ok((pump_messages, hwnd.0 as isize, accelerator))
    }
}

fn pump_messages() {
    unsafe {
        let mut message = MSG::default();
        // `GetMessageW` returns -1 on error and 0 on WM_QUIT; both end the loop.
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
        WM_HOTKEY if wparam.0 as i32 == HOTKEY_ID => {
            emit(GestureEvent::Invoke);
            LRESULT(0)
        }
        WM_HOTKEY if wparam.0 as i32 == TIMERS_HOTKEY_ID => {
            emit(GestureEvent::Timers);
            LRESULT(0)
        }
        // I-15: arithmetic on the id and a push, nothing else.
        WM_HOTKEY if wparam.0 >= COMMAND_HOTKEY_BASE => {
            emit(GestureEvent::Command(
                (wparam.0 - COMMAND_HOTKEY_BASE) as u32,
            ));
            LRESULT(0)
        }
        // Not an input callback: a request from `set_command_hotkeys`, which
        // may allocate and call the system as it likes.
        APPLY_COMMAND_HOTKEYS => {
            apply_command_hotkeys();
            LRESULT(0)
        }
        WM_INPUT => {
            handle_raw_input(lparam);
            // WM_INPUT must still reach DefWindowProc so the system can clean
            // up the input buffer. Swallowing it leaks.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_TIMER if wparam.0 == HOLD_TIMER_ID => {
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                let Some(state) = state.as_mut() else {
                    return;
                };
                unsafe {
                    let _ = KillTimer(Some(state.hwnd), HOLD_TIMER_ID);
                }
                if state.hold_down && !state.hold_active {
                    state.hold_active = true;
                    state.sink.emit(GestureEvent::Hold(HoldPhase::Began));
                }
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// I-15 applies here: no allocation, no logging, no locks, no UI state.
///
/// `RAWINPUT` is read into a stack value rather than a heap buffer precisely so
/// that this path allocates nothing, on every keystroke the user ever types.
fn handle_raw_input(lparam: LPARAM) {
    let mut raw = RAWINPUT::default();
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;

    let read = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam.0 as *mut c_void),
            RID_INPUT,
            Some(&mut raw as *mut _ as *mut c_void),
            &mut size,
            header_size,
        )
    };
    if read == u32::MAX || raw.header.dwType != RIM_TYPEKEYBOARD.0 {
        return;
    }

    let keyboard = unsafe { raw.data.keyboard };
    let released = keyboard.Flags & RI_KEY_BREAK != 0;
    let extended = keyboard.Flags & RI_KEY_E0 != 0;
    let Some(modifier) = modifier_from_raw(keyboard.VKey, keyboard.MakeCode, extended) else {
        // Any other key ends a hold in progress: the gesture is a *lone* held
        // modifier, and the moment it becomes a chord it is something else.
        if !released {
            cancel_hold();
        }
        return;
    };

    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        if state.hold_key != Some(modifier) {
            return;
        }

        if released {
            unsafe {
                let _ = KillTimer(Some(state.hwnd), HOLD_TIMER_ID);
            }
            state.hold_down = false;
            if state.hold_active {
                state.hold_active = false;
                state.sink.emit(GestureEvent::Hold(HoldPhase::Ended));
            }
        } else if !state.hold_down {
            // Only a fresh press arms the timer. Auto-repeat would otherwise
            // reset it forever and the threshold would never be crossed.
            state.hold_down = true;
            unsafe {
                SetTimer(
                    Some(state.hwnd),
                    HOLD_TIMER_ID,
                    HOLD_THRESHOLD.as_millis() as u32,
                    None,
                );
            }
        }
    });
}

fn cancel_hold() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        unsafe {
            let _ = KillTimer(Some(state.hwnd), HOLD_TIMER_ID);
        }
        state.hold_down = false;
        if state.hold_active {
            state.hold_active = false;
            state.sink.emit(GestureEvent::Hold(HoldPhase::Ended));
        }
    });
}

/// Answer every waiting request, in order, each replacing the whole set.
fn apply_command_hotkeys() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        // One post per request, but a post can find several waiting, or none
        // because an earlier post already answered its request.
        while let Ok(request) = state.requests.try_recv() {
            for slot in 0..state.commands {
                unsafe {
                    let _ = UnregisterHotKey(Some(state.hwnd), (COMMAND_HOTKEY_BASE + slot) as i32);
                }
            }
            let results: SlotResults = request
                .chords
                .iter()
                .enumerate()
                .map(|(slot, chord)| register_command(state, slot, chord))
                .collect();
            state.commands = request.chords.len();
            let _ = request.reply.send(results);
        }
    });
}

fn register_command(state: &ListenerState, slot: usize, chord: &Chord) -> Result<(), HotkeyError> {
    // Checked here rather than left to the system, which would only say so
    // when the launcher key actually bound. A launcher key another
    // application holds today must still be kept off the commands, or the
    // launcher could not take it back once it is free.
    if state.reserved.iter().any(|hotkey| chord.is_hotkey(hotkey)) {
        return Err(HotkeyError::Taken);
    }
    let id = COMMAND_HOTKEY_BASE + slot;
    if id > LAST_HOTKEY_ID {
        return Err(HotkeyError::Failed(format!(
            "at most {} command hotkeys",
            LAST_HOTKEY_ID - COMMAND_HOTKEY_BASE + 1
        )));
    }
    let Some(vk) = virtual_key(&chord.key) else {
        return Err(HotkeyError::Unsupported(format!(
            "{:?} has no Windows virtual key",
            chord.key
        )));
    };
    // MOD_NOREPEAT, as for the launcher: held down, a command runs once.
    let modifiers = hot_key_modifiers(&chord.modifiers) | MOD_NOREPEAT;
    unsafe { RegisterHotKey(Some(state.hwnd), id as i32, modifiers, vk) }.map_err(|error| {
        if error.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult() {
            HotkeyError::Taken
        } else {
            HotkeyError::Failed(error.message())
        }
    })
}

fn emit(event: GestureEvent) {
    STATE.with(|state| {
        if let Some(state) = state.borrow().as_ref() {
            state.sink.emit(event);
        }
    });
}

const RI_KEY_BREAK: u16 = 0x01;
const RI_KEY_E0: u16 = 0x02;

// Virtual-key codes. Spelled out rather than imported so that the scancode
// arithmetic below reads against the same table.
const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_LSHIFT: u16 = 0xA0;
const VK_RSHIFT: u16 = 0xA1;
const VK_LCONTROL: u16 = 0xA2;
const VK_RCONTROL: u16 = 0xA3;
const VK_LMENU: u16 = 0xA4;
const VK_RMENU: u16 = 0xA5;
const VK_LWIN: u16 = 0x5B;
const VK_RWIN: u16 = 0x5C;
const SCANCODE_RIGHT_SHIFT: u16 = 0x36;

/// Resolve a raw keyboard event to a side-specific modifier.
///
/// Raw Input reports the *generic* virtual key for shift, control and alt, so
/// the side has to be recovered: from the E0 prefix for control and alt, and
/// from the scancode for shift, which has no E0 form.
fn modifier_from_raw(vkey: u16, make_code: u16, extended: bool) -> Option<Modifier> {
    // Some drivers -- and every injected event -- report the side-specific
    // virtual key directly instead of the generic one plus an E0 prefix. Both
    // spellings mean the same physical key, and handling only the generic form
    // makes the gesture work on one keyboard and not on the next.
    match vkey {
        VK_LSHIFT => return Some(Modifier::LeftShift),
        VK_RSHIFT => return Some(Modifier::RightShift),
        VK_LCONTROL => return Some(Modifier::LeftControl),
        VK_RCONTROL => return Some(Modifier::RightControl),
        VK_LMENU => return Some(Modifier::LeftAlt),
        VK_RMENU => return Some(Modifier::RightAlt),
        _ => {}
    }

    Some(match vkey {
        VK_SHIFT => {
            if make_code == SCANCODE_RIGHT_SHIFT {
                Modifier::RightShift
            } else {
                Modifier::LeftShift
            }
        }
        VK_CONTROL => {
            if extended {
                Modifier::RightControl
            } else {
                Modifier::LeftControl
            }
        }
        VK_MENU => {
            if extended {
                Modifier::RightAlt
            } else {
                Modifier::LeftAlt
            }
        }
        VK_LWIN => Modifier::LeftSuper,
        VK_RWIN => Modifier::RightSuper,
        _ => return None,
    })
}

/// Translate a [`Hotkey`] into `RegisterHotKey` arguments.
fn accelerator_for(hotkey: &Hotkey) -> Result<(HOT_KEY_MODIFIERS, u32)> {
    let vk = virtual_key(&hotkey.key)
        .with_context(|| format!("{:?} is not a key this build can bind", hotkey.key))?;
    Ok((hot_key_modifiers(&hotkey.modifiers), vk))
}

fn hot_key_modifiers(modifiers: &Modifiers) -> HOT_KEY_MODIFIERS {
    let mut flags = HOT_KEY_MODIFIERS(0);
    if modifiers.alt {
        flags |= MOD_ALT;
    }
    if modifiers.control {
        flags |= MOD_CONTROL;
    }
    if modifiers.shift {
        flags |= MOD_SHIFT;
    }
    if modifiers.super_key {
        flags |= MOD_WIN;
    }
    flags
}

/// The virtual key for a key name, or `None` if it is not one.
///
/// Every name in [`lumen_platform::KEY_NAMES`] has one, in any spelling
/// [`canonical_key`] accepts; the launcher and timers hotkeys use the same
/// names. The punctuation keys are the `VK_OEM_*` codes, which name key
/// *positions* on the US layout -- the same thing the names describe.
fn virtual_key(name: &str) -> Option<u32> {
    let key = canonical_key(name)?;
    if let [character] = key.as_bytes()
        && character.is_ascii_alphanumeric()
    {
        // Letters and digits are their own uppercase ASCII code.
        return Some(u32::from(*character));
    }
    if let Some(number) = key.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        // VK_F1 (0x70) to VK_F24 (0x87), contiguous.
        return Some(0x70 + number - 1);
    }

    Some(match key {
        "Backspace" => 0x08,
        "Tab" => 0x09,
        "Enter" => 0x0D,
        "Escape" => 0x1B,
        "Space" => 0x20,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "End" => 0x23,
        "Home" => 0x24,
        "Left" => 0x25,
        "Up" => 0x26,
        "Right" => 0x27,
        "Down" => 0x28,
        "Insert" => 0x2D,
        "Delete" => 0x2E,
        "Semicolon" => 0xBA,    // VK_OEM_1
        "Equals" => 0xBB,       // VK_OEM_PLUS
        "Comma" => 0xBC,        // VK_OEM_COMMA
        "Minus" => 0xBD,        // VK_OEM_MINUS
        "Period" => 0xBE,       // VK_OEM_PERIOD
        "Slash" => 0xBF,        // VK_OEM_2
        "Backtick" => 0xC0,     // VK_OEM_3
        "LeftBracket" => 0xDB,  // VK_OEM_4
        "Backslash" => 0xDC,    // VK_OEM_5
        "RightBracket" => 0xDD, // VK_OEM_6
        "Quote" => 0xDE,        // VK_OEM_7
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_letter_and_named_keys() {
        assert_eq!(virtual_key("k"), Some(0x4B));
        assert_eq!(virtual_key("K"), Some(0x4B));
        assert_eq!(virtual_key("Space"), Some(0x20));
        assert_eq!(virtual_key("space"), Some(0x20));
        assert_eq!(virtual_key("F12"), Some(0x7B));
        assert_eq!(virtual_key("Nonsense"), None);
    }

    #[test]
    fn builds_the_expected_accelerator() {
        let hotkey = Hotkey {
            modifiers: Modifiers {
                alt: true,
                control: true,
                ..Default::default()
            },
            key: "Space".into(),
            hold: None,
        };
        let (modifiers, vk) = accelerator_for(&hotkey).unwrap();
        assert_eq!(vk, 0x20);
        assert_eq!(modifiers, MOD_ALT | MOD_CONTROL);
    }

    #[test]
    fn distinguishes_modifier_sides() {
        assert_eq!(
            modifier_from_raw(VK_MENU, 0x38, false),
            Some(Modifier::LeftAlt)
        );
        assert_eq!(
            modifier_from_raw(VK_MENU, 0x38, true),
            Some(Modifier::RightAlt)
        );
        // Shift has no extended form; only the scancode separates the sides.
        assert_eq!(
            modifier_from_raw(VK_SHIFT, 0x2A, false),
            Some(Modifier::LeftShift)
        );
        assert_eq!(
            modifier_from_raw(VK_SHIFT, SCANCODE_RIGHT_SHIFT, false),
            Some(Modifier::RightShift)
        );
        assert_eq!(modifier_from_raw(0x41, 0x1E, false), None);
    }

    #[test]
    fn every_chord_key_name_has_its_own_virtual_key() {
        let mut seen = std::collections::HashMap::new();
        for &name in lumen_platform::KEY_NAMES {
            let vk = virtual_key(name).unwrap_or_else(|| panic!("{name} has no virtual key"));
            if let Some(other) = seen.insert(vk, name) {
                panic!("{name} and {other} both map to {vk:#x}");
            }
        }
    }

    #[test]
    fn maps_navigation_function_and_punctuation_keys() {
        for (name, vk) in [
            ("Backspace", 0x08),
            ("PageUp", 0x21),
            ("PageDown", 0x22),
            ("End", 0x23),
            ("Home", 0x24),
            ("Left", 0x25),
            ("Up", 0x26),
            ("Right", 0x27),
            ("Down", 0x28),
            ("Insert", 0x2D),
            ("Delete", 0x2E),
            ("F13", 0x7C),
            ("F24", 0x87),
            ("Semicolon", 0xBA),
            ("Equals", 0xBB),
            ("Comma", 0xBC),
            ("Minus", 0xBD),
            ("Period", 0xBE),
            ("Slash", 0xBF),
            ("Backtick", 0xC0),
            ("LeftBracket", 0xDB),
            ("Backslash", 0xDC),
            ("RightBracket", 0xDD),
            ("Quote", 0xDE),
            // The spellings the launcher hotkey accepted before F17 still work.
            ("return", 0x0D),
            ("grave", 0xC0),
        ] {
            assert_eq!(virtual_key(name), Some(vk), "{name}");
        }
    }

    // The tests below register real global hotkeys. Each uses all four
    // modifiers and a key in F13-F24, which no keyboard has, so they cannot
    // collide with anything the machine already binds -- and a different key
    // each, since tests run in parallel in one process.

    fn chord(text: &str) -> Chord {
        text.parse().unwrap()
    }

    fn start_listener(
        launcher: &str,
        timers: Option<&str>,
    ) -> (Listener, mpsc::Receiver<GestureEvent>) {
        let hotkey = |text: &str| {
            let chord = chord(text);
            Hotkey {
                modifiers: chord.modifiers,
                key: chord.key,
                hold: None,
            }
        };
        let (tx, rx) = mpsc::channel();
        let sink = GestureSink::new(move |event| {
            let _ = tx.send(event);
        });
        let (listener, bound) = start(hotkey(launcher), timers.map(hotkey), sink).unwrap();
        bound.unwrap();
        (listener, rx)
    }

    #[test]
    fn a_chord_another_listener_holds_is_taken_until_it_lets_go() {
        let (first, _) = start_listener("Ctrl+Alt+Shift+Win+F13", None);
        let (second, _) = start_listener("Ctrl+Alt+Shift+Win+F14", None);
        let contested = [chord("Ctrl+Alt+Shift+Win+F15")];

        assert_eq!(first.set_command_hotkeys(&contested).unwrap(), [Ok(())]);
        assert_eq!(
            second.set_command_hotkeys(&contested).unwrap(),
            [Err(HotkeyError::Taken)]
        );

        // Replacing the set with nothing releases what it held.
        assert_eq!(first.set_command_hotkeys(&[]).unwrap(), []);
        assert_eq!(second.set_command_hotkeys(&contested).unwrap(), [Ok(())]);
        second.set_command_hotkeys(&[]).unwrap();
    }

    #[test]
    fn replacing_the_set_releases_the_chords_left_out() {
        let (listener, _) = start_listener("Ctrl+Alt+Shift+Win+F16", None);
        let kept = chord("Ctrl+Alt+Shift+Win+F17");
        let dropped = chord("Ctrl+Alt+Shift+Win+F18");

        let both = [kept.clone(), dropped.clone()];
        assert_eq!(
            listener.set_command_hotkeys(&both).unwrap(),
            [Ok(()), Ok(())]
        );
        // Re-applying the same set is not a conflict with itself.
        assert_eq!(
            listener.set_command_hotkeys(&both).unwrap(),
            [Ok(()), Ok(())]
        );
        // A duplicate inside one set is: the second copy is taken by the first.
        assert_eq!(
            listener
                .set_command_hotkeys(&[kept.clone(), kept.clone()])
                .unwrap(),
            [Ok(()), Err(HotkeyError::Taken)]
        );
        assert_eq!(listener.set_command_hotkeys(&[kept]).unwrap(), [Ok(())]);

        // The chord left out is free for anyone else to take.
        let (other, _) = start_listener("Ctrl+Alt+Shift+Win+F19", None);
        assert_eq!(other.set_command_hotkeys(&[dropped]).unwrap(), [Ok(())]);
        other.set_command_hotkeys(&[]).unwrap();
        listener.set_command_hotkeys(&[]).unwrap();
    }

    #[test]
    fn the_launcher_and_timers_hotkeys_are_taken_without_asking_the_system() {
        let (listener, _) =
            start_listener("Ctrl+Alt+Shift+Win+F20", Some("Ctrl+Alt+Shift+Win+F21"));
        assert_eq!(
            listener
                .set_command_hotkeys(&[
                    chord("ctrl+alt+shift+win+f20"),
                    chord("Ctrl+Alt+Shift+Win+F21"),
                    Chord {
                        modifiers: chord("Ctrl+K").modifiers,
                        key: "Nonsense".into(),
                    },
                ])
                .unwrap(),
            [
                Err(HotkeyError::Taken),
                Err(HotkeyError::Taken),
                Err(HotkeyError::Unsupported(
                    "\"Nonsense\" has no Windows virtual key".into()
                )),
            ]
        );
    }

    #[test]
    fn a_command_hotkey_is_reported_with_its_slot() {
        let (listener, events) = start_listener("Ctrl+Alt+Shift+Win+F22", None);
        // What the system posts when the chord in slot 3 is pressed.
        unsafe {
            PostMessageW(
                Some(HWND(listener.hwnd as *mut c_void)),
                WM_HOTKEY,
                WPARAM(COMMAND_HOTKEY_BASE + 3),
                LPARAM(0),
            )
            .unwrap();
        }
        assert_eq!(
            events.recv_timeout(Duration::from_secs(2)).unwrap(),
            GestureEvent::Command(3)
        );
    }

    /// Injected events, and some drivers, report the side-specific virtual key
    /// rather than the generic one plus an E0 prefix.
    #[test]
    fn accepts_side_specific_virtual_keys_too() {
        assert_eq!(
            modifier_from_raw(VK_RCONTROL, 0x1D, true),
            Some(Modifier::RightControl)
        );
        assert_eq!(
            modifier_from_raw(VK_LCONTROL, 0x1D, false),
            Some(Modifier::LeftControl)
        );
        assert_eq!(
            modifier_from_raw(VK_RMENU, 0x38, true),
            Some(Modifier::RightAlt)
        );
    }
}
