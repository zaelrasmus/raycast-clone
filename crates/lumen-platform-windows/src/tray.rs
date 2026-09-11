//! The notification area icon on Windows.
//!
//! `Shell_NotifyIconW` against a message-only window, on its own thread with
//! its own message loop — the same shape as the input listener and the
//! clipboard watcher, and for the same reason: a blocking message pump must not
//! be the UI thread's.
//!
//! # Three details that are easy to get wrong and very visible when you do
//!
//! **Explorer restarts.** When it does, the notification area is rebuilt from
//! nothing and every icon in it disappears — permanently, unless the
//! application adds its own back. Explorer broadcasts `TaskbarCreated` to say
//! so. Without handling it, Lumen silently loses its only visible trace the
//! first time Explorer crashes, which on a normal machine is a few times a
//! year.
//!
//! **The menu needs the foreground.** `TrackPopupMenu` on a window that is not
//! foreground leaves the menu on screen after the user clicks elsewhere, with
//! no way to dismiss it. The documented fix is as odd as it sounds: take the
//! foreground before, and post a null message after.
//!
//! **The icon must be the small one.** Asking for the default size gives a
//! 32 px icon that Windows squeezes into a 16 px slot, and the result is
//! noticeably softer than every icon beside it. `ExtractIconExW` hands back a
//! properly rendered small icon from the same file.

use std::os::windows::ffi::OsStrExt;

use anyhow::{Context as _, Result};
use lumen_platform::{TrayCommand, TrayInfo, TraySink};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    GetCursorPos, GetMessageW, HICON, HWND_MESSAGE, IDI_APPLICATION, LoadIconW, MF_SEPARATOR,
    MF_STRING, MSG, PostMessageW, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
    TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

/// The message the shell sends us when the icon is clicked.
///
/// `WM_APP` and above are reserved for the application, so this cannot collide
/// with anything Windows itself defines.
const WM_TRAY: u32 = WM_APP + 1;

/// Our icon's id within this window. One icon, so any value does.
const ICON_ID: u32 = 1;

/// Menu command identifiers. Zero is what `TPM_RETURNCMD` returns when the
/// menu is dismissed without a choice, so nothing may use it.
const CMD_OPEN: usize = 1;
const CMD_SETTINGS: usize = 2;
const CMD_RELOAD: usize = 3;
const CMD_QUIT: usize = 4;

struct TrayState {
    sink: TraySink,
    icon: HICON,
    tooltip: Vec<u16>,
    /// Explorer's "I have just restarted" broadcast, registered once.
    taskbar_created: u32,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<TrayState>> = const { std::cell::RefCell::new(None) };
}

/// Add the icon and start serving it.
pub fn start(info: TrayInfo, sink: TraySink) -> Result<()> {
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<()>>(1);

    std::thread::Builder::new()
        .name("lumen-tray".into())
        .spawn(move || match run(info, sink) {
            Ok(pump) => {
                let _ = ready_tx.send(Ok(()));
                pump();
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error));
            }
        })
        .context("spawning the tray thread")?;

    ready_rx
        .recv()
        .context("the tray thread stopped before reporting readiness")?
}

fn run(info: TrayInfo, sink: TraySink) -> Result<impl FnOnce()> {
    unsafe {
        let instance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        let class_name = w!("LumenTray");

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
        .context("creating the tray window")?;

        // Truncated to what the field holds, including its terminator. A
        // tooltip that overruns is not shown at all.
        let mut tooltip: Vec<u16> = info.tooltip.encode_utf16().take(127).collect();
        tooltip.push(0);

        STATE.with(|state| {
            *state.borrow_mut() = Some(TrayState {
                sink,
                icon: small_icon(),
                tooltip,
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            });
        });

        add_icon(hwnd)?;

        Ok(move || pump_messages(hwnd))
    }
}

/// The application's own icon at the size the notification area wants.
///
/// Falls back rather than failing: an icon is how the user finds this, but a
/// generic one is still better than no tray entry at all.
fn small_icon() -> HICON {
    use windows::Win32::UI::Shell::ExtractIconExW;

    unsafe {
        if let Ok(path) = std::env::current_exe() {
            let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
            wide.push(0);

            let mut small = HICON::default();
            let extracted = ExtractIconExW(
                PCWSTR(wide.as_ptr()),
                0,
                None,
                Some(&mut small as *mut HICON),
                1,
            );

            if extracted > 0 && !small.is_invalid() {
                return small;
            }
        }

        LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
    }
}

fn notify_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        ..Default::default()
    };

    STATE.with(|state| {
        if let Some(state) = state.borrow().as_ref() {
            data.hIcon = state.icon;
            let len = state.tooltip.len().min(data.szTip.len());
            data.szTip[..len].copy_from_slice(&state.tooltip[..len]);
        }
    });

    data
}

fn add_icon(hwnd: HWND) -> Result<()> {
    let data = notify_data(hwnd);
    unsafe { Shell_NotifyIconW(NIM_ADD, &data) }
        .ok()
        .context("adding the notification area icon")
}

fn pump_messages(hwnd: HWND) {
    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        // Leaving without this is what produces the ghost icons that stay in
        // the notification area until you wave the mouse over them.
        let data = notify_data(hwnd);
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Explorer restarted and the notification area is empty again. Putting the
    // icon back is the application's job; nothing else will do it.
    let restarted = STATE.with(|state| {
        state
            .borrow()
            .as_ref()
            .is_some_and(|state| state.taskbar_created != 0 && message == state.taskbar_created)
    });

    if restarted {
        if let Err(error) = add_icon(hwnd) {
            log::warn!("could not restore the tray icon after Explorer restarted: {error:#}");
        }
        return LRESULT(0);
    }

    if message == WM_TRAY {
        match lparam.0 as u32 {
            WM_LBUTTONUP => emit(TrayCommand::Toggle),
            WM_RBUTTONUP => show_menu(hwnd),
            _ => {}
        }
        return LRESULT(0);
    }

    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn emit(command: TrayCommand) {
    STATE.with(|state| {
        if let Some(state) = state.borrow().as_ref() {
            state.sink.emit(command);
        }
    });
}

fn show_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };

        // The first item is what a left click already does, and saying so is
        // what stops the menu reading like the only way in.
        let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN, w!("Open Lumen"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_SETTINGS, w!("Configure Shortcuts…"));
        let _ = AppendMenuW(menu, MF_STRING, CMD_RELOAD, w!("Reload Configuration"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit Lumen"));

        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);

        // Both halves of the documented dance. Without the first the menu does
        // not dismiss when the user clicks away; without the second it can be
        // left behind on some shells.
        let _ = SetForegroundWindow(hwnd);

        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            cursor.x,
            cursor.y,
            Some(0),
            hwnd,
            None,
        );

        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        match chosen.0 as usize {
            CMD_OPEN => emit(TrayCommand::Toggle),
            CMD_SETTINGS => emit(TrayCommand::Settings),
            CMD_RELOAD => emit(TrayCommand::Reload),
            CMD_QUIT => emit(TrayCommand::Quit),
            // Zero: dismissed without choosing anything.
            _ => {}
        }
    }
}
