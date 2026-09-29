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

use super::duration::{clock_face, describe};
use super::{Clock, Millis, Preset, Timer};
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
    let paused: Vec<&Timer> = clock.timers().iter().filter(|t| !t.is_running()).collect();

    let sections = [
        (
            "Running",
            running.into_iter().map(|t| item(t, now)).collect(),
        ),
        ("Paused", paused.into_iter().map(|t| item(t, now)).collect()),
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
        items,
    })
    .collect();

    View::List(List {
        placeholder: Some("Search timers".into()),
        filtering: Filtering::Host,
        scopes: Vec::new(),
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
    if !timer.is_running() {
        accessories.push(Accessory::Tag {
            text: "Paused".into(),
            tone: Tone::Caution,
        });
    }
    accessories.push(Accessory::Text {
        text: clock_face(timer.remaining(now)),
    });

    let toggle = if timer.is_running() {
        action(PAUSE, "Pause")
    } else {
        action(RESUME, "Resume")
    };

    Item {
        id: format!("{TIMER}{}", timer.id),
        title: timer.title(),
        // A named timer says what it counts; an unnamed one already does.
        subtitle: timer
            .name
            .as_ref()
            .map(|_| format!("{} timer", describe(timer.duration()))),
        icon: Some(Icon::Glyph { name: GLYPH.into() }),
        accessories,
        keywords: timer.name.iter().cloned().collect(),
        actions: vec![toggle, action(RESTART, "Restart"), action(STOP, "Stop")],
    }
}

fn preset(preset: &Preset) -> Item {
    Item {
        id: format!("{PRESET}{}", preset.name),
        title: preset.name.clone(),
        subtitle: Some(describe(preset.duration)),
        icon: Some(Icon::Glyph { name: GLYPH.into() }),
        // The alias, where the root list's rows show their kind: the word
        // that finds this preset from anywhere.
        accessories: preset
            .alias
            .iter()
            .map(|alias| Accessory::Text {
                text: alias.clone(),
            })
            .collect(),
        keywords: preset.alias.iter().cloned().collect(),
        actions: vec![action(START, "Start")],
    }
}

fn quick_start(duration: Duration) -> Item {
    Item {
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
            clock.start(preset.duration, Some(preset.name.clone()), now);
            return true;
        }
        if let Some(seconds) = item.strip_prefix(QUICK).and_then(|s| s.parse().ok()) {
            clock.start(Duration::from_secs(seconds), None, now);
            return true;
        }
        return false;
    }
    let Some(id) = item.strip_prefix(TIMER).and_then(|id| id.parse().ok()) else {
        return false;
    };
    match action.as_str() {
        PAUSE => clock.pause(id, now),
        RESUME => clock.resume(id, now),
        RESTART => clock.restart(id, now),
        STOP => clock.stop(id).is_some(),
        _ => false,
    }
}

/// How long until a face the view shows changes, or `None` when none will:
/// nothing is running, and the open view has no reason to wake anything.
///
/// A face reads whole seconds rounded up, so it turns over when a timer's
/// remaining milliseconds cross a multiple of a thousand -- which is not when
/// the wall clock's seconds do, since a timer starts whenever it was started.
pub fn next_tick(clock: &Clock, now: Millis) -> Option<Duration> {
    clock
        .timers()
        .iter()
        .filter(|timer| timer.is_running())
        .map(|timer| {
            let left = timer.remaining(now).as_millis() as u64;
            match left % 1_000 {
                0 => 1_000,
                part => part,
            }
        })
        .min()
        .map(Duration::from_millis)
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
                Accessory::Text { text } | Accessory::Tag { text, .. } => text.clone(),
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
