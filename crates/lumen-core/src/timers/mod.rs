//! Timers (F8).
//!
//! # One concept
//!
//! A timer counts down from a duration and says so when it ends. Everything
//! F8 adds later -- presets, sounds, cycles, focus -- is a field on it rather
//! than a second kind of thing, so there is one list, one view and one set of
//! rules for what "running" means.
//!
//! # A deadline, not a counter
//!
//! A running timer stores the wall-clock moment it ends; a paused one stores
//! what was left. Nothing counts, so nothing has to wake up to count: the
//! daemon sleeps once, until [`Clock::next_deadline`]. And a deadline survives
//! the daemon restarting and the machine sleeping, where a counter would not.
//!
//! # The decisions are a value
//!
//! [`Clock`] never reads the time. Every method that needs it is given `now`,
//! so starting, pausing and ending are unit tests rather than things to wait
//! for. The daemon (`lumen-app`) owns the sleep, the notification and the
//! file, as it does for the translator.

pub mod cycle;
pub mod duration;
pub mod focus;
pub mod presets;
pub mod sounds;
pub mod stopwatch;
pub mod store;
pub mod view;

pub use cycle::{Cycle, Phase, Plan};
pub use presets::{Preset, PresetConfig, Ring, TimersConfig};
pub use stopwatch::{Stopwatch, Watch};

/// What happened to a timer whose deadline passed.
#[derive(Debug, Clone, PartialEq)]
pub enum Ending {
    /// It is over.
    Done(Timer),
    /// Its cycle moved on; the timer as it is now, in its new phase.
    Next(Timer),
}

impl Ending {
    pub fn timer(&self) -> &Timer {
        match self {
            Ending::Done(timer) | Ending::Next(timer) => timer,
        }
    }
}

use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch: how the wall clock is written down.
pub type Millis = u64;

/// `time` as [`Millis`]. A clock set before 1970 reads as the epoch.
pub fn millis(time: SystemTime) -> Millis {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as Millis)
}

/// Something to do to the timers, from a root row or the command line.
///
/// Carried back to the daemon like a [`crate::Builtin`]: the timers are the
/// daemon's, not the list's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Command {
    /// Count down from `seconds`, under `name` if there is one; with focus
    /// on, if asked (slice 6).
    Start {
        seconds: u64,
        name: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        focus: bool,
    },
    /// Stop the running timer closest to ending.
    StopNearest,
    /// Silence every timer ringing until dismissed (slice 3).
    DismissRinging,
    /// Count up from zero, under `name` if there is one (slice 4).
    StartStopwatch { name: Option<String> },
    /// Work and breaks by `plan`, under `name` (slice 5).
    StartCycle {
        name: Option<String>,
        plan: Plan,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        focus: bool,
    },
}

/// Every timer and stopwatch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Clock {
    /// The id the next timer gets. Never reused, so an id in a view that has
    /// not re-rendered yet can never name a newer timer.
    next_id: u64,
    timers: Vec<Timer>,
    stopwatches: Vec<Stopwatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Timer {
    pub id: u64,
    pub name: Option<String>,
    /// What it counts down from, for restarting and for its description.
    pub duration_ms: u64,
    pub state: State,
    /// Where in its plan a cycling timer is; `duration_ms` is then the
    /// phase's length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle: Option<Cycle>,
    /// While it runs, Lumen will not open the blocked apps and sites
    /// (slice 6).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focus: bool,
    /// How many times something blocked was opened anyway.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub overrides: u32,
}

fn is_zero(count: &u32) -> bool {
    *count == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "kebab-case",
    rename_all_fields = "kebab-case"
)]
pub enum State {
    Running {
        ends_at: Millis,
    },
    Paused {
        remaining_ms: u64,
    },
    /// Ended, and ringing until dismissed (slice 3). Only while the daemon
    /// runs: see [`Clock::load`].
    Ringing {
        since: Millis,
    },
}

impl Timer {
    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.duration_ms)
    }

    /// What the timer is called: its name, or what it counts.
    pub fn title(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("{} timer", duration::describe(self.duration())),
        }
    }

    pub fn remaining(&self, now: Millis) -> Duration {
        Duration::from_millis(match self.state {
            State::Running { ends_at } => ends_at.saturating_sub(now),
            State::Paused { remaining_ms } => remaining_ms,
            State::Ringing { .. } => 0,
        })
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running { .. })
    }

    pub fn is_ringing(&self) -> bool {
        matches!(self.state, State::Ringing { .. })
    }
}

impl Clock {
    pub fn timers(&self) -> &[Timer] {
        &self.timers
    }

