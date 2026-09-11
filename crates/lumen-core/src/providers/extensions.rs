//! Installed extensions' commands, as rows in the root list (F4, AD-08).
//!
//! A catalogue provider, and the plainest one there is: the manifests have
//! already been read, and this turns each declared command into a row. No
//! process is started to build it, which is the whole of AD-08 and what keeps
//! I-3 keepable.

use crate::extension::Extension;
use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;

pub struct ExtensionProvider {
    items: Vec<Item>,
}

impl ExtensionProvider {
    pub fn new(extensions: &[Extension]) -> Self {
        Self {
            items: extensions
                .iter()
                .flat_map(|extension| {
                    extension
                        .manifest
                        .commands
                        .iter()
                        .map(|command| item(extension, command))
                })
                .collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Provider for ExtensionProvider {
    fn id(&self) -> &'static str {
        "extension"
    }

    fn catalogue(&self) -> Vec<Item> {
        self.items.clone()
    }
}

fn item(extension: &Extension, command: &crate::extension::Command) -> Item {
    Item {
        // Namespaced by the folder, which is the extension's identity, so that
        // two extensions offering a `search` command do not share a frecency
        // entry and take turns being ranked for each other's use.
        id: format!("extension:{}:{}", extension.id(), command.id),
        title: command.title.clone(),
        // The extension's name, because "Search" on its own says nothing about
        // which of the installed extensions is about to be started.
        subtitle: command
            .subtitle
            .clone()
            .or_else(|| Some(extension.manifest.name.clone())),
        // Left to the row's kind for now: a manifest may name a bundled glyph,
        // but resolving an extension's *own* image means reading a file the
        // host has not seen, which is a host-RPC decision rather than an
        // indexing one.
        icon: None,
        keywords: command
            .keywords
            .iter()
            .cloned()
            .chain([extension.manifest.name.clone()])
            .collect(),
        actions: vec![Action::OpenView(ViewSource::Command {
            extension: extension.directory.clone(),
            command: command.id.clone(),
        })],
        kind: Kind::Extension,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::{Command, Manifest};
    use std::path::PathBuf;

    fn extension(folder: &str, name: &str, commands: &[(&str, &str)]) -> Extension {
        Extension {
            directory: PathBuf::from("C:/extensions").join(folder),
            manifest: Manifest {
                schema_version: 1,
                name: name.into(),
                description: None,
                run: vec!["powershell".into()],
                commands: commands
                    .iter()
                    .map(|(id, title)| Command {
                        id: (*id).into(),
                        title: (*title).into(),
                        ..Default::default()
                    })
                    .collect(),
            },
        }
    }

    #[test]
    fn every_declared_command_becomes_a_row() {
        let items = ExtensionProvider::new(&[
            extension("github", "GitHub", &[("prs", "Pull Requests")]),
            extension(
                "notes",
                "Notes",
                &[("new", "New Note"), ("find", "Find Note")],
            ),
        ])
        .catalogue();

        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|item| item.kind == Kind::Extension));
    }

    /// Two extensions with a command of the same name must not share an id, or
    /// they share a frecency entry and take turns being ranked for each other.
    #[test]
    fn ids_are_namespaced_by_the_extension() {
        let items = ExtensionProvider::new(&[
            extension("github", "GitHub", &[("search", "Search")]),
            extension("notes", "Notes", &[("search", "Search")]),
        ])
        .catalogue();

        assert_eq!(items[0].id, "extension:github:search");
        assert_eq!(items[1].id, "extension:notes:search");
    }

    /// "Search" on its own says nothing about which extension is about to run.
    #[test]
    fn a_command_without_a_subtitle_borrows_the_extension_name() {
        let items = ExtensionProvider::new(&[extension("github", "GitHub", &[("s", "Search")])])
            .catalogue();

        assert_eq!(items[0].subtitle.as_deref(), Some("GitHub"));
    }

    #[test]
    fn running_a_command_opens_its_view() {
        let items =
            ExtensionProvider::new(&[extension("github", "GitHub", &[("prs", "PRs")])]).catalogue();

        assert_eq!(
            items[0].primary(),
            Action::OpenView(ViewSource::Command {
                extension: PathBuf::from("C:/extensions/github"),
                command: "prs".into(),
            })
        );
    }

    #[test]
    fn an_extension_with_no_commands_contributes_nothing() {
        assert!(ExtensionProvider::new(&[extension("empty", "Empty", &[])]).is_empty());
    }
}
