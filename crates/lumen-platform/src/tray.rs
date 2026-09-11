//! The notification area icon.
//!
//! # Why a background process needs one
//!
//! Lumen is deliberately invisible: no taskbar button, no Alt+Tab entry, a
//! window that is only ever shown and hidden (I-4). That is right for a
//! launcher and wrong for everything else about living on somebody's machine.
//! A resident process with no visible trace is a process that cannot be
//! checked on, cannot be quit without a task manager, and — when the global
//! hotkey is taken by something else, which is the single most common way this
//! goes wrong — gives no sign that it is running at all.
//!
//! The tray icon is the answer to "is it on?", and the way out.
//!
//! # Why the commands are an enum and not callbacks
//!
//! Every one of these is something the launcher can already be told to do from
//! the command line and over the socket. Modelling them as values means the
//! tray menu, `lumen settings` in a terminal, and "Configure Shortcuts" in the
//! list all arrive at the same code by the same route, in the same queue, in
//! one order.

use std::fmt;
use std::sync::Arc;

/// What the user picked from the tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    /// Show the launcher if hidden, hide it if shown. What a left click does.
    Toggle,
    Settings,
    /// Re-read everything editable by hand: sheets, scripts, snippets, files.
    Reload,
    Quit,
}

/// Where tray commands are delivered.
///
/// Like [`GestureSink`](crate::GestureSink), the closure behind this runs on a
/// platform thread pumping a message loop. It pushes and returns; it does not
/// do the work.
#[derive(Clone)]
pub struct TraySink(Arc<dyn Fn(TrayCommand) + Send + Sync>);

impl TraySink {
    pub fn new(f: impl Fn(TrayCommand) + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    #[inline]
    pub fn emit(&self, command: TrayCommand) {
        (self.0)(command)
    }
}

impl fmt::Debug for TraySink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TraySink")
    }
}

/// What the tray icon says about itself.
///
/// Carried across the boundary rather than built on the far side, because the
/// hotkey in the tooltip is configuration the platform layer has no business
/// knowing about — and a tooltip that says `Alt+Space` when the user has bound
/// something else is worse than no tooltip.
#[derive(Debug, Clone)]
pub struct TrayInfo {
    /// The tooltip. Windows truncates this at 127 characters.
    pub tooltip: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sink_delivers_what_it_is_given() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));

        let sink = {
            let seen = seen.clone();
            TraySink::new(move |command| seen.lock().unwrap().push(command))
        };

        sink.emit(TrayCommand::Toggle);
        sink.emit(TrayCommand::Quit);

        assert_eq!(
            *seen.lock().unwrap(),
            [TrayCommand::Toggle, TrayCommand::Quit]
        );
    }
}
