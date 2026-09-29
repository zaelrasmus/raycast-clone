//! Cycles (F8, slice 5): a timer whose work and breaks follow each other.
//!
//! Pomodoro is not a feature of its own. It is a timer with a *plan* -- work
//! for so long, break for so long, so many rounds, a long break at the end --
//! and the clock moves it from one phase to the next by itself, as a
//! timer's deadline passes. Each phase is an ordinary countdown, so pausing,
//! restarting and the one sleep until the next deadline work unchanged.
//!
//! # One set, then done -- unless asked to repeat
//!
//! By default a plan has an end: `rounds` work phases with a break between
//! each, then the long break, and the timer ends like any other -- with the
//! alert, and ringing if asked. A plan that `repeat`s starts the set again
//! after the long break instead, until it is stopped; that is a choice made
//! on purpose, per preset or per start, because a cycle nobody meant to
//! leave running would still be going at midnight.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::duration::describe;
use super::{Clock, Millis, State, Timer};

/// The most rounds a plan may have: a day's work, not a typo's.
pub const MAX_ROUNDS: u32 = 12;

/// How a cycle goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Plan {
    pub work_secs: u64,
    pub break_secs: u64,
    /// After the last work phase. Zero is none: the cycle ends with the work.
    pub long_break_secs: u64,
    pub rounds: u32,
    /// Start the set again after it ends, until stopped. Absent in a file
    /// written before it existed, and so off.
    #[serde(default)]
    pub repeat: bool,
}

impl Plan {
    /// Francesco Cirillo's: four rounds of 25 minutes, 5 minute breaks, and a
    /// longer one at the end.
    pub const POMODORO: Plan = Plan {
        work_secs: 25 * 60,
        break_secs: 5 * 60,
        long_break_secs: 15 * 60,
        rounds: 4,
        repeat: false,
    };

    /// The same plan, going round again after each set.
    pub fn repeating(self) -> Plan {
        Plan {
            repeat: true,
            ..self
        }
    }

    /// The plan in words: "4 × 25 min, 5 min breaks, then 15 min".
    pub fn describe(&self) -> String {
        let mut text = format!(
            "{} \u{00d7} {}, {} breaks",
            self.rounds,
            describe(Duration::from_secs(self.work_secs)),
            describe(Duration::from_secs(self.break_secs)),
        );
        if self.long_break_secs > 0 {
            text.push_str(&format!(
                ", then {}",
                describe(Duration::from_secs(self.long_break_secs))
            ));
        }
        if self.repeat {
            text.push_str(", repeating");
        }
        text
    }

    fn first(self) -> Cycle {
        Cycle {
            plan: self,
            round: 1,
            phase: Phase::Work,
        }
    }
}

/// Where in its plan a cycle is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Cycle {
    pub plan: Plan,
    /// Which work phase, from 1; a break belongs to the work before it.
    pub round: u32,
    pub phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Work,
    Break,
    LongBreak,
}

impl Cycle {
    /// How long the phase it is in lasts.
    pub fn length(&self) -> Duration {
        Duration::from_secs(match self.phase {
            Phase::Work => self.plan.work_secs,
            Phase::Break => self.plan.break_secs,
            Phase::LongBreak => self.plan.long_break_secs,
        })
    }

    /// The phase after this one, or `None` when the plan is done.
    pub fn next(&self) -> Option<Cycle> {
        let last = self.round >= self.plan.rounds;
        let repeat = self.plan.repeat;
        let (round, phase) = match self.phase {
            Phase::Work if !last => (self.round, Phase::Break),
            Phase::Work if self.plan.long_break_secs > 0 => (self.round, Phase::LongBreak),
            // No long break: a repeating set rests with an ordinary one.
            Phase::Work if repeat => (self.round, Phase::Break),
            Phase::Work => return None,
            Phase::Break if last => (1, Phase::Work),
            Phase::Break => (self.round + 1, Phase::Work),
            Phase::LongBreak if repeat => (1, Phase::Work),
            Phase::LongBreak => return None,
        };
        Some(Cycle {
            plan: self.plan,
            round,
            phase,
        })
    }

