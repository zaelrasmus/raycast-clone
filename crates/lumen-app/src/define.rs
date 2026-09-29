//! Running the Define view (F10).
//!
//! `translate.rs`'s shape, for the same reasons: the decisions are in
//! `lumen_core::dictionary::session`, where they are tested, and this only
//! carries them out -- the thread per lookup, the typing pause, and a client
//! kept briefly and dropped when nothing has been asked for a while, so a
//! word looked up in the morning leaves no connection pool in a daemon that
//! then idles all day.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::App;
use lumen_core::dictionary::session::{Effect, Request, Session};
use lumen_core::dictionary::{self, Dictionary, Entry, Failure, languages};
use lumen_core::providers::quicklinks::{OpenLink, Template};
use lumen_core::view::Event;
use lumen_platform::Platform;
use lumen_ui::{Launcher, ViewSink};
use parking_lot::Mutex;

/// How long typing must pause before a word is looked up. Translate's pause:
/// a word typed at speed is one request.
const PAUSE: Duration = Duration::from_millis(300);

/// How long the client is kept after the last lookup.
const KEEP_CLIENT: Duration = Duration::from_secs(30);

enum Input {
    View(Event),
    /// The view is gone: its sink was dropped.
    Closed,
    Settled(u64),
    Answered {
        generation: u64,
        result: Result<Option<Entry>, Failure>,
    },
    /// Nothing has been sent since request number `.0`.
    Quiet(u64),
}

type Client = Arc<Mutex<Option<Arc<dyn Dictionary>>>>;

/// The word language chosen last, for the next time the view opens.
///
/// In memory, as Translate keeps its languages: it lasts as long as the
/// daemon, which is as long as anybody remembers having chosen it.
static LAST: Mutex<Option<&'static languages::Language>> = Mutex::new(None);

/// Open the Define view.
///
/// `browser` is the preferred one from the configuration, for *Open in
/// Wiktionary*.
pub fn run(
    platform: Arc<dyn Platform>,
    browser: Option<String>,
    window: gpui::WindowHandle<Launcher>,
    cx: &mut App,
) {
    let mut session = Session::new(
        LAST.lock().unwrap_or_else(languages::english),
        dictionary::explanation_language(),
        dictionary::attribution(),
    );

    let (view_tx, view_rx) = mpsc::unbounded::<Event>();
    let (tx, rx) = mpsc::unbounded::<Input>();
    let views = view_rx
        .map(Input::View)
        .chain(futures::stream::once(futures::future::ready(Input::Closed)));
    let mut inputs = futures::stream::select(views, rx);

    let sink = ViewSink::new(move |event| {
        let _ = view_tx.unbounded_send(event);
    });
    let first = session.view();
    window
        .update(cx, |launcher, _window, cx| {
            launcher.begin_view(sink, cx);
            launcher.show_view(first, cx);
        })
        .ok();

    cx.spawn(async move |cx| {
        let client: Client = Arc::default();
        let mut sent: u64 = 0;

        while let Some(input) = inputs.next().await {
            let effects = match input {
                Input::View(event) => session.event(event),
                Input::Closed => break,
                Input::Settled(generation) => session.settle(generation),
                Input::Answered { generation, result } => session.answer(generation, result),
                Input::Quiet(count) => {
                    if count == sent && client.lock().take().is_some() {
                        log::debug!("released the dictionary client");
                    }
                    Vec::new()
                }
            };

            for effect in effects {
                match effect {
                    Effect::Render(view) => {
                        window
                            .update(cx, |launcher, _window, cx| launcher.show_view(view, cx))
                            .ok();
                    }
                    Effect::Wait { generation } => {
                        let tx = tx.clone();
                        cx.spawn(async move |cx| {
                            cx.background_executor().timer(PAUSE).await;
                            let _ = tx.unbounded_send(Input::Settled(generation));
                        })
                        .detach();
                    }
                    Effect::Send(request) => {
                        sent += 1;
                        send(request, client.clone(), tx.clone());

                        let tx = tx.clone();
                        let count = sent;
                        cx.spawn(async move |cx| {
                            cx.background_executor().timer(KEEP_CLIENT).await;
                            let _ = tx.unbounded_send(Input::Quiet(count));
                        })
                        .detach();
                    }
                    Effect::Copy(text) => {
                        if let Err(error) = platform.set_clipboard_text(&text) {
                            log::error!("could not copy the definition: {error:#}");
                            continue;
                        }
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                    }
                    Effect::Remember(language) => *LAST.lock() = Some(language),
                    Effect::Open(url) => {
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                        open(platform.clone(), url, browser.clone());
                    }
                }
            }
        }
    })
    .detach();
}

/// Open a page in the preferred browser, off this thread: finding the
/// browser reads the registry, and opening it hands the foreground away.
fn open(platform: Arc<dyn Platform>, url: String, browser: Option<String>) {
    let spawned = std::thread::Builder::new()
        .name("lumen-define-open".into())
        .spawn(move || {
            let link = match Template::parse(&url) {
                Ok(link) => link,
                Err(error) => {
                    log::error!("the page to open is not a link: {error}");
                    return;
                }
            };
            let open = OpenLink {
                link,
                query: String::new(),
                browser,
                private: false,
            };
            if let Err(error) = open.run(platform.as_ref()) {
                log::error!("could not open the page: {error:#}");
                let _ = platform.notify("Could not open Wiktionary", &format!("{error:#}"));
            }
        });
    if let Err(error) = spawned {
        log::error!("could not start opening the page: {error}");
    }
}

/// Look up on a thread of its own and report back.
fn send(request: Request, client: Client, tx: mpsc::UnboundedSender<Input>) {
    let generation = request.generation;
    let reply = tx.clone();

    let spawned = std::thread::Builder::new()
        .name("lumen-define".into())
        .spawn(move || {
            let result = dictionary_client(&client)
                .and_then(|dictionary| {
                    dictionary.define(&request.word, request.language.code, request.explained_in)
                })
                .map_err(|error| {
                    // The error, never the word: what the user typed stays
                    // out of the log.
                    log::warn!("a lookup failed: {error:#}");
                    dictionary::failure(&error)
                });
            let _ = reply.unbounded_send(Input::Answered { generation, result });
        });

    if let Err(error) = spawned {
        let _ = tx.unbounded_send(Input::Answered {
            generation,
            result: Err(Failure::Other(format!("could not start a lookup: {error}"))),
        });
    }
}

/// The shared client, built by whichever lookup needs it first.
fn dictionary_client(client: &Client) -> anyhow::Result<Arc<dyn Dictionary>> {
    let mut slot = client.lock();
    if let Some(dictionary) = slot.as_ref() {
        return Ok(dictionary.clone());
    }
    let dictionary: Arc<dyn Dictionary> = dictionary::dictionary()?.into();
    *slot = Some(dictionary.clone());
    Ok(dictionary)
}
