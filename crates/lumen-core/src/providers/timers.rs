//! Timers in the root list (F8).
//!
//! An **answer** for a typed duration -- `25m` is already the thing the user
//! wants, like `12*7` -- and a **catalogue** of the commands that exist
//! whether or not anything is running. Nothing here reads the timers: the
//! answer is parsed from the query alone, so it stays instant (I-3), and the
//! running timers are the daemon's.

use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;
use crate::timers::{Command, Plan, Preset, PresetConfig, duration};

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
    if preset.cycle.is_some() {
        keywords.push("pomodoro".into());
    }
    Item {
        // By name, so usage survives reordering the presets in the file.
        id: format!("timer:preset:{}", preset.name),
        title: preset.name.clone(),
        subtitle: Some(format!("Start {}", preset.describe())),
        icon: None,
        keywords,
        actions: starts(preset),
        kind: Kind::Timer,
    }
}

/// `start`, and for a cycle the other way of ending -- one set or repeating --
/// second, where `Ctrl+K` reaches it.
/// A plain preset's second is the other side of focus.
fn starts(preset: &Preset) -> Vec<Action> {
    let mut actions = vec![start(preset)];
    let name = Some(preset.name.clone());
    actions.push(Action::Timer(match preset.cycle {
        Some(plan) => Command::StartCycle {
            name,
            plan: Plan {
                repeat: !plan.repeat,
                ..plan
            },
            focus: preset.focus,
        },
        None => Command::Start {
            seconds: preset.duration.as_secs(),
            name,
            focus: !preset.focus,
        },
    }));
    actions.push(Action::OpenTimerForm {
        preset: preset.to_config(),
        replacing: Some(preset.name.clone()),
    });
    actions
}

/// What starting `preset` asks the daemon for.
pub fn start(preset: &Preset) -> Action {
    let name = Some(preset.name.clone());
    let focus = preset.focus;
    Action::Timer(match preset.cycle {
        Some(plan) => Command::StartCycle { name, plan, focus },
        None => Command::Start {
            seconds: preset.duration.as_secs(),
            name,
            focus,
        },
    })
}

/// Every typed duration shares one id, like every typed URL: the usage
/// history should learn that timers are started, not which lengths were.
const START: &str = "timer:start";
/// Likewise for named stopwatches.
const STOPWATCH: &str = "timer:stopwatch-named";

/// The name in `stopwatch <name>`, when the query is that.
fn stopwatch_name(query: &str) -> Option<&str> {
    let name = strip_word(query, "stopwatch")?.trim();
    (!name.is_empty()).then_some(name)
}

