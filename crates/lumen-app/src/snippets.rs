//! Running *Search Snippets* (F11).
//!
//! `currency.rs`'s shape: the decisions are in `lumen_core::snippets::session`,
//! where they are tested, and this owns the file and the clipboard.
//!
//! The file is read each time the view opens, on the background executor, so
//! a snippet added by hand is there the next time without a reload, and
//! nothing about snippets is held while the view is closed.

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use lumen_core::snippets::{self, PasteMode, session::Effect, session::Session};
use lumen_core::view::Event;
use lumen_platform::Platform;
use lumen_ui::{Launcher, ViewSink};

/// Open *Search Snippets*.
pub fn run(
    platform: Arc<dyn Platform>,
    path: PathBuf,
    paste: PasteMode,
    window: gpui::WindowHandle<Launcher>,
    cx: &mut gpui::App,
) {
    let (view_tx, mut events) = mpsc::unbounded::<Event>();
    let sink = ViewSink::new(move |event| {
        let _ = view_tx.unbounded_send(event);
    });
    // The loading state, until the file has been read.
    window
        .update(cx, |launcher, _window, cx| launcher.begin_view(sink, cx))
        .ok();

    cx.spawn(async move |cx| {
        let mut session = cx
            .background_executor()
            .spawn(async move {
                // Here rather than on the UI thread: the first call reads the
                // system's time zone.
                Session::new(snippets::load(&path), snippets::now())
            })
            .await;
        let mut effects = vec![Effect::Render(session.view())];

        loop {
            for effect in effects.drain(..) {
                match effect {
                    Effect::Render(view) => {
                        window
                            .update(cx, |launcher, _window, cx| launcher.show_view(view, cx))
                            .ok();
                    }
                    Effect::Paste(text) => {
                        // The keys are the daemon's to choose, once it knows
                        // which program the window belongs to.
                        let paste = lumen_platform::Paste {
                            text,
                            keys: lumen_platform::PasteKeys::default(),
                            restore_clipboard: paste == PasteMode::Restore,
                        };
                        window
                            .update(cx, |launcher, _window, cx| launcher.paste(paste, cx))
                            .ok();
                    }
                    Effect::Copy(text) => {
                        if let Err(error) = platform.set_clipboard_text(&text) {
                            log::error!("could not copy the snippet: {error:#}");
                            continue;
                        }
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                    }
                }
            }

            // The launcher drops the sink when the view goes, which ends this.
            let Some(event) = events.next().await else {
                break;
            };
            effects = session.event(event);
        }
    })
    .detach();
}
