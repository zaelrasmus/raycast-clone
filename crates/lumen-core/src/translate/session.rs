//! The translate view's state, as a plain value.
//!
//! The daemon owns the threads, the timer and the view; this owns every
//! decision. It is told what happened -- an event from the view, a pause in
//! typing, an answer -- and replies with [`Effect`]s: what to draw, what to
//! request, what to copy. Nothing here blocks, sleeps or touches the network,
//! so every rule the view follows is a unit test rather than a thing to try by
//! hand.
//!
//! # Languages are chosen, presets only start them
//!
//! The view has two scopes, *From* and *To*, and either can be changed at any
//! time without leaving the view or losing the text. A preset is where they
//! start: opening a preset's row sets both, and so does *Switch to* in the
//! action panel. After that the user is free.
//!
//! # Stale answers
//!
//! Every change to what should be translated -- the text, a language, a retry
//! -- starts a new *generation*. Requests carry theirs and an answer from an
//! older one is dropped, so a slow reply to "hol" can never overwrite the
//! answer to "hola".
//!
//! # The list does not change shape
//!
//! One row per target, created when the targets are chosen and kept while
//! answers arrive, under one section whose title is always present. Only the
//! text in them changes, so the window never resizes while the user reads
//! (I-10). The previous translation stays in its row while the next is
//! requested rather than flashing to empty.

use super::{Failure, LANGUAGES, Language, MAX_TARGETS, Preset, Service, Translation, languages};
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
    /// These languages were chosen; open with them next time.
    Remember(Languages),
}

/// A direction: a source, or detection, and the targets.
#[derive(Debug, Clone, PartialEq)]
pub struct Languages {
    /// `None` detects the source.
    pub from: Option<&'static Language>,
    /// Never empty, never more than [`MAX_TARGETS`].
    pub to: Vec<&'static Language>,
}

impl Languages {
    pub fn of(preset: &Preset) -> Self {
        Self {
            from: preset.from,
            to: preset.to.clone(),
        }
    }
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
    Failed(Failure),
}

pub struct Session {
    /// Offered in the action panel, and their target sets in *To*.
    presets: Vec<Preset>,
    languages: Languages,
    service: Service,
    query: String,
    generation: u64,
    rows: Vec<Row>,
    /// What the service last said the source was.
    detected: Option<&'static Language>,
}

// Ids. Opaque to the host; only this file reads them back.
const COPY: &str = "copy";
const COPY_ORIGINAL: &str = "copy-original";
const COPY_ALL: &str = "copy-all";
const RETRY: &str = "retry";
const PRESET: &str = "preset-";
const TARGET: &str = "target-";
/// The scopes, and the prefixes of their options.
const FROM: &str = "from";
const TO: &str = "to";
const DETECT: &str = "detect";
const LANGUAGE: &str = "lang-";

impl Session {
    /// A session translating `languages`, offering `presets` to switch to.
    ///
    /// # Panics
    ///
    /// If `languages` has no target.
    pub fn new(presets: Vec<Preset>, languages: Languages, service: Service) -> Self {
        assert!(
            !languages.to.is_empty(),
            "a translate session needs a target"
        );
        let rows = vec![Row::Idle; languages.to.len()];
        Self {
            presets,
            languages,
            service,
            query: String::new(),
            generation: 0,
            rows,
            detected: None,
        }
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
            Event::ScopeChanged { scope, option } => self.choose(&scope, &option),
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
        result: Result<Translation, Failure>,
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
            Err(failure) => Row::Failed(failure),
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
        self.languages
            .to
            .iter()
            .enumerate()
            .map(|(target, to)| {
                Effect::Send(Request {
                    generation: self.generation,
                    target,
                    text: self.text().to_string(),
                    from: self.languages.from,
                    to,
                })
            })
            .collect()
    }

    /// Translate `languages` from now on, asking again at once.
    fn set(&mut self, languages: Languages) -> Vec<Effect> {
        if languages == self.languages || languages.to.is_empty() {
            return Vec::new();
        }
        // Rows keep their text only when their target did not move: a French
        // translation left in what is now the German row would be a lie.
        let rows = languages
            .to
            .iter()
            .map(|to| {
                self.languages
                    .to
                    .iter()
                    .position(|old| old == to)
                    .map_or(Row::Idle, |old| self.rows[old].clone())
            })
            .collect();
        if languages.from != self.languages.from {
            self.detected = None;
        }
        self.rows = rows;
        self.languages = languages;

        let mut effects = vec![Effect::Remember(self.languages.clone())];
        effects.extend(self.restart(true));
        effects
    }

    /// An option was chosen in one of the scopes.
    fn choose(&mut self, scope: &str, option: &str) -> Vec<Effect> {
        let mut languages = self.languages.clone();
        match scope {
            FROM if option == DETECT => languages.from = None,
            FROM => match option.strip_prefix(LANGUAGE).and_then(languages::find) {
                Some(language) => languages.from = Some(language),
                None => return Vec::new(),
            },
            TO => match targets(option) {
                Some(to) => languages.to = to,
                None => return Vec::new(),
            },
            _ => return Vec::new(),
        }
        self.set(languages)
    }

