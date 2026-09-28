//! The translate view's state, as a plain value.
//!
//! The daemon owns the threads, the timer and the view; this owns every
//! decision. It is told what happened -- an event from the view, a pause in
//! typing, an answer -- and replies with [`Effect`]s: what to draw, what to
//! request, what to copy. Nothing here blocks, sleeps or touches the network,
//! so every rule the view follows is a unit test rather than a thing to try by
//! hand.
//!
//! # Stale answers
//!
//! Every change to what should be translated -- the text, the preset, a retry
//! -- starts a new *generation*. Requests carry theirs and an answer from an
//! older one is dropped, so a slow reply to "hol" can never overwrite the
//! answer to "hola".
//!
//! # The list does not change shape
//!
//! One row per target, created when the preset is chosen and kept while
//! answers arrive, under one section whose title is always present. Only the
//! text in them changes, so the window never resizes while the user reads
//! (I-10). The previous translation stays in its row while the next is
//! requested rather than flashing to empty.

use super::{Language, Preset, Service, Translation};
use crate::view::{
    Accessory, Action, EmptyState, Event, Filtering, Item, List, Scope, ScopeOption, Section, Tone,
    View,
};

/// What the driver should do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Call [`Session::settle`] with this generation after the typing pause.
    Wait { generation: u64 },
    /// Translate, on a worker thread, and report back with
    /// [`Session::answer`].
    Send(Request),
    /// Put this on the clipboard and close the launcher.
    Copy(String),
    /// This preset was chosen; open with it next time.
    Remember(String),
}

/// One translation to ask for.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub generation: u64,
    /// Which row the answer belongs in.
    pub target: usize,
    pub text: String,
    pub from: Option<&'static Language>,
    pub to: &'static Language,
}

/// One target's row.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    /// Nothing typed.
    Idle,
    /// Asked, not answered. Keeps what the row said before.
    Pending {
        previous: Option<String>,
    },
    Done(String),
    Failed(String),
}

pub struct Session {
    presets: Vec<Preset>,
    current: usize,
    service: Service,
    query: String,
    generation: u64,
    rows: Vec<Row>,
    /// What the service last said the source was.
    detected: Option<&'static Language>,
}

// Action ids. Opaque to the host; only this file reads them back.
const COPY: &str = "copy";
const COPY_ORIGINAL: &str = "copy-original";
const COPY_ALL: &str = "copy-all";
const RETRY: &str = "retry";
const PRESET: &str = "preset-";
const TARGET: &str = "target-";

impl Session {
    /// A session showing `presets[start]`.
    ///
    /// # Panics
    ///
    /// If `presets` is empty. [`super::TranslateConfig::presets`] never is.
    pub fn new(presets: Vec<Preset>, start: usize, service: Service) -> Self {
        assert!(!presets.is_empty(), "a translate session needs a preset");
        let current = start.min(presets.len() - 1);
        let rows = vec![Row::Idle; presets[current].to.len()];
        Self {
            presets,
            current,
            service,
            query: String::new(),
            generation: 0,
            rows,
            detected: None,
        }
    }

    /// Where to start: the preset called `name`, or the first.
    pub fn position(presets: &[Preset], name: Option<&str>) -> usize {
        name.and_then(|name| presets.iter().position(|preset| preset.name == name))
            .unwrap_or(0)
    }

    fn preset(&self) -> &Preset {
        &self.presets[self.current]
    }

    fn text(&self) -> &str {
        super::clip(self.query.trim())
    }

