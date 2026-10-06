//! Pinned and frequently used emoji, in `emoji.json` in the data folder
//! (F18, task 6).
//!
//! A file rather than a database, as for the dictionary: a few dozen entries,
//! written once per emoji pasted or pinned, never per keystroke. Read
//! leniently -- an unreadable file is an empty one, logged, not a view that
//! fails to open -- and written whole, through a sibling renamed over it.
//!
//! Kept by the emoji's characters. One the catalogue no longer has is kept,
//! not pruned, for `favorites.rs`'s reason: a catalogue regenerated without
//! it for a release must not cost the user their arrangement.
//!
//! Usage is ranked with `frecency.rs`'s maths -- logarithmic in count, halved
//! every two weeks -- but not with its type: *Remove from Frequently Used*
//! needs an entry dropped, which `Frecency` cannot do, and its score is
//! scaled and truncated to a bounded boost, where ranking a section only
//! needs the order.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// How long it takes for a use to decay to half its weight: frecency's.
const HALF_LIFE_SECS: f64 = 14.0 * 24.0 * 60.0 * 60.0;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct State {
    /// Characters, in the user's order.
    pub pinned: Vec<String>,
    /// By characters. Ordered, so the file does not churn between saves.
    pub usage: BTreeMap<String, Usage>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Usage {
    pub count: u32,
    /// Seconds since the Unix epoch.
    pub last_used: u64,
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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let source = serde_json::to_string_pretty(self)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, source)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
    }

    pub fn is_pinned(&self, chars: &str) -> bool {
        self.pinned.iter().any(|pinned| pinned == chars)
    }

    /// Pin at the end, once.
    pub fn pin(&mut self, chars: &str) -> bool {
        if self.is_pinned(chars) {
            return false;
        }
        self.pinned.push(chars.to_string());
        true
    }

    pub fn unpin(&mut self, chars: &str) -> bool {
        let before = self.pinned.len();
        self.pinned.retain(|pinned| pinned != chars);
        self.pinned.len() != before
    }

    /// Swap `chars` with its neighbour among `visible`, the pins that are
    /// drawn: one the catalogue lacks stays in the file but is stepped over,
    /// or a move would seem to do nothing (`favorites.rs` does the same).
    pub fn move_pinned(&mut self, chars: &str, up: bool, visible: &[&str]) -> bool {
        let Some(at) = visible.iter().position(|each| *each == chars) else {
            return false;
        };
        let neighbour = if up {
            at.checked_sub(1)
        } else {
            Some(at + 1).filter(|next| *next < visible.len())
        };
        let Some(neighbour) = neighbour.map(|index| visible[index]) else {
            return false;
        };
        let position = |wanted: &str| self.pinned.iter().position(|each| each == wanted);
        match (position(chars), position(neighbour)) {
            (Some(a), Some(b)) => {
                self.pinned.swap(a, b);
                true
            }
            _ => false,
        }
    }

    /// Record that `chars` was pasted or copied.
    pub fn record(&mut self, chars: &str) {
        self.record_at(chars, now());
    }

    fn record_at(&mut self, chars: &str, now: u64) {
        let usage = self.usage.entry(chars.to_string()).or_default();
        usage.count = usage.count.saturating_add(1);
        usage.last_used = now;
    }

    /// Forget how often `chars` was used.
    pub fn forget(&mut self, chars: &str) -> bool {
        self.usage.remove(chars).is_some()
    }

    pub fn is_used(&self, chars: &str) -> bool {
        self.usage.contains_key(chars)
    }

    /// How strongly `chars` ranks in *Frequently Used* at `now`; 0 unused.
    pub fn score(&self, chars: &str, now: u64) -> f64 {
        self.usage.get(chars).map_or(0.0, |usage| {
            let age = now.saturating_sub(usage.last_used) as f64;
            (usage.count as f64 + 1.0).ln() * 0.5f64.powf(age / HALF_LIFE_SECS)
        })
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 60 * 60;

    #[test]
    fn more_uses_rank_higher_and_age_decays_them() {
        let now = 100 * DAY;
        let mut state = State::default();
        state.record_at("a", now);
        state.record_at("b", now);
        state.record_at("b", now);
        assert!(state.score("b", now) > state.score("a", now));
        assert_eq!(state.score("never", now), 0.0);

        let mut stale = State::default();
        for _ in 0..4 {
            stale.record_at("old", now - 60 * DAY);
        }
        stale.record_at("new", now);
        assert!(stale.score("new", now) > stale.score("old", now));

        assert!(state.forget("b"));
        assert!(!state.forget("b"));
        assert!(!state.is_used("b"));
    }

    #[test]
    fn pins_are_ordered_once_and_moves_step_over_what_is_not_shown() {
        let mut state = State::default();
        assert!(state.pin("a"));
        assert!(!state.pin("a"));
        state.pin("gone");
        state.pin("c");
        let visible = ["a", "c"];
        assert!(state.move_pinned("c", true, &visible));
        assert_eq!(state.pinned, ["c", "gone", "a"]);
        assert!(!state.move_pinned("c", true, &["c", "a"]));
        assert!(!state.move_pinned("a", false, &["c", "a"]));
        assert!(state.unpin("c"));
        assert_eq!(state.pinned, ["gone", "a"], "the unknown pin is kept");
    }

    #[test]
    fn the_file_round_trips_and_a_missing_or_bad_one_is_empty() {
        let dir = std::env::temp_dir().join(format!("lumen-emoji-store-{}", std::process::id()));
        let path = dir.join("emoji.json");
        assert_eq!(State::load(&path), State::default());

        let mut state = State::default();
        state.pin("\u{2600}\u{FE0F}");
        state.record_at("\u{1F600}", 42);
        state.save(&path).unwrap();
        assert_eq!(State::load(&path), state);
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("\"pinned\""), "{json}");
        assert!(json.contains("\"last-used\": 42"), "{json}");

        std::fs::write(&path, "{ nonsense").unwrap();
        assert_eq!(State::load(&path), State::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
