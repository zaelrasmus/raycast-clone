//! Timers in the root list (F8).
//!
//! An **answer** for a typed duration -- `25m` is already the thing the user
//! wants, like `12*7` -- and a **catalogue** of the commands that exist
//! whether or not anything is running. Nothing here reads the timers: the
//! answer is parsed from the query alone, so it stays instant (I-3), and the
//! running timers are the daemon's.

use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;
use crate::timers::{Command, Preset, duration};

pub struct TimersProvider;

/// The user's presets as rows (slice 2): a catalogue, found by name and
/// alias, ranked by use like anything else. Built with the index, from
/// `config.json`, like the translate presets.
pub struct TimerPresetsProvider {
    presets: Vec<Preset>,
}

impl TimerPresetsProvider {
    pub fn new(presets: Vec<Preset>) -> Self {
        Self { presets }
    }
}

impl Provider for TimerPresetsProvider {
    fn id(&self) -> &'static str {
        "timer-preset"
    }

    fn catalogue(&self) -> Vec<Item> {
        self.presets.iter().map(preset).collect()
    }
}

fn preset(preset: &Preset) -> Item {
    // "timer" too, so typing the word lists every preset beside the view.
    let mut keywords = vec!["timer".to_string()];
    keywords.extend(preset.alias.clone());
    Item {
        // By name, so usage survives reordering the presets in the file.
        id: format!("timer:preset:{}", preset.name),
        title: preset.name.clone(),
        subtitle: Some(format!(
            "Start a {} timer",
            duration::describe(preset.duration)
        )),
        icon: None,
        keywords,
        actions: vec![Action::Timer(Command::Start {
            seconds: preset.duration.as_secs(),
            name: Some(preset.name.clone()),
        })],
        kind: Kind::Timer,
    }
}

/// Every typed duration shares one id, like every typed URL: the usage
/// history should learn that timers are started, not which lengths were.
const START: &str = "timer:start";

impl Provider for TimersProvider {
    fn id(&self) -> &'static str {
        "timer"
    }

    fn catalogue(&self) -> Vec<Item> {
        // "focus", "pomodoro" and "stopwatch" lead here too: they are the
        // same place, reached by the word the user thinks in (F8).
        let keywords = |extra: &[&str]| -> Vec<String> {
            ["timer", "timers", "countdown", "alarm"]
                .iter()
                .chain(extra)
                .map(|word| (*word).to_string())
                .collect()
        };
        vec![
            Item {
                id: "timer:open".into(),
                title: "Timers".into(),
                subtitle: Some("See, pause and stop running timers".into()),
                icon: None,
                keywords: keywords(&["pomodoro", "focus", "stopwatch"]),
                actions: vec![Action::OpenView(ViewSource::Timers)],
                kind: Kind::Timer,
            },
            Item {
                id: "timer:stop-nearest".into(),
                title: "Stop Running Timer".into(),
                subtitle: Some("Stop the timer closest to ending".into()),
                icon: None,
                keywords: keywords(&["stop", "cancel"]),
                actions: vec![Action::Timer(Command::StopNearest)],
                kind: Kind::Timer,
            },
        ]
    }

    fn answer(&self, query: &str) -> Vec<Item> {
        // Rejected on the first character, as the contract asks: a duration
        // starts with a digit or the word "timer".
        if !query.starts_with(|c: char| c.is_ascii_digit() || c == 't' || c == 'T') {
            return Vec::new();
        }
        let Some(typed) = duration::parse(query) else {
            return Vec::new();
        };

        let length = duration::describe(typed.duration);
        let (title, subtitle) = match &typed.name {
            Some(name) => (
                format!("Start \u{201c}{name}\u{201d}"),
                format!("A {length} timer"),
            ),
            None => (
                format!("Start {length} timer"),
                "Notifies you when it ends".to_string(),
            ),
        };

        vec![Item {
            id: START.into(),
            title,
            subtitle: Some(subtitle),
            icon: None,
            keywords: Vec::new(),
            actions: vec![Action::Timer(Command::Start {
                seconds: typed.duration.as_secs(),
                name: typed.name,
            })],
            kind: Kind::Timer,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_is_answered_with_a_timer_to_start() {
        let answers = TimersProvider.answer("25m");
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].title, "Start 25 min timer");
        assert_eq!(
            answers[0].primary(),
            Action::Timer(Command::Start {
                seconds: 1500,
                name: None
            })
        );
    }

    #[test]
    fn a_named_timer_says_its_name() {
        let answers = TimersProvider.answer("1h30 deep work");
        assert_eq!(answers[0].title, "Start \u{201c}deep work\u{201d}");
        assert_eq!(answers[0].subtitle.as_deref(), Some("A 1 h 30 min timer"));
    }

    #[test]
    fn anything_else_is_not_answered() {
        for query in ["25", "firefox", "12*7", "tea", "timer"] {
            assert!(TimersProvider.answer(query).is_empty(), "{query}");
        }
    }

    #[test]
    fn every_duration_shares_one_usage_id() {
        let a = TimersProvider.answer("5m");
        let b = TimersProvider.answer("10m tea");
        assert_eq!(a[0].id, b[0].id);
    }

    #[test]
    fn a_preset_is_a_row_that_starts_it_under_its_name() {
        let items = TimerPresetsProvider::new(vec![Preset {
            name: "Deep work".into(),
            alias: Some("dw".into()),
            duration: std::time::Duration::from_secs(90 * 60),
        }])
        .catalogue();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Deep work");
        assert_eq!(
            items[0].subtitle.as_deref(),
            Some("Start a 1 h 30 min timer")
        );
        assert!(items[0].keywords.contains(&"dw".to_string()));
        assert_eq!(
            items[0].primary(),
            Action::Timer(Command::Start {
                seconds: 90 * 60,
                name: Some("Deep work".into())
            })
        );
    }

    #[test]
    fn the_view_is_found_by_the_words_people_use() {
        let items = TimersProvider.catalogue();
        let open = items.iter().find(|item| item.title == "Timers").unwrap();
        for word in ["timer", "pomodoro", "focus", "stopwatch"] {
            assert!(
                open.keywords.iter().any(|keyword| keyword == word),
                "{word}"
            );
        }
        assert_eq!(open.primary(), Action::OpenView(ViewSource::Timers));
    }
}
