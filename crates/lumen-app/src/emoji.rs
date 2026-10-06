//! Running *Search Emoji & Symbols* (F18).
//!
//! `clipboard.rs`'s shape: the decisions are in `lumen_core::emoji::session`,
//! where they are tested, and this owns the clipboard and `emoji.json`. The
//! session is made on the background executor: it reads the catalogue the
//! first time, and the pins and usage with it, so the first view drawn
//! already has them (I-10).

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use lumen_core::emoji::session::{Effect, Session};
use lumen_core::emoji::store::State;
use lumen_core::view::Event;
use lumen_platform::Platform;
use lumen_ui::{Launcher, ViewSink};

pub fn run(
    platform: Arc<dyn Platform>,
    file: PathBuf,
    window: gpui::WindowHandle<Launcher>,
    cx: &mut gpui::App,
) {
    let (view_tx, mut events) = mpsc::unbounded::<Event>();
    let sink = ViewSink::new(move |event| {
        let _ = view_tx.unbounded_send(event);
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
                    Effect::Paste(text) => {
                        let paste = paste(text);
                        window
                            .update(cx, |launcher, _window, cx| launcher.paste(paste, cx))
                            .ok();
                    }
                    Effect::Save(state) => {
                        let file = file.clone();
                        cx.background_executor()
                            .spawn(async move {
                                if let Err(error) = state.save(&file) {
                                    log::warn!(
                                        "could not save the emoji pins and usage: {error:#}"
                                    );
                                }
                            })
                            .detach();
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
            let Some(event) = events.next().await else {
                break;
            };
            effects = session.event(event);
        }
    })
    .detach();
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
}
