//! The one result type the UI knows how to draw.
//!
//! Providers (F3) and, later, extensions (F4) all produce this. Keeping the
//! result shape closed rather than letting each source define its own is the
//! same reasoning as AD-07: consistency is obtained by construction, not by
//! asking every contributor to be disciplined.

use lumen_platform::{AppEntry, IconSource, LaunchTarget};

/// A single row in the result list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Stable across reindexes: this is the frecency key.
    pub id: String,
    pub title: String,
    /// The secondary line. Reserved space is allocated whether or not it is
    /// present, so that a row with a subtitle and one without are the same
    /// height (I-10).
    pub subtitle: Option<String>,
    pub icon: Option<IconSource>,
    /// Matched against, never displayed.
    pub keywords: Vec<String>,
    /// What this row can do, most important first.
    ///
    /// A list rather than a single action because F2 requires every action to
    /// be reachable from the keyboard, which means there has to be somewhere
    /// for the secondary ones to live. `Enter` runs the first; the action panel
    /// shows all of them.
    ///
    /// Never empty -- see [`Item::primary`].
    pub actions: Vec<Action>,
    /// The right-hand label describing what kind of thing this row is.
    ///
    /// Drawn from a small fixed vocabulary rather than computed per item, so
    /// the column never changes width between keystrokes (I-10).
    pub kind: Kind,
}

/// What sort of result a row is, for the label on its right-hand edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Application,
    Command,
    Calculation,
    Script,
    Snippet,
    Clipboard,
    Window,
    File,
    Extension,
    Quicklink,
    /// A URL typed into the search field (F6).
    Website,
    /// A timer to start, or one of the commands about them (F8).
    Timer,
    /// An amount converted into another currency, in the root list (F9).
    Currency,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Application => "Application",
            Kind::Command => "Command",
            Kind::Calculation => "Calculator",
            Kind::Script => "Script",
            Kind::Snippet => "Snippet",
            Kind::Clipboard => "Clipboard",
            Kind::Window => "Window",
            Kind::File => "File",
            Kind::Extension => "Extension",
            Kind::Quicklink => "Quicklink",
            Kind::Website => "Website",
            Kind::Timer => "Timer",
            Kind::Currency => "Currency",
        }
    }
}

/// What activating a row does.
///
/// An enum rather than a boxed closure: actions have to be comparable, cheap to
/// clone, and eventually serialisable across the extension boundary (AD-06).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Open it, raising a copy that is already running rather than starting a
    /// second one.
    Launch(LaunchTarget),
    /// Start another copy, whatever is already running.
    ///
    /// A separate action rather than a modifier on the first, because it is a
    /// separate *intention* and the action panel is where intentions live. An
    /// editor is the case that needs it: one window per project is normal, and
    /// "open Zed" cannot mean both things.
    LaunchNew(LaunchTarget),
    CopyToClipboard(String),
    /// Something Lumen does to itself. Carried back to the daemon as an event
    /// rather than performed in the view, because opening a window or quitting
    /// is the shell's business and not the list's.
    Builtin(Builtin),
    /// Arrange the window the user was looking at when they opened the
    /// launcher.
    ///
    /// Carried back to the daemon like a [`Builtin`] rather than performed
    /// here, and for a sharper reason than that one: the target is the window
    /// that held the foreground *before* the launcher appeared, and only the
    /// daemon still has that handle. Asking the system which window is in
    /// front at the moment the action runs would be a race, and losing it
    /// means resizing a window the user did not choose.
    ArrangeWindow(lumen_platform::WindowLayout),
    /// Replace the launcher's list with an extension's view (F4).
    ///
    /// Handled by the launcher itself rather than carried back to the daemon
    /// like a [`Builtin`]: a view takes over the list's own region, which is
    /// the list's business and not the shell's. Today the only source is the
    /// example compiled into the binary; when there are extensions it is
    /// whichever one's command was run.
    OpenView(ViewSource),
    /// Run a tier-T1 script command.
    ///
    /// Execution goes through `trait Platform` rather than `std::process` here,
    /// because AD-09 draws the boundary at the host: every capability an
    /// extension needs is requested from it, and T1 is the first thing to need
    /// execution. Drawing the line now means adding a sandbox later changes the
    /// runtime rather than redesigning the system.
    RunScript(crate::providers::ScriptRun),
    /// Open a quicklink (F6).
    ///
    /// The query is already in it; the clipboard and the browser are looked
    /// up when it runs, off the UI thread.
    OpenLink(crate::providers::quicklinks::OpenLink),
    /// Ask for what a quicklink's `{query}` should be (F6).
    ///
    /// The launcher's to carry out, like `OpenView`: it turns the search field
    /// into the link's argument, with the link's name beside it, and nothing
    /// is opened until the user presses Enter on what they typed.
    AskArgument(crate::providers::quicklinks::ArgumentRequest),
    /// Pick a browser, then open the link in it (F6).
    ///
    /// The launcher's: the browser list is the platform's, read when the
    /// action panel asks for it, and the choice is made in the panel.
    ChooseBrowser(crate::providers::quicklinks::OpenLink),
    /// Show the quicklink form filled in with `name` and `link` (F6), for a
    /// typed URL worth keeping. The launcher's, like `CreateQuicklink`.
    SaveAsQuicklink {
        name: String,
        link: String,
    },
    /// Show the form that saves a new quicklink (F6).
    ///
    /// The launcher's, like `OpenView`: the form takes the list's place.
    CreateQuicklink,
    /// Show the form that saves a new snippet (F11). The launcher's, like
    /// `CreateQuicklink`.
    CreateSnippet,
    /// Show the timer preset form in place of the list (F8): empty to create
    /// one, filled in to save a timer as one, or editing the preset on disk
    /// called `replacing`. The launcher's, like `CreateQuicklink`.
    OpenTimerForm {
        preset: crate::timers::PresetConfig,
        replacing: Option<String>,
    },
    /// Start or stop a timer (F8).
    ///
    /// Carried back to the daemon like a [`Builtin`]: the timers, the one
    /// sleep until the next of them ends and the file they are kept in are
    /// all the daemon's, and the list only asks.
    Timer(crate::timers::Command),
}

