//! Running the timers (F8).
//!
//! # Where this sits
//!
//! Beside `translate.rs`, for the same reason: every decision -- what ends
//! when, what the view shows, what an action does -- is in
//! `lumen_core::timers`, where it is tested. This file carries them out. It
//! owns the one sleep, the notification, the file and the open view.
//!
//! # One sleep, not a tick
//!
//! The daemon sleeps until the nearest deadline and nothing else: no timer
//! wakes it to count, so a day of timers costs one wake-up each (section 5's
//! idle budget). Every change re-plans. A sleep from an older plan still
//! wakes, once, and does nothing, because its plan number is stale.
//!
//! A deadline is wall-clock time and the sleep is not, so a machine that slept
//! through a deadline may wake the daemon late. Every summons of the launcher
//! catches up first, so the view never shows a timer that has already ended.
//!
//! # The view redraws only while it is open
//!
//! When a second on one of its faces turns over, and not otherwise. With the
//! launcher hidden nothing redraws, whatever is running.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::AsyncApp;
use lumen_core::timers::{self, Clock, Command, Timer, duration, view};
use lumen_core::view::Event;
use lumen_platform::Platform;
use lumen_ui::{Launcher, ViewSink};

/// Drawn a little after the face turns over, never a little before: waking
/// early would redraw the same second and then wake again at once.
const SETTLE: Duration = Duration::from_millis(5);

/// What the open view's task hears, on one channel so it has one order.
enum Input {
    View(Event),
    /// The view is gone: its sink was dropped.
    Closed,
    /// A face may have turned over, if this is still the latest tick asked for.
    Tick(u64),
    /// The timers changed from somewhere else: a deadline, the command line.
    Refresh,
}

/// The open view, to tell it the timers changed. Tagged with the view's
/// number so a view closing late cannot unhook the one that replaced it.
type Viewer = (u64, mpsc::UnboundedSender<Input>);

/// The timers, shared by the event loop and the open view. Cheap to clone.
#[derive(Clone)]
pub struct Timers {
    clock: Rc<RefCell<Clock>>,
    path: PathBuf,
    platform: Arc<dyn Platform>,
    /// Bumped on every change; a sleep from an older plan does nothing.
    plan: Rc<Cell<u64>>,
    viewer: Rc<RefCell<Option<Viewer>>>,
    views: Rc<Cell<u64>>,
}

fn now() -> timers::Millis {
    timers::millis(SystemTime::now())
}

impl Timers {
    /// Read the timers left from the last run, report any that ended while
    /// nothing was running, and plan the next wake-up.
    pub fn start(platform: Arc<dyn Platform>, path: PathBuf, cx: &AsyncApp) -> Self {
        let clock = Clock::load(&path);
        if !clock.is_empty() {
            log::info!("{} timers carried over", clock.timers().len());
        }
        let timers = Self {
            clock: Rc::new(RefCell::new(clock)),
            path,
            platform,
            plan: Rc::default(),
            viewer: Rc::default(),
            views: Rc::default(),
        };
        timers.catch_up(cx);
        timers.plan(cx);
        timers
    }

    /// Carry out a command from a root row or `lumen timer`.
    pub fn command(&self, command: Command, cx: &AsyncApp) {
        let stopped = self.clock.borrow_mut().apply(&command, now());
        match (&command, stopped) {
            (Command::Start { seconds, name }, _) => log::info!(
                "started a {} timer{}",
                duration::describe(Duration::from_secs(*seconds)),
                if name.is_some() { ", named" } else { "" }
            ),
            (Command::StopNearest, Some(timer)) => {
                log::info!("stopped the nearest timer");
                self.say("Timer stopped", &timer.title());
            }
            // The launcher has closed, so this is the only place to say it.
            (Command::StopNearest, None) => {
                self.say("No timer is running", "There was nothing to stop.");
                return;
            }
        }
        self.changed(cx);
    }

    /// Report every timer that has ended. Called by the sleep, and by every
    /// summons in case the sleep woke late.
    pub fn catch_up(&self, cx: &AsyncApp) {
        let ended = self.clock.borrow_mut().expire(now());
        if ended.is_empty() {
            return;
        }
        for timer in &ended {
            self.ended(timer);
        }
        self.changed(cx);
    }

    fn ended(&self, timer: &Timer) {
        let body = match &timer.name {
            Some(name) => format!("{name} \u{00b7} {}", duration::describe(timer.duration())),
            None => timer.title(),
        };
        self.say("Time\u{2019}s up", &body);
    }

