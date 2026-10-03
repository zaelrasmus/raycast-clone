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
    /// A folder from the file index (F16).
    Folder,
    Extension,
    Quicklink,
    /// A URL typed into the search field (F6).
    Website,
    /// A timer to start, or one of the commands about them (F8).
    Timer,
    /// An amount converted into another currency, in the root list (F9).
    Currency,
    /// A time somewhere, or converted, in the root list (F13).
    Clock,
    /// The weather somewhere, in the root list (F13).
    Weather,
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
            Kind::Folder => "Folder",
            Kind::Extension => "Extension",
            Kind::Quicklink => "Quicklink",
            Kind::Website => "Website",
            Kind::Timer => "Timer",
            Kind::Currency => "Currency",
            Kind::Clock => "Clock",
            Kind::Weather => "Weather",
        }
    }
}

/// What activating a row does.
///
/// An enum rather than a boxed closure: actions have to be comparable, cheap to
/// clone, and eventually serialisable across the extension boundary (AD-06).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Add the row to the favourites, take it out, or move it (F15).
    ///
    /// Never one of an item's own actions: the launcher offers it beside
    /// them, for any row of the index, and carries it out on the list it
    /// owns -- nothing is opened and the launcher stays.
    Favorite(crate::favorites::Change),
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
    /// Copy the calculator's answer: [`Action::CopyToClipboard`] under a name
    /// that says which of the answer's two copies it is.
    CopyAnswer(String),
    /// Copy what was typed into the calculator, beside its answer.
    CopyExpression(String),
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

/// A run of the action panel's rows, between two rules.
///
/// Declared in the order the panel shows them, so the derived ordering is the
/// panel's: what the row is for, then the other ways of doing it, then
/// copying, then changing what is saved, then the favourites. A small closed
/// set on purpose -- every row of every provider has to fall into one, and a
/// group with a single member in most menus is a rule drawn for nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActionGroup {
    /// What the row is for: what `Enter` runs.
    Primary,
    /// The same thing done another way: a new window, a private one, another
    /// browser.
    Alternate,
    /// Putting something on the clipboard.
    Copy,
    /// Making, editing or saving something Lumen keeps: a quicklink from a
    /// typed address, a timer preset.
    Manage,
    /// Adding the row to the favourites, or moving it among them (F15).
    Favorite,
}

/// Commands Lumen provides for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    OpenSettings,
    ReloadSheets,
    /// Throw the file index away and walk every scope again (F16): the
    /// answer to "something is wrong", not how the index stays current.
    RebuildFileIndex,
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
    /// *Weather* (F13), on a city's report by its key, or on the list.
    Weather { city: Option<String> },
    /// *Calculator History* (F14).
    CalculatorHistory,
}