/// `query` after a leading `word` and the space after it.
fn strip_word<'a>(query: &'a str, word: &str) -> Option<&'a str> {
    let head = query.get(..word.len())?;
    let rest = &query[word.len()..];
    (head.eq_ignore_ascii_case(word) && rest.starts_with(char::is_whitespace))
        .then(|| rest.trim_start())
}

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
                id: "timer:create-preset".into(),
                title: "Create Timer Preset".into(),
                subtitle: Some("Save a timer you start often, with a name and an alias".into()),
                icon: None,
                keywords: keywords(&["preset", "new", "save", "create"]),
                actions: vec![Action::OpenTimerForm {
                    preset: PresetConfig::default(),
                    replacing: None,
                }],
                kind: Kind::Timer,
            },
            Item {
                id: "timer:pomodoro".into(),
                title: "Start Pomodoro".into(),
                subtitle: Some(Plan::POMODORO.describe()),
                icon: None,
                keywords: keywords(&["pomodoro", "focus", "tomato", "work", "break"]),
                actions: vec![
                    Action::Timer(Command::StartCycle {
                        name: Some("Pomodoro".into()),
                        plan: Plan::POMODORO,
                        focus: false,
                    }),
                    Action::Timer(Command::StartCycle {
                        name: Some("Pomodoro".into()),
                        plan: Plan::POMODORO.repeating(),
                        focus: false,
                    }),
                    Action::Timer(Command::StartCycle {
                        name: Some("Pomodoro".into()),
                        plan: Plan::POMODORO,
                        focus: true,
                    }),
                ],
                kind: Kind::Timer,
            },
            Item {
                id: "timer:focus".into(),
                title: "Start Focus Session".into(),
                subtitle: Some(
                    "25 min, your distractions blocked in Lumen \u{2014} or type focus 45m and your goal"
                        .into(),
                ),
                icon: None,
                keywords: keywords(&["focus", "session", "block", "distraction", "deep", "work"]),
                actions: vec![Action::Timer(Command::Start {
                    seconds: 25 * 60,
                    name: Some("Focus".into()),
                    focus: true,
                })],
                kind: Kind::Timer,
            },
            Item {
                id: "timer:stopwatch".into(),
                title: "Start Stopwatch".into(),
                subtitle: Some("Count up from zero; type a name after \"stopwatch\"".into()),
                icon: None,
                keywords: keywords(&["stopwatch", "count", "elapsed"]),
                actions: vec![Action::Timer(Command::StartStopwatch { name: None })],
                kind: Kind::Timer,
            },
            Item {
                id: "timer:dismiss".into(),
                title: "Dismiss Ringing Timer".into(),
                subtitle: Some("Silence a timer set to ring until dismissed".into()),
                icon: None,
                keywords: keywords(&["dismiss", "silence", "ringing", "snooze"]),
                actions: vec![Action::Timer(Command::DismissRinging)],
                kind: Kind::Timer,
            },
            Item {
                id: "timer:sounds".into(),
                title: "Preview Alert Sounds".into(),
                subtitle: Some("Hear the alert sounds, and choose one".into()),
                icon: None,
                keywords: keywords(&["sound", "sounds", "alert", "ring", "preview"]),
                actions: vec![Action::OpenView(ViewSource::AlertSounds)],
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
        // starts with a digit or the word "timer", a named stopwatch with
        // the word "stopwatch".
        if !query.starts_with(|c: char| {
            c.is_ascii_digit() || matches!(c, 't' | 'T' | 's' | 'S' | 'f' | 'F')
        }) {
            return Vec::new();
        }
        // `25 min in s` is a conversion, not a timer named *in s* (F14).
        // What follows a duration is a name, so the calculator, which
        // declines any word it does not know, settles it: `10m in the
        // oven` is still a timer.
        if crate::calc::evaluate(query).is_some() {
            return Vec::new();
        }
        if let Some(name) = stopwatch_name(query) {
            return vec![Item {
                id: STOPWATCH.into(),
                title: format!("Start stopwatch \u{201c}{name}\u{201d}"),
                subtitle: Some("Counts up until you stop it".into()),
                icon: None,
                keywords: Vec::new(),
                actions: vec![Action::Timer(Command::StartStopwatch {
                    name: Some(name.to_string()),
                })],
                kind: Kind::Timer,
            }];
        }
        // `focus 45m write chapter 3`: the same grammar, with focus on.
        let (focus, rest) = match strip_word(query, "focus") {
            Some(rest) => (true, rest),
            None => (false, query),
        };
        let Some(typed) = duration::parse(rest) else {
            return Vec::new();
        };

        let length = duration::describe(typed.duration);
        let (title, subtitle) = match (&typed.name, focus) {
            (Some(name), true) => (
                format!("Focus on \u{201c}{name}\u{201d} for {length}"),
                "Blocks your distractions in Lumen until it ends".to_string(),
            ),
            (None, true) => (
                format!("Focus for {length}"),
                "Blocks your distractions in Lumen until it ends".to_string(),
            ),
            (Some(name), false) => (
                format!("Start \u{201c}{name}\u{201d}"),
                format!("A {length} timer"),
            ),
            (None, false) => (
                format!("Start {length} timer"),
                "Notifies you when it ends".to_string(),
            ),
        };

        // The other side of focus second, where Ctrl+K reaches it; then
        // keeping it, as typed, for next time.
        let start = |focus: bool| {
            Action::Timer(Command::Start {
                seconds: typed.duration.as_secs(),
                name: typed.name.clone(),
                focus,
            })
        };
        let save = Action::OpenTimerForm {
            preset: PresetConfig {
                name: typed.name.clone().unwrap_or_default(),
                duration: duration::written(typed.duration),
                focus,
                ..PresetConfig::default()
            },
            replacing: None,
        };
        vec![Item {
            id: START.into(),
            title,
            subtitle: Some(subtitle),
            icon: None,
            keywords: Vec::new(),
            actions: vec![start(focus), start(!focus), save],
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
                name: None,
                focus: false,
            })
        );
        assert_eq!(answers[0].actions[1].verb(), "Start with Focus");
        assert_eq!(answers[0].actions[2].verb(), "Save as Preset");
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
            cycle: None,
            focus: false,
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
                name: Some("Deep work".into()),
                focus: false,
            })
        );
        assert_eq!(items[0].actions[1].verb(), "Start with Focus");
        assert!(matches!(
            &items[0].actions[2],
            Action::OpenTimerForm { replacing: Some(name), preset } if name == "Deep work" && preset.duration == "1h30m"
        ));
    }

    #[test]
    fn a_cycling_preset_starts_its_cycle() {
        let items = TimerPresetsProvider::new(vec![Preset {
            name: "Sprint".into(),
            alias: None,
            duration: std::time::Duration::from_secs(50 * 60),
            cycle: Some(Plan::POMODORO),
            focus: false,
        }])
        .catalogue();
        assert!(matches!(
            items[0].primary(),
            Action::Timer(Command::StartCycle { .. })
        ));
        assert!(items[0].keywords.contains(&"pomodoro".to_string()));
        // The other ending, one Ctrl+K away.
        assert_eq!(items[0].actions[1].verb(), "Start Repeating");
    }

    #[test]
    fn pomodoro_is_there_without_configuring_anything() {
        let items = TimersProvider.catalogue();
        let pomodoro = items
            .iter()
            .find(|item| item.title == "Start Pomodoro")
            .unwrap();
        assert_eq!(
            pomodoro.primary(),
            Action::Timer(Command::StartCycle {
                name: Some("Pomodoro".into()),
                plan: Plan::POMODORO,
                focus: false,
            })
        );
        assert_eq!(pomodoro.actions[2].verb(), "Start with Focus");
    }

    #[test]
    fn focus_and_a_duration_start_a_focus_timer() {
        let answers = TimersProvider.answer("focus 45m write chapter 3");
        assert_eq!(
            answers[0].title,
            "Focus on \u{201c}write chapter 3\u{201d} for 45 min"
        );
        assert_eq!(
            answers[0].primary(),
            Action::Timer(Command::Start {
                seconds: 45 * 60,
                name: Some("write chapter 3".into()),
                focus: true,
            })
        );
        assert_eq!(
            TimersProvider.answer("focus 25m")[0].title,
            "Focus for 25 min"
        );
        assert!(TimersProvider.answer("focus").is_empty());
        assert!(TimersProvider.answer("focus mode").is_empty());
        assert!(TimersProvider.answer("firefox").is_empty());
    }

    #[test]
    fn stopwatch_and_a_name_start_a_named_stopwatch() {
        let answers = TimersProvider.answer("stopwatch standup call");
        assert_eq!(
            answers[0].title,
            "Start stopwatch \u{201c}standup call\u{201d}"
        );
        assert_eq!(
            answers[0].primary(),
            Action::Timer(Command::StartStopwatch {
                name: Some("standup call".into())
            })
        );
        // The bare word is the catalogue row's, not an answer.
        assert!(TimersProvider.answer("stopwatch").is_empty());
        assert!(TimersProvider.answer("stopwatches x").is_empty());
        assert!(TimersProvider.answer("spotify").is_empty());
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