    fn act(&mut self, item: Option<&str>, action: &str) -> Vec<Effect> {
        if let Some(preset) = index(action, PRESET) {
            return match self.presets.get(preset) {
                Some(preset) => self.set(Languages::of(preset)),
                None => Vec::new(),
            };
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
        let empty = EmptyState {
            title: "Type something to translate".into(),
            description: Some(format!(
                "Sent to {} as you type. Tab chooses the languages.",
                self.service.label()
            )),
            icon: None,
        };

        let sections = if self.text().is_empty() {
            Vec::new()
        } else {
            vec![Section {
                title: Some(if self.offline() {
                    format!("Offline \u{00b7} {}", self.service.label())
                } else {
                    format!("{} \u{00b7} {}", self.source_title(), self.service.label())
                }),
                subtitle: None,
                items: self
                    .rows
                    .iter()
                    .zip(&self.languages.to)
                    .enumerate()
                    .map(|(index, (row, to))| self.item(index, row, to))
                    .collect(),
            }]
        };

        View::List(List {
            placeholder: Some(format!(
                "Translate {}",
                super::describe(self.languages.from, &self.languages.to)
            )),
            filtering: Filtering::Extension,
            scopes: vec![self.source_scope(), self.target_scope()],
            empty: Some(empty),
            sections,
        })
    }

    /// *From*: detection, then every language.
    fn source_scope(&self) -> Scope {
        let options = std::iter::once(ScopeOption {
            id: DETECT.into(),
            title: "Detect language".into(),
        })
        .chain(LANGUAGES.iter().map(|language| ScopeOption {
            id: format!("{LANGUAGE}{}", language.code),
            title: language.name.into(),
        }))
        .collect();
        Scope {
            id: FROM.into(),
            title: "From".into(),
            options,
            selected: match self.languages.from {
                Some(language) => format!("{LANGUAGE}{}", language.code),
                None => DETECT.into(),
            },
        }
    }

    /// *To*: the presets' sets of several targets, and the one in force if it
    /// is one, then every language on its own.
    fn target_scope(&self) -> Scope {
        let mut sets: Vec<&[&'static Language]> = Vec::new();
        for to in self
            .presets
            .iter()
            .map(|preset| preset.to.as_slice())
            .chain(std::iter::once(self.languages.to.as_slice()))
        {
            if to.len() > 1 && !sets.contains(&to) {
                sets.push(to);
            }
        }

        let options = sets
            .into_iter()
            .map(|to| ScopeOption {
                id: target_id(to),
                title: super::names(to),
            })
            .chain(LANGUAGES.iter().map(|language| ScopeOption {
                id: target_id(&[language]),
                title: language.name.into(),
            }))
            .collect();
        Scope {
            id: TO.into(),
            title: "To".into(),
            options,
            selected: target_id(&self.languages.to),
        }
    }

    /// Every row failed for want of a connection.
    fn offline(&self) -> bool {
        self.rows
            .iter()
            .all(|row| matches!(row, Row::Failed(Failure::Offline)))
    }

    fn source_title(&self) -> String {
        if let Some(from) = self.languages.from {
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
            // Said as what it is, with what to do: the raw error names a
            // socket, not the Wi-Fi.
            Row::Failed(Failure::Offline) => (
                "No internet connection".into(),
                Some(format!(
                    "Translate needs the internet to reach {}. Enter tries again.",
                    self.service.label()
                )),
                Tone::Caution,
            ),
            Row::Failed(Failure::Other(message)) => (
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
                    .filter(|(_, preset)| Languages::of(preset) != self.languages)
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

/// The *To* option for these targets: `"lang-en"`, or `"lang-en,fr"`.
fn target_id(to: &[&Language]) -> String {
    let codes: Vec<&str> = to.iter().map(|language| language.code).collect();
    format!("{LANGUAGE}{}", codes.join(","))
}

/// The targets a *To* option names, if every one is known.
fn targets(option: &str) -> Option<Vec<&'static Language>> {
    let to = option
        .strip_prefix(LANGUAGE)?
        .split(',')
        .map(languages::find)
        .collect::<Option<Vec<_>>>()?;
    (!to.is_empty() && to.len() <= MAX_TARGETS).then_some(to)
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

    fn presets() -> Vec<Preset> {
        vec![
            preset("Spanish → English", Some("es"), &["en"]),
            preset("Any → Many", None, &["en", "fr", "de"]),
        ]
    }

    /// Opened on the first preset, as a preset's row would.
    fn session() -> Session {
        let presets = presets();
        let start = Languages::of(&presets[0]);
        Session::new(presets, start, Service::Google)
    }

    fn typed(session: &mut Session, text: &str) -> Vec<Effect> {
        session.event(Event::QueryChanged { query: text.into() })
    }

    fn chose(session: &mut Session, scope: &str, option: &str) -> Vec<Effect> {
        session.event(Event::ScopeChanged {
            scope: scope.into(),
            option: option.into(),
        })
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

    fn done(text: &str) -> Result<Translation, Failure> {
        Ok(Translation {
            text: text.into(),
            detected: None,
        })
    }

    fn scope<'a>(list: &'a List, id: &str) -> &'a Scope {
        list.scopes.iter().find(|scope| scope.id == id).unwrap()
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

    /// The whole point of the scopes: any source, any target, at any time.
    #[test]
    fn both_languages_can_be_chosen_freely() {
        let mut session = session();
        typed(&mut session, "bonjour");

        let effects = chose(&mut session, "from", "lang-fr");
        let requests = sends(&effects);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].from.unwrap().code, "fr");
        assert_eq!(requests[0].to.code, "en");

        let effects = chose(&mut session, "to", "lang-ja");
        assert_eq!(sends(&effects)[0].to.code, "ja");
        assert!(effects.contains(&Effect::Remember(Languages {
            from: find("fr"),
            to: vec![find("ja").unwrap()],
        })));

        let effects = chose(&mut session, "from", "detect");
        assert_eq!(sends(&effects)[0].from, None);

        let list = list(&effects);
        assert_eq!(
            scope(&list, "from").current().unwrap().title,
            "Detect language"
        );
        assert_eq!(scope(&list, "to").current().unwrap().title, "Japanese");
        assert_eq!(
            list.placeholder.as_deref(),
            Some("Translate any language to Japanese")
        );
    }

    #[test]
    fn every_language_is_offered_on_both_sides() {
        let list = list(&[Effect::Render(session().view())]);
        // Detection, then the table.
        assert_eq!(scope(&list, "from").options.len(), LANGUAGES.len() + 1);
        // The preset's set of three, then the table.
        let to = scope(&list, "to");
        assert_eq!(to.options.len(), LANGUAGES.len() + 1);
        assert_eq!(to.options[0].title, "English, French and German");
        assert_eq!(to.options[0].id, "lang-en,fr,de");
    }

    /// I-10: the list keeps its shape while answers arrive.
    #[test]
    fn one_row_per_target_from_the_first_keystroke() {
        let mut session = session();
        chose(&mut session, "to", "lang-en,fr,de");
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

    /// A target kept across a change keeps its row's text while it is asked
    /// again; a new one starts empty rather than showing another language.
    #[test]
    fn a_changed_target_does_not_inherit_another_languages_text() {
        let mut session = session();
        typed(&mut session, "hola");
        let generation = session.generation;
        session.answer(generation, 0, done("Hello"));

        let list = list(&chose(&mut session, "to", "lang-en,fr,de"));
        assert_eq!(list.item(0).unwrap().title, "Hello");
        assert_eq!(list.item(1).unwrap().title, "Translating\u{2026}");
    }

    #[test]
    fn an_unknown_option_changes_nothing() {
        let mut session = session();
        assert!(chose(&mut session, "to", "lang-xx").is_empty());
        assert!(chose(&mut session, "from", "lang-").is_empty());
        assert!(chose(&mut session, "elsewhere", "detect").is_empty());
        assert!(chose(&mut session, "to", "lang-en,fr,de,it,pt").is_empty());
        // Choosing what is already in force asks for nothing again.
        assert!(chose(&mut session, "to", "lang-en").is_empty());
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
    fn switching_to_a_preset_sets_both_languages() {
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
        let requests = sends(&effects);
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|request| request.from.is_none()));
    }

    #[test]
    fn a_detected_source_titles_the_section() {
        let mut session = session();
        chose(&mut session, "from", "detect");
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
        chose(&mut session, "to", "lang-en,fr,de");
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
        let list = list(&session.answer(
            generation,
            0,
            Err(Failure::Other("Google answered 429".into())),
        ));

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

    /// Offline is said plainly, in the row and the section, and Enter retries.
    #[test]
    fn no_connection_is_said_as_such_and_can_be_retried() {
        let mut session = session();
        typed(&mut session, "hola");
        let generation = session.generation;
        let list = list(&session.answer(generation, 0, Err(Failure::Offline)));

        assert_eq!(
            list.sections[0].title.as_deref(),
            Some("Offline \u{00b7} Google Translate")
        );
        let item = list.item(0).unwrap();
        assert_eq!(item.title, "No internet connection");
        assert!(
            item.subtitle
                .as_deref()
                .unwrap()
                .contains("Enter tries again")
        );
        assert_eq!(item.actions[0].id, "retry");
        assert!(matches!(
            &item.accessories[0],
            Accessory::Tag {
                tone: Tone::Caution,
                ..
            }
        ));

        let effects = session.event(Event::ActionInvoked {
            item: Some("target-0".into()),
            action: "retry".into(),
        });
        assert_eq!(sends(&effects).len(), 1);
    }

    #[test]
    fn the_view_says_who_receives_the_text() {
        let start = Languages {
            from: None,
            to: vec![find("en").unwrap()],
        };
        let session = Session::new(Vec::new(), start, Service::Deepl);
        let View::List(list) = session.view() else {
            panic!("expected a list")
        };
        let description = list.empty.unwrap().description.unwrap();
        assert!(description.contains("DeepL"));
        assert_eq!(
            list.placeholder.as_deref(),
            Some("Translate any language to English")
        );
    }
}
