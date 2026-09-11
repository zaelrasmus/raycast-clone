//! Frequency and recency of use.
//!
//! A launcher that ranks purely by match quality is wrong within a week: the
//! user types `co` and gets `Component Services` because it matched better than
//! the `Code` they open forty times a day. Usage has to outweigh string
//! similarity, but it must not outweigh it *completely*, or the launcher stops
//! being searchable and becomes a most-recently-used list.

use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// How long it takes for a single launch to decay to half its weight.
///
/// Two weeks: long enough that a tool used every few days keeps its place,
/// short enough that last month's installer stops shadowing today's work.
const HALF_LIFE_SECS: f64 = 14.0 * 24.0 * 60.0 * 60.0;

/// Ceiling on the frecency contribution, in units of nucleo's score scale.
///
/// Bounded on purpose. Unbounded usage weight means a heavily used entry wins
/// every query forever, including queries that plainly describe something else.
const MAX_BOOST: f64 = 180.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Usage {
    launches: u32,
    /// Seconds since the Unix epoch.
    last_used: u64,
}

/// Per-entry launch history, persisted between sessions.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Frecency {
    #[serde(default)]
    entries: HashMap<String, Usage>,
}

impl Frecency {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load from disk, falling back to an empty history.
    ///
    /// A corrupt or absent file is not an error worth surfacing: the cost is
    /// that ranking is cold for a few days, and refusing to start over it would
    /// be absurd.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(source) => serde_json::from_str(&source).unwrap_or_else(|error| {
                log::warn!(
                    "ignoring unreadable frecency store at {}: {error}",
                    path.display()
                );
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let source = serde_json::to_string(self)?;
        std::fs::write(path, source).with_context(|| format!("writing {}", path.display()))
    }

    /// Record that `id` was launched.
    pub fn record(&mut self, id: &str) {
        let usage = self.entries.entry(id.to_string()).or_insert(Usage {
            launches: 0,
            last_used: 0,
        });
        usage.launches = usage.launches.saturating_add(1);
        usage.last_used = now();
    }

    /// The ranking bonus for `id`, to be added to a match score.
    pub fn boost(&self, id: &str) -> u32 {
        self.boost_at(id, now())
    }

    fn boost_at(&self, id: &str, now: u64) -> u32 {
        match self.entries.get(id) {
            Some(usage) => Self::score(*usage, now),
            None => 0,
        }
    }

    /// Drop entries that have decayed to nothing, so the file does not grow
    /// without bound across years of use.
    pub fn prune(&mut self) {
        let now = now();
        // Scored against the entry in hand rather than through `boost_at`, which
        // would need to borrow the map that `retain` is already holding.
        self.entries.retain(|_, usage| Self::score(*usage, now) > 0);
    }

    /// The decayed score for one usage record.
    fn score(usage: Usage, now: u64) -> u32 {
        let age = now.saturating_sub(usage.last_used) as f64;
        let recency = 0.5f64.powf(age / HALF_LIFE_SECS);

        // Logarithmic in launch count: the difference between 1 and 5 launches
        // is meaningful, between 100 and 104 it is noise. Linear weighting here
        // is what produces a launcher that cannot be steered away from its
        // favourites.
        let frequency = (usage.launches as f64 + 1.0).ln();

        (frequency * recency * 60.0).min(MAX_BOOST) as u32
    }
}

fn now() -> u64 {
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
    fn unknown_entries_get_no_boost() {
        assert_eq!(Frecency::new().boost("app:nothing"), 0);
    }

    #[test]
    fn more_launches_rank_higher() {
        let mut frecency = Frecency::new();
        frecency.record("app:rare");
        for _ in 0..20 {
            frecency.record("app:common");
        }

        assert!(frecency.boost("app:common") > frecency.boost("app:rare"));
    }

    #[test]
    fn the_boost_decays_with_age() {
        let now = now();
        let mut frecency = Frecency::new();
        frecency.entries.insert(
            "app:stale".into(),
            Usage {
                launches: 10,
                last_used: now - 60 * DAY,
            },
        );
        frecency.entries.insert(
            "app:fresh".into(),
            Usage {
                launches: 10,
                last_used: now,
            },
        );

        assert!(frecency.boost_at("app:fresh", now) > frecency.boost_at("app:stale", now));
    }

    /// The bound is the point: without it a favourite outranks every query.
    #[test]
    fn the_boost_is_bounded() {
        let now = now();
        let mut frecency = Frecency::new();
        frecency.entries.insert(
            "app:obsession".into(),
            Usage {
                launches: u32::MAX,
                last_used: now,
            },
        );

        assert!(frecency.boost_at("app:obsession", now) <= MAX_BOOST as u32);
    }

    #[test]
    fn recording_is_saturating_rather_than_panicking() {
        let mut frecency = Frecency::new();
        frecency.entries.insert(
            "app:worn".into(),
            Usage {
                launches: u32::MAX,
                last_used: 0,
            },
        );
        frecency.record("app:worn");
        assert_eq!(frecency.entries["app:worn"].launches, u32::MAX);
    }
}
