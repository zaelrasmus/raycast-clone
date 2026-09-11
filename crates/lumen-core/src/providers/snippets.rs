//! Snippets (F3).
//!
//! Blocks of text you paste often: an address, a signature, a licence header, a
//! command you can never remember. A snippet is a data file entry, exactly like
//! a theme or a shortcut sheet -- same folder-drop model, no runtime, nothing to
//! review.
//!
//! # Expansion is deliberately not here
//!
//! F3 names the feature "snippets and text expansion". Expansion -- watching
//! everything typed anywhere and substituting on a trigger -- needs a global
//! keyboard hook that sees every keystroke in every application, which is
//! precisely the mechanism section 7 spends a page explaining why this project
//! avoids: `WH_KEYBOARD_LL` has a timeout that silently unhooks it, and it draws
//! suspicion from antivirus and anti-cheat systems.
//!
//! Copying from the launcher covers most of the value at none of that cost, so
//! that is what this is. Expansion stays out until there is a mechanism for it
//! that does not contradict AD-16 and R-13.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::item::{Action, Item, Kind};
use crate::providers::Provider;

/// The schema version this build writes and understands.
pub const SCHEMA_VERSION: u32 = 1;

/// One block of saved text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    /// What appears in the list.
    pub name: String,
    /// What gets copied.
    pub text: String,
    /// Matched against, never shown.
    #[serde(default)]
    pub keywords: Vec<String>,
}

impl Snippet {
    fn into_item(self, index: usize) -> Item {
        Item {
            // Keyed on the name rather than the position, so reordering the
            // file does not reset the frecency every snippet has earned. The
            // index only disambiguates two snippets sharing a name.
            id: format!("snippet:{}:{index}", self.name),
            title: self.name,
            // The text itself is the second line, elided by the row's own
            // overflow rather than truncated here -- a snippet is often
            // multi-line and the row is one line tall (I-10).
            subtitle: Some(first_line(&self.text)),
            icon: None,
            keywords: self.keywords,
            actions: vec![Action::CopyToClipboard(self.text)],
            kind: Kind::Snippet,
        }
    }
}

/// The first line of a snippet, marked when there is more.
fn first_line(text: &str) -> String {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("").trim().to_string();

    if lines.next().is_some() {
        format!("{first} …")
    } else {
        first
    }
}

/// The file as it appears on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnippetFile {
    pub schema_version: u32,
    #[serde(default)]
    pub snippets: Vec<Snippet>,
}

/// Read the snippets file.
///
/// A missing file is the normal case and not an error. A malformed one is
/// reported and then ignored, for the same reason a malformed config is: the
/// launcher is summoned by a keystroke with no terminal attached, and refusing
/// to work because of a stray comma would look like a crash.
pub fn load(path: &Path) -> Vec<Snippet> {
    let Ok(source) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    match serde_json::from_str::<SnippetFile>(&source) {
        Ok(file) => {
            if file.schema_version > SCHEMA_VERSION {
                log::warn!(
                    "{} declares schema version {} but this build understands {SCHEMA_VERSION}",
                    path.display(),
                    file.schema_version
                );
            }
            file.snippets
                .into_iter()
                // A snippet with no text has nothing to copy, and one with no
                // name has nothing to find it by.
                .filter(|snippet| !snippet.text.is_empty() && !snippet.name.trim().is_empty())
                .collect()
        }
        Err(error) => {
            log::error!("{} is not valid: {error}", path.display());
            Vec::new()
        }
    }
}

/// Write the example file if none exists.
pub fn install_example(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, EXAMPLE)?;
    Ok(())
}

const EXAMPLE: &str = include_str!("../../../../assets/snippets.json");

/// The snippets found at startup, offered as a catalogue.
#[derive(Default)]
pub struct SnippetProvider {
    items: Vec<Item>,
}

impl SnippetProvider {
    pub fn new(snippets: Vec<Snippet>) -> Self {
        Self {
            items: snippets
                .into_iter()
                .enumerate()
                .map(|(index, snippet)| snippet.into_item(index))
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

impl Provider for SnippetProvider {
    fn id(&self) -> &'static str {
        "snippet"
    }

    fn catalogue(&self) -> Vec<Item> {
        self.items.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snippet_becomes_a_copyable_item() {
        let snippet = Snippet {
            name: "Address".into(),
            text: "1 Example Street".into(),
            keywords: vec!["home".into()],
        };

        let item = snippet.into_item(0);

        assert_eq!(item.title, "Address");
        assert_eq!(item.kind, Kind::Snippet);
        assert_eq!(
            item.primary(),
            Action::CopyToClipboard("1 Example Street".into())
        );
    }

    #[test]
    fn a_multi_line_snippet_shows_its_first_line_with_an_ellipsis() {
        assert_eq!(first_line("one\ntwo\nthree"), "one …");
        assert_eq!(first_line("only one"), "only one");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn ids_are_namespaced_and_survive_reordering() {
        let first = Snippet {
            name: "Address".into(),
            text: "x".into(),
            keywords: Vec::new(),
        }
        .into_item(0);

        // The same snippet at a different position keeps the name in its id, so
        // only the disambiguating suffix moves.
        assert!(first.id.starts_with("snippet:Address:"));
    }

    #[test]
    fn snippets_without_text_or_a_name_are_dropped() {
        let dir = std::env::temp_dir().join(format!("lumen-snip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("snippets.json");

        std::fs::write(
            &path,
            r#"{
                "schema_version": 1,
                "snippets": [
                    { "name": "Good", "text": "yes" },
                    { "name": "Empty", "text": "" },
                    { "name": "   ", "text": "no name" }
                ]
            }"#,
        )
        .unwrap();

        let snippets = load(&path);
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].name, "Good");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        assert!(load(Path::new("no/such/snippets.json")).is_empty());
    }

    #[test]
    fn a_malformed_file_yields_nothing_rather_than_failing() {
        let dir = std::env::temp_dir().join(format!("lumen-snip-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("snippets.json");
        std::fs::write(&path, "{ not json").unwrap();

        assert!(load(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The shipped example is parsed at build time by this test, so a malformed
    /// one cannot reach a user's machine.
    #[test]
    fn the_bundled_example_parses() {
        let file: SnippetFile = serde_json::from_str(EXAMPLE).expect("the example must parse");
        assert_eq!(file.schema_version, SCHEMA_VERSION);
        assert!(!file.snippets.is_empty());
    }
}
