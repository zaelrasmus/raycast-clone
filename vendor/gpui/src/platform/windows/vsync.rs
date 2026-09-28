use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use util::ResultExt;
use windows::Win32::{
    Foundation::{HANDLE, HWND, LPARAM, WPARAM},
    Graphics::Dwm::{DWM_TIMING_INFO, DwmFlush, DwmGetCompositionTimingInfo},
    System::Performance::QueryPerformanceFrequency,
    // LUMEN PATCH: parking the thread while every window is hidden.
    System::Threading::{CreateEventW, SetEvent, WaitForSingleObject},
    UI::WindowsAndMessaging::{SWP_SHOWWINDOW, WINDOWPOS, WM_SHOWWINDOW, WM_WINDOWPOSCHANGED},
};

static QPC_TICKS_PER_SECOND: LazyLock<u64> = LazyLock::new(|| {
    let mut frequency = 0;
    // On systems that run Windows XP or later, the function will always succeed and
    // will thus never return zero.
    unsafe { QueryPerformanceFrequency(&mut frequency).unwrap() };
    frequency as u64
});

const VSYNC_INTERVAL_THRESHOLD: Duration = Duration::from_millis(1);
const DEFAULT_VSYNC_INTERVAL: Duration = Duration::from_micros(16_666); // ~60Hz

pub(crate) struct VSyncProvider {
    interval: Duration,
    f: Box<dyn Fn() -> bool>,
}

impl VSyncProvider {
    pub(crate) fn new() -> Self {
        let interval = get_dwm_interval()
            .context("Failed to get DWM interval")
            .log_err()
            .unwrap_or(DEFAULT_VSYNC_INTERVAL);
        let f = Box::new(|| unsafe { DwmFlush().is_ok() });
        Self { interval, f }
    }

    pub(crate) fn wait_for_vsync(&self) {
        let vsync_start = Instant::now();
        let wait_succeeded = (self.f)();
        let elapsed = vsync_start.elapsed();
        // DwmFlush and DCompositionWaitForCompositorClock returns very early
        // instead of waiting until vblank when the monitor goes to sleep or is
        // unplugged (nothing to present due to desktop occlusion). We use 1ms as
        // a threshold for the duration of the wait functions and fallback to
        // Sleep() if it returns before that. This could happen during normal
        // operation for the first call after the vsync thread becomes non-idle,
        // but it shouldn't happen often.
        if !wait_succeeded || elapsed < VSYNC_INTERVAL_THRESHOLD {
            log::trace!("VSyncProvider::wait_for_vsync() took less time than expected");
            std::thread::sleep(self.interval);
        }
    }
}

fn get_dwm_interval() -> Result<Duration> {
    let mut timing_info = DWM_TIMING_INFO {
        cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut timing_info) }?;
    let interval = retrieve_duration(timing_info.qpcRefreshPeriod, *QPC_TICKS_PER_SECOND);
    // Check for interval values that are impossibly low. A 29 microsecond
    // interval was seen (from a qpcRefreshPeriod of 60).
    if interval < VSYNC_INTERVAL_THRESHOLD {
        Ok(retrieve_duration(
            timing_info.rateRefresh.uiDenominator as u64,
            timing_info.rateRefresh.uiNumerator as u64,
        ))
    } else {
        Ok(interval)
    }
}

#[inline]
fn retrieve_duration(counts: u64, ticks_per_second: u64) -> Duration {
    let ticks_per_microsecond = ticks_per_second / 1_000_000;
    Duration::from_micros(counts / ticks_per_microsecond)
}

// LUMEN PATCH (docs/ARCHITECTURE.md, 7.35): park the vsync thread while no
// window is visible.
//
// Upstream, the thread calls `DwmFlush` at the refresh rate for the life of the
// process and invalidates every window, visible or not. For an application
// that stays open that is the price of a frame clock. For a resident daemon
// that is hidden nearly all of the time it is most of its idle CPU: 60 to 144
// round trips into dwm.exe a second, to redraw nothing.
//
// So when no window is visible the thread blocks on an event instead, and the
// window procedure sets it as soon as any window is about to be shown. The
// first frame of a shown window never depended on this thread -- showing a
// window paints it -- only the frames after it do, and by then the thread is
// awake.

/// How long a parked thread waits before looking again without being woken.
///
/// A safety net, not the mechanism: every way of showing a window sends a
/// message that wakes the thread. If one were ever missed, a visible window
/// would stop animating for at most this long instead of for good.
const PARKED_RECHECK_MS: u32 = 1000;

struct WakeEvent(HANDLE);

// An event handle is a kernel object: setting and waiting on it from any thread
// is what it is for.
unsafe impl Send for WakeEvent {}
unsafe impl Sync for WakeEvent {}

/// Auto-reset, so one wake is consumed by one wait. `None` if the event could
/// not be created, in which case the thread never parks: upstream behaviour.
static WAKE: LazyLock<Option<WakeEvent>> = LazyLock::new(|| {
    unsafe { CreateEventW(None, false, false, None) }
        .context("creating the vsync wake event")
        .log_err()
        .map(WakeEvent)
});

/// Whether a window message means the window is becoming visible.
///
/// `WM_WINDOWPOSCHANGED` with `SWP_SHOWWINDOW` covers every path, `ShowWindow`
/// and `SetWindowPos` alike; `WM_SHOWWINDOW` is not sent by `SetWindowPos`, so
/// it is only a second chance.
pub(crate) fn is_becoming_visible(msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    match msg {
        WM_SHOWWINDOW => wparam.0 != 0,
        WM_WINDOWPOSCHANGED => {
            let position = unsafe { &*(lparam.0 as *const WINDOWPOS) };
            position.flags.contains(SWP_SHOWWINDOW)
        }
        _ => false,
    }
}

/// Wake the vsync thread if it is parked. Cheap and non-blocking.
pub(crate) fn wake_vsync_thread() {
    if let Some(event) = WAKE.as_ref() {
        unsafe { SetEvent(event.0) }.log_err();
    }
}

/// Block until a window is shown, or the recheck interval passes.
///
/// Returns `false` without waiting if the thread cannot be parked. Showing a
/// window between the caller's visibility check and this wait is not lost:
/// the event stays set and the wait returns at once.
///
/// The caller must not hold the window list's lock while calling this, or
/// creating a window would block for as long as the thread is parked.
pub(crate) fn park_until_shown() -> bool {
    let Some(event) = WAKE.as_ref() else {
        return false;
    };
    unsafe { WaitForSingleObject(event.0, PARKED_RECHECK_MS) };
    true
}
