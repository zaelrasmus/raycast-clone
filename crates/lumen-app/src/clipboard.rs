//! Running *Clipboard History* (F12).
//!
//! `snippets.rs`'s shape: the decisions are in `lumen_core::clipboard::session`,
//! where they are tested, and this owns the database and the clipboard. Every
//! read and write of the history is on the background executor: it is disk
//! I/O behind a lock the recorder also takes.

use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use lumen_core::clipboard::session::{Effect, Session};
use lumen_core::clipboard::{Entry, History, Payload, images};
use lumen_core::view::Event;
use lumen_platform::{ClipboardContent, ClipboardImage, LaunchMode, LaunchTarget, Platform};
use lumen_ui::{Launcher, ViewSink};
use parking_lot::Mutex;

/// The most entries the view reads. Above the retention default, so the view
/// is the whole history unless the user asked to keep more than this.
const LOADED: usize = 1000;

pub fn run(platform: Arc<dyn Platform>, window: gpui::WindowHandle<Launcher>, cx: &mut gpui::App) {
    let (view_tx, mut events) = mpsc::unbounded::<Event>();
    let sink = ViewSink::new(move |event| {
        let _ = view_tx.unbounded_send(event);
    });
    // The loading state, until the history has been read.
    window
        .update(cx, |launcher, _window, cx| launcher.begin_view(sink, cx))
        .ok();

    let history = window
        .read_with(cx, |launcher, _| launcher.clipboard_history())
        .ok()
        .flatten();

    cx.spawn(async move |cx| {
        let mut session = {
            let history = history.clone();
            cx.background_executor()
                .spawn(async move {
                    // Here rather than on the UI thread: the first call reads
                    // the system's time zone.
                    let dir = history
                        .as_ref()
                        .and_then(|history| history.lock().images_dir().map(Into::into));
                    Session::new(read(&history), lumen_core::snippets::now()).with_images(dir)
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
                    Effect::Paste(entry) => {
                        let (text, files, image) = match entry.payload {
                            Payload::Text => (entry.text.clone(), Vec::new(), None),
                            Payload::Files => (entry.text.clone(), entry.files(), None),
                            Payload::Image => match picture(cx, &history, &entry).await {
                                Some(image) => (String::new(), Vec::new(), Some(image)),
                                None => continue,
                            },
                        };
                        // Left on the clipboard, as a paste from history is
                        // in Raycast, and kept out of the history: the entry
                        // is moved to the top below, with its own source.
                        let paste = lumen_platform::Paste {
                            text,
                            html: None,
                            keys: lumen_platform::PasteKeys::default(),
                            caret_back: 0,
                            restore_clipboard: false,
                            files,
                            image,
                            keep_out_of_history: true,
                        };
                        window
                            .update(cx, |launcher, _window, cx| launcher.paste(paste, cx))
                            .ok();
                        touch(cx, &history, entry.id).await;
                    }
                    Effect::Copy(entry) => {
                        let content = match entry.payload {
                            Payload::Text => ClipboardContent::Text(entry.text.clone()),
                            Payload::Files => ClipboardContent::Files(entry.files()),
                            Payload::Image => match picture(cx, &history, &entry).await {
                                Some(image) => ClipboardContent::Image(image),
                                None => continue,
                            },
                        };
                        if let Err(error) = platform.set_clipboard(&content, true) {
                            log::error!("could not copy a clipboard entry: {error:#}");
                            continue;
                        }
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                        touch(cx, &history, entry.id).await;
                    }
                    Effect::Open(target) => {
                        // The launcher hides first, so what opens comes to
                        // the front over the window it was opened on.
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                        let platform = platform.clone();
                        cx.background_executor()
                            .spawn(async move {
                                let target = LaunchTarget::ShellExecute(target);
                                if let Err(error) = platform.launch(&target, LaunchMode::Reuse) {
                                    log::warn!("could not open a clipboard entry: {error:#}");
                                }
                            })
                            .await;
                    }
                    // A picture's address rather than the picture (F12).
                    Effect::PasteText { id, text } => {
                        let paste = lumen_platform::Paste {
                            text,
                            html: None,
                            keys: lumen_platform::PasteKeys::default(),
                            caret_back: 0,
                            restore_clipboard: false,
                            files: Vec::new(),
                            image: None,
                            keep_out_of_history: true,
                        };
                        window
                            .update(cx, |launcher, _window, cx| launcher.paste(paste, cx))
                            .ok();
                        touch(cx, &history, id).await;
                    }
                    Effect::CopyText { id, text } => {
                        if let Err(error) =
                            platform.set_clipboard(&ClipboardContent::Text(text), true)
                        {
                            log::error!("could not copy a picture's address: {error:#}");
                            continue;
                        }
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                        touch(cx, &history, id).await;
                    }
                    Effect::Delete(id) => {
                        let history = history.clone();
                        let entries = cx
                            .background_executor()
                            .spawn(async move {
                                if let Some(history) = &history
                                    && let Err(error) = history.lock().delete(id)
                                {
                                    log::warn!("could not delete a clipboard entry: {error:#}");
                                }
                                read(&history)
                            })
                            .await;
                        follow.extend(session.reload(entries));
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

/// The history, newest first, or why it could not be read.
fn read(history: &Option<Arc<Mutex<History>>>) -> Result<Vec<Entry>, String> {
    let Some(history) = history else {
        return Err("It is turned off in config.json, or could not be opened".into());
    };
    history
        .lock()
        .recent(LOADED)
        .map_err(|error| format!("{error:#}"))
}

/// Move an entry to the top: using it again is copying it again.
async fn touch(cx: &mut gpui::AsyncApp, history: &Option<Arc<Mutex<History>>>, id: i64) {
    let Some(history) = history.clone() else {
        return;
    };
    cx.background_executor()
        .spawn(async move {
            if let Err(error) = history.lock().touch(id) {
                log::warn!("could not move a clipboard entry to the top: {error:#}");
            }
        })
        .await;
}

/// A picture's pixels and PNG, read and decoded off the UI thread. `None`,
/// and logged, when its file is gone.
async fn picture(
    cx: &mut gpui::AsyncApp,
    history: &Option<Arc<Mutex<History>>>,
    entry: &Entry,
) -> Option<ClipboardImage> {
    let history = history.clone()?;
    let hash = entry.text.clone();
    cx.background_executor()
        .spawn(async move {
            let dir = history.lock().images_dir()?.to_path_buf();
            images::load(&dir, &hash)
                .inspect_err(|error| log::warn!("could not read a clipboard picture: {error:#}"))
                .ok()
        })
        .await
}
