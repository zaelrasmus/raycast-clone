//! Installed extensions, and what the launcher knows about them (F4, AD-08).
//!
//! # Nothing here starts a process
//!
//! AD-08: "Static manifest indexed at install time. With zero extension
//! processes running, commands appear because they are in the local index."
//!
//! That is what makes I-3 keepable -- root search never waits on an extension
//! process, and there is no exception and no "short" timeout -- and it is the
//! single decision that stops an extension system from making a launcher slow.
//! Twenty installed extensions cost twenty JSON files read once, not twenty
//! processes resident.
//!
//! A process is started when the user runs one of its commands, and not before.
//!
//! # The manifest is deliberately dull
//!
//! It says what commands exist and how to start the thing, and nothing else.
//! There is no permission list yet, no settings schema, no dependency
//! declaration. Each of those is a real feature with a real design behind it,
//! and a field added now in anticipation is a field that will be wrong.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::view::Icon;

/// The file that makes a folder an extension.
pub const MANIFEST: &str = "extension.json";

/// The schema this build understands.
///
/// A manifest declaring a *later* version is skipped with a log line rather
/// than parsed optimistically: a theme missing a token can fall back (I-12),
/// but half an understanding of how to run a process is not a safe thing to
/// act on.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Manifest {
    pub schema_version: u32,
    /// What the user sees. The folder name is the identity.
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// How to start it: the program, then its arguments.
    ///
    /// A list rather than a shell string on purpose. A string has to be split,
    /// splitting has quoting rules, and quoting rules are how an argument
    /// containing a space becomes two arguments on somebody else's machine.
    pub run: Vec<String>,
    #[serde(default)]
    pub commands: Vec<Command>,
}

/// One command an extension offers, as the index sees it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Command {
    /// Sent to the extension when the command is run. Opaque to the host.
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<Icon>,
    pub keywords: Vec<String>,
}

/// An extension as it sits on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Extension {
    /// The folder, which is both the identity and the working directory the
    /// process is started in.
    pub directory: PathBuf,
    pub manifest: Manifest,
}

impl Extension {
    /// The stable identifier, which is the folder's name.
    ///
    /// The folder rather than anything inside the manifest: two extensions
    /// cannot share a folder, and a `name` field can be copied between them by
    /// anyone who starts from someone else's example.
    pub fn id(&self) -> &str {
        self.directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
    }

    pub fn command(&self, id: &str) -> Option<&Command> {
        self.manifest
            .commands
            .iter()
            .find(|command| command.id == id)
    }
}

/// Read every extension in `directory`.
///
/// A folder that is not an extension, a manifest that will not parse, and a
/// manifest from a later schema are all skipped with a log line. One broken
/// extension must not stop the other nineteen from appearing -- and it must not
/// stop the launcher from starting either, which is the same reasoning as a
/// malformed config being reported rather than fatal.
pub fn scan(directory: &Path) -> Vec<Extension> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };

    let mut extensions = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest = path.join(MANIFEST);
        let Ok(source) = std::fs::read_to_string(&manifest) else {
            continue;
        };

        match serde_json::from_str::<Manifest>(&source) {
            Ok(manifest) if manifest.schema_version > SCHEMA_VERSION => {
                log::warn!(
                    "{} needs a newer Lumen (schema {} against {SCHEMA_VERSION})",
                    path.display(),
                    manifest.schema_version
                );
            }
            Ok(manifest) if manifest.run.is_empty() => {
                log::warn!("{} declares no way to run it", manifest_path(&path));
            }
            Ok(manifest) => extensions.push(Extension {
                directory: path,
                manifest,
            }),
            Err(error) => {
                log::warn!("{} is not a valid manifest: {error}", manifest_path(&path));
            }
        }
    }

    // Sorted so the index is the same on every start. An order that depends on
    // the filesystem is an order that changes when a folder is renamed, and a
    // root list that reshuffles for no visible reason is worse than one in an
    // arbitrary but fixed order.
    extensions.sort_by(|a, b| a.directory.cmp(&b.directory));
    extensions
}

fn manifest_path(directory: &Path) -> String {
    directory.join(MANIFEST).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("lumen-ext-{name}"));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn extension(&self, name: &str, manifest: &str) -> &Self {
            let dir = self.0.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(MANIFEST), manifest).unwrap();
            self
        }

        fn scan(&self) -> Vec<Extension> {
            scan(&self.0)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const GOOD: &str = r#"{
        "schema-version": 1,
        "name": "GitHub",
        "run": ["node", "main.js"],
        "commands": [{ "id": "prs", "title": "Pull Requests" }]
    }"#;

    #[test]
    fn an_extension_is_read_from_its_folder() {
        let folder = Folder::new("basic");
        folder.extension("github", GOOD);

        let found = folder.scan();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id(), "github");
        assert_eq!(found[0].manifest.name, "GitHub");
        assert_eq!(found[0].command("prs").unwrap().title, "Pull Requests");
        assert!(found[0].command("nothing").is_none());
    }

    /// One broken extension must not take the others with it.
    #[test]
    fn a_broken_manifest_is_skipped_and_the_rest_still_load() {
        let folder = Folder::new("broken");
        folder
            .extension("github", GOOD)
            .extension("bad", "{ not json")
            .extension(
                "empty-run",
                r#"{ "schema-version": 1, "name": "X", "run": [] }"#,
            );

        let found = folder.scan();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id(), "github");
    }

    /// A manifest from a later Lumen is declined rather than half-understood.
    #[test]
    fn a_newer_schema_is_declined() {
        let folder = Folder::new("newer");
        folder.extension(
            "future",
            r#"{ "schema-version": 99, "name": "Future", "run": ["x"] }"#,
        );

        assert!(folder.scan().is_empty());
    }

    #[test]
    fn a_folder_without_a_manifest_is_not_an_extension() {
        let folder = Folder::new("plain");
        std::fs::create_dir_all(folder.0.join("notes")).unwrap();

        assert!(folder.scan().is_empty());
    }

    #[test]
    fn a_missing_folder_is_not_an_error() {
        assert!(scan(Path::new("no/such/folder")).is_empty());
    }

    /// The order has to be the same on every start, or the root list reshuffles
    /// for reasons the user cannot see.
    #[test]
    fn the_scan_is_ordered() {
        let folder = Folder::new("order");
        folder
            .extension("zebra", GOOD)
            .extension("alpha", GOOD)
            .extension("middle", GOOD);

        let ids: Vec<String> = folder.scan().iter().map(|e| e.id().to_string()).collect();
        assert_eq!(ids, ["alpha", "middle", "zebra"]);
    }

    #[test]
    fn a_manifest_round_trips() {
        let manifest: Manifest = serde_json::from_str(GOOD).unwrap();
        let json = serde_json::to_string(&manifest).unwrap();
        assert_eq!(serde_json::from_str::<Manifest>(&json).unwrap(), manifest);
    }
}
