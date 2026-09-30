//! Favourites and history, in `dictionary.json` in the data folder (F10).
//!
//! A file rather than a database, as for currency: a few hundred rows written
//! once per lookup looked at, never per keystroke. Read leniently -- an
//! unreadable file is an empty one, logged, not a view that fails to open --
//! and written whole, through a sibling renamed over it.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use super::Entry;

/// The most words history keeps, newest first.
pub const MAX_HISTORY: usize = 100;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct State {
    /// Whole entries, so a favourite opens offline and survives the source
    /// changing or dropping the word. Newest first.
    pub favourites: Vec<Entry>,
    /// The word and its language only; opening one looks it up again.
    pub history: Vec<Recent>,
}

/// A word looked at, in its language.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Recent {
    pub word: String,
    /// The word language's code.
    pub language: String,
}

impl State {
    pub fn load(path: &Path) -> Self {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                log::warn!("could not read {}: {error}", path.display());
                return Self::default();
            }
        };
        serde_json::from_str(&source).unwrap_or_else(|error| {
            log::warn!("ignoring {}, which does not parse: {error}", path.display());
            Self::default()
        })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let source = serde_json::to_string_pretty(self)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, source)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
    }

    /// Whether this word, in this language, is a favourite.
    pub fn is_favourite(&self, word: &str, language: &str) -> bool {
        self.favourite(word, language).is_some()
    }

    fn favourite(&self, word: &str, language: &str) -> Option<usize> {
        self.favourites
            .iter()
            .position(|entry| entry.word == word && entry.language == language)
    }

    /// Add `entry` to the favourites, first; or take it out if it is one.
    /// Returns whether it is a favourite now.
    pub fn toggle_favourite(&mut self, entry: &Entry) -> bool {
        match self.favourite(&entry.word, &entry.language) {
            Some(index) => {
                self.favourites.remove(index);
                false
            }
            None => {
                self.favourites.insert(0, entry.clone());
                true
            }
        }
    }

    /// Put a word at the top of history, once, keeping [`MAX_HISTORY`].
    pub fn record(&mut self, word: &str, language: &str) {
        self.history
            .retain(|recent| !(recent.word == word && recent.language == language));
        self.history.insert(
            0,
            Recent {
                word: word.to_string(),
                language: language.to_string(),
            },
        );
        self.history.truncate(MAX_HISTORY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(word: &str, language: &str) -> Entry {
        Entry {
            word: word.into(),
            language: language.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_word_is_recorded_once_newest_first_and_bounded() {
        let mut state = State::default();
        state.record("display", "en");
        state.record("mostrar", "es");
        state.record("display", "en");
        let words: Vec<&str> = state.history.iter().map(|r| r.word.as_str()).collect();
        assert_eq!(words, ["display", "mostrar"]);

        // The same spelling in another language is another word.
        state.record("display", "fr");
        assert_eq!(state.history.len(), 3);

        for index in 0..MAX_HISTORY + 10 {
            state.record(&format!("w{index}"), "en");
        }
        assert_eq!(state.history.len(), MAX_HISTORY);
        assert_eq!(state.history[0].word, format!("w{}", MAX_HISTORY + 9));
    }

    #[test]
    fn a_favourite_toggles_by_word_and_language() {
        let mut state = State::default();
        assert!(state.toggle_favourite(&entry("display", "en")));
        assert!(state.is_favourite("display", "en"));
        assert!(!state.is_favourite("display", "fr"));
        assert!(!state.toggle_favourite(&entry("display", "en")));
        assert!(state.favourites.is_empty());
    }

    #[test]
    fn the_file_round_trips_and_a_bad_one_is_empty() {
        let dir = std::env::temp_dir().join(format!("lumen-dictionary-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dictionary.json");

        assert_eq!(State::load(&path), State::default());

        let mut state = State::default();
        state.toggle_favourite(&entry("display", "en"));
        state.record("mostrar", "es");
        state.save(&path).unwrap();
        assert_eq!(State::load(&path), state);

        std::fs::write(&path, "{ nonsense").unwrap();
        assert_eq!(State::load(&path), State::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