/// Commands Lumen provides for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    OpenSettings,
    ReloadSheets,
    Quit,
}

/// Where a view comes from.
///
/// One variant, and it is a variant rather than a bare unit type because the
/// next one is an extension identifier and the shape of the call site should
/// not have to change when it arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewSource {
    /// The hand-written list compiled into the binary (F4, step 1).
    ///
    /// The plan's build order starts here on purpose: "Render it in GPUI from a
    /// static file. No protocol, no processes." A renderer proved against a
    /// file is a renderer nothing can blame on a transport.
    Example,
    /// A command declared by an installed extension (F4, step 3).
    ///
    /// The folder rather than a name, because the folder is the identity and
    /// it is also the working directory the process is started in.
    Command {
        extension: std::path::PathBuf,
        command: String,
    },
    /// The translate view (F7), on a preset by name, or on the one used last.
    Translate { preset: Option<String> },
    /// The running timers (F8).
    Timers,
    /// The system's alert sounds, to hear and choose from (F8, slice 3).
    AlertSounds,
    /// The currency converter (F9).
    Currency,
    /// The dictionary (F10).
    Dictionary,
    /// *Search Snippets* (F11).
    Snippets,
    /// *Clipboard History* (F12).
    Clipboard,
    /// *World Clock* (F13).
    Clock,
}

impl Action {
    /// The verb shown in the footer and the action panel.
    pub fn verb(&self) -> &'static str {
        match self {
            Action::Launch(_) => "Open",
            Action::LaunchNew(_) => "Open New Window",
            Action::CopyToClipboard(_) => "Copy",
            Action::Builtin(Builtin::OpenSettings) => "Open Settings",
            Action::Builtin(Builtin::ReloadSheets) => "Reload",
            Action::Builtin(Builtin::Quit) => "Quit",
            Action::ArrangeWindow(_) => "Arrange",
            Action::OpenView(_) => "Open",
            Action::RunScript(_) => "Run",
            Action::OpenLink(open) if open.private => "Open in Private Window",
            Action::OpenLink(_) => "Open",
            Action::ChooseBrowser(_) => "Open With\u{2026}",
            Action::AskArgument(_) => "Search",
            Action::CreateQuicklink | Action::CreateSnippet => "Create",
            Action::SaveAsQuicklink { .. } => "Save as Quicklink",
            Action::OpenTimerForm {
                replacing: Some(_), ..
            } => "Edit Preset",
            Action::OpenTimerForm { preset, .. } if preset.duration.is_empty() => "Create",
            Action::OpenTimerForm { .. } => "Save as Preset",
            Action::Timer(crate::timers::Command::Start { focus: true, .. })
            | Action::Timer(crate::timers::Command::StartCycle { focus: true, .. }) => {
                "Start with Focus"
            }
            Action::Timer(crate::timers::Command::Start { .. }) => "Start",
            Action::Timer(crate::timers::Command::StopNearest) => "Stop",
            Action::Timer(crate::timers::Command::DismissRinging) => "Dismiss",
            Action::Timer(crate::timers::Command::StartStopwatch { .. }) => "Start",
            Action::Timer(crate::timers::Command::StartCycle { plan, .. }) if plan.repeat => {
                "Start Repeating"
            }
            Action::Timer(crate::timers::Command::StartCycle { .. }) => "Start One Set",
        }
    }

    /// Carry out an action that needs nothing but the platform.
    ///
    /// Blocking -- a launch or a script can take a while -- so it is called off
    /// the UI thread, after the launcher has been dismissed.
    ///
    /// The other three are refused rather than silently ignored, because each
    /// needs something only its caller holds: a [`Builtin`] opens windows or
    /// stops the process (the daemon's business), `ArrangeWindow` needs the
    /// window captured before the launcher appeared (7.15), and `OpenView`
    /// replaces the list the launcher is drawing.
    pub fn perform(&self, platform: &dyn lumen_platform::Platform) -> anyhow::Result<()> {
        use lumen_platform::LaunchMode;

        match self {
            Action::Launch(target) => platform.launch(target, LaunchMode::Reuse),
            Action::LaunchNew(target) => platform.launch(target, LaunchMode::Fresh),
            Action::CopyToClipboard(text) => platform.set_clipboard_text(text),
            Action::RunScript(run) => run.run(platform),
            Action::OpenLink(open) => open.run(platform),
            Action::Builtin(_)
            | Action::ArrangeWindow(_)
            | Action::OpenView(_)
            | Action::AskArgument(_)
            | Action::ChooseBrowser(_)
            | Action::SaveAsQuicklink { .. }
            | Action::CreateQuicklink
            | Action::CreateSnippet
            | Action::OpenTimerForm { .. }
            | Action::Timer(_) => {
                anyhow::bail!("{self:?} is carried out by the launcher's owner, not the platform")
            }
        }
    }
}

