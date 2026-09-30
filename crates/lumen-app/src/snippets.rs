//! Running *Search Snippets* (F11).
//!
//! `currency.rs`'s shape: the decisions are in `lumen_core::snippets::session`,
//! where they are tested, and this owns the file and the clipboard.
//!
//! The file is read each time the view opens, on the background executor, so
//! a snippet added by hand is there the next time without a reload, and
//! nothing about snippets is held while the view is closed.
//!
//! Placeholders are filled in here, off the UI thread, when a snippet is
//! pasted or copied: reading the clipboard and asking Windows for a date are
//! system calls, and the rules for them are `template`'s, in core.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use lumen_core::clipboard::History;
use lumen_core::snippets::template::{Expanded, Sources, Template};
use lumen_core::snippets::{self, PasteMode, Snippet, session::Effect, session::Session};
use lumen_core::view::Event;
use lumen_platform::{DateStyle, LocalTime, Platform};
use lumen_ui::{Launcher, ViewSink};
use parking_lot::Mutex;

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

    let history = window
        .read_with(cx, |launcher, _| launcher.clipboard_history())
        .ok()
        .flatten();

    cx.spawn(async move |cx| {
        let (mut session, all) = cx
            .background_executor()
            .spawn(async move {
                // Here rather than on the UI thread: the first call reads the
                // system's time zone.
                let loaded = snippets::load(&path);
                // Kept beside the session, for `{snippet name=...}`.
                let all = Arc::new(loaded.snippets.clone());
                (Session::new(loaded, snippets::now()), all)
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
                    Effect::SetQuery(query) => {
                        window
                            .update(cx, |launcher, _window, cx| launcher.set_query(query, cx))
                            .ok();
                    }
                    Effect::Paste { text, answers } => {
                        let expanded = expand(cx, &platform, &all, &history, text, answers).await;
                        // The keys are the daemon's to choose, once it knows
                        // which program the window belongs to.
                        let paste = lumen_platform::Paste {
                            text: expanded.text,
                            keys: lumen_platform::PasteKeys::default(),
                            caret_back: expanded.caret_back,
                            restore_clipboard: paste == PasteMode::Restore,
                        };
                        window
                            .update(cx, |launcher, _window, cx| launcher.paste(paste, cx))
                            .ok();
                    }
                    Effect::Copy { text, answers } => {
                        // A copy has no caret to place.
                        let text = expand(cx, &platform, &all, &history, text, answers)
                            .await
                            .text;
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

/// `text` with its placeholders filled in, on the background executor.
async fn expand(
    cx: &mut gpui::AsyncApp,
    platform: &Arc<dyn Platform>,
    all: &Arc<Vec<Snippet>>,
    history: &Option<Arc<Mutex<History>>>,
    text: String,
    answers: HashMap<String, String>,
) -> Expanded {
    let template = Template::parse(&text);
    if !template.has_placeholders() {
        return Expanded {
            text,
            caret_back: 0,
        };
    }
    let sources = Live {
        platform: platform.clone(),
        snippets: all.clone(),
        history: history.clone(),
    };
    cx.background_executor()
        .spawn(async move { template.expand(&snippets::now(), &sources, &answers) })
        .await
}

/// Where placeholders are filled in from, for real.
struct Live {
    platform: Arc<dyn Platform>,
    snippets: Arc<Vec<Snippet>>,
    history: Option<Arc<Mutex<History>>>,
}

impl Sources for Live {
    fn clipboard(&self, offset: usize) -> Option<String> {
        if offset == 0 {
            // The clipboard itself rather than the history's newest entry:
            // what the history declined to keep is still what was copied.
            return self.platform.clipboard_text().ok().flatten();
        }
        let history = self.history.as_ref()?.lock();
        let entries = history
            .recent(offset + 1)
            .inspect_err(|error| log::warn!("could not read clipboard history: {error:#}"))
            .ok()?;
        entries.into_iter().nth(offset).map(|entry| entry.text)
    }

    fn snippet(&self, name: &str) -> Option<String> {
        self.snippets
            .iter()
            .find(|snippet| snippet.name.trim().eq_ignore_ascii_case(name.trim()))
            .map(|snippet| snippet.text.clone())
    }

    fn system_date(
        &self,
        when: &LocalTime,
        style: DateStyle,
        locale: Option<&str>,
    ) -> Option<String> {
        self.platform
            .format_date(when, style, locale)
            .inspect_err(|error| log::debug!("could not write a date: {error:#}"))
            .ok()
    }

    fn uuid(&self) -> String {
        // Uppercase, as Raycast writes it; `| lowercase` is there for the rest.
        uuid::Uuid::new_v4()
            .hyphenated()
            .encode_upper(&mut uuid::Uuid::encode_buffer())
            .to_string()
    }
}
