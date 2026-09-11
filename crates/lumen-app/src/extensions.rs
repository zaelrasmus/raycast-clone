//! Running an extension (F4, step 3).
//!
//! # Where this sits
//!
//! The daemon owns processes; the launcher owns rendering. So this file starts
//! the process, reads what it says and answers its requests, and hands views to
//! the launcher through a handle. The launcher never spawns anything and never
//! sees a pipe.
//!
//! # Three threads, and why
//!
//! A child's pipes are blocking, and the UI thread cannot block. So a session
//! is a reader thread turning lines into messages, a writer thread turning
//! messages into lines, and a task on the foreground executor in between that
//! does the deciding. The task is where the `App` is touched; the threads never
//! touch it.
//!
//! That is the same discipline as I-15 for input callbacks, applied for the
//! same reason: the thread that must not stall pushes to a channel and returns.
//!
//! # I-3 is not at risk here
//!
//! "Root search never waits on an extension process. No exceptions and no
//! 'short' timeouts." Nothing in this file runs during a search. Commands are
//! in the root list because they are in the manifest index (AD-08), and a
//! process starts only once the user has chosen one. Until its first view
//! arrives the launcher shows that it is loading, which is a state rather than
//! a wait.

use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext as _};
use lumen_core::extension::Extension;
use lumen_core::protocol::{Answer, Capability, FromExtension, ToExtension};
use lumen_core::view::Event;
use lumen_platform::{LaunchTarget, Platform};
use lumen_ui::{Launcher, ViewSink};

/// How a running extension is spoken to.
///
/// Cloneable and cheap: it is a channel sender and nothing else, so the
/// launcher can hold one without holding the process.
#[derive(Clone)]
pub struct Session {
    to_extension: mpsc::UnboundedSender<ToExtension>,
}

impl Session {
    /// Tell the extension something happened. Never blocks.
    pub fn send(&self, event: Event) {
        let _ = self
            .to_extension
            .unbounded_send(ToExtension::Event { event });
    }
}

/// Start `command` from `extension` and drive it until it closes.
///
/// Returns immediately. The view appears when the extension sends one.
pub fn run(
    platform: Arc<dyn Platform>,
    extension: Extension,
    command: String,
    window: gpui::WindowHandle<Launcher>,
    cx: &mut App,
) -> Result<()> {
    let program = extension
        .manifest
        .run
        .first()
        .context("the manifest declares no program to run")?
        .clone();
    let arguments: Vec<String> = extension.manifest.run[1..].to_vec();

    let mut child = platform
        .spawn_extension(&program, &arguments, &extension.directory)
        .with_context(|| format!("starting the extension {}", extension.id()))?;

    let stdin = child.stdin.take().context("the child has no stdin")?;
    let stdout = child.stdout.take().context("the child has no stdout")?;

    let (to_extension, mut outgoing) = mpsc::unbounded::<ToExtension>();
    let (incoming_tx, mut incoming) = mpsc::unbounded::<FromExtension>();

    // Reader. Ends when the child closes its output or exits, which is how the
    // session learns the extension is gone without anybody polling for it.
    let name = extension.id().to_string();
    std::thread::Builder::new()
        .name(format!("lumen-ext-{name}"))
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match lumen_core::protocol::parse(&line) {
                    // The channel being closed means the view is gone; stop
                    // reading rather than filling a queue nobody reads.
                    Ok(message) => {
                        if incoming_tx.unbounded_send(message).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        log::warn!("{name} sent something unreadable: {error}");
                    }
                }
            }
        })
        .context("starting the extension reader")?;

    // Writer.
    let (writes_tx, writes_rx) = std::sync::mpsc::channel::<ToExtension>();
    std::thread::Builder::new()
        .name(format!("lumen-ext-{}-in", extension.id()))
        .spawn(move || {
            let mut stdin = stdin;
            while let Ok(message) = writes_rx.recv() {
                let Ok(line) = lumen_core::protocol::line(&message) else {
                    continue;
                };
                if stdin.write_all(line.as_bytes()).is_err() || stdin.flush().is_err() {
                    break;
                }
            }
        })
        .context("starting the extension writer")?;

    let session = Session {
        to_extension: to_extension.clone(),
    };

    // Everything the extension will be told, in order, over one channel: the
    // initial `Run`, every event from the view, and every answer to a request.
    let _ = to_extension.unbounded_send(ToExtension::Run { command });

    cx.spawn({
        let session = session.clone();
        async move |cx| {
            // Pumping the outgoing channel onto the writer thread. A separate
            // task so that a slow write cannot delay a view that has already
            // arrived.
            let pump = cx.background_spawn(async move {
                while let Some(message) = outgoing.next().await {
                    if writes_tx.send(message).is_err() {
                        break;
                    }
                }
            });

            // The launcher is handed a sink, not the session: it must not
            // know whether there is a process on the other end (AD-06), and the
            // day the transport is a runtime call this closure is the only
            // thing that changes.
            let sink = ViewSink::new(move |event| session.send(event));
            window
                .update(cx, |launcher, _window, cx| launcher.begin_view(sink, cx))
                .ok();

            while let Some(message) = incoming.next().await {
                match message {
                    FromExtension::Render { view } => {
                        window
                            .update(cx, |launcher, _window, cx| launcher.show_view(view, cx))
                            .ok();
                    }
                    FromExtension::Toast { message, tone } => {
                        // No toast surface yet, and inventing one here would be
                        // a feature smuggled in under a protocol change. Logged
                        // so an extension author can see their message arrived.
                        log::info!("extension says ({tone:?}): {message}");
                    }
                    FromExtension::Request { id, capability } => {
                        let result = perform(&platform, capability);
                        let _ = to_extension.unbounded_send(ToExtension::Response { id, result });
                    }
                    FromExtension::Close => break,
                }
            }

            // The extension has closed its output or asked to go away. Either
            // way the view goes with it: a view whose extension has exited
            // cannot answer anything the user does next.
            window
                .update(cx, |launcher, _window, cx| launcher.end_view(cx))
                .ok();

            drop(pump);
            let _ = child.kill();
            let _ = child.wait();
        }
    })
    .detach();

    Ok(())
}

/// Do what an extension asked, through the platform (I-7, AD-09).
///
/// Every arm goes through `trait Platform`. Reaching for `std` here instead
/// would work today and would be the thing that has to be found and undone the
/// day a sandbox arrives.
fn perform(platform: &Arc<dyn Platform>, capability: Capability) -> Answer {
    let outcome = match capability {
        Capability::CopyToClipboard { text } => platform.set_clipboard_text(&text).map(|_| None),
        Capability::ReadClipboard => platform.clipboard_text(),
        Capability::Open { target } => platform
            .launch(&LaunchTarget::ShellExecute(target))
            .map(|_| None),
    };

    match outcome {
        Ok(value) => Answer::Ok { value },
        Err(error) => {
            // Reported to the extension rather than only logged: it asked, and
            // it is the only thing that knows what to do about the answer.
            log::warn!("an extension's request failed: {error:#}");
            Answer::Failed {
                message: format!("{error}"),
            }
        }
    }
}
