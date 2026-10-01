//! The *Timers* view, as a function of the clock (F8).
//!
//! Built from nothing but the [`Clock`] and the time, so what the view shows
//! and what each action does are unit tests. The daemon redraws it when a
//! second on one of its faces turns over, and only while it is open: see
//! [`next_tick`].
//!
//! The vocabulary is F4's, unchanged: the time left is a text accessory and
//! *Paused* a tag. Every row has the same shape whatever state it is in, so
//! pausing a timer changes its text and never the list's geometry (I-10).

use std::time::Duration;

use super::duration::{clock_face, describe, stopwatch_face};
use super::{Clock, Millis, Preset, State, Stopwatch, Timer};
use crate::view::{
    Accessory, Action, EmptyState, Event, Filtering, Icon, Item, List, Section, Tone, View,
};

// Ids. Opaque to the host; only this file reads them back.
const TIMER: &str = "timer-";
const PAUSE: &str = "pause";
const RESUME: &str = "resume";
const RESTART: &str = "restart";
const STOP: &str = "stop";
const START: &str = "start";
const DISMISS: &str = "dismiss";
const WATCH: &str = "watch-";
const RESET: &str = "reset";
const SKIP: &str = "skip";
const REPEAT: &str = "repeat";
const ONE_SET: &str = "one-set";
const FOCUS_ON: &str = "focus-on";
const FOCUS_OFF: &str = "focus-off";
const SAVE: &str = "save-preset";
const PRESET: &str = "preset-";
const QUICK: &str = "quick-";

/// The glyph every timer row and the empty state draw.
pub const GLYPH: &str = "timer";

/// What the view shows at `now`: the timers, then what can be started --
/// the user's presets, then the quick durations (slice 2). Starting one keeps
/// the view open, so the new timer appears under *Running* as the answer.
pub fn view(clock: &Clock, presets: &[Preset], quick: &[Duration], now: Millis) -> View {
    let mut running: Vec<&Timer> = clock.timers().iter().filter(|t| t.is_running()).collect();
    running.sort_by_key(|timer| (timer.remaining(now), timer.id));
    let paused: Vec<&Timer> = clock
        .timers()
        .iter()
        .filter(|t| matches!(t.state, State::Paused { .. }))
        .collect();
    let ringing: Vec<&Timer> = clock.timers().iter().filter(|t| t.is_ringing()).collect();

    let sections = [
        // First: it is making a noise, and Enter should be what stops it.
        (
            "Ringing",
            ringing.into_iter().map(|t| item(t, now)).collect(),
        ),
        (
            "Running",
            running.into_iter().map(|t| item(t, now)).collect(),
        ),
        ("Paused", paused.into_iter().map(|t| item(t, now)).collect()),
        (
            "Stopwatches",
            clock
                .stopwatches()
                .iter()
                .map(|w| stopwatch(w, now))
                .collect(),
        ),
        ("Presets", presets.iter().map(preset).collect()),
        (
            "Quick Start",
            quick.iter().map(|d| quick_start(*d)).collect(),
        ),
    ]
    .into_iter()
    .filter(|(_, items): &(&str, Vec<Item>)| !items.is_empty())
    .map(|(title, items)| Section {
        title: Some(title.into()),
        subtitle: None,
        items,
    })
    .collect();

    View::List(List {
        showing_detail: false,
        placeholder: Some("Search timers".into()),
        filtering: Filtering::Host,
        scopes: Vec::new(),
        path: Vec::new(),
        empty: Some(EmptyState {
            title: "No timers running".into(),
            description: Some("Type a duration such as 25m in the launcher to start one.".into()),
            icon: Some(Icon::Glyph { name: GLYPH.into() }),
        }),
        sections,
    })
}

