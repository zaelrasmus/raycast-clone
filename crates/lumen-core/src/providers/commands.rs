//! Lumen's own commands.
//!
//! The launcher launches things, including its own settings. Making these
//! ordinary rows rather than a separate menu means they are searchable, ranked
//! by use like anything else, and reachable without the user learning a second
//! way to reach things.

use crate::item::{Action, Builtin, Item, Kind};
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
        COMMANDS.iter().map(item).collect()
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
        assert_eq!(items.len(), COMMANDS.len());
        assert!(items.iter().all(|item| item.kind == Kind::Command));
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
