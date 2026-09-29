//! Stopwatches (F8, slice 4): a timer with no duration, counting up.
//!
//! The same two ideas as a timer, the other way round. A running stopwatch
//! stores the wall-clock moment it was last started and what it had counted
//! before; a paused one stores what it has counted. It has no deadline, so it
//! never wakes the daemon: a stopwatch left running for a week costs nothing
//! until somebody opens *Timers* to look at it.
//!
//! No laps yet. A lap is a list that grows, and a row has one line; laps
//! want a detail of their own, and nobody has asked for them.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{Clock, Millis};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Stopwatch {
    /// From the same sequence as timers' ids, so the two never collide.
    pub id: u64,
    pub name: Option<String>,
    pub state: Watch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "kebab-case",
    rename_all_fields = "kebab-case"
)]
pub enum Watch {
    /// Counting since `since`, having counted `before_ms` until then.
    Running {
        since: Millis,
        before_ms: u64,
    },
    Paused {
        elapsed_ms: u64,
    },
}

impl Stopwatch {
    pub fn title(&self) -> String {
        self.name.clone().unwrap_or_else(|| "Stopwatch".into())
    }

    pub fn elapsed(&self, now: Millis) -> Duration {
        Duration::from_millis(match self.state {
            Watch::Running { since, before_ms } => before_ms + now.saturating_sub(since),
            Watch::Paused { elapsed_ms } => elapsed_ms,
        })
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, Watch::Running { .. })
    }
}

impl Clock {
    pub fn stopwatches(&self) -> &[Stopwatch] {
        &self.stopwatches
    }

    fn watch_mut(&mut self, id: u64) -> Option<&mut Stopwatch> {
        self.stopwatches.iter_mut().find(|watch| watch.id == id)
    }

    /// Start counting from zero, and return the new stopwatch's id.
    pub fn start_stopwatch(&mut self, name: Option<String>, now: Millis) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.stopwatches.push(Stopwatch {
            id,
            name,
            state: Watch::Running {
                since: now,
                before_ms: 0,
            },
        });
        id
    }

    pub fn pause_stopwatch(&mut self, id: u64, now: Millis) -> bool {
        let Some(watch) = self.watch_mut(id) else {
            return false;
        };
        if !watch.is_running() {
            return false;
        }
        watch.state = Watch::Paused {
            elapsed_ms: watch.elapsed(now).as_millis() as u64,
        };
        true
    }

    pub fn resume_stopwatch(&mut self, id: u64, now: Millis) -> bool {
        let Some(watch) = self.watch_mut(id) else {
            return false;
        };
        let Watch::Paused { elapsed_ms } = watch.state else {
            return false;
        };
        watch.state = Watch::Running {
            since: now,
            before_ms: elapsed_ms,
        };
        true
    }

    /// Back to zero, counting.
    pub fn reset_stopwatch(&mut self, id: u64, now: Millis) -> bool {
        let Some(watch) = self.watch_mut(id) else {
            return false;
        };
        watch.state = Watch::Running {
            since: now,
            before_ms: 0,
        };
        true
    }

    pub fn stop_stopwatch(&mut self, id: u64) -> Option<Stopwatch> {
        let at = self.stopwatches.iter().position(|watch| watch.id == id)?;
        Some(self.stopwatches.remove(at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: u64 = 1_000;

    #[test]
    fn a_stopwatch_counts_up_from_when_it_started() {
        let mut clock = Clock::default();
        let id = clock.start_stopwatch(None, 5 * SEC);
        let watch = &clock.stopwatches()[0];
        assert_eq!(watch.id, id);
        assert_eq!(watch.title(), "Stopwatch");
        assert_eq!(watch.elapsed(95 * SEC), Duration::from_secs(90));
    }

    #[test]
    fn pausing_keeps_the_count_and_resuming_carries_on_from_it() {
        let mut clock = Clock::default();
        let id = clock.start_stopwatch(Some("call".into()), 0);

        assert!(clock.pause_stopwatch(id, 30 * SEC));
        assert!(!clock.pause_stopwatch(id, 30 * SEC));
        // Time passing while paused does not count.
        assert_eq!(
            clock.stopwatches()[0].elapsed(500 * SEC),
            Duration::from_secs(30)
        );

        assert!(clock.resume_stopwatch(id, 500 * SEC));
        assert_eq!(
            clock.stopwatches()[0].elapsed(510 * SEC),
            Duration::from_secs(40)
        );
    }

    #[test]
    fn reset_counts_from_zero_again() {
        let mut clock = Clock::default();
        let id = clock.start_stopwatch(None, 0);
        clock.pause_stopwatch(id, 60 * SEC);

        assert!(clock.reset_stopwatch(id, 100 * SEC));
        assert!(clock.stopwatches()[0].is_running());
        assert_eq!(
            clock.stopwatches()[0].elapsed(105 * SEC),
            Duration::from_secs(5)
        );
    }

    /// A stopwatch has nothing to end, so nothing to wake the daemon for.
    #[test]
    fn a_running_stopwatch_wakes_nobody_and_never_ends() {
        let mut clock = Clock::default();
        clock.start_stopwatch(None, 0);
        assert_eq!(clock.next_deadline(), None);
        assert!(clock.expire(u64::MAX, true).is_empty());
        assert_eq!(clock.stopwatches().len(), 1);
        // And "stop running timer" is about timers.
        assert_eq!(clock.stop_nearest(0), None);
        assert_eq!(clock.stopwatches().len(), 1);
    }

    #[test]
    fn stopwatches_and_timers_never_share_an_id() {
        let mut clock = Clock::default();
        let timer = clock.start(Duration::from_secs(60), None, 0);
        let watch = clock.start_stopwatch(None, 0);
        assert_ne!(timer, watch);
    }

    #[test]
    fn a_stopwatch_survives_a_restart_and_kept_counting() {
        let dir = std::env::temp_dir().join(format!("lumen-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("timers.json");

        let mut clock = Clock::default();
        clock.start_stopwatch(Some("build".into()), 0);
        clock.save(&path).unwrap();

        let loaded = Clock::load(&path);
        assert_eq!(loaded, clock);
        // Counted by the wall clock, the daemon being down changes nothing.
        assert_eq!(
            loaded.stopwatches()[0].elapsed(3_600 * SEC),
            Duration::from_secs(3_600)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stopping_removes_it() {
        let mut clock = Clock::default();
        let id = clock.start_stopwatch(None, 0);
        assert_eq!(clock.stop_stopwatch(id).unwrap().id, id);
        assert!(clock.stopwatches().is_empty());
        assert!(clock.stop_stopwatch(id).is_none());
    }
}