impl Action {
    /// The verb shown in the footer and the action panel.
    pub fn verb(&self) -> &'static str {
        match self {
            Action::Favorite(change) => change.verb(),
            Action::Launch(_) => "Open Application",
            Action::LaunchNew(_) => "New Application Window",
            Action::CopyToClipboard(_) => "Copy",
            Action::CopyAnswer(_) => "Copy Answer",
            Action::CopyExpression(_) => "Copy Expression",
            Action::Builtin(Builtin::OpenSettings) => "Open Settings",
            Action::Builtin(Builtin::ReloadSheets) => "Reload",
            Action::Builtin(Builtin::RebuildFileIndex) => "Rebuild",
            Action::Builtin(Builtin::Quit) => "Quit",
            Action::ArrangeWindow(_) => "Arrange",
            Action::OpenView(_) => "Open Command",
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

    /// Which run of the action panel this action belongs to.
    ///
    /// The panel draws a rule wherever the group changes between two
    /// neighbouring rows, so the group is a property of the action rather
    /// than of the panel: the panel cannot be told the wrong thing about what
    /// kind of action it is drawing, and an extension's view, which has no
    /// groups of its own, is left as a single run.
    ///
    /// Decided by what the action *does*, not by where a provider happened
    /// to put it in its list.
    pub fn group(&self) -> ActionGroup {
        match self {
            Action::Favorite(_) => ActionGroup::Favorite,
            Action::LaunchNew(_) | Action::ChooseBrowser(_) => ActionGroup::Alternate,
            Action::OpenLink(open) if open.private => ActionGroup::Alternate,
            Action::CopyToClipboard(_) | Action::CopyAnswer(_) | Action::CopyExpression(_) => {
                ActionGroup::Copy
            }
            Action::SaveAsQuicklink { .. } => ActionGroup::Manage,
            // A preset's form is a creation when it starts empty, and
            // housekeeping when it edits or saves one that exists.
            Action::OpenTimerForm {
                replacing: Some(_), ..
            } => ActionGroup::Manage,
            Action::OpenTimerForm { preset, .. } if !preset.duration.is_empty() => {
                ActionGroup::Manage
            }
            Action::Launch(_)
            | Action::Builtin(_)
            | Action::ArrangeWindow(_)
            | Action::OpenView(_)
            | Action::RunScript(_)
            | Action::OpenLink(_)
            | Action::AskArgument(_)
            | Action::CreateQuicklink
            | Action::CreateSnippet
            | Action::OpenTimerForm { .. }
            | Action::Timer(_) => ActionGroup::Primary,
        }
    }

    /// The bundled glyph drawn beside the action in the panel, by the name an
    /// extension's [`Icon::Glyph`](crate::view::Icon) would use: `icons/{name}.svg`.
    ///
    /// A name rather than a path, so that `core` stays free of where the
    /// assets live (AD-03); the UI owns the table the names are checked
    /// against, and a test there fails if one is missing.
    pub fn glyph(&self) -> &'static str {
        use crate::favorites::Change;

        match self {
            Action::Favorite(Change::Add) => "star",
            Action::Favorite(Change::Remove) => "star-off",
            Action::Favorite(Change::MoveUp) => "arrow-up",
            Action::Favorite(Change::MoveDown) => "arrow-down",
            Action::Launch(_) | Action::OpenLink(_) => "open",
            Action::LaunchNew(_) | Action::ArrangeWindow(_) => "window",
            Action::CopyToClipboard(_) | Action::CopyAnswer(_) | Action::CopyExpression(_) => {
                "copy"
            }
            Action::Builtin(_) | Action::OpenView(_) => "command",
            Action::RunScript(_) => "script",
            Action::AskArgument(_) => "search",
            Action::ChooseBrowser(_) => "globe",
            Action::CreateQuicklink | Action::CreateSnippet | Action::SaveAsQuicklink { .. } => {
                "plus"
            }
            Action::OpenTimerForm { preset, replacing } => {
                if replacing.is_none() && preset.duration.is_empty() {
                    "plus"
                } else {
                    "timer"
                }
            }
            Action::Timer(_) => "timer",
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
            Action::CopyToClipboard(text)
            | Action::CopyAnswer(text)
            | Action::CopyExpression(text) => platform.set_clipboard_text(text),
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
            | Action::Favorite(_)
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

    /// Only the calculator's answer names what it copies; every other copy
    /// (a path, the clipboard history's entries) keeps the plain verb.
    #[test]
    fn only_the_calculators_copies_are_named() {
        assert_eq!(Action::CopyToClipboard("x".into()).verb(), "Copy");
        assert_eq!(Action::CopyAnswer("x".into()).verb(), "Copy Answer");
        assert_eq!(Action::CopyExpression("x".into()).verb(), "Copy Expression");
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
        assert_eq!(item.primary().verb(), "Open Application");
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

    fn firefox() -> Item {
        Item::from_app(AppEntry {
            id: "app:Firefox.lnk".into(),
            name: "Firefox".into(),
            detail: None,
            target: LaunchTarget::ShellExecute("shell:AppsFolder\\Firefox.lnk".into()),
            icon: None,
            keywords: Vec::new(),
        })
    }

    /// The verbs the action panel reads: what Raycast calls them, so the
    /// panel's first rows say what the footer will.
    #[test]
    fn the_first_actions_of_an_application_and_a_command_have_their_own_names() {
        let item = firefox();
        let verbs: Vec<_> = item.actions.iter().map(Action::verb).collect();
        assert_eq!(
            verbs,
            ["Open Application", "New Application Window", "Copy"]
        );
        assert_eq!(Action::OpenView(ViewSource::Example).verb(), "Open Command");
    }

    /// The panel draws a rule wherever the group changes, so an application's
    /// own actions have to arrive already in the order the groups are
    /// declared in: the ways of opening it, then the copy, then what can be
    /// done to it as a favourite.
    #[test]
    fn an_applications_actions_fall_into_groups_in_panel_order() {
        let item = firefox();
        let mut groups: Vec<_> = item.actions.iter().map(Action::group).collect();
        groups.push(Action::Favorite(crate::favorites::Change::Add).group());

        assert_eq!(
            groups,
            [
                ActionGroup::Primary,
                ActionGroup::Alternate,
                ActionGroup::Copy,
                ActionGroup::Favorite,
            ]
        );
        assert!(groups.is_sorted(), "declaration order is panel order");
    }

    /// A private window and a chosen browser are other ways of opening the
    /// link, not the way Enter opens it.
    #[test]
    fn a_private_window_is_an_alternate_and_the_plain_link_is_not() {
        let link = |private| {
            Action::OpenLink(crate::providers::quicklinks::OpenLink {
                link: crate::providers::quicklinks::Template::parse("https://x.com").unwrap(),
                query: String::new(),
                browser: None,
                private,
            })
        };
        assert_eq!(link(false).group(), ActionGroup::Primary);
        assert_eq!(link(true).group(), ActionGroup::Alternate);
    }

    /// Creating something from nothing is what the row is for; saving what
    /// is already there as a new thing is housekeeping, set apart.
    #[test]
    fn saving_what_exists_is_managing_it() {
        let save = Action::SaveAsQuicklink {
            name: "x".into(),
            link: "https://x.com".into(),
        };
        assert_eq!(save.group(), ActionGroup::Manage);
        assert_eq!(Action::CreateQuicklink.group(), ActionGroup::Primary);
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
            Kind::File,
            Kind::Folder,
        ] {
            assert!(!kind.label().is_empty());
        }
    }
}
