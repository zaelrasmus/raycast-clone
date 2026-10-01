//! Running *Calculator History*, and keeping it (F14).
//!
//! `clipboard.rs`'s shape: the decisions -- what is kept, in what order, how
//! many, what is shown and what `Enter` does -- are in `lumen_core::calc`,
//! where they are tested; this file owns the file and the clipboard. Every
//! read and write is on the background executor.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use lumen_core::calc::history::{Entry, History};
use lumen_core::calc::session::{Effect, Session};
use lumen_core::view::Event;
use lumen_platform::Platform;
use lumen_ui::{Launcher, ViewSink};

/// Keep `expression` and the `result` copied from it, at most `limit` in all.
pub fn record(path: &Path, expression: String, result: String, limit: usize) {
    change(path, |history| {
        history.record(
            Entry {
                expression,
                result,
                at: lumen_core::clock::now().as_second(),
            },
            limit,
        )
    });
}

/// [`History::change`], with a failure logged: the history as it now is, or
/// as the file had it.
fn change(path: &Path, edit: impl FnOnce(&mut History) -> bool) -> History {
    History::change(path, edit).unwrap_or_else(|error| {
        log::warn!("could not keep the calculator's history: {error:#}");
        History::load(path)
    })
}

pub fn run(
    platform: Arc<dyn Platform>,
    path: PathBuf,
    limit: usize,
    window: gpui::WindowHandle<Launcher>,
    cx: &mut gpui::App,
) {
    let (view_tx, mut events) = mpsc::unbounded::<Event>();
    let sink = ViewSink::new(move |event| {
        let _ = view_tx.unbounded_send(event);
    });
    // The loading state, until the history has been read.
    window
        .update(cx, |launcher, _window, cx| launcher.begin_view(sink, cx))
        .ok();

    cx.spawn(async move |cx| {
        let mut session = {
            let path = path.clone();
            cx.background_executor()
                .spawn(async move {
                    // A limit lowered since is applied here, so the view
                    // never shows more than is kept. Here rather than on
                    // the UI thread: the first call reads the time zone.
                    let history = change(&path, |history| history.trim(limit));
                    Session::new(history.entries, limit == 0, lumen_core::snippets::now())
                })
                .await
        };
        let mut effects = vec![Effect::Render(session.view())];

        loop {
            let mut follow = Vec::new();
            for effect in effects.drain(..) {
                match effect {
                    Effect::Render(view) => {
                        window
                            .update(cx, |launcher, _window, cx| launcher.show_view(view, cx))
                            .ok();
                    }
                    Effect::Copy(entry) => {
                        copy(&platform, &entry.result);
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                        used_again(cx, &path, entry, limit).await;
                    }
                    Effect::Paste(entry) => {
                        let paste = lumen_platform::Paste {
                            text: entry.result.clone(),
                            html: None,
                            keys: lumen_platform::PasteKeys::default(),
                            caret_back: 0,
                            restore_clipboard: false,
                            files: Vec::new(),
                            image: None,
                            keep_out_of_history: false,
                        };
                        window
                            .update(cx, |launcher, _window, cx| launcher.paste(paste, cx))
                            .ok();
                        used_again(cx, &path, entry, limit).await;
                    }
                    Effect::CopyText(text) => {
                        copy(&platform, &text);
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                    }
                    Effect::Delete(expression) => {
                        let path = path.clone();
                        let history = cx
                            .background_executor()
                            .spawn(
                                async move { change(&path, |history| history.delete(&expression)) },
                            )
                            .await;
                        follow.extend(session.reload(history.entries));
                    }
                }
            }
            if !follow.is_empty() {
                effects = follow;
                continue;
            }
            let Some(event) = events.next().await else {
                break;
            };
            effects = session.event(event);
        }
    })
    .detach();
}

fn copy(platform: &Arc<dyn Platform>, text: &str) {
    if let Err(error) = platform.set_clipboard_text(text) {
        log::error!("could not copy a calculation: {error:#}");
    }
}

/// Keep `entry` again, first, as of now: using it again is using it.
async fn used_again(cx: &mut gpui::AsyncApp, path: &Path, entry: Entry, limit: usize) {
    let path = path.to_path_buf();
    cx.background_executor()
        .spawn(async move { record(&path, entry.expression, entry.result, limit) })
        .await;
}
