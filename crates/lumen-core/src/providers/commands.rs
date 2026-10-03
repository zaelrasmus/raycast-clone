//! Lumen's own commands.
//!
//! The launcher launches things, including its own settings. Making these
//! ordinary rows rather than a separate menu means they are searchable, ranked
//! by use like anything else, and reachable without the user learning a second
//! way to reach things.

use crate::item::{Action, Builtin, Item, Kind, ViewSource};
use crate::providers::Provider;

pub struct CommandsProvider;

/// The table, so adding a command is one line and never two.
const COMMANDS: &[(&str, &str, &str, Builtin)] = &[
    (
        "settings",
        "Configure Shortcuts",
        "Edit the sheets the hold-gesture overlay shows",
        Builtin::OpenSettings,
    ),
    (
        "reload",
        "Reload Configuration",
        "Re-read sheets, scripts and snippets from disk",
        Builtin::ReloadSheets,
    ),
    (
        "rebuild-file-index",
        "Rebuild File Index",
        "Forget the indexed files and folders and look through every scope again",
        Builtin::RebuildFileIndex,
    ),
    ("quit", "Quit Lumen", "Stop the daemon", Builtin::Quit),
];

impl Provider for CommandsProvider {
    fn id(&self) -> &'static str {
        "command"
    }

    // A catalogue, not an answer: these rows exist whether or not anyone is
    // searching, and the matcher decides which of them a query surfaces.
    // Filtering here as well would mean two different notions of "matches",
    // and the one in this file would be the worse of them.
    fn catalogue(&self) -> Vec<Item> {
        COMMANDS
            .iter()
            .map(item)
            .chain([create_quicklink(), example_view()])
            .collect()
    }
}

/// The command that saves a new quicklink (F6).
fn create_quicklink() -> Item {
    Item {
        id: "command:create-quicklink".into(),
        title: "Create Quicklink".into(),
        subtitle: Some("Save a link, or a search with {query} in it".into()),
        icon: None,
        keywords: vec![
            "link".into(),
            "bookmark".into(),
            "url".into(),
            "quicklink".into(),
            "new".into(),
        ],
        actions: vec![Action::CreateQuicklink],
        kind: Kind::Command,
    }
}

/// The command that opens F4's hand-written example view.
///
/// A real row in the real list rather than a hidden developer flag, because the
/// point of step 1 is to look at the thing in the interface it will live in.
/// It goes away when there are extensions to open instead -- and the note in
/// its subtitle is there so that whoever finds it knows that.
fn example_view() -> Item {
    Item {
        id: "command:example-view".into(),
        title: "Example Extension View".into(),
        subtitle: Some("A hand-written view, for developing the renderer".into()),
        icon: None,
        keywords: vec!["extension".into(), "view".into(), "developer".into()],
        actions: vec![Action::OpenView(ViewSource::Example)],
        kind: Kind::Command,
    }
}

fn item(&(id, title, subtitle, builtin): &(&str, &str, &str, Builtin)) -> Item {
    Item {
        id: format!("command:{id}"),
        title: title.to_string(),
        subtitle: Some(subtitle.to_string()),
        icon: None,
        keywords: vec![
            "lumen".into(),
            "settings".into(),
            "preferences".into(),
            "shortcuts".into(),
        ],
        actions: vec![Action::Builtin(builtin)],
        kind: Kind::Command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_its_commands_in_the_catalogue() {
        let items = CommandsProvider.catalogue();
        // The table, plus the two that open something in place of the list.
        assert_eq!(items.len(), COMMANDS.len() + 2);
        assert!(items.iter().all(|item| item.kind == Kind::Command));
    }

    #[test]
    fn creating_a_quicklink_is_reachable_from_the_launcher() {
        let items = CommandsProvider.catalogue();
        let create = items
            .iter()
            .find(|item| item.title == "Create Quicklink")
            .expect("the form has a row");

        assert_eq!(create.primary(), Action::CreateQuicklink);
    }

    #[test]
    fn the_example_view_is_reachable_from_the_launcher() {
        let items = CommandsProvider.catalogue();
        let example = items
            .iter()
            .find(|item| item.title == "Example Extension View")
            .expect("the example view has a row");

        assert_eq!(example.primary(), Action::OpenView(ViewSource::Example));
    }

    /// The answer to "a file is missing": a row in root, as the other
    /// commands are (F16).
    #[test]
    fn rebuilding_the_file_index_is_reachable_from_the_launcher() {
        let items = CommandsProvider.catalogue();
        let rebuild = items
            .iter()
            .find(|item| item.id == "command:rebuild-file-index")
            .expect("rebuilding the file index has a row");

        assert_eq!(rebuild.title, "Rebuild File Index");
        assert_eq!(
            rebuild.primary(),
            Action::Builtin(Builtin::RebuildFileIndex)
        );
    }

    #[test]
    fn settings_is_reachable_and_carries_its_command() {
        let items = CommandsProvider.catalogue();
        let settings = items
            .iter()
            .find(|item| item.id == "command:settings")
            .expect("settings must be offered");

        assert_eq!(settings.primary(), Action::Builtin(Builtin::OpenSettings));
    }

    /// Ids are namespaced so a command can never collide with an application in
    /// the frecency store.
    #[test]
    fn ids_are_namespaced() {
        assert!(
            CommandsProvider
                .catalogue()
                .iter()
                .all(|item| item.id.starts_with("command:"))
        );
    }

    /// Commands are a catalogue, so they contribute nothing as answers: the
    /// matcher is what decides which of them a query surfaces.
    #[test]
    fn commands_contribute_no_answers() {
        assert!(CommandsProvider.answer("set").is_empty());
    }
}