/// What to tell the user when `action` failed after the launcher had closed:
/// a title naming what was attempted, and the whole chain of causes.
///
/// The launcher is gone by then -- an action that launches something hands
/// the foreground away -- so this goes to a system notification rather than
/// a line in a window nobody is looking at.
pub fn failure_notice(action: &Action, error: &anyhow::Error) -> (String, String) {
    let verb = action.verb().trim_end_matches('\u{2026}').to_lowercase();
    (format!("Couldn't {verb}"), format!("{error:#}"))
}

impl Item {
    pub fn from_app(entry: AppEntry) -> Self {
        Self {
            id: entry.id,
            title: entry.name.clone(),
            subtitle: entry.detail,
            icon: entry.icon,
            keywords: entry.keywords,
            actions: vec![
                Action::Launch(entry.target.clone()),
                // Second, so Ctrl+K reaches it and Enter does not. An editor
                // legitimately wants a window per project; everything else
                // wants the window it already has.
                Action::LaunchNew(entry.target),
                // Copying the name is the one thing people reliably want from
                // an application row besides opening it.
                Action::CopyToClipboard(entry.name),
            ],
            kind: Kind::Application,
        }
    }

    /// The action `Enter` runs.
    ///
    /// Falls back to a copy of the title rather than returning an `Option`: a
    /// row with no action at all is a bug in whichever provider produced it,
    /// and the useful response at the point of use is to do something harmless
    /// rather than to make every call site handle a case that should not exist.
    pub fn primary(&self) -> Action {
        self.actions
            .first()
            .cloned()
            .unwrap_or_else(|| Action::CopyToClipboard(self.title.clone()))
    }

    pub fn kind_label(&self) -> &'static str {
        self.kind.label()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the user reads when an action fails after the launcher closed:
    /// what was attempted, then why, with the whole chain of causes.
    #[test]
    fn a_failed_action_is_described_in_the_users_terms() {
        let action = Action::CopyToClipboard("x".into());
        let error = anyhow::anyhow!("the clipboard is locked").context("writing the clipboard");

        let (title, body) = failure_notice(&action, &error);

        assert_eq!(title, "Couldn't copy");
        assert_eq!(body, "writing the clipboard: the clipboard is locked");
    }

    #[test]
    fn a_multi_word_verb_reads_as_a_sentence() {
        let open = crate::providers::quicklinks::OpenLink {
            link: crate::providers::quicklinks::Template::parse("https://x.com").unwrap(),
            query: String::new(),
            browser: None,
            private: true,
        };
        let (title, _) = failure_notice(&Action::OpenLink(open), &anyhow::anyhow!("no"));
        assert_eq!(title, "Couldn't open in private window");
    }

    #[test]
    fn an_app_entry_becomes_a_launchable_item() {
        let entry = AppEntry {
            id: "app:Firefox.lnk".into(),
            name: "Firefox".into(),
            detail: None,
            target: LaunchTarget::ShellExecute("shell:AppsFolder\\Firefox.lnk".into()),
            icon: None,
            keywords: vec!["browser".into()],
        };

        let item = Item::from_app(entry);

        assert_eq!(item.title, "Firefox");
        assert_eq!(item.kind_label(), "Application");
        assert_eq!(item.primary().verb(), "Open");
        // The id must survive the conversion, or frecency resets on reindex.
        assert_eq!(item.id, "app:Firefox.lnk");
    }

    /// A provider that forgets to give a row an action must not make `Enter`
    /// panic or do something destructive.
    #[test]
    fn an_item_with_no_actions_still_has_a_harmless_primary() {
        let item = Item {
            id: "broken".into(),
            title: "Broken".into(),
            subtitle: None,
            icon: None,
            keywords: Vec::new(),
            actions: Vec::new(),
            kind: Kind::Command,
        };

        assert_eq!(item.primary(), Action::CopyToClipboard("Broken".into()));
    }

    #[test]
    fn every_kind_has_a_label() {
        for kind in [
            Kind::Application,
            Kind::Command,
            Kind::Calculation,
            Kind::Script,
            Kind::Snippet,
            Kind::Clipboard,
            Kind::Quicklink,
            Kind::Timer,
        ] {
            assert!(!kind.label().is_empty());
        }
    }
}
