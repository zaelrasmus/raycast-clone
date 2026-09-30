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

pub mod markup;
pub mod placeholders;
pub mod session;
pub mod store;
pub mod template;

use std::path::Path;

use lumen_platform::PasteKeys;
use serde::{Deserialize, Serialize};

/// The `snippets` section of `config.json` (F11).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SnippetsConfig {
    pub paste: PasteMode,
    /// The keystroke that pastes, by program: `{ "noctty.exe":
    /// "ctrl+shift+v" }`. Over [`BUILTIN_PASTE_KEYS`]; everything else is
    /// `Ctrl+V`.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub paste_keys: std::collections::BTreeMap<String, PasteKeys>,
}

/// Terminals whose default bindings do not paste on `Ctrl+V` but pass it on
/// to the program running in them, from each one's own documentation of its
/// defaults. A user's `paste-keys` entry for the same program wins.
pub const BUILTIN_PASTE_KEYS: &[(&str, PasteKeys)] = &[
    ("wezterm-gui.exe", PasteKeys::CtrlShiftV),
    ("alacritty.exe", PasteKeys::CtrlShiftV),
    ("mintty.exe", PasteKeys::ShiftInsert),
];

impl SnippetsConfig {
    /// The keystroke that pastes into `executable`, the lowercased file name
    /// the platform reports -- compared without case, as sheets are.
    pub fn keys_for(&self, executable: Option<&str>) -> PasteKeys {
        let Some(executable) = executable else {
            return PasteKeys::default();
        };
        self.paste_keys
            .iter()
            .find(|(name, _)| name.trim().eq_ignore_ascii_case(executable))
            .map(|(_, keys)| *keys)
            .or_else(|| {
                BUILTIN_PASTE_KEYS
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(executable))
                    .map(|(_, keys)| *keys)
            })
            .unwrap_or_default()
    }
}

/// What a paste leaves on the clipboard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PasteMode {
    /// Put back what was there, and keep the snippet out of clipboard
    /// history: a paste is not a copy the user made.
    #[default]
    Restore,
    /// Leave the snippet, which enters the history like any copy.
    Keep,
}

/// The schema version this build writes and understands.
///
/// 2 added `keyword`, `tags`, `created`, `modified` and `format` (F11), all
/// optional, so a version-1 file loads unchanged.
pub const SCHEMA_VERSION: u32 = 2;

/// One block of saved text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Fields this build does not know, kept so that saving from Lumen does
    /// not strip what a newer one wrote (see [`store`]).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// How a snippet's text is read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Pasted exactly as written. What a snippet without the field is, since
    /// a command with a `*` in it, written for F3, is not emphasis.
    #[default]
    Text,
    /// Markdown. To be pasted as HTML and as text with the markers taken
    /// out (F11, slice 6); until then it is shown as Markdown in the pane and
    /// pasted as written.
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

/// This moment, for stamping a snippet that is saved.
pub fn stamp() -> jiff::Timestamp {
    jiff::Timestamp::now()
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
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The file as it is first read: each snippet still a JSON value, so one
/// mistyped field costs that snippet and not the file.
#[derive(Deserialize)]
struct LooseFile {
    schema_version: u32,
    #[serde(default)]
    snippets: Vec<serde_json::Value>,
}

/// What reading the file found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    pub snippets: Vec<Snippet>,
    /// Entries that are in the file but could not be read.
    pub broken: Vec<Broken>,
    /// Why the file as a whole could not be read. A missing file is not
    /// this: it is the normal case, and simply has no snippets.
    pub error: Option<String>,
}

/// A snippet in the file that could not be read, said in the view rather
/// than lost: a hand edit that dropped a snippet without a word is one the
/// user goes looking for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broken {
    /// Its name, when that much could be read.
    pub name: Option<String>,
    /// Where it is in the file, from 1.
    pub position: usize,
    pub reason: String,
}

impl From<Vec<Snippet>> for Loaded {
    fn from(snippets: Vec<Snippet>) -> Self {
        Self {
            snippets,
            ..Default::default()
        }
    }
}

/// Read the snippets file.
///
/// Nothing here fails: the launcher is summoned by a keystroke with no
/// terminal attached, so what went wrong is carried to the view, which says
/// it, and logged.
pub fn load(path: &Path) -> Loaded {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Loaded::default(),
        Err(error) => {
            log::error!("could not read {}: {error}", path.display());
            return Loaded {
                error: Some(format!("could not be read: {error}")),
                ..Default::default()
            };
        }
    };
    let loaded = parse(&source);
    if let Some(error) = &loaded.error {
        log::error!("{} is not valid: {error}", path.display());
    }
    for broken in &loaded.broken {
        log::warn!(
            "{}: skipped snippet {}: {}",
            path.display(),
            broken.position,
            broken.reason
        );
    }
    loaded
}

