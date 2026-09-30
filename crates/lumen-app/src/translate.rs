//! Running the translate view (F7).
//!
//! # Where this sits
//!
//! The same place as `extensions.rs`, and for the same reason: the launcher
//! draws views, the daemon does the work behind them. Here the work is
//! requests to a translation service rather than a process, and the
//! decisions -- what to draw, when to ask, which answers are stale -- are all
//! in `lumen_core::translate::session`, where they are tested. This file only
//! carries them out: it owns the threads, the timers and the client.
//!
//! # Nothing waits
//!
//! Requests run on threads of their own and report back over a channel; the
//! task that reads it is on the foreground executor and never blocks. Root
//! search is not involved at all (I-3): this starts only once the user has
//! chosen the command, and the view shows its empty state straight away.
//!
//! # The client is kept briefly, not forever
//!
//! Building one reads the certificate store, so it is kept between requests
//! -- but dropped once nothing has been asked for a while, so a translation
//! done in the morning does not leave a connection pool and its thread in a
//! daemon that then idles all day (section 5's idle budgets).

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::App;
use lumen_core::dictionary::languages as dictionary_languages;
use lumen_core::translate::session::{Effect, Languages, Request, Session};
use lumen_core::translate::{self, Failure, TranslateConfig, Translation, Translator};
use lumen_core::view::Event;
use lumen_platform::Platform;
use lumen_ui::{Launcher, ViewSink};
use parking_lot::Mutex;

/// How long typing must pause before anything is sent.
///
/// Long enough that a word typed at speed is one request, short enough that
/// the answer seems to follow the typing rather than wait for permission.
const PAUSE: Duration = Duration::from_millis(300);

/// How long the client is kept after the last request.
const KEEP_CLIENT: Duration = Duration::from_secs(30);

/// The languages used last, for the plain *Translate* command: whatever was
/// last chosen in the view, or the preset last opened.
///
/// In memory: it lasts as long as the daemon, which is as long as anybody
/// remembers having chosen it.
static LAST: Mutex<Option<Languages>> = Mutex::new(None);

/// Everything the session is told, on one channel so it has one order.
enum Input {
    View(Event),
    /// The view is gone: its sink was dropped.
    Closed,
    Settled(u64),
    Answered {
        generation: u64,
        target: usize,
        result: Result<Translation, Failure>,
    },
    /// Nothing has been sent since request number `.0`.
    Quiet(u64),
}

/// The client, shared by the request threads and dropped when quiet.
type Client = Arc<Mutex<Option<Arc<dyn Translator>>>>;

/// Open the translate view on `preset`, or on the languages used last, with
/// `text` already typed when it comes from Define (F10).
///
/// `define` is only passed on, for when Translate opens Define in turn.
pub fn run(
    platform: Arc<dyn Platform>,
    config: TranslateConfig,
    preset: Option<String>,
    text: Option<String>,
    define: crate::define::Setup,
    window: gpui::WindowHandle<Launcher>,
    cx: &mut App,
) {
    let presets = config.presets();
    let chosen = preset
        .as_deref()
        .and_then(|name| presets.iter().find(|preset| preset.name == name))
        .map(Languages::of);
    let start = match chosen {
        // Choosing a preset's own row counts as using it.
        Some(languages) => {
            *LAST.lock() = Some(languages.clone());
            languages
        }
        None => LAST
            .lock()
            .clone()
            .unwrap_or_else(|| Languages::of(&presets[0])),
    };
    let mut session = Session::new(presets, start, config.service);

    // The view's events on a channel of their own, which ends when the
    // launcher drops the sink -- the user left, or the launcher was shown
    // afresh. `Closed` marks that, because the other channel never ends: this
    // task holds a sender for the timers and threads it starts.
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
            if let Some(text) = text {
                launcher.set_query(text, cx);
            }
        })
        .ok();

    let config = Arc::new(config);
    cx.spawn(async move |cx| {
        let client: Client = Arc::default();
        let mut sent: u64 = 0;

        while let Some(input) = inputs.next().await {
            let effects = match input {
                Input::View(event) => session.event(event),
                Input::Closed => break,
                Input::Settled(generation) => session.settle(generation),
                Input::Answered {
                    generation,
                    target,
                    result,
                } => session.answer(generation, target, result),
                Input::Quiet(count) => {
                    if count == sent && client.lock().take().is_some() {
                        log::debug!("released the translation client");
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
                        send(request, client.clone(), config.clone(), tx.clone());

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
                            log::error!("could not copy the translation: {error:#}");
                            continue;
                        }
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                    }
                    Effect::Remember(languages) => *LAST.lock() = Some(languages),
                    // Opening Define drops this view's sink, which ends
                    // this loop on its next turn.
                    Effect::Define { word, language } => {
                        let platform = platform.clone();
                        let translate = (*config).clone();
                        let setup = define.clone();
                        let start = (word, language.and_then(dictionary_languages::find));
                        cx.update(|cx| {
                            crate::define::run(platform, setup, translate, Some(start), window, cx)
                        })
                        .ok();
                    }
                }
            }
        }
    })
    .detach();
}

/// Translate on a thread of its own and report back.
///
/// A thread per request rather than the background executor: the client
/// blocks for as long as the service takes, up to its timeout, and a pool
/// thread held that long is one the file index cannot use. There are at most
/// four per pause in typing.
fn send(
    request: Request,
    client: Client,
    config: Arc<TranslateConfig>,
    tx: mpsc::UnboundedSender<Input>,
) {
    let generation = request.generation;
    let target = request.target;
    let reply = tx.clone();

    let spawned = std::thread::Builder::new()
        .name("lumen-translate".into())
        .spawn(move || {
            let result = translator(&client, &config)
                .and_then(|translator| {
                    translator.translate(&request.text, request.from, request.to)
                })
                .map_err(|error| {
                    // The error, never the text: what the user typed stays
                    // out of the log.
                    log::warn!("translation failed: {error:#}");
                    translate::failure(&error)
                });
            let _ = reply.unbounded_send(Input::Answered {
                generation,
                target,
                result,
            });
        });

    if let Err(error) = spawned {
        let _ = tx.unbounded_send(Input::Answered {
            generation,
            target,
            result: Err(Failure::Other(format!(
                "could not start a request: {error}"
            ))),
        });
    }
}

/// The shared client, built by whichever request needs it first.
///
/// The lock is held while building, so the other targets of the same pause
/// wait for this one rather than each reading the certificate store.
fn translator(client: &Client, config: &TranslateConfig) -> anyhow::Result<Arc<dyn Translator>> {
    let mut slot = client.lock();
    if let Some(translator) = slot.as_ref() {
        return Ok(translator.clone());
    }
    let translator: Arc<dyn Translator> = translate::translator(config)?.into();
    *slot = Some(translator.clone());
    Ok(translator)
}