    /// The phase in words, for a row: "Work 2 of 4", "Break", "Long break".
    pub fn label(&self) -> String {
        match self.phase {
            Phase::Work => format!("Work {} of {}", self.round, self.plan.rounds),
            Phase::Break => "Break".into(),
            Phase::LongBreak => "Long break".into(),
        }
    }
}

impl Timer {
    /// Move to the phase after this one, its deadline counted from `from`.
    /// Returns whether there was one.
    pub(super) fn advance(&mut self, from: Millis) -> bool {
        let Some(next) = self.cycle.as_ref().and_then(Cycle::next) else {
            return false;
        };
        let length = next.length().as_millis() as u64;
        self.cycle = Some(next);
        self.duration_ms = length;
        self.state = State::Running {
            ends_at: from + length,
        };
        true
    }
}

impl Clock {
    /// Start `plan` at its first work phase, and return the new timer's id.
    pub fn start_cycle(&mut self, plan: Plan, name: Option<String>, now: Millis) -> u64 {
        let first = plan.first();
        let id = self.start(first.length(), name, now);
        if let Some(timer) = self.get_mut(id) {
            timer.cycle = Some(first);
        }
        id
    }

    /// Make a running cycle repeat, or end after this set. Returns whether
    /// anything changed.
    pub fn set_repeat(&mut self, id: u64, repeat: bool) -> bool {
        let Some(cycle) = self.get_mut(id).and_then(|timer| timer.cycle.as_mut()) else {
            return false;
        };
        if cycle.plan.repeat == repeat {
            return false;
        }
        cycle.plan.repeat = repeat;
        true
    }

