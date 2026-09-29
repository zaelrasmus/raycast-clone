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
use super::{Clock, Millis, Timer};
use crate::view::{
    Accessory, Action, EmptyState, Event, Filtering, Icon, Item, List, Section, Tone, View,
};

// Ids. Opaque to the host; only this file reads them back.
const TIMER: &str = "timer-";
const PAUSE: &str = "pause";
const RESUME: &str = "resume";
const RESTART: &str = "restart";
const STOP: &str = "stop";

/// The glyph every timer row and the empty state draw.
pub const GLYPH: &str = "timer";

/// What the view shows at `now`.
pub fn view(clock: &Clock, now: Millis) -> View {
    let mut running: Vec<&Timer> = clock.timers().iter().filter(|t| t.is_running()).collect();
    running.sort_by_key(|timer| (timer.remaining(now), timer.id));
    let paused: Vec<&Timer> = clock.timers().iter().filter(|t| !t.is_running()).collect();

    let sections = [("Running", running), ("Paused", paused)]
        .into_iter()
        .filter(|(_, timers)| !timers.is_empty())
        .map(|(title, timers)| Section {
            title: Some(title.into()),
            items: timers.into_iter().map(|timer| item(timer, now)).collect(),
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

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
    }
}

/// Carry out what the view reported. Returns whether the clock changed, and
/// with it the file and the next deadline.
pub fn act(clock: &mut Clock, event: &Event, now: Millis) -> bool {
    let Event::ActionInvoked {
        item: Some(item),
        action,
    } = event
    else {
        return false;
    };
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
        let list = list(view(&Clock::default(), 0));
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

        let list = list(view(&clock, 0));

        let titles: Vec<_> = list.items().map(|item| item.title.clone()).collect();
        assert_eq!(titles, ["short", "long", "paused"]);
        assert_eq!(list.sections[0].title.as_deref(), Some("Running"));
        assert_eq!(list.sections[1].title.as_deref(), Some("Paused"));
    }

    #[test]
    fn a_row_shows_the_time_left_and_a_paused_one_says_so() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(25 * 60), None, 0);

        let running = list(view(&clock, 60_000));
        assert_eq!(texts(running.item(0).unwrap()), ["24:00"]);

        clock.pause(id, 60_000);
        let paused = list(view(&clock, 10 * 60_000));
        assert_eq!(texts(paused.item(0).unwrap()), ["Paused", "24:00"]);
    }

    #[test]
    fn enter_pauses_a_running_timer_and_resumes_a_paused_one() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(60), None, 0);
        let row = format!("timer-{id}");

        let first = list(view(&clock, 0)).item(0).unwrap().actions[0].id.clone();
        assert!(act(&mut clock, &invoke(&row, &first), 0));
        assert!(!clock.get(id).unwrap().is_running());

        let first = list(view(&clock, 0)).item(0).unwrap().actions[0].id.clone();
        assert!(act(&mut clock, &invoke(&row, &first), 0));
        assert!(clock.get(id).unwrap().is_running());
    }

    #[test]
    fn stop_removes_the_timer_and_restart_counts_again() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(60), None, 0);
        let row = format!("timer-{id}");

        assert!(act(&mut clock, &invoke(&row, "restart"), 30_000));
        assert_eq!(clock.next_deadline(), Some(90_000));
        assert!(act(&mut clock, &invoke(&row, "stop"), 30_000));
        assert!(clock.is_empty());
    }

    /// A view that has not re-rendered yet may name a timer that has ended.
    #[test]
    fn an_action_on_a_timer_that_is_gone_changes_nothing() {
        let mut clock = Clock::default();
        assert!(!act(&mut clock, &invoke("timer-7", "stop"), 0));
        assert!(!act(&mut clock, &invoke("nonsense", "stop"), 0));
        assert!(!act(&mut clock, &Event::Dismissed, 0));
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
