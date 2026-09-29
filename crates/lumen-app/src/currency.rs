//! Running the currency view (F9).
//!
//! # Where this sits
//!
//! Beside `translate.rs`, and for the same reason: the launcher draws the
//! view, the daemon does the work behind it. The decisions are in
//! `lumen_core::currency::session`, where they are tested; this file owns
//! the thread that fetches, the file the table is kept in, and nothing else.
//!
//! # Nothing waits
//!
//! `currency.json` is read and written on the background executor, and a
//! fetch runs on a thread of its own. The client is built by the fetch and
//! dropped with it: one request a day does not need a pool kept warm.

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::App;
use lumen_core::currency::session::{Effect, Session};
use lumen_core::currency::{self, CurrencyConfig, Failure, Rates, State, When};
use lumen_core::view::Event;
use lumen_platform::Platform;
use lumen_ui::{Launcher, ViewSink};

/// Everything the session is told, on one channel so it has one order.
enum Input {
    View(Event),
    /// The view is gone: its sink was dropped.
    Closed,
    Fetched(Result<Rates, Failure>),
}

/// Open the currency view.
pub fn run(
    platform: Arc<dyn Platform>,
    config: CurrencyConfig,
    path: PathBuf,
    window: gpui::WindowHandle<Launcher>,
    cx: &mut App,
) {
    // The view's events end when the launcher drops the sink; `Closed` marks
    // that, because the other channel never ends while this task holds a
    // sender for the fetch thread.
    let (view_tx, view_rx) = mpsc::unbounded::<Event>();
    let (tx, rx) = mpsc::unbounded::<Input>();
    let views = view_rx
        .map(Input::View)
        .chain(futures::stream::once(futures::future::ready(Input::Closed)));
    let mut inputs = futures::stream::select(views, rx);

    let sink = ViewSink::new(move |event| {
        let _ = view_tx.unbounded_send(event);
    });
    // The loading state, until the file has been read.
    window
        .update(cx, |launcher, _window, cx| launcher.begin_view(sink, cx))
        .ok();

    let config = Arc::new(config);
    cx.spawn(async move |cx| {
        let state = {
            let path = path.clone();
            cx.background_executor()
                .spawn(async move { State::load(&path) })
                .await
        };
        let mut session = Session::new(state, config.service);
        let mut effects = session.start(currency::now());

        loop {
            for effect in effects.drain(..) {
                match effect {
                    Effect::Render(view) => {
                        window
                            .update(cx, |launcher, _window, cx| launcher.show_view(view, cx))
                            .ok();
                    }
                    Effect::Fetch => {
                        let answer = fetch(config.clone());
                        let tx = tx.clone();
                        cx.background_executor()
                            .spawn(async move {
                                let _ = tx.unbounded_send(Input::Fetched(answered(answer).await));
                            })
                            .detach();
                    }
                    Effect::Copy(text) => {
                        if let Err(error) = platform.set_clipboard_text(&text) {
                            log::error!("could not copy the amount: {error:#}");
                            continue;
                        }
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                    }
                    Effect::Save(state) => {
                        // The root list converts with the same table.
                        let rates = state.rates.clone();
                        window
                            .update(cx, |launcher, _window, cx| launcher.set_rates(rates, cx))
                            .ok();
                        let path = path.clone();
                        cx.background_executor()
                            .spawn(async move {
                                if let Err(error) = state.save(&path) {
                                    log::warn!("could not keep the exchange rates: {error:#}");
                                }
                            })
                            .detach();
                    }
                }
            }

            let Some(input) = inputs.next().await else {
                break;
            };
            effects = match input {
                Input::View(event) => session.event(event),
                Input::Closed => break,
                Input::Fetched(result) => session.fetched(result),
            };
        }
    })
    .detach();
}

/// Fetch a table for a conversion typed in the root list (F9, slice 5).
///
/// The same fetch the view makes, with no view: the table is kept in
/// `currency.json` beside what the view chose, and handed to the launcher,
/// whose next ranking answers with it. A failure is only logged -- the root
/// list has nowhere to say it, and the view says it when opened.
pub fn refresh(
    config: CurrencyConfig,
    path: PathBuf,
    window: gpui::WindowHandle<Launcher>,
    cx: &mut App,
) {
    let answer = fetch(Arc::new(config));
    cx.spawn(async move |cx| {
        let Ok(mut rates) = answered(answer).await else {
            return;
        };
        rates.retain_known();
        let kept = rates.clone();
        cx.background_executor()
            .spawn(async move {
                // Read again rather than remembered: the view may have
                // changed the currency or the pins since.
                let mut state = State::load(&path);
                state.rates = Some(kept);
                if let Err(error) = state.save(&path) {
                    log::warn!("could not keep the exchange rates: {error:#}");
                }
            })
            .detach();
        window
            .update(cx, |launcher, _window, cx| {
                launcher.set_rates(Some(rates), cx)
            })
            .ok();
    })
    .detach();
}

/// Fetch the latest table on a thread of its own; the answer arrives on the
/// channel returned.
///
/// A thread rather than the background executor: the client blocks for as
/// long as the service takes, and a pool thread held that long is one the
/// file index cannot use.
fn fetch(config: Arc<CurrencyConfig>) -> oneshot::Receiver<Result<Rates, Failure>> {
    let (tx, rx) = oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("lumen-currency".into())
        .spawn(move || {
            let result = currency::sources::source(&config)
                .and_then(|source| source.fetch(When::Latest))
                .map_err(|error| {
                    log::warn!("could not fetch exchange rates: {error:#}");
                    currency::failure(&error)
                });
            if let Ok(rates) = &result {
                log::info!("fetched {} exchange rates", rates.rates.len());
            }
            let _ = tx.send(result);
        });
    // The sender went with the closure, so the answer is a cancellation.
    if let Err(error) = spawned {
        log::error!("could not start a request for exchange rates: {error}");
    }
    rx
}

/// What [`fetch`] answered, a thread that never started included.
async fn answered(answer: oneshot::Receiver<Result<Rates, Failure>>) -> Result<Rates, Failure> {
    answer
        .await
        .unwrap_or_else(|_| Err(Failure::Other("Could not start a request".into())))
}
