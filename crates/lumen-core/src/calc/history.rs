//! The calculations used, in `calculator-history.json` in the data folder
//! (F14).
//!
//! What was *used*, not what was typed -- F10's rule: an answer is kept when
//! it is copied, or history would be `1`, `12`, `12*` on the way to `12*7`.
//! A file rather than a database, as for currency and the dictionary: a few
//! rows written once per use. Read leniently -- an unreadable file is an
//! empty history, logged -- and written whole, through a sibling renamed
//! over it.

use std::path::Path;

use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Held while the file is read, changed and written. A use is kept on a
/// background thread, and the settings window clears on another: without
/// it, two of them a moment apart would both read the file before either
/// wrote it, and one would be lost.
static WRITING: Mutex<()> = Mutex::new(());

/// How many calculations are kept when the configuration does not say.
pub const DEFAULT_LIMIT: usize = 20;

/// The most a configuration may ask to keep. A history is something read,
/// and past this it is a log.
pub const MAX_LIMIT: usize = 500;

/// The `calculator` section of `config.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct CalculatorConfig {
    /// How many calculations the history keeps, newest first. `0` keeps
    /// none, and clears what was kept: a record nobody can see is one
    /// nobody can erase.
    pub history_limit: usize,
}

impl Default for CalculatorConfig {
    fn default() -> Self {
        Self {
            history_limit: DEFAULT_LIMIT,
        }
    }
}

impl CalculatorConfig {
    /// The limit, within what a history is for.
    pub fn limit(&self) -> usize {
        self.history_limit.min(MAX_LIMIT)
    }
}

/// One calculation used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Entry {
    /// As typed: `25% off 399`.
    pub expression: String,
    /// As shown and copied: `299.25`.
    pub result: String,
    /// When it was used, in seconds since the epoch.
    pub at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct History {
    /// Newest first.
    pub entries: Vec<Entry>,
}

impl History {
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

    /// Read the file at `path`, change it, and write it back if `edit` says
    /// it changed, all under one lock. Returns the history as it now is.
    pub fn change(path: &Path, edit: impl FnOnce(&mut History) -> bool) -> Result<History> {
        let _writing = WRITING.lock();
        let mut history = History::load(path);
        if edit(&mut history) {
            history.save(path)?;
        }
        Ok(history)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let source = serde_json::to_string_pretty(self)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, source)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
    }

    /// Keep `entry` first, at most `limit` entries in all. The same
    /// expression used again moves to the top with its new result -- `days
    /// until 25 dec` is a different number tomorrow -- rather than being
    /// kept twice. Returns whether anything changed.
    pub fn record(&mut self, entry: Entry, limit: usize) -> bool {
        let before = self.entries.clone();
        self.entries
            .retain(|kept| !same(&kept.expression, &entry.expression));
        self.entries.insert(0, entry);
        self.entries.truncate(limit);
        self.entries != before
    }

    /// Forget the entry for `expression`. Returns whether there was one.
    pub fn delete(&mut self, expression: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|kept| !same(&kept.expression, expression));
        self.entries.len() != before
    }

    /// Keep at most `limit` entries. Returns whether any were dropped.
    pub fn trim(&mut self, limit: usize) -> bool {
        let before = self.entries.len();
        self.entries.truncate(limit);
        self.entries.len() != before
    }
}

/// Whether two expressions are one, written with different spacing or case:
/// `25%off 399` is `25% OFF 399`.
fn same(left: &str, right: &str) -> bool {
    let squeezed = |text: &str| {
        text.chars()
            .filter(|character| !character.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    squeezed(left) == squeezed(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(expression: &str, result: &str, at: i64) -> Entry {
        Entry {
            expression: expression.into(),
            result: result.into(),
            at,
        }
    }

    fn expressions(history: &History) -> Vec<&str> {
        history
            .entries
            .iter()
            .map(|entry| entry.expression.as_str())
            .collect()
    }

    #[test]
    fn newest_first_within_the_limit() {
        let mut history = History::default();
        for (at, expression) in ["1+1", "2+2", "3+3"].into_iter().enumerate() {
            assert!(history.record(entry(expression, "x", at as i64), 2));
        }
        assert_eq!(expressions(&history), ["3+3", "2+2"]);
    }

    #[test]
    fn the_same_expression_moves_to_the_top_with_its_new_result() {
        let mut history = History::default();
        history.record(entry("days until 25 dec", "85 days", 1), 20);
        history.record(entry("2+2", "4", 2), 20);
        history.record(entry("Days  Until 25 Dec", "84 days", 3), 20);
        assert_eq!(expressions(&history), ["Days  Until 25 Dec", "2+2"]);
        assert_eq!(history.entries[0].result, "84 days");
    }

    #[test]
    fn a_limit_of_zero_keeps_nothing() {
        let mut history = History::default();
        history.record(entry("2+2", "4", 1), 20);
        assert!(history.record(entry("3+3", "6", 2), 0));
        assert!(history.entries.is_empty());
        assert!(
            !history.record(entry("3+3", "6", 3), 0),
            "nothing to change"
        );
    }

    #[test]
    fn a_lower_limit_trims_what_was_kept() {
        let mut history = History::default();
        for at in 0..5 {
            history.record(entry(&format!("{at}+1"), "x", at), 20);
        }
        assert!(history.trim(3));
        assert_eq!(expressions(&history), ["4+1", "3+1", "2+1"]);
        assert!(!history.trim(3));
    }

    #[test]
    fn an_entry_is_deleted_by_its_expression() {
        let mut history = History::default();
        history.record(entry("2+2", "4", 1), 20);
        history.record(entry("3+3", "6", 2), 20);
        assert!(history.delete("2 + 2"));
        assert_eq!(expressions(&history), ["3+3"]);
        assert!(!history.delete("2+2"));
    }

    #[test]
    fn the_configured_limit_is_bounded() {
        let config: CalculatorConfig =
            serde_json::from_str(r#"{ "history-limit": 100000 }"#).unwrap();
        assert_eq!(config.limit(), MAX_LIMIT);
        let config: CalculatorConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.limit(), DEFAULT_LIMIT);
    }

    #[test]
    fn a_file_round_trips_and_a_broken_one_is_empty() {
        let folder =
            std::env::temp_dir().join(format!("lumen-calc-history-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("calculator-history.json");

        let mut history = History::default();
        history.record(entry("25% off 399", "299.25", 7), 20);
        history.save(&path).unwrap();
        assert_eq!(History::load(&path), history);

        std::fs::write(&path, "not json").unwrap();
        assert_eq!(History::load(&path), History::default());
        assert_eq!(
            History::load(&folder.join("missing.json")),
            History::default()
        );

        std::fs::remove_dir_all(&folder).ok();
    }
}