fn parse(source: &str) -> Loaded {
    let file = match serde_json::from_str::<LooseFile>(source) {
        Ok(file) => file,
        Err(error) => {
            return Loaded {
                error: Some(error.to_string()),
                ..Default::default()
            };
        }
    };
    if file.schema_version > SCHEMA_VERSION {
        log::warn!(
            "snippets.json declares schema version {} but this build understands {SCHEMA_VERSION}",
            file.schema_version
        );
    }

    let mut loaded = Loaded::default();
    for (index, value) in file.snippets.into_iter().enumerate() {
        let name = value
            .get("name")
            .and_then(|name| name.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(String::from);
        match serde_json::from_value::<Snippet>(value) {
            // A snippet with no text has nothing to paste, and one with no
            // name has nothing to find it by. Left out quietly, as F3 did:
            // an empty entry is a snippet not yet written, not a mistake.
            Ok(snippet) if snippet.text.is_empty() || snippet.name.trim().is_empty() => {}
            Ok(snippet) => loaded.snippets.push(snippet),
            Err(error) => loaded.broken.push(Broken {
                name,
                position: index + 1,
                reason: error.to_string(),
            }),
        }
    }
    loaded
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

        let loaded = load(&path);
        assert_eq!(loaded.snippets.len(), 1);
        assert_eq!(loaded.snippets[0].name, "Good");
        assert!(loaded.broken.is_empty());

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

    /// Restoring is the default; keeping is asked for by name.
    #[test]
    fn a_paste_puts_the_clipboard_back_unless_told_to_keep() {
        assert_eq!(SnippetsConfig::default().paste, PasteMode::Restore);
        let config: crate::Config =
            serde_json::from_str(r#"{ "snippets": { "paste": "keep" } }"#).unwrap();
        assert_eq!(config.snippets.paste, PasteMode::Keep);
    }

    #[test]
    fn the_paste_keystroke_is_ctrl_v_unless_the_program_is_named() {
        let config: crate::Config = serde_json::from_str(
            r#"{ "snippets": { "paste-keys": {
                "Noctty.exe": "ctrl+shift+v",
                "wezterm-gui.exe": "shift+insert"
            } } }"#,
        )
        .unwrap();
        let snippets = &config.snippets;

        assert_eq!(snippets.keys_for(Some("noctty.exe")), PasteKeys::CtrlShiftV);
        // The user's entry wins over the built-in one.
        assert_eq!(
            snippets.keys_for(Some("wezterm-gui.exe")),
            PasteKeys::ShiftInsert
        );
        assert_eq!(
            snippets.keys_for(Some("mintty.exe")),
            PasteKeys::ShiftInsert
        );
        assert_eq!(snippets.keys_for(Some("notepad.exe")), PasteKeys::CtrlV);
        assert_eq!(snippets.keys_for(None), PasteKeys::CtrlV);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        assert_eq!(load(Path::new("no/such/snippets.json")), Loaded::default());
    }

    #[test]
    fn a_malformed_file_yields_nothing_and_says_why() {
        let (dir, path) = temp_file("bad", "{ not json");
        let loaded = load(&path);
        assert!(loaded.snippets.is_empty());
        assert!(loaded.error.unwrap().contains("line 1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A tag written as a word rather than a list costs that snippet, not
    /// the file, and is named so it can be found.
    #[test]
    fn a_mistyped_field_costs_its_snippet_alone() {
        let loaded = parse(
            r#"{
                "schema_version": 2,
                "snippets": [
                    { "name": "Good", "text": "yes" },
                    { "name": "Tagged wrong", "text": "t", "tags": "work" },
                    { "text": 5 }
                ]
            }"#,
        );

        assert_eq!(loaded.error, None);
        assert_eq!(loaded.snippets.len(), 1);
        assert_eq!(loaded.broken.len(), 2);
        assert_eq!(loaded.broken[0].name.as_deref(), Some("Tagged wrong"));
        assert_eq!(loaded.broken[0].position, 2);
        assert!(loaded.broken[0].reason.contains("expected a sequence"));
        assert_eq!(loaded.broken[1].name, None);
        assert_eq!(loaded.broken[1].position, 3);
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
