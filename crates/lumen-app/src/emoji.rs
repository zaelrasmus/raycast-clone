//! Running *Search Emoji & Symbols* (F18).
//!
//! `clipboard.rs`'s shape: the decisions are in `lumen_core::emoji::session`,
//! where they are tested, and this owns the clipboard and `emoji.json`. The
//! session is made on the background executor: it reads the catalogue the
//! first time, and the pins and usage with it, so the first view drawn
//! already has them (I-10).
//!
//! The custom keywords form is the launcher's, opened over the grid; what it
//! saves comes back into the same loop as the view's events, so the session
//! is only ever touched from here.

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use lumen_core::emoji::session::{Effect, Session};
use lumen_core::emoji::store::State;
use lumen_core::view::Event;
use lumen_platform::Platform;
use lumen_ui::{Launcher, ViewSink};

/// What the loop hears: the view, or the keywords form saving.
enum Input {
    View(Event),
    Keywords { item: String, text: String },
}

pub fn run(
    platform: Arc<dyn Platform>,
    file: PathBuf,
    window: gpui::WindowHandle<Launcher>,
    cx: &mut gpui::App,
) {
    let (view_tx, mut events) = mpsc::unbounded::<Input>();
    let view_tx = Arc::new(view_tx);
    // Weak, so the loop ends when the view does: the sink, and a form open
    // over it, hold the only strong ones.
    let inputs = Arc::downgrade(&view_tx);
    let sink = ViewSink::new(move |event| {
        let _ = view_tx.unbounded_send(Input::View(event));
    });
    // The loading state, until the catalogue has been read.
    window
        .update(cx, |launcher, _window, cx| launcher.begin_view(sink, cx))
        .ok();

    cx.spawn(async move |cx| {
        let loading = file.clone();
        let mut session = cx
            .background_executor()
            .spawn(async move { Session::new(State::load(&loading)) })
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
                    Effect::Paste { text, keep_open } => {
                        window
                            .update(cx, |launcher, _window, cx| match request(text, keep_open) {
                                Request::Close(paste) => launcher.paste(paste, cx),
                                Request::KeepOpen(paste) => launcher.paste_keep_open(paste, cx),
                            })
                            .ok();
                    }
                    Effect::Save(state) => {
                        let file = file.clone();
                        cx.background_executor()
                            .spawn(async move {
                                if let Err(error) = state.save(&file) {
                                    log::warn!(
                                        "could not save the emoji pins, usage and keywords: {error:#}"
                                    );
                                }
                            })
                            .detach();
                    }
                    Effect::EditKeywords { item, title, text } => {
                        let Some(inputs) = inputs.upgrade() else {
                            continue;
                        };
                        let on_save = move |text| {
                            let item = item.clone();
                            let _ = inputs.unbounded_send(Input::Keywords { item, text });
                        };
                        window
                            .update(cx, |launcher, window, cx| {
                                launcher.open_emoji_form(title, text, on_save, window, cx)
                            })
                            .ok();
                    }
                    Effect::Copy(text) => {
                        if let Err(error) = platform.set_clipboard_text(&text) {
                            log::error!("could not copy an emoji: {error:#}");
                            continue;
                        }
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                    }
                }
            }
            let Some(input) = events.next().await else {
                break;
            };
            effects = match input {
                Input::View(event) => session.event(event),
                Input::Keywords { item, text } => session.save_keywords(&item, &text),
            };
        }
    })
    .detach();
}

/// How the launcher is asked to paste an emoji.
#[derive(Debug, PartialEq)]
enum Request {
    /// *Paste*: the launcher closes.
    Close(lumen_platform::Paste),
    /// *Paste and Keep Window Open*: the grid stays as it is, to paste
    /// another.
    KeepOpen(lumen_platform::Paste),
}

fn request(text: String, keep_open: bool) -> Request {
    let paste = paste(text);
    if keep_open {
        Request::KeepOpen(paste)
    } else {
        Request::Close(paste)
    }
}

/// An emoji typed into the window the launcher was opened over, as Raycast
/// does it: what was on the clipboard is put back, and the emoji is not
/// recorded in the history -- it was inserted, not copied.
fn paste(text: String) -> lumen_platform::Paste {
    lumen_platform::Paste {
        text,
        html: None,
        keys: lumen_platform::PasteKeys::default(),
        caret_back: 0,
        restore_clipboard: true,
        files: Vec::new(),
        image: None,
        keep_out_of_history: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paste_puts_the_clipboard_back_and_stays_out_of_the_history() {
        let paste = paste("\u{2600}\u{FE0F}".into());
        assert_eq!(paste.text, "\u{2600}\u{FE0F}");
        assert!(paste.restore_clipboard);
        assert!(paste.keep_out_of_history);
        assert_eq!(paste.html, None);
        assert_eq!(paste.caret_back, 0);
        assert!(paste.files.is_empty());
        assert!(paste.image.is_none());
    }

    /// *Paste and Keep Window Open* is the same paste, asked for without
    /// closing; *Paste* closes.
    #[test]
    fn a_paste_that_keeps_the_window_open_is_the_same_paste() {
        let Request::KeepOpen(kept) = request("\u{1F600}".into(), true) else {
            panic!("a keep-open paste closed the launcher");
        };
        assert_eq!(kept, paste("\u{1F600}".into()));
        assert!(kept.restore_clipboard);
        assert!(kept.keep_out_of_history);

        assert_eq!(
            request("\u{1F600}".into(), false),
            Request::Close(paste("\u{1F600}".into()))
        );
    }
}
