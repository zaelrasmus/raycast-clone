//! Snippets (F3, F11).
//!
//! Blocks of text you paste often: an address, a signature, a licence header, a
//! command you can never remember. A snippet is an entry in `snippets.json`,
//! exactly like a theme or a shortcut sheet -- same folder-drop model, no
//! runtime, nothing to review.
//!
//! F3 matched them in the root list and copied them. F11 moves them into a
//! view of their own, with tags, dates and a pane beside the list; the
//! decisions are in `docs/ARCHITECTURE.md`, under F11.
//!
//! # Expansion is deliberately not here
//!
//! Expansion -- watching everything typed anywhere and substituting on a
//! keyword -- needs every keystroke in every application. F11 writes down the
//! way it could be done without a `WH_KEYBOARD_LL` hook, and defers it.

pub mod session;

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The schema version this build writes and understands.
///
/// 2 added `keyword`, `tags`, `created`, `modified` and `format` (F11), all
/// optional, so a version-1 file loads unchanged.
pub const SCHEMA_VERSION: u32 = 2;

/// One block of saved text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    /// What appears in the list.
    pub name: String,
    /// What gets pasted.
    pub text: String,
    /// Matched against, never shown (F3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    /// One word, shown beside the name (F11). Reserved for expansion, which
    /// is deferred; until then it is matched and shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// RFC 3339, as written by the form. Strings rather than timestamps so a
    /// date mistyped by hand costs that snippet its date, not the whole file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(default)]
    pub format: Format,
}

/// How a snippet's text is read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Pasted exactly as written. What a snippet without the field is, since
    /// a command with a `*` in it, written for F3, is not emphasis.
    #[default]
    Text,
    /// Markdown, pasted as HTML and as text with the markers taken out.
    Markdown,
    /// A format from a later Lumen, read as text rather than refusing the
    /// file (I-12's reasoning, for data).
    #[serde(other)]
    Unknown,
}

impl Format {
    /// What the pane calls it.
    pub fn label(self) -> &'static str {
        match self {
            Format::Markdown => "Markdown",
            Format::Text | Format::Unknown => "Text",
        }
    }
}

impl Snippet {
    /// When it last changed: `modified`, or `created` when it never has.
    pub fn changed(&self) -> Option<jiff::Timestamp> {
        timestamp(self.modified.as_deref()).or_else(|| timestamp(self.created.as_deref()))
    }

    pub fn created_at(&self) -> Option<jiff::Timestamp> {
        timestamp(self.created.as_deref())
    }

    pub fn modified_at(&self) -> Option<jiff::Timestamp> {
        timestamp(self.modified.as_deref())
    }

    /// The tags, trimmed, without empty ones.
    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.tags
            .iter()
            .map(|tag| tag.trim())
            .filter(|tag| !tag.is_empty())
    }

    /// The keyword, trimmed, if there is one.
    pub fn keyword(&self) -> Option<&str> {
        self.keyword
            .as_deref()
            .map(str::trim)
            .filter(|keyword| !keyword.is_empty())
    }
}

/// Now, in the user's time zone: what *Today* means to the view.
///
/// The first call reads the system's zone and, on Windows, the bundled
/// database, so it is made off the UI thread.
pub fn now() -> jiff::Zoned {
    jiff::Zoned::now()
}

fn timestamp(text: Option<&str>) -> Option<jiff::Timestamp> {
    text?.trim().parse().ok()
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
                // A snippet with no text has nothing to paste, and one with no
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, contents: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lumen-snip-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("snippets.json");
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[test]
    fn snippets_without_text_or_a_name_are_dropped() {
        let (dir, path) = temp_file(
            "drop",
            r#"{
                "schema_version": 1,
                "snippets": [
                    { "name": "Good", "text": "yes" },
                    { "name": "Empty", "text": "" },
                    { "name": "   ", "text": "no name" }
                ]
            }"#,
        );

        let snippets = load(&path);
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].name, "Good");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file written for F3 means what it meant: plain text, no tags, no
    /// dates.
    #[test]
    fn a_version_one_snippet_is_text_with_nothing_new() {
        let snippet: Snippet =
            serde_json::from_str(r#"{ "name": "Star", "text": "a * b", "keywords": ["x"] }"#)
                .unwrap();

        assert_eq!(snippet.format, Format::Text);
        assert!(snippet.tags.is_empty());
        assert_eq!(snippet.keyword(), None);
        assert_eq!(snippet.changed(), None);
    }

    #[test]
    fn a_format_from_a_later_lumen_is_read_as_text() {
        let snippet: Snippet =
            serde_json::from_str(r#"{ "name": "N", "text": "t", "format": "html" }"#).unwrap();
        assert_eq!(snippet.format, Format::Unknown);
        assert_eq!(snippet.format.label(), "Text");
    }

    #[test]
    fn a_mistyped_date_costs_the_date_not_the_snippet() {
        let snippet: Snippet = serde_json::from_str(
            r#"{ "name": "N", "text": "t", "created": "2026-09-29T21:00:19Z", "modified": "last tuesday" }"#,
        )
        .unwrap();

        assert_eq!(snippet.modified_at(), None);
        // Falls back to when it was made.
        assert_eq!(
            snippet.changed(),
            Some("2026-09-29T21:00:19Z".parse().unwrap())
        );
    }

    #[test]
    fn blank_tags_and_keywords_are_not_tags_or_keywords() {
        let snippet: Snippet = serde_json::from_str(
            r#"{ "name": "N", "text": "t", "keyword": "  ", "tags": [" work ", ""] }"#,
        )
        .unwrap();
        assert_eq!(snippet.keyword(), None);
        assert_eq!(snippet.tags().collect::<Vec<_>>(), ["work"]);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        assert!(load(Path::new("no/such/snippets.json")).is_empty());
    }

    #[test]
    fn a_malformed_file_yields_nothing_rather_than_failing() {
        let (dir, path) = temp_file("bad", "{ not json");
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
        assert!(file.snippets.iter().any(|snippet| !snippet.tags.is_empty()));
    }
}
