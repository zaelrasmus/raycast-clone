//! Spike for "Paste and Keep Window Open": can a process that owns a visible,
//! topmost, foreground window hand the foreground to another program's
//! window, have `Ctrl+V` land there, and take the foreground back, with its
//! own window never hidden?
//!
//! Measurement only. Plain Win32, no GPUI. The target is a classic EDIT
//! control in a second process (this same executable, `--target`), so the
//! foreground moves between processes exactly as it would with a real target,
//! and delivery is observed as the control's text growing by the two UTF-16
//! units of the emoji.
//!
//! The launcher stand-in gets the foreground the way Lumen does: a
//! `RegisterHotKey` chord, pressed here with `SendInput`, and
//! `SetForegroundWindow` from the `WM_HOTKEY` handler.
//!
//! ```text
//! cargo run --release -p lumen-platform-windows --example paste_round_trip [trials]
//! ```
//!
//! Do not type or click while it runs: real input changes who may take the
//! foreground.

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    spike::main();
}

#[cfg(windows)]
mod spike {
    use std::cell::Cell;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, GetOpenClipboardWindow,
        IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
        MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, SendInput, SetFocus, UnregisterHotKey,
        VIRTUAL_KEY, VK_CONTROL, VK_F24,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, ES_AUTOVSCROLL,
        ES_MULTILINE, GUITHREADINFO, GWL_EXSTYLE, GetForegroundWindow, GetGUIThreadInfo,
        GetMessageW, GetWindowLongPtrW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        LSFW_LOCK, LSFW_UNLOCK, LockSetForegroundWindow, MSG, MsgWaitForMultipleObjects, PM_REMOVE,
        PeekMessageW, PostMessageW, PostQuitMessage, QS_ALLINPUT, RegisterClassW, SMTO_ABORTIFHUNG,
        SPI_GETFOREGROUNDLOCKTIMEOUT, SW_HIDE, SW_SHOWNOACTIVATE,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SendMessageTimeoutW, SetForegroundWindow, ShowWindow,
        SystemParametersInfoW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE,
        WM_APP, WM_CLOSE, WM_DESTROY, WM_GETTEXTLENGTH, WM_HOTKEY, WM_SETFOCUS, WM_SETTEXT,
        WNDCLASSW, WS_BORDER, WS_CHILD, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW,
        WS_POPUP, WS_VISIBLE,
    };
    use windows::core::{PCWSTR, w};

    const HOTKEY_ID: i32 = 0x5EED;
    const STEP_TIMEOUT: Duration = Duration::from_millis(1000);
    /// `wparam` 1 turns the target's `LockSetForegroundWindow` on activation
    /// on, 0 turns it off and unlocks.
    const WM_SET_LOCK: u32 = WM_APP + 1;
    const VK_V: VIRTUAL_KEY = VIRTUAL_KEY(0x56);
    const EMOJI: &str = "\u{1F600}";

    thread_local! {
        static EDIT: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
        static LOCK_MODE: Cell<bool> = const { Cell::new(false) };
        static LOCK_RESULT: Cell<Option<bool>> = const { Cell::new(None) };
        static HOTKEY_SEEN: Cell<u32> = const { Cell::new(0) };
        static HOTKEY_SET_OK: Cell<bool> = const { Cell::new(false) };
    }

    pub fn main() {
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == "--target") {
            target_main();
            return;
        }
        let trials = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(50usize);
        unsafe { timeBeginPeriod(1) };
        launcher_main(trials);
        unsafe { timeEndPeriod(1) };
    }

    // ---------------------------------------------------------------- target

    fn target_main() {
        unsafe {
            let instance = GetModuleHandleW(None).unwrap();
            let class = WNDCLASSW {
                lpfnWndProc: Some(target_proc),
                hInstance: instance.into(),
                lpszClassName: w!("LumenSpikeTarget"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("LumenSpikeTarget"),
                w!("Lumen spike target"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                80,
                80,
                900,
                500,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap();
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL) as u32),
                0,
                0,
                880,
                460,
                Some(hwnd),
                None,
                Some(instance.into()),
                None,
            )
            .unwrap();
            EDIT.with(|e| e.set(edit));
            println!("{} {}", hwnd.0 as isize, edit.0 as isize);
            std::io::stdout().flush().ok();

            // If the parent goes away, so does this.
            let raw = hwnd.0 as isize;
            std::thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = std::io::stdin().read_to_end(&mut sink);
                let _ = PostMessageW(Some(HWND(raw as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            });

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = LockSetForegroundWindow(LSFW_UNLOCK);
        }
    }

    extern "system" fn target_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_SETFOCUS => {
                    let _ = SetFocus(Some(EDIT.with(|e| e.get())));
                    LRESULT(0)
                }
                WM_ACTIVATE => {
                    if (wp.0 & 0xFFFF) != 0 && LOCK_MODE.with(|m| m.get()) {
                        let ok = LockSetForegroundWindow(LSFW_LOCK).is_ok();
                        if LOCK_RESULT.with(|r| r.replace(Some(ok))).is_none() {
                            eprintln!("target: LockSetForegroundWindow(LSFW_LOCK) ok={ok}");
                        }
                    }
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
                WM_SET_LOCK => {
                    LOCK_MODE.with(|m| m.set(wp.0 == 1));
                    if wp.0 != 1 {
                        let _ = LockSetForegroundWindow(LSFW_UNLOCK);
                    }
                    LRESULT(0)
                }
                WM_CLOSE => {
                    let _ = LockSetForegroundWindow(LSFW_UNLOCK);
                    let _ = DestroyWindow(hwnd);
                    LRESULT(0)
                }
                WM_DESTROY => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
    }

    // -------------------------------------------------------------- launcher

    extern "system" fn launcher_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_HOTKEY if wp.0 as i32 == HOTKEY_ID => {
                    // What Lumen does on a summon: take the foreground now,
                    // while the hotkey's permission holds.
                    let ok = SetForegroundWindow(hwnd).as_bool();
                    HOTKEY_SEEN.with(|c| c.set(c.get() + 1));
                    HOTKEY_SET_OK.with(|c| c.set(ok));
                    LRESULT(0)
                }
                WM_SETFOCUS => {
                    let _ = SetFocus(Some(EDIT.with(|e| e.get())));
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Method {
        Plain,
        Attach,
    }

    #[derive(Clone, Copy)]
    struct Variant {
        name: &'static str,
        give: Method,
        back: Method,
        /// A keystroke into the launcher before the trial, standing in for
        /// the user's `Ctrl+Shift+Enter`.
        user_key: bool,
        /// Send `Ctrl+V` at all. Without it, no input is injected between
        /// giving the foreground away and taking it back.
        paste: bool,
        /// The target calls `LockSetForegroundWindow(LSFW_LOCK)` when it
        /// becomes active.
        target_locks: bool,
        /// The fallback: hide, paste, show without activating, activate.
        hide: bool,
        /// Before `Ctrl+V`, wait until the target's thread has its edit
        /// control focused, not only its window in front.
        wait_focus: bool,
        /// Before `Ctrl+V`, wait until nobody has the clipboard open, and
        /// has not for this many milliseconds.
        settle_ms: u64,
    }

    #[derive(Default)]
    struct Trial {
        started_fg: bool,
        give: Option<f64>,
        deliver: Option<f64>,
        back: Option<f64>,
        never_hidden: bool,
        max_hidden_ms: f64,
        topmost_kept: bool,
        recovered: bool,
        /// Whether the target's edit control had the focus when `Ctrl+V` was
        /// sent.
        focus_at_send: bool,
        /// A paste not seen within the timeout that arrived later.
        late: bool,
        /// Another program (not this one, not the target) seen holding the
        /// clipboard open between `Ctrl+V` and the paste.
        other_reader: Option<u32>,
    }

    struct Monitor {
        hwnd: AtomicIsize,
        active: AtomicBool,
        hidden_samples: AtomicU64,
        max_hidden_us: AtomicU64,
        stop: AtomicBool,
    }

    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            // Closing stdin makes the target close its window and exit.
            drop(self.0.stdin.take());
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if let Ok(Some(_)) = self.0.try_wait() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn launcher_main(trials: usize) {
        let saved_clipboard = read_clipboard_text();

        let exe = std::env::current_exe().unwrap();
        let mut child = Command::new(exe)
            .arg("--target")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("starting the target");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let guard = ChildGuard(child);
        let mut parts = line.split_whitespace().map(|p| p.parse::<isize>().unwrap());
        let target = HWND(parts.next().unwrap() as *mut _);
        let target_edit = HWND(parts.next().unwrap() as *mut _);
        let target_thread = unsafe { GetWindowThreadProcessId(target, None) };

        let launcher = unsafe { create_launcher() };
        let mut lock_timeout = 0u32;
        unsafe {
            let _ = SystemParametersInfoW(
                SPI_GETFOREGROUNDLOCKTIMEOUT,
                0,
                Some(&mut lock_timeout as *mut u32 as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
        }
        println!("ForegroundLockTimeout: {lock_timeout} ms");

        let monitor = Arc::new(Monitor {
            hwnd: AtomicIsize::new(launcher.0 as isize),
            active: AtomicBool::new(false),
            hidden_samples: AtomicU64::new(0),
            max_hidden_us: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let watcher = {
            let monitor = monitor.clone();
            std::thread::spawn(move || watch(&monitor))
        };

        // How the foreground was first obtained.
        pump_for(Duration::from_millis(300));
        let before = unsafe { GetForegroundWindow() };
        let unprompted = unsafe { SetForegroundWindow(launcher) }.as_bool();
        pump_for(Duration::from_millis(50));
        let unprompted_fg = unsafe { GetForegroundWindow() } == launcher;
        println!(
            "initial: foreground was {}; SetForegroundWindow with no rights returned {unprompted}, \
             launcher foreground: {unprompted_fg}",
            describe(before, launcher, target)
        );
        // Give it away so the summon below is a real one.
        if unprompted_fg {
            let _ = unsafe { SetForegroundWindow(target) };
            pump_for(Duration::from_millis(100));
        }
        let summoned = summon(launcher);
        println!(
            "summon via RegisterHotKey(Ctrl+F24) + SendInput: WM_HOTKEY seen {}, \
             SetForegroundWindow returned {}, launcher foreground: {summoned}",
            HOTKEY_SEEN.with(|c| c.get()),
            HOTKEY_SET_OK.with(|c| c.get())
        );

        let variants = [
            Variant {
                name: "give plain / back plain",
                give: Method::Plain,
                back: Method::Plain,
                user_key: true,
                paste: true,
                target_locks: false,
                hide: false,
                wait_focus: false,
                settle_ms: 0,
            },
            Variant {
                name: "give attach / back attach",
                give: Method::Attach,
                back: Method::Attach,
                user_key: true,
                paste: true,
                target_locks: false,
                hide: false,
                wait_focus: false,
                settle_ms: 0,
            },
            Variant {
                name: "give plain / back attach",
                give: Method::Plain,
                back: Method::Attach,
                user_key: true,
                paste: true,
                target_locks: false,
                hide: false,
                wait_focus: false,
                settle_ms: 0,
            },
            Variant {
                name: "plain/plain, no user key",
                give: Method::Plain,
                back: Method::Plain,
                user_key: false,
                paste: true,
                target_locks: false,
                hide: false,
                wait_focus: false,
                settle_ms: 0,
            },
            Variant {
                name: "plain/plain, no Ctrl+V",
                give: Method::Plain,
                back: Method::Plain,
                user_key: false,
                paste: false,
                target_locks: false,
                hide: false,
                wait_focus: false,
                settle_ms: 0,
            },
            Variant {
                name: "target locks, back plain",
                give: Method::Plain,
                back: Method::Plain,
                user_key: true,
                paste: true,
                target_locks: true,
                hide: false,
                wait_focus: false,
                settle_ms: 0,
            },
            Variant {
                name: "target locks, back attach",
                give: Method::Plain,
                back: Method::Attach,
                user_key: true,
                paste: true,
                target_locks: true,
                hide: false,
                wait_focus: false,
                settle_ms: 0,
            },
            Variant {
                name: "plain/plain, wait focus",
                give: Method::Plain,
                back: Method::Plain,
                user_key: true,
                paste: true,
                target_locks: false,
                hide: false,
                wait_focus: true,
                settle_ms: 0,
            },
            Variant {
                name: "plain/plain, clipboard settle",
                give: Method::Plain,
                back: Method::Plain,
                user_key: true,
                paste: true,
                target_locks: false,
                hide: false,
                wait_focus: true,
                settle_ms: 30,
            },
            Variant {
                name: "fallback hide/show",
                give: Method::Plain,
                back: Method::Plain,
                user_key: true,
                paste: true,
                target_locks: false,
                hide: true,
                wait_focus: false,
                settle_ms: 0,
            },
        ];

        println!(
            "\n{:<28} {:>5} {:>5} {:>5} {:>5} | {:>13} {:>13} {:>13} | {:>6} {:>8} {:>5} {:>5}",
            "variant",
            "give",
            "paste",
            "back",
            "all",
            "give p50/p95",
            "deliv p50/p95",
            "back p50/p95",
            "hidden",
            "maxgap",
            "top",
            "recov"
        );
        for variant in variants {
            unsafe {
                let _ = PostMessageW(
                    Some(target),
                    WM_SET_LOCK,
                    WPARAM(variant.target_locks as usize),
                    LPARAM(0),
                );
            }
            set_text(target_edit, "");
            pump_for(Duration::from_millis(50));
            let mut results = Vec::with_capacity(trials);
            for _ in 0..trials {
                results.push(trial(
                    variant,
                    launcher,
                    target,
                    target_edit,
                    target_thread,
                    &monitor,
                ));
                pump_for(Duration::from_millis(30));
            }
            unsafe {
                let _ = PostMessageW(Some(target), WM_SET_LOCK, WPARAM(0), LPARAM(0));
            }
            report(variant, &results);
        }

        monitor.stop.store(true, Ordering::SeqCst);
        let _ = watcher.join();
        unsafe {
            let _ = UnregisterHotKey(Some(launcher), HOTKEY_ID);
            let _ = DestroyWindow(launcher);
        }
        drop(guard);
        if let Some(text) = saved_clipboard {
            set_clipboard(HWND(std::ptr::null_mut()), &text, false);
        }
    }

    fn trial(
        v: Variant,
        launcher: HWND,
        target: HWND,
        target_edit: HWND,
        target_thread: u32,
        monitor: &Monitor,
    ) -> Trial {
        let me = unsafe { GetCurrentThreadId() };
        let mut t = Trial::default();
        if unsafe { GetForegroundWindow() } != launcher {
            t.recovered = true;
            summon(launcher);
        }
        t.started_fg = unsafe { GetForegroundWindow() } == launcher;

        if v.user_key {
            send_keys(&[(VK_F24, false), (VK_F24, true)]);
            pump_for(Duration::from_millis(5));
        }

        monitor.hidden_samples.store(0, Ordering::SeqCst);
        monitor.max_hidden_us.store(0, Ordering::SeqCst);
        monitor.active.store(true, Ordering::SeqCst);

        if v.hide {
            unsafe {
                let _ = ShowWindow(launcher, SW_HIDE);
            }
        }

        // 1. Hand the foreground over.
        let started = Instant::now();
        unsafe {
            match v.give {
                Method::Plain => {
                    let _ = SetForegroundWindow(target);
                }
                Method::Attach => {
                    let attached = AttachThreadInput(me, target_thread, true).as_bool();
                    let _ = SetForegroundWindow(target);
                    if attached {
                        let _ = AttachThreadInput(me, target_thread, false);
                    }
                }
            }
        }
        t.give = wait_until(|| unsafe { GetForegroundWindow() } == target, started);

        // 2. Paste.
        if v.paste && t.give.is_some() {
            if v.wait_focus {
                let focused = wait_until(|| focus_of(target_thread) == target_edit, started);
                t.give = focused;
            }
            set_clipboard(launcher, EMOJI, true);
            if v.settle_ms > 0 {
                let mut free_since = Instant::now();
                let begun = Instant::now();
                while free_since.elapsed() < Duration::from_millis(v.settle_ms)
                    && begun.elapsed() < STEP_TIMEOUT
                {
                    if unsafe { GetOpenClipboardWindow() }.is_ok() {
                        free_since = Instant::now();
                    }
                    pump();
                    unsafe { MsgWaitForMultipleObjects(None, false, 1, QS_ALLINPUT) };
                }
            }
            let base = text_length(target_edit);
            t.focus_at_send = focus_of(target_thread) == target_edit;
            let started = Instant::now();
            send_keys(&[
                (VK_CONTROL, false),
                (VK_V, false),
                (VK_V, true),
                (VK_CONTROL, true),
            ]);
            let target_pid = process_of(target);
            let own_pid = std::process::id();
            let mut other = None;
            t.deliver = wait_until(
                || {
                    if let Ok(reader) = unsafe { GetOpenClipboardWindow() } {
                        let pid = process_of(reader);
                        if pid != target_pid && pid != own_pid {
                            other.get_or_insert(pid);
                        }
                    }
                    text_length(target_edit) >= base + 2
                },
                started,
            );
            t.other_reader = other;
            if t.deliver.is_none() {
                pump_for(Duration::from_millis(500));
                t.late = text_length(target_edit) >= base + 2;
                println!(
                    "  {}: paste not seen in {} ms; target edit focused at send: {},                      arrived later: {}, foreground now: {}, other clipboard reader: {}",
                    v.name,
                    STEP_TIMEOUT.as_millis(),
                    t.focus_at_send,
                    t.late,
                    describe(unsafe { GetForegroundWindow() }, launcher, target),
                    t.other_reader
                        .map_or("none seen".into(), |pid| format!("pid {pid}"))
                );
            }
        }

        // 3. Take it back.
        let started = Instant::now();
        unsafe {
            if v.hide {
                let _ = ShowWindow(launcher, SW_SHOWNOACTIVATE);
            }
            match v.back {
                Method::Plain => {
                    let _ = SetForegroundWindow(launcher);
                }
                Method::Attach => {
                    let front = GetForegroundWindow();
                    let front_thread = if front.0.is_null() {
                        0
                    } else {
                        GetWindowThreadProcessId(front, None)
                    };
                    let attached = front_thread != 0
                        && front_thread != me
                        && AttachThreadInput(me, front_thread, true).as_bool();
                    let _ = SetForegroundWindow(launcher);
                    let _ = SetFocus(Some(launcher));
                    if attached {
                        let _ = AttachThreadInput(me, front_thread, false);
                    }
                }
            }
        }
        t.back = wait_until(|| unsafe { GetForegroundWindow() } == launcher, started);
        if v.hide && t.back.is_none() {
            // Make sure it is at least visible for the next trial.
            unsafe {
                let _ = ShowWindow(launcher, SW_SHOWNOACTIVATE);
            }
        }
        // Let the watcher take one more sample.
        pump_for(Duration::from_millis(2));
        monitor.active.store(false, Ordering::SeqCst);

        t.never_hidden = monitor.hidden_samples.load(Ordering::SeqCst) == 0;
        t.max_hidden_ms = monitor.max_hidden_us.load(Ordering::SeqCst) as f64 / 1000.0;
        t.topmost_kept =
            unsafe { GetWindowLongPtrW(launcher, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST.0 != 0;
        t
    }

    /// Sample the launcher's visibility as fast as the timer allows.
    fn watch(m: &Monitor) {
        let mut hidden_since: Option<Instant> = None;
        while !m.stop.load(Ordering::SeqCst) {
            if m.active.load(Ordering::SeqCst) {
                let hwnd = HWND(m.hwnd.load(Ordering::SeqCst) as *mut _);
                let visible =
                    unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() };
                if visible {
                    hidden_since = None;
                } else {
                    m.hidden_samples.fetch_add(1, Ordering::SeqCst);
                    let since = *hidden_since.get_or_insert_with(Instant::now);
                    let us = since.elapsed().as_micros() as u64;
                    m.max_hidden_us.fetch_max(us, Ordering::SeqCst);
                }
            } else {
                hidden_since = None;
            }
            std::thread::sleep(Duration::from_micros(250));
        }
    }

    fn summon(launcher: HWND) -> bool {
        let seen = HOTKEY_SEEN.with(|c| c.get());
        send_keys(&[
            (VK_CONTROL, false),
            (VK_F24, false),
            (VK_F24, true),
            (VK_CONTROL, true),
        ]);
        let started = Instant::now();
        wait_until(|| HOTKEY_SEEN.with(|c| c.get()) > seen, started);
        wait_until(
            || unsafe { GetForegroundWindow() } == launcher,
            Instant::now(),
        )
        .is_some()
    }

    unsafe fn create_launcher() -> HWND {
        unsafe {
            let instance = GetModuleHandleW(None).unwrap();
            let class = WNDCLASSW {
                lpfnWndProc: Some(launcher_proc),
                hInstance: instance.into(),
                lpszClassName: w!("LumenSpikeLauncher"),
                ..Default::default()
            };
            RegisterClassW(&class);
            // The launcher's extended styles (window.rs): topmost tool window,
            // never WS_EX_NOACTIVATE.
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                w!("LumenSpikeLauncher"),
                w!("Lumen spike launcher"),
                WS_POPUP | WS_BORDER,
                300,
                250,
                640,
                120,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap();
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE,
                10,
                10,
                600,
                30,
                Some(hwnd),
                None,
                Some(instance.into()),
                None,
            )
            .unwrap();
            EDIT.with(|e| e.set(edit));
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            RegisterHotKey(
                Some(hwnd),
                HOTKEY_ID,
                MOD_CONTROL | MOD_NOREPEAT,
                VK_F24.0 as u32,
            )
            .expect("RegisterHotKey(Ctrl+F24)");
            hwnd
        }
    }

    // --------------------------------------------------------------- helpers

    fn pump() {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    fn pump_for(d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            pump();
            unsafe { MsgWaitForMultipleObjects(None, false, 1, QS_ALLINPUT) };
        }
        pump();
    }

    /// Milliseconds from `started` until `done`, pumping meanwhile, or `None`
    /// after [`STEP_TIMEOUT`].
    fn wait_until(mut done: impl FnMut() -> bool, started: Instant) -> Option<f64> {
        loop {
            pump();
            if done() {
                return Some(started.elapsed().as_secs_f64() * 1000.0);
            }
            if started.elapsed() > STEP_TIMEOUT {
                return None;
            }
            unsafe { MsgWaitForMultipleObjects(None, false, 1, QS_ALLINPUT) };
        }
    }

    fn send_keys(keys: &[(VIRTUAL_KEY, bool)]) {
        let inputs: Vec<INPUT> = keys
            .iter()
            .map(|(vk, up)| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: *vk,
                        dwFlags: if *up {
                            KEYEVENTF_KEYUP
                        } else {
                            KEYBD_EVENT_FLAGS(0)
                        },
                        ..Default::default()
                    },
                },
            })
            .collect();
        unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    }

    fn process_of(hwnd: HWND) -> u32 {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        pid
    }

    fn focus_of(thread: u32) -> HWND {
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetGUIThreadInfo(thread, &mut info) }.is_ok() {
            info.hwndFocus
        } else {
            HWND(std::ptr::null_mut())
        }
    }

    fn text_length(edit: HWND) -> usize {
        let mut result = 0usize;
        unsafe {
            SendMessageTimeoutW(
                edit,
                WM_GETTEXTLENGTH,
                WPARAM(0),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                200,
                Some(&mut result),
            )
        };
        result
    }

    fn set_text(edit: HWND, text: &str) {
        let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
        unsafe {
            SendMessageTimeoutW(
                edit,
                WM_SETTEXT,
                WPARAM(0),
                LPARAM(wide.as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                500,
                None,
            )
        };
    }

    fn open_clipboard(owner: HWND) -> bool {
        let owner = (!owner.0.is_null()).then_some(owner);
        for _ in 0..100 {
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    fn global(bytes: &[u8]) -> Option<HGLOBAL> {
        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).ok()?;
            let ptr = GlobalLock(handle) as *mut u8;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            let _ = GlobalUnlock(handle);
            Some(handle)
        }
    }

    /// Put `text` on the clipboard; `private` keeps it out of clipboard
    /// histories (Lumen's included), as the real paste does.
    fn set_clipboard(owner: HWND, text: &str, private: bool) {
        if !open_clipboard(owner) {
            eprintln!("could not open the clipboard");
            return;
        }
        unsafe {
            let _ = EmptyClipboard();
            let bytes: Vec<u8> = text
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect();
            if let Some(h) = global(&bytes) {
                let _ = SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(h.0)));
            }
            if private {
                let exclude =
                    RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
                if let Some(h) = global(&[0]) {
                    let _ = SetClipboardData(exclude, Some(HANDLE(h.0)));
                }
                let history = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
                if let Some(h) = global(&0u32.to_le_bytes()) {
                    let _ = SetClipboardData(history, Some(HANDLE(h.0)));
                }
            }
            let _ = CloseClipboard();
        }
    }

    fn read_clipboard_text() -> Option<String> {
        unsafe {
            IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)).ok()?;
            if !open_clipboard(HWND(std::ptr::null_mut())) {
                return None;
            }
            let text = GetClipboardData(u32::from(CF_UNICODETEXT.0))
                .ok()
                .and_then(|h| {
                    let ptr = GlobalLock(HGLOBAL(h.0)) as *const u16;
                    if ptr.is_null() {
                        return None;
                    }
                    let mut len = 0;
                    while *ptr.add(len) != 0 {
                        len += 1;
                    }
                    let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                    let _ = GlobalUnlock(HGLOBAL(h.0));
                    Some(s)
                });
            let _ = CloseClipboard();
            text
        }
    }

    fn describe(hwnd: HWND, launcher: HWND, target: HWND) -> &'static str {
        if hwnd == launcher {
            "the launcher"
        } else if hwnd == target {
            "the target"
        } else if hwnd.0.is_null() {
            "nothing"
        } else {
            "another program"
        }
    }

    fn percentiles(values: &mut [f64]) -> String {
        if values.is_empty() {
            return "-".into();
        }
        values.sort_by(|a, b| a.total_cmp(b));
        let at =
            |q: f64| values[((q * values.len() as f64).ceil() as usize).clamp(1, values.len()) - 1];
        format!("{:.1}/{:.1}", at(0.5), at(0.95))
    }

    fn report(v: Variant, results: &[Trial]) {
        let n = results.len();
        let count = |f: &dyn Fn(&Trial) -> bool| results.iter().filter(|t| f(t)).count();
        let give_ok = count(&|t| t.give.is_some());
        let paste_ok = count(&|t| t.deliver.is_some());
        let back_ok = count(&|t| t.back.is_some());
        let all_ok = count(&|t| {
            t.started_fg
                && t.give.is_some()
                && (!v.paste || t.deliver.is_some())
                && t.back.is_some()
                && (v.hide || t.never_hidden)
        });
        let mut give: Vec<f64> = results.iter().filter_map(|t| t.give).collect();
        let mut deliver: Vec<f64> = results.iter().filter_map(|t| t.deliver).collect();
        let mut back: Vec<f64> = results.iter().filter_map(|t| t.back).collect();
        let hidden = count(&|t| !t.never_hidden);
        let max_gap = results
            .iter()
            .map(|t| t.max_hidden_ms)
            .fold(0.0f64, f64::max);
        let topmost = count(&|t| t.topmost_kept);
        let recovered = count(&|t| t.recovered);
        let unfocused = count(&|t| v.paste && t.give.is_some() && !t.focus_at_send);
        let late = count(&|t| t.late);
        let others = count(&|t| t.other_reader.is_some());
        println!(
            "{:<28} {:>5} {:>5} {:>5} {:>5} | {:>13} {:>13} {:>13} | {:>6} {:>8.1} {:>5} {:>5}",
            v.name,
            format!("{give_ok}/{n}"),
            if v.paste {
                format!("{paste_ok}")
            } else {
                "-".into()
            },
            back_ok,
            all_ok,
            percentiles(&mut give),
            percentiles(&mut deliver),
            percentiles(&mut back),
            hidden,
            max_gap,
            topmost,
            recovered
        );
        if unfocused > 0 || late > 0 || others > 0 {
            println!(
                "{:<28} sent before the edit had focus: {unfocused}, pastes that arrived late:                  {late}, trials with another clipboard reader: {others}",
                ""
            );
        }
        if v.hide {
            let mut gaps: Vec<f64> = results.iter().map(|t| t.max_hidden_ms).collect();
            println!(
                "{:<28} hidden gap p50/p95 ms: {}",
                "",
                percentiles(&mut gaps)
            );
        }
    }
}
