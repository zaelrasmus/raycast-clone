//! Definitions (F10).
//!
//! # Two languages per word
//!
//! A definition involves two languages, and they are kept apart. The *word
//! language* is the one the word belongs to -- `mostrar` is Spanish -- and the
//! user chooses it. The *explanation language* is the one the definition is
//! written in, and it is the app's: [`explanation_language`], English today,
//! because Lumen has one language. Wiktionary is organised the same way --
//! each edition explains every language in its own -- so a Spanish interface
//! is another edition behind [`Dictionary`], not a redesign. Both languages
//! are arguments of the trait and part of every cache key for that reason.
//!
//! # Where the network is
//!
//! Only in [`Dictionary::define`], called from a worker thread the view
//! starts. Root search offers the command as a row and never looks anything
//! up (I-3).

pub mod free;
pub mod languages;
pub mod session;
pub mod store;

use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub use crate::net::{Failure, failure};

/// The `dictionary` section of `config.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct DictionaryConfig {
    /// Keep the words looked at, and show them when nothing is typed. Off
    /// clears what was kept: a record nobody can see is one nobody can erase.
    pub history: bool,
}

impl Default for DictionaryConfig {
    fn default() -> Self {
        Self { history: true }
    }
}

/// The most text looked up, in characters. A word or a short phrase; a
/// paragraph pasted into the field is not sent.
pub const MAX_CHARS: usize = 64;

/// How long a request may take before it is reported as failed.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The language definitions are written in: the app's.
///
/// The one place that decides it. When Lumen's interface has more than one
/// language, this returns the one in force, and every source, cache key and
/// saved favourite already carries it.
pub fn explanation_language() -> &'static str {
    "en"
}

/// Everything a source says about one word in one language.
///
/// Serialisable because a favourite keeps the whole entry (F10), so it opens
/// offline and survives the source changing its mind.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Entry {
    /// As the source spells it, which may differ from what was typed in case.
    pub word: String,
    /// The word language's code.
    pub language: String,
    /// The explanation language's code.
    pub explained_in: String,
    /// The page the entry comes from, for attribution and to open.
    pub url: String,
    /// One per part of speech, in the source's order.
    pub parts: Vec<Part>,
}

/// A part of speech and its senses.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Part {
    /// As the source names it, lower case: "noun", "verb".
    pub name: String,
    /// IPA, most common first, without repeats.
    pub pronunciations: Vec<String>,
    /// Principal inflected forms: "displays (plural)".
    pub forms: Vec<Form>,
    pub senses: Vec<Sense>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Form {
    pub word: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Sense {
    pub definition: String,
    /// Usage labels: "countable", "slang", "transitive".
    pub tags: Vec<String>,
    pub examples: Vec<String>,
    pub synonyms: Vec<String>,
    /// Narrower senses under this one.
    pub subsenses: Vec<Sense>,
}

impl Entry {
    /// Every sense, in order, with the part it belongs to.
    pub fn senses(&self) -> impl Iterator<Item = (&Part, &Sense)> {
        self.parts
            .iter()
            .flat_map(|part| part.senses.iter().map(move |sense| (part, sense)))
    }

    pub fn is_empty(&self) -> bool {
        self.parts.iter().all(|part| part.senses.is_empty())
    }
}

/// A source of definitions.
///
/// Blocking, on purpose: only ever called from a worker thread, with the
/// blocking client already in the binary (F6, step 3).
pub trait Dictionary: Send + Sync {
    /// `word` in the language `language`, explained in `explained_in`, both
    /// codes: at most one entry, none when the source does not have it.
    ///
    /// With no language, *Detect*: an entry for every language in the table
    /// that has the word, in the source's order; choosing among them is the
    /// view's. A source that cannot write in `explained_in` refuses with an
    /// error rather than answering in another language.
    fn define(&self, word: &str, language: Option<&str>, explained_in: &str) -> Result<Vec<Entry>>;
}

/// Who to credit for the definitions, said in every pane: the source's
/// terms ask for it, and the data is CC BY-SA.
pub fn attribution() -> &'static str {
    free::ATTRIBUTION
}

/// The source, with a client ready to use.
///
/// Building the client reads the system's certificate store, which is why
/// the caller keeps it for a while rather than building one per lookup.
pub fn dictionary() -> Result<Box<dyn Dictionary>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("Lumen/", env!("CARGO_PKG_VERSION")))
        .build()?;
    Ok(Box::new(free::FreeDictionary::new(client)))
}

/// What was typed, as it is looked up: trimmed, and `None` when it is empty
/// or too long to be a word.
pub fn word(query: &str) -> Option<&str> {
    let word = query.trim();
    (!word.is_empty() && word.chars().count() <= MAX_CHARS).then_some(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_is_trimmed_and_bounded() {
        assert_eq!(word("  display "), Some("display"));
        assert_eq!(word("   "), None);
        assert_eq!(word(&"a".repeat(MAX_CHARS + 1)), None);
        assert_eq!(
            word(&"é".repeat(MAX_CHARS)).map(str::len),
            Some(MAX_CHARS * 2)
        );
    }

    #[test]
    fn senses_are_listed_with_their_part() {
        let entry = Entry {
            parts: vec![
                Part {
                    name: "noun".into(),
                    senses: vec![Sense::default(), Sense::default()],
                    ..Default::default()
                },
                Part {
                    name: "verb".into(),
                    senses: vec![Sense::default()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let parts: Vec<&str> = entry.senses().map(|(part, _)| part.name.as_str()).collect();
        assert_eq!(parts, ["noun", "noun", "verb"]);
        assert!(!entry.is_empty());
        assert!(Entry::default().is_empty());
    }

    /// A favourite written today must load a year from now, so every field
    /// has to be optional (I-12's reasoning, for saved data).
    #[test]
    fn an_entry_reads_leniently() {
        let entry: Entry = serde_json::from_str(r#"{ "word": "x" }"#).unwrap();
        assert_eq!(entry.word, "x");
        assert!(entry.parts.is_empty());
    }
}
