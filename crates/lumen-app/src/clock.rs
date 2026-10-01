//! Running *World Clock* (F13).
//!
//! `currency.rs`'s shape: the decisions are in `lumen_core::clock::session`,
//! where they are tested; this owns the minute tick and the clipboard.
//!
//! # Nothing waits, and nothing wakes while hidden
//!
//! The places are read on the background executor the first time the view
//! opens, and kept; *My Places* is read beside them each time, and written
//! there too. The view redraws when the minute turns over, and only
//! while the launcher is active: a tick that finds it hidden asks for no
//! next one, and the view itself is gone the next time the launcher shows.

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::App;
use lumen_core::clock::session::{Effect, Session};
use lumen_core::clock::{self, ClockConfig};
use lumen_core::places::Favourites;
use lumen_core::view::Event;
use lumen_platform::{DateStyle, Platform};
use lumen_ui::{Launcher, ViewSink};

/// Everything the session is told, on one channel so it has one order.
enum Input {
    View(Event),
    /// The view is gone: its sink was dropped.
    Closed,
    /// The minute turned over.
    Tick,
}

/// Open the World Clock view.
pub fn run(
    platform: Arc<dyn Platform>,
    config: ClockConfig,
    home: Option<String>,
    favourites: PathBuf,
    window: gpui::WindowHandle<Launcher>,
    cx: &mut App,
) {
    let (view_tx, view_rx) = mpsc::unbounded::<Event>();
    let (tx, rx) = mpsc::unbounded::<Input>();
    let views = view_rx
        .map(Input::View)
        .chain(futures::stream::once(futures::future::ready(Input::Closed)));
    let mut inputs = futures::stream::select(views, rx);

    let sink = ViewSink::new(move |event| {
        let _ = view_tx.unbounded_send(event);
    });
    // The loading state, until the places have been read.
    window
        .update(cx, |launcher, _window, cx| launcher.begin_view(sink, cx))
        .ok();

    cx.spawn(async move |cx| {
        let (places, here, kept) = {
            let favourites = favourites.clone();
            cx.background_executor()
                .spawn(async move {
                    (
                        lumen_core::places::catalogue(),
                        clock::here(),
                        Favourites::load(&favourites),
                    )
                })
                .await
        };

        // Times are written as the user's regional settings write them,
        // so a 12-hour machine reads `9:14 AM`.
        let format_platform = platform.clone();
        let time = Box::new(move |when: &lumen_platform::LocalTime| {
            format_platform
                .format_date(when, DateStyle::Time, None)
                .unwrap_or_else(|error| {
                    log::warn!("could not write a time the system's way: {error:#}");
                    format!("{:02}:{:02}", when.hour, when.minute)
                })
        });
        let mut session = Session::new(config, places, kept, here, time).with_home(home.as_deref());
        let mut effects = session.start(clock::now());
        tick(&tx, cx);

        loop {
            for effect in effects.drain(..) {
                match effect {
                    Effect::Render(view) => {
                        window
                            .update(cx, |launcher, _window, cx| launcher.show_view(view, cx))
                            .ok();
                    }
                    Effect::ClearQuery => {
                        window
                            .update(cx, |launcher, _window, cx| {
                                launcher.set_query(String::new(), cx)
                            })
                            .ok();
                    }
                    Effect::Save(kept) => {
                        let path = favourites.clone();
                        cx.background_executor()
                            .spawn(async move {
                                if let Err(error) = kept.save(&path) {
                                    log::warn!("could not keep My Places: {error:#}");
                                }
                            })
                            .detach();
                    }
                    Effect::Copy(text) => {
                        if let Err(error) = platform.set_clipboard_text(&text) {
                            log::error!("could not copy the time: {error:#}");
                            continue;
                        }
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                    }
                }
            }

            let Some(input) = inputs.next().await else {
                break;
            };
            effects = match input {
                Input::View(event) => session.event(event),
                Input::Closed => break,
                Input::Tick => {
                    let active = window
                        .read_with(cx, |launcher, _| launcher.is_active())
                        .unwrap_or(false);
                    if !active {
                        // Hidden: no redraw, and no next tick. The view is
                        // dropped the next time the launcher shows.
                        Vec::new()
                    } else {
                        tick(&tx, cx);
                        session.tick(clock::now())
                    }
                }
            };
        }
    })
    .detach();
}

/// Ask for a tick when the minute next turns over.
fn tick(tx: &mpsc::UnboundedSender<Input>, cx: &mut gpui::AsyncApp) {
    // A little after, never before: early would redraw the same minute.
    let wait = clock::until_next_minute(clock::now()) + std::time::Duration::from_millis(5);
    let tx = tx.clone();
    let timer = cx.background_executor().timer(wait);
    cx.background_executor()
        .spawn(async move {
            timer.await;
            let _ = tx.unbounded_send(Input::Tick);
        })
        .detach();
}