fn item(timer: &Timer, now: Millis) -> Item {
    let mut accessories = Vec::new();
    match timer.state {
        State::Paused { .. } => accessories.push(Accessory::Tag {
            text: "Paused".into(),
            tone: Tone::Caution,
        }),
        State::Ringing { .. } => accessories.push(Accessory::Tag {
            text: "Ringing".into(),
            tone: Tone::Accent,
        }),
        State::Running { .. } => {}
    }
    if timer.focus {
        accessories.push(Accessory::Tag {
            text: "Focus".into(),
            tone: Tone::Positive,
        });
    }
    accessories.push(Accessory::Text {
        text: clock_face(timer.remaining(now)),
        strong: false,
    });

    let toggle = match timer.state {
        State::Running { .. } => action(PAUSE, "Pause"),
        State::Paused { .. } => action(RESUME, "Resume"),
        State::Ringing { .. } => action(DISMISS, "Dismiss"),
    };

    // A cycle can move on early: a break not needed, work done sooner.
    let mut actions = vec![toggle];
    if let Some(cycle) = timer.cycle.filter(|_| !timer.is_ringing()) {
        actions.push(action(SKIP, "Skip to Next Phase"));
        actions.push(if cycle.plan.repeat {
            action(ONE_SET, "End After This Set")
        } else {
            action(REPEAT, "Repeat the Set")
        });
    }
    if !timer.is_ringing() {
        actions.push(if timer.focus {
            action(FOCUS_OFF, "Turn Focus Off")
        } else {
            action(FOCUS_ON, "Turn Focus On")
        });
    }
    actions.extend([
        action(RESTART, "Restart"),
        action(SAVE, "Save as Preset"),
        action(STOP, "Stop"),
    ]);

    // Every time something blocked was opened anyway, said where it counts.
    let overrides = match timer.overrides {
        0 => None,
        1 => Some("opened anyway once".to_string()),
        count => Some(format!("opened anyway {count} times")),
    };

    Item {
        detail: None,
        id: format!("{TIMER}{}", timer.id),
        title: timer.title(),
        // Where a cycle is; what a named timer counts; nothing for an
        // unnamed one, whose title already says it.
        subtitle: match (
            match (&timer.cycle, &timer.name) {
                (Some(cycle), _) if timer.is_ringing() => {
                    Some(format!("{} done", cycle.plan.describe()))
                }
                (Some(cycle), _) => Some(cycle.label()),
                (None, Some(_)) => Some(format!("{} timer", describe(timer.duration()))),
                (None, None) => None,
            },
            overrides,
        ) {
            (Some(what), Some(overrides)) => Some(format!("{what} \u{00b7} {overrides}")),
            (what, overrides) => what.or(overrides),
        },
        icon: Some(Icon::Glyph { name: GLYPH.into() }),
        accessories,
        keywords: timer.name.iter().cloned().collect(),
        actions,
    }
}

fn stopwatch(watch: &Stopwatch, now: Millis) -> Item {
    let mut accessories = Vec::new();
    if !watch.is_running() {
        accessories.push(Accessory::Tag {
            text: "Paused".into(),
            tone: Tone::Caution,
        });
    }
    accessories.push(Accessory::Text {
        text: stopwatch_face(watch.elapsed(now)),
        strong: false,
    });
    let toggle = if watch.is_running() {
        action(PAUSE, "Pause")
    } else {
        action(RESUME, "Resume")
    };
    Item {
        detail: None,
        id: format!("{WATCH}{}", watch.id),
        title: watch.title(),
        subtitle: None,
        icon: Some(Icon::Glyph { name: GLYPH.into() }),
        accessories,
        keywords: watch
            .name
            .iter()
            .cloned()
            .chain(["stopwatch".to_string()])
            .collect(),
        actions: vec![toggle, action(RESET, "Reset"), action(STOP, "Stop")],
    }
}