    /// A system notification, off this thread: it is a call into the shell.
    fn say(&self, title: &str, body: &str) {
        let platform = self.platform.clone();
        let (title, body) = (title.to_string(), body.to_string());
        std::thread::spawn(move || {
            if let Err(error) = platform.notify(&title, &body) {
                log::warn!("could not say that a timer ended: {error:#}");
            }
        });
    }

    /// The timers changed: keep them, re-plan the sleep, redraw the view.
    fn changed(&self, cx: &AsyncApp) {
        self.save(cx);
        self.plan(cx);
        if let Some((_, viewer)) = self.viewer.borrow().as_ref() {
            let _ = viewer.unbounded_send(Input::Refresh);
        }
    }

    /// Written on a background thread: the file is small, but it is disk.
    fn save(&self, cx: &AsyncApp) {
        let clock = self.clock.borrow().clone();
        let path = self.path.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = clock.save(&path) {
                    log::warn!("could not keep the timers: {error:#}");
                }
            })
            .detach();
    }

    /// Sleep until the nearest deadline, if anything is running.
    fn plan(&self, cx: &AsyncApp) {
        let plan = self.plan.get() + 1;
        self.plan.set(plan);

        let Some(deadline) = self.clock.borrow().next_deadline() else {
            return;
        };
        let wait = Duration::from_millis(deadline.saturating_sub(now())) + SETTLE;
        let this = self.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(wait).await;
            if this.plan.get() != plan {
                return;
            }
            this.catch_up(cx);
            // Woke early, or nothing ended: sleep again until the deadline.
            if this.plan.get() == plan {
                this.plan(cx);
            }
        })
        .detach();
    }

    /// Show the *Timers* view in the launcher, and keep it current while it
    /// is open.
    pub fn open_view(&self, window: gpui::WindowHandle<Launcher>, cx: &mut AsyncApp) {
        self.catch_up(cx);

        let number = self.views.get() + 1;
        self.views.set(number);

        // The view's events on a channel of their own, which ends when the
        // launcher drops the sink; `Closed` marks that, because the other
        // channel never ends while this task holds a sender for its ticks.
        let (view_tx, view_rx) = mpsc::unbounded::<Event>();
        let (tx, rx) = mpsc::unbounded::<Input>();
        let views = view_rx
            .map(Input::View)
            .chain(futures::stream::once(futures::future::ready(Input::Closed)));
        let mut inputs = futures::stream::select(views, rx);
        *self.viewer.borrow_mut() = Some((number, tx.clone()));

        let sink = ViewSink::new(move |event| {
            let _ = view_tx.unbounded_send(event);
        });
        let first = view::view(&self.clock.borrow(), now());
        window
            .update(cx, |launcher, _window, cx| {
                launcher.begin_view(sink, cx);
                launcher.show_view(first, cx);
            })
            .ok();

        let this = self.clone();
        cx.spawn(async move |cx| {
            let mut tick: u64 = 0;
            this.next_tick(&mut tick, &tx, cx);

            while let Some(input) = inputs.next().await {
                match input {
                    Input::View(event) => {
                        let acted = view::act(&mut this.clock.borrow_mut(), &event, now());
                        if acted {
                            this.changed(cx);
                        }
                    }
                    Input::Closed => break,
                    Input::Tick(asked) if asked != tick => {}
                    Input::Tick(_) | Input::Refresh => {
                        let view = view::view(&this.clock.borrow(), now());
                        window
                            .update(cx, |launcher, _window, cx| launcher.show_view(view, cx))
                            .ok();
                        this.next_tick(&mut tick, &tx, cx);
                    }
                }
            }

            let mut viewer = this.viewer.borrow_mut();
            if viewer.as_ref().is_some_and(|(open, _)| *open == number) {
                *viewer = None;
            }
        })
        .detach();
    }

    /// Ask to be woken when a face turns over, superseding any earlier ask.
    fn next_tick(&self, tick: &mut u64, tx: &mpsc::UnboundedSender<Input>, cx: &AsyncApp) {
        *tick += 1;
        let Some(wait) = view::next_tick(&self.clock.borrow(), now()) else {
            return;
        };
        let (asked, tx) = (*tick, tx.clone());
        cx.spawn(async move |cx| {
            cx.background_executor().timer(wait + SETTLE).await;
            let _ = tx.unbounded_send(Input::Tick(asked));
        })
        .detach();
    }
}
