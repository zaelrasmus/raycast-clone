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
use std::time::Duration;

use anyhow::{Context as _, Result};
use lumen_platform::{GestureEvent, GestureSink, HoldPhase, Hotkey, Modifier};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HID_USAGE_GENERIC_KEYBOARD, HID_USAGE_PAGE_GENERIC,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    RIDEV_INPUTSINK, RIM_TYPEKEYBOARD, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, KillTimer, MSG,
    RegisterClassW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_HOTKEY, WM_INPUT,
    WM_TIMER, WNDCLASSW,
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
const HOLD_TIMER_ID: usize = 1;

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
}

thread_local! {
    static STATE: std::cell::RefCell<Option<ListenerState>> = const { std::cell::RefCell::new(None) };
}

/// Start the listener on a dedicated thread and return once it is registered.
///
/// The thread owns a message-only window for its whole life. It is never joined:
/// it lives exactly as long as the daemon does.
pub fn start(hotkey: Hotkey, sink: GestureSink) -> Result<()> {
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<()>>(1);

    std::thread::Builder::new()
        .name("lumen-gestures".into())
        .spawn(move || {
            match run(hotkey, sink) {
                // The listener is up either way. `accelerator` reports only
                // whether the launcher hotkey could be bound, and the pump runs
                // regardless so the hold gesture keeps working.
                Ok((pump, accelerator)) => {
                    let _ = ready_tx.send(accelerator);
                    pump();
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            }
        })
        .context("spawning the gesture listener thread")?;

    ready_rx
        .recv()
        .context("the gesture listener thread stopped before reporting readiness")?
}

/// Register everything, then hand back the message pump plus the hotkey outcome.
///
/// Split this way so registration failures surface to the caller instead of
/// disappearing into a thread that simply never delivers events -- and so that a
/// hotkey that is already taken is reported without tearing down a listener that
/// is otherwise working.
fn run(hotkey: Hotkey, sink: GestureSink) -> Result<(impl FnOnce(), Result<()>)> {
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

        Ok((pump_messages, accelerator))
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
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    if hotkey.modifiers.alt {
        modifiers |= MOD_ALT;
    }
    if hotkey.modifiers.control {
        modifiers |= MOD_CONTROL;
    }
    if hotkey.modifiers.shift {
        modifiers |= MOD_SHIFT;
    }
    if hotkey.modifiers.super_key {
        modifiers |= MOD_WIN;
    }

    let vk = virtual_key(&hotkey.key)
        .with_context(|| format!("{:?} is not a key this build can bind", hotkey.key))?;

    Ok((modifiers, vk))
}

/// Names accepted in the `key` field of the configured hotkey.
///
/// Kept small on purpose. Section 7 notes that translating between scancodes and
/// virtual keys is the tedious, easy-to-get-wrong part; the right long-term move
/// is the `keyboard-types` vocabulary, which is a dependency worth taking when
/// the Linux port needs the same table rather than now, for one key.
fn virtual_key(name: &str) -> Option<u32> {
    let key = name.trim();
    if let Some(character) = key.chars().next()
        && key.chars().count() == 1
    {
        let upper = character.to_ascii_uppercase();
        if upper.is_ascii_alphanumeric() {
            return Some(upper as u32);
        }
    }

    Some(match key.to_ascii_lowercase().as_str() {
        "space" => 0x20,
        "enter" | "return" => 0x0D,
        "escape" | "esc" => 0x1B,
        "tab" => 0x09,
        "backtick" | "grave" => 0xC0,
        "f1" => 0x70,
        "f2" => 0x71,
        "f3" => 0x72,
        "f4" => 0x73,
        "f5" => 0x74,
        "f6" => 0x75,
        "f7" => 0x76,
        "f8" => 0x77,
        "f9" => 0x78,
        "f10" => 0x79,
        "f11" => 0x7A,
        "f12" => 0x7B,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_platform::Modifiers;

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