    pub fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }

    /// Whether any timer is ringing, and so the alert should be playing.
    pub fn is_ringing(&self) -> bool {
        self.timers.iter().any(Timer::is_ringing)
    }

    pub fn get(&self, id: u64) -> Option<&Timer> {
        self.timers.iter().find(|timer| timer.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Timer> {
        self.timers.iter_mut().find(|timer| timer.id == id)
    }

    /// Start counting down from `duration`, and return the new timer's id.
    pub fn start(&mut self, duration: Duration, name: Option<String>, now: Millis) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let duration_ms = duration.as_millis() as u64;
        self.timers.push(Timer {
            id,
            name,
            duration_ms,
            state: State::Running {
                ends_at: now + duration_ms,
            },
            cycle: None,
            focus: false,
            overrides: 0,
        });
        id
    }

    /// Carry out a [`Command`]. Returns the timer it stopped, if it stopped
    /// one, so the caller can say which.
    pub fn apply(&mut self, command: &Command, now: Millis) -> Option<Timer> {
        match command {
            Command::Start {
                seconds,
                name,
                focus,
            } => {
                let id = self.start(Duration::from_secs(*seconds), name.clone(), now);
                self.set_focus(id, *focus);
                None
            }
            Command::StopNearest => self.stop_nearest(now),
            Command::DismissRinging => self.dismiss_ringing().into_iter().next(),
            Command::StartStopwatch { name } => {
                self.start_stopwatch(name.clone(), now);
                None
            }
            Command::StartCycle { name, plan, focus } => {
                let id = self.start_cycle(*plan, name.clone(), now);
                self.set_focus(id, *focus);
                None
            }
        }
    }

    /// Take out every ringing timer: the alert has been heard.
    pub fn dismiss_ringing(&mut self) -> Vec<Timer> {
        let (ringing, rest) = std::mem::take(&mut self.timers)
            .into_iter()
            .partition(Timer::is_ringing);
        self.timers = rest;
        ringing
    }

    /// Freeze a running timer. Returns whether anything changed.
    pub fn pause(&mut self, id: u64, now: Millis) -> bool {
        let Some(timer) = self.get_mut(id) else {
            return false;
        };
        let State::Running { ends_at } = timer.state else {
            return false;
        };
        timer.state = State::Paused {
            remaining_ms: ends_at.saturating_sub(now),
        };
        true
    }

    /// Carry on from where a paused timer stopped.
    pub fn resume(&mut self, id: u64, now: Millis) -> bool {
        let Some(timer) = self.get_mut(id) else {
            return false;
        };
        let State::Paused { remaining_ms } = timer.state else {
            return false;
        };
        timer.state = State::Running {
            ends_at: now + remaining_ms,
        };
        true
    }

    /// Count down from the whole duration again, running.
    pub fn restart(&mut self, id: u64, now: Millis) -> bool {
        let Some(timer) = self.get_mut(id) else {
            return false;
        };
        timer.state = State::Running {
            ends_at: now + timer.duration_ms,
        };
        true
    }

    /// Remove a timer without it ending.
    pub fn stop(&mut self, id: u64) -> Option<Timer> {
        let at = self.timers.iter().position(|timer| timer.id == id)?;
        Some(self.timers.remove(at))
    }

    /// Stop the running timer that would end first.
    pub fn stop_nearest(&mut self, now: Millis) -> Option<Timer> {
        let id = self
            .timers
            .iter()
            .filter(|timer| timer.is_running())
            .min_by_key(|timer| timer.remaining(now))?
            .id;
        self.stop(id)
    }

    /// Every timer whose deadline has passed by `now`, first to end first.
    ///
    /// A timer with a cycle moves on to its next phase, counted from the
    /// deadline rather than from `now` so a late wake does not stretch the
    /// plan -- through several phases if the machine slept through them, and
    /// reported once, as where it is now. A timer with nothing after it ends:
    /// with `ring` it stays, ringing until dismissed, otherwise it is taken
    /// out. Either way each is returned once, so each is announced once.
    pub fn expire(&mut self, now: Millis, ring: bool) -> Vec<Ending> {
        let (mut ended, running): (Vec<Timer>, Vec<Timer>) = std::mem::take(&mut self.timers)
            .into_iter()
            .partition(|timer| matches!(timer.state, State::Running { ends_at } if ends_at <= now));
        self.timers = running;
        ended.sort_by_key(|timer| match timer.state {
            State::Running { ends_at } => ends_at,
            State::Paused { .. } | State::Ringing { .. } => Millis::MAX,
        });

        let mut endings = Vec::new();
        for mut timer in ended {
            let mut advanced = false;
            while let State::Running { ends_at } = timer.state {
                if ends_at > now || !timer.advance(ends_at) {
                    break;
                }
                advanced = true;
            }
            let still_running = matches!(timer.state, State::Running { ends_at } if ends_at > now);
            if advanced && still_running {
                self.timers.push(timer.clone());
                endings.push(Ending::Next(timer));
                continue;
            }
            if ring {
                self.timers.push(Timer {
                    state: State::Ringing { since: now },
                    ..timer.clone()
                });
            }
            endings.push(Ending::Done(timer));
        }
        endings
    }

    /// When the next running timer ends: the one moment the daemon has to be
    /// awake for. `None` when nothing is running, and then nothing wakes.
    pub fn next_deadline(&self) -> Option<Millis> {
        self.timers
            .iter()
            .filter_map(|timer| match timer.state {
                State::Running { ends_at } => Some(ends_at),
                State::Paused { .. } | State::Ringing { .. } => None,
            })
            .min()
    }

    /// Read the timers left running when the daemon last stopped.
    ///
    /// Lenient: a file that is missing or unreadable is no timers, with a log
    /// line for the second. Losing a timer is better than not starting.
    ///
    /// A timer that was ringing is not brought back: it was announced when it
    /// ended, and an alarm that starts again at the next login, hours later,
    /// is an alarm about nothing.
    pub fn load(path: &Path) -> Self {
        let mut clock = Self::read(path);
        clock.dismiss_ringing();
        clock
    }

    fn read(path: &Path) -> Self {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                log::warn!("could not read {}: {error}", path.display());
                return Self::default();
            }
        };
        serde_json::from_str(&source).unwrap_or_else(|error| {
            log::warn!("{} is not a list of timers: {error}", path.display());
            Self::default()
        })
    }

    /// Write the timers, replacing the file by renaming a sibling over it so a
    /// crash mid-write leaves the old file rather than half of the new one.
    pub fn save(&self, path: &Path) -> Result<()> {
        let source = serde_json::to_string_pretty(self)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, source)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    #[test]
    fn a_timer_ends_its_duration_after_it_starts() {
        let mut clock = Clock::default();
        clock.start(Duration::from_secs(25 * 60), None, 1_000);

        assert_eq!(clock.next_deadline(), Some(1_000 + 25 * MIN));
        assert_eq!(
            clock.timers()[0].remaining(1_000 + MIN),
            Duration::from_secs(24 * 60)
        );
    }

    #[test]
    fn a_paused_timer_keeps_what_was_left_and_wakes_nobody() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(10 * 60), None, 0);

        assert!(clock.pause(id, 4 * MIN));
        assert_eq!(clock.next_deadline(), None);
        // Time passing does not touch it.
        assert_eq!(
            clock.get(id).unwrap().remaining(60 * MIN),
            Duration::from_secs(6 * 60)
        );

        assert!(clock.resume(id, 100 * MIN));
        assert_eq!(clock.next_deadline(), Some(106 * MIN));
    }

    #[test]
    fn pausing_twice_or_resuming_a_running_timer_changes_nothing() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(60), None, 0);
        assert!(!clock.resume(id, 0));
        assert!(clock.pause(id, 0));
        assert!(!clock.pause(id, 0));
        assert!(!clock.pause(99, 0));
    }

    #[test]
    fn restarting_counts_the_whole_duration_again() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(5 * 60), None, 0);
        clock.pause(id, MIN);

        assert!(clock.restart(id, 10 * MIN));
        assert_eq!(clock.next_deadline(), Some(15 * MIN));
    }

    #[test]
    fn expiring_takes_out_only_what_has_ended_first_to_end_first() {
        let mut clock = Clock::default();
        clock.start(Duration::from_secs(3 * 60), Some("tea".into()), 0);
        clock.start(Duration::from_secs(60), Some("eggs".into()), 0);
        clock.start(Duration::from_secs(10 * 60), Some("bread".into()), 0);

        let ended = clock.expire(3 * MIN, false);

        let names: Vec<_> = ended.iter().map(|ending| ending.timer().title()).collect();
        assert_eq!(names, ["eggs", "tea"]);
        assert_eq!(clock.timers().len(), 1);
        assert_eq!(clock.next_deadline(), Some(10 * MIN));
    }

    #[test]
    fn a_paused_timer_never_expires() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(60), None, 0);
        clock.pause(id, 0);
        assert!(clock.expire(u64::MAX, false).is_empty());
    }

    #[test]
    fn a_timer_that_rings_stays_until_dismissed_and_is_announced_once() {
        let mut clock = Clock::default();
        clock.start(Duration::from_secs(60), Some("tea".into()), 0);

        assert_eq!(clock.expire(MIN, true).len(), 1);
        assert!(clock.is_ringing());
        assert_eq!(clock.next_deadline(), None);
        // Ringing is not running: it does not end a second time.
        assert!(clock.expire(10 * MIN, true).is_empty());
        assert_eq!(clock.stop_nearest(10 * MIN), None);

        let dismissed = clock.dismiss_ringing();
        assert_eq!(dismissed[0].title(), "tea");
        assert!(clock.is_empty());
    }

    #[test]
    fn restarting_a_ringing_timer_silences_it() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(60), None, 0);
        clock.expire(MIN, true);

        assert!(clock.restart(id, MIN));
        assert!(!clock.is_ringing());
        assert_eq!(clock.next_deadline(), Some(2 * MIN));
    }

    #[test]
    fn a_ringing_timer_does_not_come_back_after_a_restart() {
        let dir = std::env::temp_dir().join(format!("lumen-timers-ring-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("timers.json");

        let mut clock = Clock::default();
        clock.start(Duration::from_secs(60), Some("rang".into()), 0);
        clock.start(Duration::from_secs(600), Some("runs".into()), 0);
        clock.expire(MIN, true);
        clock.save(&path).unwrap();

        let loaded = Clock::load(&path);
        let titles: Vec<_> = loaded.timers().iter().map(Timer::title).collect();
        assert_eq!(titles, ["runs"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stopping_the_nearest_ignores_paused_timers() {
        let mut clock = Clock::default();
        let paused = clock.start(Duration::from_secs(30), Some("paused".into()), 0);
        clock.pause(paused, 0);
        clock.start(Duration::from_secs(600), Some("long".into()), 0);
        clock.start(Duration::from_secs(120), Some("short".into()), 0);

        let stopped = clock.stop_nearest(0).unwrap();
        assert_eq!(stopped.title(), "short");
        assert_eq!(clock.timers().len(), 2);
    }

    #[test]
    fn ids_are_never_reused() {
        let mut clock = Clock::default();
        let first = clock.start(Duration::from_secs(60), None, 0);
        clock.stop(first);
        let second = clock.start(Duration::from_secs(60), None, 0);
        assert_ne!(first, second);
    }

    #[test]
    fn an_unnamed_timer_is_called_by_its_duration() {
        let mut clock = Clock::default();
        clock.start(Duration::from_secs(90 * 60), None, 0);
        assert_eq!(clock.timers()[0].title(), "1 h 30 min timer");
    }

    #[test]
    fn a_command_starts_or_stops() {
        let mut clock = Clock::default();
        clock.apply(
            &Command::Start {
                seconds: 60,
                name: Some("tea".into()),
                focus: false,
            },
            0,
        );
        assert_eq!(clock.timers()[0].title(), "tea");
        let stopped = clock.apply(&Command::StopNearest, 0);
        assert_eq!(stopped.unwrap().title(), "tea");
        assert!(clock.is_empty());
    }

    #[test]
    fn the_timers_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("lumen-timers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("timers.json");

        let mut clock = Clock::default();
        clock.start(Duration::from_secs(60), Some("tea".into()), 5);
        let paused = clock.start(Duration::from_secs(120), None, 5);
        clock.pause(paused, 65);
        clock.save(&path).unwrap();

        assert_eq!(Clock::load(&path), clock);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_or_broken_file_is_no_timers() {
        let dir = std::env::temp_dir().join(format!("lumen-timers-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("timers.json");

        assert!(Clock::load(&path).is_empty());
        std::fs::write(&path, "{ nonsense").unwrap();
        assert!(Clock::load(&path).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The file is edited by nobody, but read by whoever wonders what Lumen
    /// keeps: one spelling throughout, like every other file of Lumen's.
    #[test]
    fn the_file_is_spelled_in_kebab_case() {
        let mut clock = Clock::default();
        clock.start(Duration::from_secs(60), None, 0);
        let json = serde_json::to_string(&clock).unwrap();
        assert!(json.contains(r#""ends-at":60000"#), "{json}");
        assert!(!json.contains('_'), "{json}");
    }

    /// The command crosses the socket with the rest of `Message`, so its
    /// spelling is part of what `lumen timer` sends.
    #[test]
    fn a_command_is_written_as_tagged_json() {
        let json = serde_json::to_string(&Command::Start {
            seconds: 1500,
            name: None,
            focus: false,
        })
        .unwrap();
        assert_eq!(json, r#"{"type":"start","seconds":1500,"name":null}"#);
    }
}
