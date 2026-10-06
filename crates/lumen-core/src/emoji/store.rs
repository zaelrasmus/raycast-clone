//! Pinned and frequently used emoji, custom keywords and the skin tone, in
//! `emoji.json` in the data folder (F18, tasks 6 to 8).
//!
//! A file rather than a database, as for the dictionary: a few dozen entries,
//! written once per emoji pasted or pinned, never per keystroke. Read
//! leniently -- an unreadable file is an empty one, logged, not a view that
//! fails to open -- and written whole, through a sibling renamed over it.
//!
//! Kept by the emoji's own characters, never a toned variant's, so a change
//! of tone keeps every pin and use. One the catalogue no longer has is kept,
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

use super::{CustomKeywords, SkinTone};

/// How long it takes for a use to decay to half its weight: frecency's.
const HALF_LIFE_SECS: f64 = 14.0 * 24.0 * 60.0 * 60.0;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct State {
    /// Characters, in the user's order.
    pub pinned: Vec<String>,
    /// By characters. Ordered, so the file does not churn between saves.
    pub usage: BTreeMap<String, Usage>,
    /// The user's own keywords, by characters (task 7): lowercase, trimmed,
    /// once each, never an empty list. A file written before them has none.
    pub keywords: CustomKeywords,
    /// The tone emoji are shown and pasted in (task 8). A file written
    /// before it, or naming a tone this version does not know, has the
    /// default.
    pub skin_tone: SkinTone,
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

    /// Give `chars` these keywords, or take its own away with none. Whether
    /// that changed anything.
    pub fn set_keywords(&mut self, chars: &str, keywords: Vec<String>) -> bool {
        if keywords.is_empty() {
            return self.keywords.remove(chars).is_some();
        }
        if self.keywords.get(chars) == Some(&keywords) {
            return false;
        }
        self.keywords.insert(chars.to_string(), keywords);
        true
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

/// Keywords as typed in the form: split on commas, trimmed, lowercased,
/// without the empty ones and without repeats, in the order typed.
pub fn parse_keywords(text: &str) -> Vec<String> {
    let mut keywords: Vec<String> = Vec::new();
    for keyword in text.split(',').map(|k| k.trim().to_lowercase()) {
        if !keyword.is_empty() && !keywords.contains(&keyword) {
            keywords.push(keyword);
        }
    }
    keywords
}

/// Keywords as the form shows them, to be parsed back the same.
pub fn join_keywords(keywords: &[String]) -> String {
    keywords.join(", ")
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

    #[test]
    fn custom_keywords_round_trip_and_an_older_file_has_none() {
        let dir = std::env::temp_dir().join(format!("lumen-emoji-keywords-{}", std::process::id()));
        let path = dir.join("emoji.json");
        let mut state = State::default();
        assert!(state.set_keywords("\u{2600}\u{FE0F}", vec!["sunny".into(), "warm".into()]));
        // Kept though the catalogue does not have it.
        assert!(state.set_keywords("not an emoji", vec!["kept".into()]));
        state.save(&path).unwrap();
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("\"keywords\""), "{json}");
        assert_eq!(State::load(&path), state);

        // Written before custom keywords existed.
        std::fs::write(&path, r#"{ "pinned": ["x"], "usage": {} }"#).unwrap();
        let old = State::load(&path);
        assert_eq!(old.pinned, ["x"]);
        assert!(old.keywords.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_skin_tone_round_trips_and_an_older_or_unknown_one_is_the_default() {
        let dir = std::env::temp_dir().join(format!("lumen-emoji-tone-{}", std::process::id()));
        let path = dir.join("emoji.json");
        let state = State {
            skin_tone: SkinTone::MediumDark,
            ..State::default()
        };
        state.save(&path).unwrap();
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("\"skin-tone\": \"medium-dark\""), "{json}");
        assert_eq!(State::load(&path), state);

        // Written before the tone existed.
        std::fs::write(&path, r#"{ "pinned": ["x"] }"#).unwrap();
        let old = State::load(&path);
        assert_eq!(old.pinned, ["x"]);
        assert_eq!(old.skin_tone, SkinTone::Default);

        // A tone from a later version, or not a tone at all: the default,
        // and the rest of the file still read.
        for tone in [r#""purple""#, "3", "null"] {
            std::fs::write(
                &path,
                format!(r#"{{ "pinned": ["x"], "skin-tone": {tone} }}"#),
            )
            .unwrap();
            let read = State::load(&path);
            assert_eq!(read.pinned, ["x"], "{tone}");
            assert_eq!(read.skin_tone, SkinTone::Default, "{tone}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn setting_no_keywords_removes_them() {
        let mut state = State::default();
        assert!(!state.set_keywords("a", Vec::new()));
        assert!(state.set_keywords("a", vec!["one".into()]));
        assert!(!state.set_keywords("a", vec!["one".into()]));
        assert!(state.set_keywords("a", Vec::new()));
        assert!(state.keywords.is_empty());
    }

    #[test]
    fn keywords_are_parsed_from_what_was_typed() {
        assert_eq!(parse_keywords("Sunny, warm , ,SUNNY"), ["sunny", "warm"]);
        assert!(parse_keywords("").is_empty());
        assert!(parse_keywords(" ,  , ").is_empty());
        assert_eq!(parse_keywords("two words,x"), ["two words", "x"]);
        assert_eq!(
            join_keywords(&["sunny".into(), "warm".into()]),
            "sunny, warm"
        );
        assert_eq!(join_keywords(&[]), "");
    }
}
