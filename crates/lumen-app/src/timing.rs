//! Measuring the budgets that only the running daemon can measure (I-8).
//!
//! Two of section 5's six budgets are checked in CI: ranking cost, by a test
//! with a threshold, and installed size, by comparing bytes. The other four
//! cannot be, honestly. Gesture-to-first-frame needs a compositor presenting to
//! a real display; idle CPU and idle memory need a desktop session that is
//! actually idle; cold start needs a GPU. A hosted runner has none of those,
//! and a number measured on a virtual display under an arbitrary load is worse
//! than no number, because it looks like evidence.
//!
//! So they are measured here instead, by the daemon, on the machine the user is
//! actually running. They are logged at `info`, so they appear in the ordinary
//! log rather than behind a flag; the README records what they were when last
//! taken, and on what hardware.
//!
//! What is measured, and what each measurement excludes:
//!
//! - **Cold start to usable.** From the process entering `main` to the daemon
//!   being able to answer a summons: window created, keymap installed, message
//!   loop running. Deliberately from `main` rather than from the window
//!   opening, because the budget is about how long the user waits, and config
//!   parsing and GPU initialisation are part of that wait. It does *not* wait
//!   for the application index -- by I-3 root search never does either, and the
//!   launcher is usable with an empty list.
//! - **Gesture to first frame.** From the message arriving -- which is what the
//!   hotkey thread and the socket both produce -- to the first frame painted
//!   after the window is shown. It excludes the few hundred microseconds
//!   between the physical keypress and `WM_HOTKEY`, which nothing in this
//!   process can see, and the compositor's present after the frame is drawn,
//!   which no API here reports. Both exclusions are stated rather than glossed.

use std::sync::OnceLock;
use std::time::Instant;

use gpui::Window;

/// When the process started.
///
/// Set on the first line of `main`, before anything else, so that everything
/// the daemon does before it can answer is inside the measurement.
static STARTED: OnceLock<Instant> = OnceLock::new();

pub fn mark_process_start() {
    let _ = STARTED.set(Instant::now());
}

/// Report how long the daemon took to become summonable.
pub fn report_ready() {
    if let Some(started) = STARTED.get() {
        log::info!(
            "cold start to usable: {:.0} ms (budget: 400 ms)",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Report on the next frame this window paints.
///
/// Called from the show path, with `since` taken when the message arrived.
/// `on_next_frame` fires after the frame is drawn, which is as close to
/// "visible" as anything inside the process can get.
pub fn report_first_frame(window: &Window, since: Instant) {
    window.on_next_frame(move |_window, _cx| {
        log::info!(
            "gesture to first frame: {:.0} ms (budget: p50 40 ms, p95 80 ms)",
            since.elapsed().as_secs_f64() * 1000.0
        );
    });
}