    /// Something happened in the view.
    pub fn event(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::QueryChanged { query } => {
                if query == self.query {
                    return Vec::new();
                }
                self.query = query;
                self.restart(false)
            }
            Event::ScopeChanged { scope } => match index(&scope, PRESET) {
                Some(preset) => self.switch(preset),
                None => Vec::new(),
            },
            Event::ActionInvoked { item, action } => self.act(item.as_deref(), &action),
            Event::SelectionChanged { .. } | Event::Dismissed => Vec::new(),
        }
    }

    /// The typing pause for `generation` is over.
    pub fn settle(&mut self, generation: u64) -> Vec<Effect> {
        if generation != self.generation || self.text().is_empty() {
            return Vec::new();
        }
        self.requests()
    }

    /// A request came back.
    pub fn answer(
        &mut self,
        generation: u64,
        target: usize,
        result: Result<Translation, String>,
    ) -> Vec<Effect> {
        if generation != self.generation || target >= self.rows.len() {
            return Vec::new();
        }
        self.rows[target] = match result {
            Ok(translation) => {
                if translation.detected.is_some() {
                    self.detected = translation.detected;
                }
                Row::Done(translation.text)
            }
            Err(message) => Row::Failed(message),
        };
        vec![Effect::Render(self.view())]
    }

    /// Something about the input changed: start a new generation.
    ///
    /// `now` skips the typing pause, for a change that was not typing.
    fn restart(&mut self, now: bool) -> Vec<Effect> {
        self.generation += 1;

        if self.text().is_empty() {
            self.rows.fill(Row::Idle);
            self.detected = None;
            return vec![Effect::Render(self.view())];
        }

        for row in &mut self.rows {
            let previous = match row {
                Row::Done(text) => Some(text.clone()),
                Row::Pending { previous } => previous.clone(),
                Row::Idle | Row::Failed(_) => None,
            };
            *row = Row::Pending { previous };
        }

        let mut effects = vec![Effect::Render(self.view())];
        if now {
            effects.extend(self.requests());
        } else {
            effects.push(Effect::Wait {
                generation: self.generation,
            });
        }
        effects
    }

    fn requests(&self) -> Vec<Effect> {
        let preset = self.preset();
        preset
            .to
            .iter()
            .enumerate()
            .map(|(target, to)| {
                Effect::Send(Request {
                    generation: self.generation,
                    target,
                    text: self.text().to_string(),
                    from: preset.from,
                    to,
                })
            })
            .collect()
    }

    fn switch(&mut self, preset: usize) -> Vec<Effect> {
        if preset >= self.presets.len() || preset == self.current {
            return Vec::new();
        }
        self.current = preset;
        self.rows = vec![Row::Idle; self.preset().to.len()];
        self.detected = None;

        let mut effects = vec![Effect::Remember(self.preset().name.clone())];
        effects.extend(self.restart(true));
        effects
    }

    fn act(&mut self, item: Option<&str>, action: &str) -> Vec<Effect> {
        if let Some(preset) = index(action, PRESET) {
            return self.switch(preset);
        }
        match action {
            COPY => {
                let row = item.and_then(|item| index(item, TARGET));
                match row.and_then(|row| self.rows.get(row)) {
                    Some(Row::Done(text)) => vec![Effect::Copy(text.clone())],
                    _ => Vec::new(),
                }
            }
            COPY_ORIGINAL => vec![Effect::Copy(self.query.trim().to_string())],
            COPY_ALL => {
                let all: Vec<&str> = self
                    .rows
                    .iter()
                    .filter_map(|row| match row {
                        Row::Done(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                vec![Effect::Copy(all.join("\n"))]
            }
            RETRY => self.restart(true),
            _ => Vec::new(),
        }
    }

    /// What the view should show now.
    pub fn view(&self) -> View {
        let preset = self.preset();

        let scope = Scope {
            options: self
                .presets
                .iter()
                .enumerate()
                .map(|(index, preset)| ScopeOption {
                    id: format!("{PRESET}{index}"),
                    title: preset.name.clone(),
                })
                .collect(),
            selected: format!("{PRESET}{}", self.current),
        };

        let hint = if self.presets.len() > 1 {
            " Tab switches presets."
        } else {
            ""
        };
        let empty = EmptyState {
            title: "Type something to translate".into(),
            description: Some(format!(
                "Sent to {} as you type.{hint}",
                self.service.label()
            )),
            icon: None,
        };

        let sections = if self.text().is_empty() {
            Vec::new()
        } else {
            vec![Section {
                title: Some(format!(
                    "{} \u{00b7} {}",
                    self.source_title(),
                    self.service.label()
                )),
                items: self
                    .rows
                    .iter()
                    .zip(&preset.to)
                    .enumerate()
                    .map(|(index, (row, to))| self.item(index, row, to))
                    .collect(),
            }]
        };

        View::List(List {
            placeholder: Some(format!("Translate {}", preset.describe())),
            filtering: Filtering::Extension,
            scope: Some(scope),
            empty: Some(empty),
            sections,
        })
    }

    fn source_title(&self) -> String {
        if let Some(from) = self.preset().from {
            return format!("From {}", from.name);
        }
        match self.detected {
            Some(language) => format!("Detected {}", language.name),
            None if self
                .rows
                .iter()
                .any(|row| matches!(row, Row::Pending { .. })) =>
            {
                "Detecting language\u{2026}".into()
            }
            None => "Source language unknown".into(),
        }
    }

    fn item(&self, index: usize, row: &Row, to: &Language) -> Item {
        let (title, subtitle, tone) = match row {
            Row::Done(text) => (one_line(text), None, Tone::Neutral),
            Row::Pending {
                previous: Some(text),
            } => (one_line(text), None, Tone::Neutral),
            Row::Pending { previous: None } | Row::Idle => {
                ("Translating\u{2026}".into(), None, Tone::Neutral)
            }
            Row::Failed(message) => (
                "Could not translate".into(),
                Some(message.clone()),
                Tone::Danger,
            ),
        };

        let action = |id: &str, title: &str| Action {
            id: id.into(),
            title: title.into(),
            shortcut: None,
        };
        let mut actions = match row {
            Row::Done(_) => {
                let mut actions = vec![
                    action(COPY, "Copy Translation"),
                    action(COPY_ORIGINAL, "Copy Original"),
                ];
                if self.rows.len() > 1 {
                    actions.push(action(COPY_ALL, "Copy All Translations"));
                }
                actions
            }
            Row::Failed(_) => vec![
                action(RETRY, "Try Again"),
                action(COPY_ORIGINAL, "Copy Original"),
            ],
            // Enter on a row still waiting does nothing rather than copying
            // what it said a moment ago.
            Row::Pending { .. } | Row::Idle => Vec::new(),
        };
        if !actions.is_empty() {
            actions.extend(
                self.presets
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != self.current)
                    .map(|(index, preset)| Action {
                        id: format!("{PRESET}{index}"),
                        title: format!("Switch to {}", preset.name),
                        shortcut: None,
                    }),
            );
        }

        Item {
            id: format!("{TARGET}{index}"),
            title,
            subtitle,
            icon: None,
            accessories: vec![Accessory::Tag {
                text: to.name.into(),
                tone,
            }],
            keywords: Vec::new(),
            actions,
        }
    }
}

/// `"preset-2"` with prefix `"preset-"` is 2.
fn index(id: &str, prefix: &str) -> Option<usize> {
    id.strip_prefix(prefix)?.parse().ok()
}

/// A translation drawn on one row: its line breaks become spaces. What is
/// copied keeps them.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::languages::find;

    fn preset(name: &str, from: Option<&str>, to: &[&str]) -> Preset {
        Preset {
            name: name.into(),
            alias: None,
            from: from.map(|code| find(code).unwrap()),
            to: to.iter().map(|code| find(code).unwrap()).collect(),
        }
    }

    fn session() -> Session {
        Session::new(
            vec![
                preset("Spanish → English", Some("es"), &["en"]),
                preset("Any → Many", None, &["en", "fr", "de"]),
            ],
            0,
            Service::Google,
        )
    }

    fn typed(session: &mut Session, text: &str) -> Vec<Effect> {
        session.event(Event::QueryChanged { query: text.into() })
    }

    fn list(effects: &[Effect]) -> List {
        effects
            .iter()
            .rev()
            .find_map(|effect| match effect {
                Effect::Render(View::List(list)) => Some(list.clone()),
                _ => None,
            })
            .expect("a render")
    }

    fn sends(effects: &[Effect]) -> Vec<Request> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Send(request) => Some(request.clone()),
                _ => None,
            })
            .collect()
    }

    fn done(text: &str) -> Result<Translation, String> {
        Ok(Translation {
            text: text.into(),
            detected: None,
        })
    }

    #[test]
    fn typing_waits_for_a_pause_before_asking() {
        let mut session = session();
        let effects = typed(&mut session, "hola");

        assert!(sends(&effects).is_empty());
        let Some(Effect::Wait { generation }) = effects.last() else {
            panic!("expected a wait, got {effects:?}")
        };

        let requests = sends(&session.settle(*generation));
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].text, "hola");
        assert_eq!(requests[0].from.unwrap().code, "es");
        assert_eq!(requests[0].to.code, "en");
    }

    /// A pause that belongs to an earlier keystroke asks for nothing.
    #[test]
    fn a_pause_overtaken_by_more_typing_is_ignored() {
        let mut session = session();
        typed(&mut session, "hol");
        let first = session.generation;
        typed(&mut session, "hola");

        assert!(session.settle(first).is_empty());
    }

    #[test]
    fn an_answer_to_an_older_query_is_dropped() {
        let mut session = session();
        typed(&mut session, "hol");
        let old = session.generation;
        typed(&mut session, "hola");

        assert!(session.answer(old, 0, done("Hol")).is_empty());

        let current = session.generation;
        let list = list(&session.answer(current, 0, done("Hello")));
        assert_eq!(list.sections[0].items[0].title, "Hello");
    }

    #[test]
    fn the_previous_translation_stays_while_the_next_is_asked_for() {
        let mut session = session();
        typed(&mut session, "hola");
        let generation = session.generation;
        session.answer(generation, 0, done("Hello"));

        let list = list(&typed(&mut session, "hola amigo"));
        assert_eq!(list.sections[0].items[0].title, "Hello");
        // Waiting, so Enter must not copy the stale text.
        assert!(list.sections[0].items[0].actions.is_empty());
    }

    /// I-10: the list keeps its shape while answers arrive.
    #[test]
    fn one_row_per_target_from_the_first_keystroke() {
        let mut session = session();
        session.event(Event::ScopeChanged {
            scope: "preset-1".into(),
        });
        let list = list(&typed(&mut session, "hola"));

        assert_eq!(list.len(), 3);
        assert!(list.sections[0].title.is_some());
        let tags: Vec<_> = list
            .items()
            .map(|item| match &item.accessories[0] {
                Accessory::Tag { text, .. } => text.clone(),
                other => panic!("expected a tag, got {other:?}"),
            })
            .collect();
        assert_eq!(tags, ["English", "French", "German"]);

        let generation = session.generation;
        let list = list_after(session.answer(generation, 1, done("Bonjour")));
        assert_eq!(list.len(), 3);
        assert_eq!(list.item(1).unwrap().title, "Bonjour");
        assert_eq!(list.item(0).unwrap().title, "Translating\u{2026}");
    }

    fn list_after(effects: Vec<Effect>) -> List {
        list(&effects)
    }

    #[test]
    fn nothing_typed_shows_the_empty_state_and_asks_nothing() {
        let mut session = session();
        typed(&mut session, "hola");
        let effects = typed(&mut session, "   ");

        assert!(list(&effects).is_empty());
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Wait { .. }))
        );
    }

    #[test]
    fn switching_preset_asks_at_once_and_is_remembered() {
        let mut session = session();
        typed(&mut session, "hola");

        let effects = session.event(Event::ScopeChanged {
            scope: "preset-1".into(),
        });
        assert!(effects.contains(&Effect::Remember("Any → Many".into())));
        let requests = sends(&effects);
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|request| request.from.is_none()));

        let scope = list(&effects).scope.unwrap();
        assert_eq!(scope.current().unwrap().title, "Any → Many");
    }

    #[test]
    fn switching_from_the_action_panel_is_the_same_as_the_scope() {
        let mut session = session();
        typed(&mut session, "hola");
        let generation = session.generation;
        let list = list(&session.answer(generation, 0, done("Hello")));
        let switch = list.item(0).unwrap().actions.last().unwrap().clone();
        assert_eq!(switch.title, "Switch to Any → Many");

        let effects = session.event(Event::ActionInvoked {
            item: Some("target-0".into()),
            action: switch.id,
        });
        assert_eq!(sends(&effects).len(), 3);
    }

    #[test]
    fn a_detected_source_titles_the_section() {
        let mut session = session();
        session.event(Event::ScopeChanged {
            scope: "preset-1".into(),
        });
        let list = list(&typed(&mut session, "hola"));
        assert!(
            list.sections[0]
                .title
                .as_deref()
                .unwrap()
                .starts_with("Detecting")
        );

        let generation = session.generation;
        let list = list_after(session.answer(
            generation,
            0,
            Ok(Translation {
                text: "Hello".into(),
                detected: find("es"),
            }),
        ));
        assert_eq!(
            list.sections[0].title.as_deref(),
            Some("Detected Spanish \u{00b7} Google Translate")
        );
    }

    #[test]
    fn enter_copies_the_translation_with_its_line_breaks() {
        let mut session = session();
        typed(&mut session, "hola\nadiós");
        let generation = session.generation;
        let list = list(&session.answer(generation, 0, done("Hello\nGoodbye")));
        assert_eq!(list.item(0).unwrap().title, "Hello Goodbye");

        let effects = session.event(Event::ActionInvoked {
            item: Some("target-0".into()),
            action: "copy".into(),
        });
        assert_eq!(effects, vec![Effect::Copy("Hello\nGoodbye".into())]);
    }

    #[test]
    fn copy_all_puts_every_translation_on_its_own_line() {
        let mut session = session();
        session.event(Event::ScopeChanged {
            scope: "preset-1".into(),
        });
        typed(&mut session, "hola");
        let generation = session.generation;
        session.answer(generation, 0, done("Hello"));
        session.answer(generation, 1, done("Bonjour"));
        session.answer(generation, 2, done("Hallo"));

        let effects = session.event(Event::ActionInvoked {
            item: Some("target-1".into()),
            action: "copy-all".into(),
        });
        assert_eq!(effects, vec![Effect::Copy("Hello\nBonjour\nHallo".into())]);
    }

    #[test]
    fn a_failure_is_shown_and_can_be_retried() {
        let mut session = session();
        typed(&mut session, "hola");
        let generation = session.generation;
        let list = list(&session.answer(generation, 0, Err("Google answered 429".into())));

        let item = list.item(0).unwrap();
        assert_eq!(item.title, "Could not translate");
        assert_eq!(item.subtitle.as_deref(), Some("Google answered 429"));
        assert_eq!(item.actions[0].id, "retry");

        let effects = session.event(Event::ActionInvoked {
            item: Some("target-0".into()),
            action: "retry".into(),
        });
        assert_eq!(sends(&effects).len(), 1);
    }

    #[test]
    fn the_view_says_who_receives_the_text() {
        let session = Session::new(vec![preset("P", None, &["en"])], 0, Service::Deepl);
        let View::List(list) = session.view() else {
            panic!("expected a list")
        };
        let description = list.empty.unwrap().description.unwrap();
        assert!(description.contains("DeepL"));
        // One preset: nothing to switch to, so no hint about switching.
        assert!(!description.contains("Tab"));
        assert_eq!(
            list.placeholder.as_deref(),
            Some("Translate any language to English")
        );
    }

    #[test]
    fn the_session_starts_on_the_named_preset() {
        let presets = vec![preset("A", None, &["en"]), preset("B", None, &["fr"])];
        assert_eq!(Session::position(&presets, Some("B")), 1);
        assert_eq!(Session::position(&presets, Some("gone")), 0);
        assert_eq!(Session::position(&presets, None), 0);
    }
}