fn preset(preset: &Preset) -> Item {
    Item {
        detail: None,
        id: format!("{PRESET}{}", preset.name),
        title: preset.name.clone(),
        subtitle: Some(match &preset.cycle {
            Some(plan) => plan.describe(),
            None => describe(preset.duration),
        }),
        icon: Some(Icon::Glyph { name: GLYPH.into() }),
        // The alias, where the root list's rows show their kind: the word
        // that finds this preset from anywhere.
        accessories: preset
            .alias
            .iter()
            .map(|alias| Accessory::Text {
                text: alias.clone(),
                strong: false,
            })
            .collect(),
        keywords: preset.alias.iter().cloned().collect(),
        actions: vec![action(START, "Start")],
    }
}

fn quick_start(duration: Duration) -> Item {
    Item {
        detail: None,
        id: format!("{QUICK}{}", duration.as_secs()),
        title: describe(duration),
        subtitle: None,
        icon: Some(Icon::Glyph { name: GLYPH.into() }),
        accessories: Vec::new(),
        keywords: Vec::new(),
        actions: vec![action(START, "Start")],
    }
}

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
    }
}

/// Carry out what the view reported. Returns whether the clock changed, and
/// with it the file and the next deadline.
///
/// A preset is found by name in `presets` as they are now, so one deleted
/// since the view was drawn starts nothing.
pub fn act(clock: &mut Clock, event: &Event, presets: &[Preset], now: Millis) -> bool {
    let Event::ActionInvoked {
        item: Some(item),
        action,
    } = event
    else {
        return false;
    };
    if action == START {
        if let Some(name) = item.strip_prefix(PRESET) {
            let Some(preset) = presets.iter().find(|preset| preset.name == name) else {
                return false;
            };
            let name = Some(preset.name.clone());
            match preset.cycle {
                Some(plan) => clock.start_cycle(plan, name, now),
                None => clock.start(preset.duration, name, now),
            };
            return true;
        }
        if let Some(seconds) = item.strip_prefix(QUICK).and_then(|s| s.parse().ok()) {
            clock.start(Duration::from_secs(seconds), None, now);
            return true;
        }
        return false;
    }
    if let Some(id) = item.strip_prefix(WATCH).and_then(|id| id.parse().ok()) {
        return match action.as_str() {
            PAUSE => clock.pause_stopwatch(id, now),
            RESUME => clock.resume_stopwatch(id, now),
            RESET => clock.reset_stopwatch(id, now),
            STOP => clock.stop_stopwatch(id).is_some(),
            _ => false,
        };
    }
    let Some(id) = item.strip_prefix(TIMER).and_then(|id| id.parse().ok()) else {
        return false;
    };
    match action.as_str() {
        PAUSE => clock.pause(id, now),
        RESUME => clock.resume(id, now),
        RESTART => clock.restart(id, now),
        STOP | DISMISS => clock.stop(id).is_some(),
        SKIP => clock.skip(id, now),
        REPEAT => clock.set_repeat(id, true),
        ONE_SET => clock.set_repeat(id, false),
        FOCUS_ON => clock.set_focus(id, true),
        FOCUS_OFF => clock.set_focus(id, false),
        _ => false,
    }
}

/// One line for the launcher's footer (F8): the thing most worth knowing,
/// then how many more there are. Ringing first, then the running timer
/// ending soonest, then a paused one, then a stopwatch. `None` with nothing
/// running, so the footer is the launcher's again.
pub fn summary(clock: &Clock, now: Millis) -> Option<String> {
    let timer_line = |timer: &Timer| {
        let title = match timer.cycle {
            Some(cycle) => format!("{} \u{00b7} {}", timer.title(), cycle.label()),
            None => timer.title(),
        };
        match timer.state {
            State::Ringing { .. } => format!("Ringing \u{00b7} {title}"),
            State::Paused { .. } => {
                format!(
                    "Paused {} \u{00b7} {title}",
                    clock_face(timer.remaining(now))
                )
            }
            State::Running { .. } => {
                format!("{} \u{00b7} {title}", clock_face(timer.remaining(now)))
            }
        }
    };
    let timers = clock.timers();
    let first = timers
        .iter()
        .find(|timer| timer.is_ringing())
        .or_else(|| {
            timers
                .iter()
                .filter(|timer| timer.is_running())
                .min_by_key(|timer| timer.remaining(now))
        })
        .or_else(|| timers.first())
        .map(timer_line)
        .or_else(|| {
            let watches = clock.stopwatches();
            watches
                .iter()
                .find(|watch| watch.is_running())
                .or_else(|| watches.first())
                .map(|watch| {
                    let paused = if watch.is_running() { "" } else { "Paused " };
                    format!(
                        "{paused}{} \u{00b7} {}",
                        stopwatch_face(watch.elapsed(now)),
                        watch.title()
                    )
                })
        })?;
    let more = timers.len() + clock.stopwatches().len() - 1;
    Some(match more {
        0 => first,
        more => format!("{first}  +{more}"),
    })
}

