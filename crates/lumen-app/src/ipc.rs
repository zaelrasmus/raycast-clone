//! The daemon/client socket (AD-02).
//!
//! # Why this exists on Windows, where it is not needed
//!
//! On Windows the global hotkey reaches the resident process directly and no
//! socket is required. It is built anyway, from F1, because Wayland has no
//! global shortcut API at all: there, the compositor runs the binary and a
//! second execution must toggle the instance that is already running. Building
//! that path only when the Linux port starts would mean discovering in F5 that
//! the daemon was never structured to be driven from outside.
//!
//! `interprocess` gives a named pipe on Windows and a Unix domain socket on
//! Linux behind one API, so this file contains no conditional compilation.

use std::io::{BufRead, BufReader, Write};

use anyhow::{Context as _, Result};
use interprocess::local_socket::traits::{ListenerExt as _, Stream as _};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName};
use serde::{Deserialize, Serialize};

/// The namespaced socket name, identical on both platforms.
const SOCKET: &str = "lumen-launcher.sock";

/// What a client can ask the daemon to do.
///
/// Newline-delimited JSON rather than a packed binary encoding: this carries a
/// handful of messages per day, and being able to drive the daemon from a
/// terminal while debugging is worth more than the bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Message {
    /// Show the launcher if hidden, hide it if shown.
    Toggle,
    Show,
    Hide,
    /// The hold gesture began. On Hyprland this arrives from `bind`, and the
    /// daemon measures the duration itself (section 8).
    OverlayPress,
    /// The hold gesture ended, from Hyprland's `bindr`.
    OverlayRelease,
    /// Open the settings window.
    Settings,
    /// Re-read everything the user edits by hand: the configuration, the
    /// sheets, scripts and snippets. Named for the first thing it reloaded;
    /// `lumen reload` is the same message.
    ReloadSheets,
    /// The configuration file changed on disk. Sent by the daemon's own
    /// watcher, through the same queue as everything else.
    ReloadConfig,
    /// Something was installed or removed, so the application index is stale.
    /// Sent by the platform's watch on the places applications register.
    ApplicationsChanged,
    /// Stop the daemon.
    Quit,
    /// Used by the client to detect a running daemon without side effects.
    Ping,
    /// Start an extension's command and give it the launcher (F4).
    ///
    /// In the same enum as the rest even though no client sends it. One queue
    /// means one order of events -- a view opening cannot overtake the hide
    /// that was already on its way -- and it costs a variant rather than a
    /// second channel with its own ordering to reason about.
    RunExtension {
        extension: std::path::PathBuf,
        command: String,
    },
}

/// Send `message` to a running daemon.
///
/// `Ok(false)` means no daemon is listening, which is the normal first-run case
/// and not an error.
pub fn send(message: Message) -> Result<bool> {
    let name = SOCKET
        .to_ns_name::<GenericNamespaced>()
        .context("building the socket name")?;

    let mut stream = match Stream::connect(name) {
        Ok(stream) => stream,
        // Any connection failure is treated as "no daemon". Distinguishing a
        // missing socket from a refused one differs between platforms and buys
        // nothing: either way this process must start the daemon itself.
        Err(_) => return Ok(false),
    };

    let mut line = serde_json::to_string(&message)?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .context("writing to the daemon socket")?;
    stream.flush().ok();

    Ok(true)
}

/// Listen for client messages on a dedicated thread.
///
/// `handler` runs on that thread, so it must not block: it forwards to the UI
/// thread and returns, exactly like the gesture sink does (I-15).
pub fn serve(handler: impl Fn(Message) + Send + 'static) -> Result<()> {
    let name = SOCKET
        .to_ns_name::<GenericNamespaced>()
        .context("building the socket name")?;

    let listener = ListenerOptions::new()
        .name(name)
        .create_sync()
        .context("another Lumen daemon is already listening")?;

    std::thread::Builder::new()
        .name("lumen-ipc".into())
        .spawn(move || {
            for connection in listener.incoming() {
                let connection = match connection {
                    Ok(connection) => connection,
                    Err(error) => {
                        log::warn!("rejected an IPC connection: {error}");
                        continue;
                    }
                };

                // One message per connection, so a client that dies mid-write
                // cannot wedge the listener.
                let mut line = String::new();
                if BufReader::new(connection).read_line(&mut line).is_err() {
                    continue;
                }

                match serde_json::from_str::<Message>(line.trim()) {
                    Ok(Message::Ping) => {}
                    Ok(message) => handler(message),
                    Err(error) => log::warn!("ignoring unintelligible IPC message: {error}"),
                }
            }
        })
        .context("spawning the IPC listener thread")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_as_kebab_case() {
        let encoded = serde_json::to_string(&Message::OverlayPress).unwrap();
        assert_eq!(encoded, "\"overlay-press\"");
        assert_eq!(
            serde_json::from_str::<Message>(&encoded).unwrap(),
            Message::OverlayPress
        );
    }

    /// A daemon from a future version may send something this build does not
    /// know. It must be ignored, not crash the listener.
    #[test]
    fn unknown_messages_are_rejected_rather_than_panicking() {
        assert!(serde_json::from_str::<Message>("\"invented-later\"").is_err());
    }
}