    /// End the phase now and start the next: a break not needed, a work
    /// phase finished early. The last phase skipped is the cycle stopped.
    pub fn skip(&mut self, id: u64, now: Millis) -> bool {
        let Some(timer) = self.get_mut(id) else {
            return false;
        };
        if timer.cycle.is_none() {
            return false;
        }
        if !timer.advance(now) {
            return self.stop(id).is_some();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    fn plan() -> Plan {
        Plan {
            work_secs: 25 * 60,
            break_secs: 5 * 60,
            long_break_secs: 15 * 60,
            rounds: 2,
            repeat: false,
        }
    }

    fn phase(clock: &Clock, id: u64) -> (u32, Phase) {
        let cycle = clock.get(id).unwrap().cycle.unwrap();
        (cycle.round, cycle.phase)
    }

    #[test]
    fn a_plan_goes_work_break_work_long_break_then_ends() {
        let mut cycle = plan().first();
        let mut seen = vec![(cycle.round, cycle.phase)];
        while let Some(next) = cycle.next() {
            seen.push((next.round, next.phase));
            cycle = next;
        }
        assert_eq!(
            seen,
            [
                (1, Phase::Work),
                (1, Phase::Break),
                (2, Phase::Work),
                (2, Phase::LongBreak)
            ]
        );
    }

    #[test]
    fn a_repeating_plan_starts_again_after_the_long_break() {
        let long_break = Cycle {
            plan: plan().repeating(),
            round: 2,
            phase: Phase::LongBreak,
        };
        let next = long_break.next().unwrap();
        assert_eq!((next.round, next.phase), (1, Phase::Work));
    }

    #[test]
    fn a_repeating_plan_with_no_long_break_rests_with_a_short_one() {
        let last_work = Cycle {
            plan: Plan {
                long_break_secs: 0,
                ..plan().repeating()
            },
            round: 2,
            phase: Phase::Work,
        };
        let rest = last_work.next().unwrap();
        assert_eq!((rest.round, rest.phase), (2, Phase::Break));
        let again = rest.next().unwrap();
        assert_eq!((again.round, again.phase), (1, Phase::Work));
    }

    #[test]
    fn repeating_can_be_turned_on_and_off_while_it_runs() {
        let mut clock = Clock::default();
        let id = clock.start_cycle(plan(), None, 0);
        assert!(clock.set_repeat(id, true));
        assert!(!clock.set_repeat(id, true));
        assert!(clock.get(id).unwrap().cycle.unwrap().plan.repeat);

        // Past the end of one set, it is still going.
        clock.expire(2 * 60 * MIN, false);
        assert_eq!(clock.timers().len(), 1);
        assert!(clock.set_repeat(id, false));
    }

    /// Files written before `repeat` existed still load.
    #[test]
    fn a_plan_without_repeat_does_not_repeat() {
        let plan: Plan = serde_json::from_str(
            r#"{ "work-secs": 1500, "break-secs": 300, "long-break-secs": 900, "rounds": 4 }"#,
        )
        .unwrap();
        assert_eq!(plan, Plan::POMODORO);
    }

    #[test]
    fn with_no_long_break_it_ends_after_the_last_work() {
        let short = Plan {
            long_break_secs: 0,
            ..plan()
        };
        let last_work = Cycle {
            plan: short,
            round: 2,
            phase: Phase::Work,
        };
        assert_eq!(last_work.next(), None);
    }

    #[test]
    fn each_phase_follows_the_last_by_itself() {
        let mut clock = Clock::default();
        let id = clock.start_cycle(plan(), Some("Pomodoro".into()), 0);
        assert_eq!(clock.next_deadline(), Some(25 * MIN));

        let ended = clock.expire(25 * MIN, false);
        assert!(matches!(ended.as_slice(), [super::super::Ending::Next(_)]));
        assert_eq!(phase(&clock, id), (1, Phase::Break));
        // Counted from the deadline, not from when the daemon woke.
        assert_eq!(clock.next_deadline(), Some(30 * MIN));
    }

    /// The machine slept through a break and into the next work phase: one
    /// announcement, of where the cycle is now.
    #[test]
    fn a_late_wake_catches_up_through_several_phases_at_once() {
        let mut clock = Clock::default();
        let id = clock.start_cycle(plan(), None, 0);

        let ended = clock.expire(40 * MIN, false);

        assert_eq!(ended.len(), 1);
        assert_eq!(phase(&clock, id), (2, Phase::Work));
        assert_eq!(clock.next_deadline(), Some(55 * MIN));
    }

    #[test]
    fn the_last_phase_ends_the_timer_like_any_other() {
        let mut clock = Clock::default();
        clock.start_cycle(plan(), None, 0);

        let ended = clock.expire(24 * 60 * MIN, true);

        assert!(matches!(ended.as_slice(), [super::super::Ending::Done(_)]));
        assert!(clock.is_ringing());
    }

    #[test]
    fn skipping_starts_the_next_phase_now_and_the_last_skip_stops_it() {
        let mut clock = Clock::default();
        let id = clock.start_cycle(plan(), None, 0);
        clock.pause(id, MIN);

        assert!(clock.skip(id, 2 * MIN));
        assert_eq!(phase(&clock, id), (1, Phase::Break));
        assert_eq!(clock.next_deadline(), Some(7 * MIN));

        assert!(clock.skip(id, 3 * MIN));
        assert!(clock.skip(id, 4 * MIN));
        assert_eq!(phase(&clock, id), (2, Phase::LongBreak));
        assert!(clock.skip(id, 5 * MIN));
        assert!(clock.is_empty());
    }

    #[test]
    fn a_plain_timer_cannot_be_skipped() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(60), None, 0);
        assert!(!clock.skip(id, 0));
        assert_eq!(clock.timers().len(), 1);
    }

    #[test]
    fn a_plan_is_described_in_words() {
        assert_eq!(
            Plan::POMODORO.describe(),
            "4 \u{00d7} 25 min, 5 min breaks, then 15 min"
        );
        assert!(
            Plan::POMODORO
                .repeating()
                .describe()
                .ends_with(", repeating")
        );
    }

    #[test]
    fn a_cycle_survives_a_restart_where_it_was() {
        let dir = std::env::temp_dir().join(format!("lumen-cycle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("timers.json");

        let mut clock = Clock::default();
        let id = clock.start_cycle(plan(), Some("Pomodoro".into()), 0);
        clock.expire(25 * MIN, false);
        clock.save(&path).unwrap();

        let loaded = Clock::load(&path);
        assert_eq!(loaded, clock);
        assert_eq!(phase(&loaded, id), (1, Phase::Break));
        std::fs::remove_dir_all(&dir).ok();
    }
}