/// The preset to fill the form with, when the view asked to save a timer
/// as one. The daemon hands it to the launcher's form; the clock is not
/// changed.
pub fn saving(clock: &Clock, event: &Event) -> Option<super::PresetConfig> {
    let Event::ActionInvoked {
        item: Some(item),
        action,
    } = event
    else {
        return None;
    };
    if action != SAVE {
        return None;
    }
    let id = item.strip_prefix(TIMER)?.parse().ok()?;
    clock.get(id).map(Timer::as_preset)
}

/// How long until a face the view shows changes, or `None` when none will:
/// nothing is running, and the open view has no reason to wake anything.
///
/// A face reads whole seconds rounded up, so it turns over when a timer's
/// remaining milliseconds cross a multiple of a thousand -- which is not when
/// the wall clock's seconds do, since a timer starts whenever it was started.
pub fn next_tick(clock: &Clock, now: Millis) -> Option<Duration> {
    let timers = clock
        .timers()
        .iter()
        .filter(|timer| timer.is_running())
        .map(|timer| {
            let left = timer.remaining(now).as_millis() as u64;
            match left % 1_000 {
                0 => 1_000,
                part => part,
            }
        });
    // A stopwatch's face rounds down, so it turns over when the count
    // reaches the next whole second.
    let watches = clock
        .stopwatches()
        .iter()
        .filter(|watch| watch.is_running())
        .map(|watch| 1_000 - watch.elapsed(now).as_millis() as u64 % 1_000);
    timers.chain(watches).min().map(Duration::from_millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(view: View) -> List {
        match view {
            View::List(list) => list,
            View::Detail(_) => panic!("the timers view is a list"),
        }
    }

    fn texts(item: &Item) -> Vec<String> {
        item.accessories
            .iter()
            .map(|accessory| match accessory {
                Accessory::Text { text, .. } | Accessory::Tag { text, .. } => text.clone(),
            })
            .collect()
    }

    fn invoke(item: &str, action: &str) -> Event {
        Event::ActionInvoked {
            item: Some(item.into()),
            action: action.into(),
        }
    }

    #[test]
    fn no_timers_is_an_empty_state_that_says_how_to_start_one() {
        let list = list(view(&Clock::default(), &[], &[], 0));
        assert!(list.is_empty());
        assert!(list.empty.unwrap().description.unwrap().contains("25m"));
    }

    #[test]
    fn running_timers_come_first_soonest_first_then_paused() {
        let mut clock = Clock::default();
        clock.start(Duration::from_secs(600), Some("long".into()), 0);
        let paused = clock.start(Duration::from_secs(60), Some("paused".into()), 0);
        clock.start(Duration::from_secs(120), Some("short".into()), 0);
        clock.pause(paused, 0);

        let list = list(view(&clock, &[], &[], 0));

        let titles: Vec<_> = list.items().map(|item| item.title.clone()).collect();
        assert_eq!(titles, ["short", "long", "paused"]);
        assert_eq!(list.sections[0].title.as_deref(), Some("Running"));
        assert_eq!(list.sections[1].title.as_deref(), Some("Paused"));
    }

    #[test]
    fn a_row_shows_the_time_left_and_a_paused_one_says_so() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(25 * 60), None, 0);

        let running = list(view(&clock, &[], &[], 60_000));
        assert_eq!(texts(running.item(0).unwrap()), ["24:00"]);

        clock.pause(id, 60_000);
        let paused = list(view(&clock, &[], &[], 10 * 60_000));
        assert_eq!(texts(paused.item(0).unwrap()), ["Paused", "24:00"]);
    }

    #[test]
    fn enter_pauses_a_running_timer_and_resumes_a_paused_one() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(60), None, 0);
        let row = format!("timer-{id}");

        let first = list(view(&clock, &[], &[], 0)).item(0).unwrap().actions[0]
            .id
            .clone();
        assert!(act(&mut clock, &invoke(&row, &first), &[], 0));
        assert!(!clock.get(id).unwrap().is_running());

        let first = list(view(&clock, &[], &[], 0)).item(0).unwrap().actions[0]
            .id
            .clone();
        assert!(act(&mut clock, &invoke(&row, &first), &[], 0));
        assert!(clock.get(id).unwrap().is_running());
    }

    #[test]
    fn stop_removes_the_timer_and_restart_counts_again() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(60), None, 0);
        let row = format!("timer-{id}");

        assert!(act(&mut clock, &invoke(&row, "restart"), &[], 30_000));
        assert_eq!(clock.next_deadline(), Some(90_000));
        assert!(act(&mut clock, &invoke(&row, "stop"), &[], 30_000));
        assert!(clock.is_empty());
    }

    /// A view that has not re-rendered yet may name a timer that has ended.
    #[test]
    fn an_action_on_a_timer_that_is_gone_changes_nothing() {
        let mut clock = Clock::default();
        assert!(!act(&mut clock, &invoke("timer-7", "stop"), &[], 0));
        assert!(!act(&mut clock, &invoke("nonsense", "stop"), &[], 0));
        assert!(!act(&mut clock, &Event::Dismissed, &[], 0));
    }

    fn deep_work() -> Preset {
        Preset {
            name: "Deep work".into(),
            alias: Some("dw".into()),
            duration: Duration::from_secs(90 * 60),
            cycle: None,
            focus: false,
        }
    }

    #[test]
    fn what_can_be_started_follows_what_is_running() {
        let mut clock = Clock::default();
        clock.start(Duration::from_secs(60), None, 0);
        let quick = [Duration::from_secs(300)];

        let list = list(view(&clock, &[deep_work()], &quick, 0));

        let sections: Vec<_> = list
            .sections
            .iter()
            .map(|s| s.title.clone().unwrap())
            .collect();
        assert_eq!(sections, ["Running", "Presets", "Quick Start"]);
        let preset = list.item(1).unwrap();
        assert_eq!(preset.title, "Deep work");
        assert_eq!(preset.subtitle.as_deref(), Some("1 h 30 min"));
        assert_eq!(texts(preset), ["dw"]);
        assert_eq!(list.item(2).unwrap().title, "5 min");
    }

    #[test]
    fn starting_a_preset_names_the_timer_and_a_quick_one_does_not() {
        let mut clock = Clock::default();
        let presets = [deep_work()];

        assert!(act(
            &mut clock,
            &invoke("preset-Deep work", "start"),
            &presets,
            0
        ));
        assert!(act(&mut clock, &invoke("quick-300", "start"), &presets, 0));

        let titles: Vec<_> = clock.timers().iter().map(Timer::title).collect();
        assert_eq!(titles, ["Deep work", "5 min timer"]);
        assert_eq!(clock.next_deadline(), Some(300_000));
    }

    /// The view was drawn before the preset was deleted in settings.
    #[test]
    fn a_preset_that_is_gone_starts_nothing() {
        let mut clock = Clock::default();
        assert!(!act(&mut clock, &invoke("preset-Gone", "start"), &[], 0));
        assert!(!act(&mut clock, &invoke("quick-soon", "start"), &[], 0));
        assert!(clock.is_empty());
    }

    #[test]
    fn a_cycle_says_where_it_is_and_can_skip() {
        let mut clock = Clock::default();
        let id = clock.start_cycle(super::super::Plan::POMODORO, Some("Pomodoro".into()), 0);

        let row = list(view(&clock, &[], &[], 0)).item(0).unwrap().clone();
        assert_eq!(row.title, "Pomodoro");
        assert_eq!(row.subtitle.as_deref(), Some("Work 1 of 4"));
        assert_eq!(row.actions[1].title, "Skip to Next Phase");
        assert_eq!(row.actions[2].title, "Repeat the Set");
        assert!(act(
            &mut clock,
            &invoke(&format!("timer-{id}"), "repeat"),
            &[],
            0
        ));
        assert!(clock.get(id).unwrap().cycle.unwrap().plan.repeat);

        assert!(act(
            &mut clock,
            &invoke(&format!("timer-{id}"), "skip"),
            &[],
            60_000
        ));
        let row = list(view(&clock, &[], &[], 60_000))
            .item(0)
            .unwrap()
            .clone();
        assert_eq!(row.subtitle.as_deref(), Some("Break"));
        assert_eq!(texts(&row), ["5:00"]);
    }

    #[test]
    fn a_focus_timer_says_so_and_counts_what_was_opened_anyway() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(600), Some("Write".into()), 0);
        let row_id = format!("timer-{id}");

        let row = list(view(&clock, &[], &[], 0)).item(0).unwrap().clone();
        assert_eq!(row.actions[1].title, "Turn Focus On");
        assert!(act(&mut clock, &invoke(&row_id, "focus-on"), &[], 0));
        clock.count_override(id);
        clock.count_override(id);

        let row = list(view(&clock, &[], &[], 0)).item(0).unwrap().clone();
        assert_eq!(texts(&row), ["Focus", "10:00"]);
        assert_eq!(
            row.subtitle.as_deref(),
            Some("10 min timer \u{00b7} opened anyway 2 times")
        );
        assert_eq!(row.actions[1].title, "Turn Focus Off");
    }

    #[test]
    fn a_running_timer_can_be_saved_as_a_preset() {
        let mut clock = Clock::default();
        let id = clock.start_cycle(super::super::Plan::POMODORO, Some("Pomodoro".into()), 0);
        clock.set_focus(id, true);

        let preset = saving(&clock, &invoke(&format!("timer-{id}"), "save-preset")).unwrap();
        assert_eq!(preset.name, "Pomodoro");
        assert_eq!(preset.duration, "25m");
        assert_eq!(preset.short_break.as_deref(), Some("5m"));
        assert!(preset.focus);
        assert!(saving(&clock, &invoke(&format!("timer-{id}"), "stop")).is_none());
    }

    #[test]
    fn the_footer_names_what_ends_soonest_and_counts_the_rest() {
        let mut clock = Clock::default();
        assert_eq!(summary(&clock, 0), None);

        clock.start(Duration::from_secs(600), Some("Write".into()), 0);
        clock.start(Duration::from_secs(120), Some("Tea".into()), 0);
        clock.start_stopwatch(None, 0);
        assert_eq!(summary(&clock, 0).as_deref(), Some("2:00 \u{00b7} Tea  +2"));

        clock.expire(120_000, true);
        assert_eq!(
            summary(&clock, 120_000).as_deref(),
            Some("Ringing \u{00b7} Tea  +2")
        );
    }

    #[test]
    fn a_lone_stopwatch_or_cycle_says_what_it_is() {
        let mut clock = Clock::default();
        clock.start_stopwatch(Some("Call".into()), 0);
        assert_eq!(
            summary(&clock, 65_000).as_deref(),
            Some("1:05 \u{00b7} Call")
        );

        let mut clock = Clock::default();
        clock.start_cycle(super::super::Plan::POMODORO, Some("Pomodoro".into()), 0);
        assert_eq!(
            summary(&clock, 0).as_deref(),
            Some("25:00 \u{00b7} Pomodoro \u{00b7} Work 1 of 4")
        );
    }

    #[test]
    fn a_plain_timer_offers_no_skip() {
        let mut clock = Clock::default();
        clock.start(Duration::from_secs(60), None, 0);
        let row = list(view(&clock, &[], &[], 0)).item(0).unwrap().clone();
        assert!(row.actions.iter().all(|action| action.id != "skip"));
    }

    #[test]
    fn a_ringing_timer_leads_and_enter_dismisses_it() {
        let mut clock = Clock::default();
        let runs = clock.start(Duration::from_secs(600), Some("runs".into()), 0);
        let rang = clock.start(Duration::from_secs(60), Some("rang".into()), 0);
        clock.expire(60_000, true);

        let list = list(view(&clock, &[], &[], 60_000));
        assert_eq!(list.sections[0].title.as_deref(), Some("Ringing"));
        let row = list.item(0).unwrap();
        assert_eq!(row.title, "rang");
        assert_eq!(texts(row), ["Ringing", "0:00"]);
        assert_eq!(row.actions[0].title, "Dismiss");
        // Nothing to count on its face: it does not wake the view.
        clock.pause(runs, 60_000);
        assert_eq!(next_tick(&clock, 60_000), None);

        let id = format!("timer-{rang}");
        assert!(act(&mut clock, &invoke(&id, "dismiss"), &[], 60_000));
        assert!(!clock.is_ringing());
    }

    #[test]
    fn a_stopwatch_counts_up_below_the_timers_and_can_be_reset() {
        let mut clock = Clock::default();
        clock.start(Duration::from_secs(600), None, 0);
        let id = clock.start_stopwatch(Some("call".into()), 0);

        let list = list(view(&clock, &[], &[], 75_400));
        let sections: Vec<_> = list
            .sections
            .iter()
            .map(|s| s.title.clone().unwrap())
            .collect();
        assert_eq!(sections, ["Running", "Stopwatches"]);
        let row = list.item(1).unwrap();
        assert_eq!(row.title, "call");
        assert_eq!(texts(row), ["1:15"]);

        let watch = format!("watch-{id}");
        assert!(act(&mut clock, &invoke(&watch, "pause"), &[], 80_000));
        let paused = list_of(&clock, 999_000);
        assert_eq!(texts(paused.item(1).unwrap()), ["Paused", "1:20"]);

        assert!(act(&mut clock, &invoke(&watch, "reset"), &[], 1_000_000));
        assert_eq!(clock.stopwatches()[0].elapsed(1_000_000), Duration::ZERO);
        assert!(act(&mut clock, &invoke(&watch, "stop"), &[], 1_000_000));
        assert!(clock.stopwatches().is_empty());
    }

    fn list_of(clock: &Clock, now: Millis) -> List {
        list(view(clock, &[], &[], now))
    }

    #[test]
    fn a_running_stopwatch_wakes_the_open_view_at_the_next_whole_second() {
        let mut clock = Clock::default();
        let id = clock.start_stopwatch(None, 0);
        assert_eq!(next_tick(&clock, 250), Some(Duration::from_millis(750)));
        assert_eq!(next_tick(&clock, 1_000), Some(Duration::from_millis(1_000)));
        clock.pause_stopwatch(id, 1_000);
        assert_eq!(next_tick(&clock, 1_000), None);
    }

    #[test]
    fn the_view_wakes_when_a_face_turns_over_and_not_otherwise() {
        let mut clock = Clock::default();
        assert_eq!(next_tick(&clock, 0), None);

        // Started at 0, so at 250 it has 59_750 left: the face reads 1:00
        // until 59_000 are left, 750 ms from now.
        let id = clock.start(Duration::from_secs(60), None, 0);
        assert_eq!(next_tick(&clock, 250), Some(Duration::from_millis(750)));
        assert_eq!(next_tick(&clock, 1_000), Some(Duration::from_millis(1_000)));

        clock.pause(id, 1_000);
        assert_eq!(next_tick(&clock, 1_000), None);
    }
}
