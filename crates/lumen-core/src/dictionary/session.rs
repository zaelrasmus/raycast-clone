//! The Define view's state, as a plain value (F10).
//!
//! Translate's shape (F7): the daemon owns the threads and the timer, this
//! owns every decision. It is told what happened -- an event, a pause in
//! typing, an answer -- and replies with [`Effect`]s. Nothing here blocks or
//! touches the network, so every rule below is a unit test.
//!
//! # One lookup per word
//!
//! A lookup starts after typing pauses and carries a generation; an answer
//! from an older one is dropped. Answers are kept for the life of the view,
//! keyed by the word, its language and the explanation language, so going
//! back to a word costs no request -- which matters against a service that
//! allows 1,000 an hour.
//!
//! # The pane
//!
//! A found word is a list with a detail pane: a section per part of speech, a
//! row per sense with its first example, and the sense in full beside it.
//! While the next word is looked up the previous one stays on screen without
//! its actions, so `Enter` cannot copy a definition that is about to change.

use std::collections::HashMap;

use super::languages::{self, LANGUAGES, Language};
use super::{Entry, Failure, Part, Sense};
use crate::view::{
    Accessory, Action, EmptyState, Event, Filtering, Icon, Item, ItemDetail, List, MetadataRow,
    Scope, ScopeOption, Section, Tone, View,
};

/// What the driver should do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Call [`Session::settle`] with this generation after the typing pause.
    Wait { generation: u64 },
    /// Look this up on a worker thread and report back with
    /// [`Session::answer`].
    Send(Request),
    /// Put this on the clipboard and close the launcher.
    Copy(String),
    /// Open this page in the browser and close the launcher.
    Open(String),
    /// This word language was chosen; open with it next time.
    Remember(&'static Language),
}

/// One lookup to make.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub generation: u64,
    pub word: String,
    pub language: &'static Language,
    pub explained_in: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    /// Nothing typed.
    Idle,
    /// Typed, but longer than a word.
    TooLong,
    /// Asked, not answered. Keeps the entry shown before.
    Pending {
        previous: Option<Entry>,
    },
    Found(Entry),
    NotFound,
    Failed(Failure),
}

type Key = (String, &'static str, &'static str);

pub struct Session {
    language: &'static Language,
    explained_in: &'static str,
    /// Who the definitions come from, said in every pane.
    attribution: &'static str,
    query: String,
    generation: u64,
    state: State,
    /// Every answer this view has had.
    answers: HashMap<Key, Option<Entry>>,
}

// Ids. Opaque to the host; only this file reads them back.
const COPY: &str = "copy";
const COPY_WORD: &str = "copy-word";
const OPEN: &str = "open";
const RETRY: &str = "retry";
const SENSE: &str = "sense-";
const FAILURE: &str = "failure";
/// The word language's scope, and the prefix of its options.
const FROM: &str = "from";
const LANGUAGE: &str = "lang-";

impl Session {
    pub fn new(
        language: &'static Language,
        explained_in: &'static str,
        attribution: &'static str,
    ) -> Self {
        Self {
            language,
            explained_in,
            attribution,
            query: String::new(),
            generation: 0,
            state: State::Idle,
            answers: HashMap::new(),
        }
    }

    fn word(&self) -> Option<&str> {
        super::word(&self.query)
    }

    fn key(&self) -> Option<Key> {
        Some((
            self.word()?.to_string(),
            self.language.code,
            self.explained_in,
        ))
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
            Event::ActionInvoked { item, action } => self.act(item.as_deref(), &action),
            Event::ScopeChanged { scope, option } => self.choose(&scope, &option),
            Event::SelectionChanged { .. } | Event::Dismissed => Vec::new(),
        }
    }

    /// The typing pause for `generation` is over.
    pub fn settle(&mut self, generation: u64) -> Vec<Effect> {
        if generation != self.generation || !matches!(self.state, State::Pending { .. }) {
            return Vec::new();
        }
        self.request()
    }

    /// A lookup came back.
    pub fn answer(
        &mut self,
        generation: u64,
        result: Result<Option<Entry>, Failure>,
    ) -> Vec<Effect> {
        if generation != self.generation {
            return Vec::new();
        }
        self.state = match result {
            Ok(entry) => {
                if let Some(key) = self.key() {
                    self.answers.insert(key, entry.clone());
                }
                match entry {
                    Some(entry) => State::Found(entry),
                    None => State::NotFound,
                }
            }
            Err(failure) => State::Failed(failure),
        };
        vec![Effect::Render(self.view())]
    }

    /// A word language was chosen in *From*: look the same text up in it, at
    /// once. What the view already had for the other language stays in its
    /// answers, so switching back costs nothing.
    fn choose(&mut self, scope: &str, option: &str) -> Vec<Effect> {
        if scope != FROM {
            return Vec::new();
        }
        let Some(language) = option.strip_prefix(LANGUAGE).and_then(languages::find) else {
            return Vec::new();
        };
        if language == self.language {
            return Vec::new();
        }
        self.language = language;
        // The entry on screen is in the other language: it must not stay up
        // as if it were the answer being waited for.
        if matches!(self.state, State::Found(_) | State::Pending { .. }) {
            self.state = State::Pending { previous: None };
        }
        let mut effects = vec![Effect::Remember(language)];
        effects.extend(self.restart(true));
        effects
    }

    /// *From*: every language the source covers well.
    fn source_scope(&self) -> Scope {
        Scope {
            id: FROM.into(),
            title: "From".into(),
            options: LANGUAGES
                .iter()
                .map(|language| ScopeOption {
                    id: format!("{LANGUAGE}{}", language.code),
                    title: language.name.into(),
                    keywords: vec![language.code.into()],
                })
                .collect(),
            selected: format!("{LANGUAGE}{}", self.language.code),
        }
    }

    /// What should be looked up changed: start a new generation.
    ///
    /// `now` skips the typing pause, for a change that was not typing.
    fn restart(&mut self, now: bool) -> Vec<Effect> {
        self.generation += 1;

        if self.query.trim().is_empty() {
            self.state = State::Idle;
            return vec![Effect::Render(self.view())];
        }
        let Some(key) = self.key() else {
            self.state = State::TooLong;
            return vec![Effect::Render(self.view())];
        };
        if let Some(answer) = self.answers.get(&key) {
            self.state = match answer.clone() {
                Some(entry) => State::Found(entry),
                None => State::NotFound,
            };
            return vec![Effect::Render(self.view())];
        }

        let previous = match &self.state {
            State::Found(entry) => Some(entry.clone()),
            State::Pending { previous } => previous.clone(),
            _ => None,
        };
        self.state = State::Pending { previous };

        let mut effects = vec![Effect::Render(self.view())];
        if now {
            effects.extend(self.request());
        } else {
            effects.push(Effect::Wait {
                generation: self.generation,
            });
        }
        effects
    }

    fn request(&self) -> Vec<Effect> {
        let Some(word) = self.word() else {
            return Vec::new();
        };
        vec![Effect::Send(Request {
            generation: self.generation,
            word: word.to_string(),
            language: self.language,
            explained_in: self.explained_in,
        })]
    }

    fn act(&mut self, item: Option<&str>, action: &str) -> Vec<Effect> {
        let State::Found(entry) = &self.state else {
            return match action {
                RETRY => self.restart(true),
                _ => Vec::new(),
            };
        };
        match action {
            COPY => item
                .and_then(|item| sense(entry, item))
                .map(|(_, sense)| vec![Effect::Copy(sense.definition.clone())])
                .unwrap_or_default(),
            COPY_WORD => vec![Effect::Copy(entry.word.clone())],
            OPEN if !entry.url.is_empty() => vec![Effect::Open(entry.url.clone())],
            _ => Vec::new(),
        }
    }

    /// What the view should show now.
    pub fn view(&self) -> View {
        let empty = |title: String, description: Option<String>| EmptyState {
            title,
            description,
            icon: None,
        };
        let quoted = format!("\u{201c}{}\u{201d}", self.query.trim());

        let (entry, actionable) = match &self.state {
            State::Found(entry) => (Some(entry), true),
            State::Pending {
                previous: Some(entry),
            } => (Some(entry), false),
            _ => (None, false),
        };
        let (empty, sections) = match (&self.state, entry) {
            (_, Some(entry)) => (None, self.sections(entry, actionable)),
            (State::Idle, _) => (
                Some(empty(
                    "Type a word to define".into(),
                    Some(format!(
                        "Looked up as you type, in {}. Tab chooses the language.",
                        self.attribution_short()
                    )),
                )),
                Vec::new(),
            ),
            (State::TooLong, _) => (
                Some(empty(
                    "That is longer than a word".into(),
                    Some(format!(
                        "Define looks up words and short phrases, up to {} characters.",
                        super::MAX_CHARS
                    )),
                )),
                Vec::new(),
            ),
            (State::NotFound, _) => (
                Some(empty(
                    format!("No definition for {quoted}"),
                    Some(format!(
                        "Wiktionary has no {} entry for it. Check the spelling, or the language.",
                        self.language.name
                    )),
                )),
                Vec::new(),
            ),
            (State::Failed(failure), _) => (None, vec![self.failure(failure)]),
            (State::Pending { .. }, None) | (State::Found(_), None) => (
                Some(empty(format!("Looking up {quoted}\u{2026}"), None)),
                Vec::new(),
            ),
        };

        View::List(List {
            placeholder: Some(format!("Define a word in {}", self.language.name)),
            filtering: Filtering::Extension,
            scopes: vec![self.source_scope()],
            empty,
            showing_detail: entry.is_some(),
            sections,
        })
    }

    fn attribution_short(&self) -> &'static str {
        "English Wiktionary"
    }

    /// A section per part of speech, a row per sense.
    fn sections(&self, entry: &Entry, actionable: bool) -> Vec<Section> {
        entry
            .parts
            .iter()
            .enumerate()
            .map(|(p, part)| Section {
                title: Some(capitalised(&part.name)),
                subtitle: part.pronunciations.first().cloned(),
                items: part
                    .senses
                    .iter()
                    .enumerate()
                    .map(|(s, sense)| {
                        let actions = if actionable {
                            let mut actions = vec![
                                action(COPY, "Copy Definition"),
                                action(COPY_WORD, &format!("Copy \u{201c}{}\u{201d}", entry.word)),
                            ];
                            if !entry.url.is_empty() {
                                actions.push(action(OPEN, "Open in Wiktionary"));
                            }
                            actions
                        } else {
                            Vec::new()
                        };
                        Item {
                            id: format!("{SENSE}{p}-{s}"),
                            title: one_line(&sense.definition),
                            subtitle: sense.examples.first().map(|example| one_line(example)),
                            icon: Some(Icon::Text {
                                text: (s + 1).to_string(),
                            }),
                            accessories: Vec::new(),
                            keywords: Vec::new(),
                            actions,
                            detail: Some(self.detail(entry, part, sense)),
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    /// The pane: the word, the part and its pronunciation, the sense in full,
    /// its examples, and what else the source says.
    fn detail(&self, entry: &Entry, part: &Part, sense: &Sense) -> ItemDetail {
        let mut markdown = format!("# {}\n\n", entry.word);

        let mut heading = vec![format!("*{}*", part.name)];
        heading.extend(part.pronunciations.iter().take(2).cloned());
        markdown.push_str(&heading.join(" \u{00b7} "));
        markdown.push_str("\n\n");

        // A definition that opens with its labels -- "(transitive) to show" --
        // already says them.
        if !sense.tags.is_empty() && !sense.definition.starts_with('(') {
            markdown.push_str(&format!("*{}*\n\n", sense.tags.join(", ")));
        }
        markdown.push_str(&sense.definition);
        markdown.push_str("\n\n");
        for example in &sense.examples {
            markdown.push_str(&format!("- *{}*\n", one_line(example)));
        }

        if !sense.subsenses.is_empty() {
            markdown.push_str("\n### More precisely\n\n");
            for subsense in &sense.subsenses {
                markdown.push_str(&format!("- {}\n", one_line(&subsense.definition)));
            }
        }
        if !sense.synonyms.is_empty() {
            markdown.push_str(&format!(
                "\n### Synonyms\n\n{}\n",
                sense.synonyms.join(", ")
            ));
        }
        if !part.forms.is_empty() {
            markdown.push_str("\n### Forms\n\n");
            let forms: Vec<String> = part
                .forms
                .iter()
                .map(|form| match form.tags.is_empty() {
                    true => form.word.clone(),
                    false => format!("{} *({})*", form.word, form.tags.join(", ")),
                })
                .collect();
            markdown.push_str(&forms.join(", "));
            markdown.push('\n');
        }

        ItemDetail {
            markdown,
            metadata: vec![
                MetadataRow {
                    label: "Language".into(),
                    value: self.language.name.into(),
                    tone: Tone::Neutral,
                },
                MetadataRow {
                    label: "Source".into(),
                    value: self.attribution.into(),
                    tone: Tone::Neutral,
                },
            ],
        }
    }

    fn failure(&self, failure: &Failure) -> Section {
        let (title, subtitle, tone) = match failure {
            Failure::Offline => (
                "No internet connection".to_string(),
                "Define needs the internet to reach Wiktionary. Enter tries again.".to_string(),
                Tone::Caution,
            ),
            Failure::Other(message) => (
                "Could not look it up".to_string(),
                message.clone(),
                Tone::Danger,
            ),
        };
        Section {
            title: None,
            subtitle: None,
            items: vec![Item {
                id: FAILURE.into(),
                title,
                subtitle: Some(subtitle),
                icon: None,
                accessories: vec![Accessory::Tag {
                    text: self.language.name.into(),
                    tone,
                }],
                keywords: Vec::new(),
                actions: vec![action(RETRY, "Try Again")],
                detail: None,
            }],
        }
    }
}

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
    }
}

/// The sense a row's id names.
fn sense<'a>(entry: &'a Entry, item: &str) -> Option<(&'a Part, &'a Sense)> {
    let (p, s) = item.strip_prefix(SENSE)?.split_once('-')?;
    let part = entry.parts.get(p.parse::<usize>().ok()?)?;
    Some((part, part.senses.get(s.parse::<usize>().ok()?)?))
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Text drawn on one row: line breaks become spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::{free, languages};

    fn display() -> Entry {
        free::parse(include_str!("fixtures/display.json"), "en")
            .unwrap()
            .unwrap()
    }

    fn session() -> Session {
        Session::new(languages::english(), "en", "Wiktionary")
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

    fn act(session: &mut Session, item: &str, action: &str) -> Vec<Effect> {
        session.event(Event::ActionInvoked {
            item: Some(item.into()),
            action: action.into(),
        })
    }

    /// Typed, paused, answered: the usual path.
    fn found(session: &mut Session, word: &str) -> List {
        let effects = typed(session, word);
        let Some(Effect::Wait { generation }) = effects.last() else {
            panic!("expected a wait, got {effects:?}")
        };
        let generation = *generation;
        let requests = sends(&session.settle(generation));
        assert_eq!(requests.len(), 1);
        list(&session.answer(generation, Ok(Some(display()))))
    }

    #[test]
    fn typing_waits_for_a_pause_then_asks_once() {
        let mut session = session();
        let effects = typed(&mut session, "  display ");
        assert!(sends(&effects).is_empty());
        let Some(Effect::Wait { generation }) = effects.last() else {
            panic!("expected a wait")
        };

        let requests = sends(&session.settle(*generation));
        assert_eq!(
            requests,
            [Request {
                generation: *generation,
                word: "display".into(),
                language: languages::english(),
                explained_in: "en",
            }]
        );
    }

    #[test]
    fn a_pause_or_answer_overtaken_by_typing_is_ignored() {
        let mut session = session();
        typed(&mut session, "disp");
        let old = session.generation;
        typed(&mut session, "display");
        assert!(session.settle(old).is_empty());
        assert!(session.answer(old, Ok(None)).is_empty());
    }

    /// A section per part of speech, a row per sense, the pane beside them.
    #[test]
    fn a_found_word_is_senses_by_part_of_speech_with_a_pane() {
        let list = found(&mut session(), "display");

        assert!(list.showing_detail);
        let titles: Vec<_> = list
            .sections
            .iter()
            .map(|section| section.title.clone().unwrap())
            .collect();
        assert_eq!(titles, ["Noun", "Verb"]);
        assert_eq!(list.sections[0].subtitle.as_deref(), Some("/dɪsˈpleɪ/"));

        let first = list.item(0).unwrap();
        assert_eq!(first.title, "A show or spectacle.");
        assert!(first.subtitle.as_deref().unwrap().contains("trapeze"));
        assert_eq!(first.icon, Some(Icon::Text { text: "1".into() }));

        let detail = first.detail.as_ref().unwrap();
        assert!(detail.markdown.starts_with("# display\n"));
        assert!(detail.markdown.contains("A show or spectacle."));
        assert!(detail.markdown.contains("displays *(plural)*"));
        assert!(
            detail
                .metadata
                .iter()
                .any(|row| row.label == "Source" && row.value == "Wiktionary")
        );
    }

    #[test]
    fn enter_copies_the_definition_and_the_panel_offers_the_rest() {
        let mut session = session();
        let list = found(&mut session, "display");
        let item = list.item(1).unwrap();
        let titles: Vec<&str> = item.actions.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Copy Definition",
                "Copy \u{201c}display\u{201d}",
                "Open in Wiktionary"
            ]
        );

        let effects = act(&mut session, &item.id, &item.actions[0].id);
        assert_eq!(effects, [Effect::Copy(item.title.clone())]);
        assert_eq!(
            act(&mut session, &item.id, "copy-word"),
            [Effect::Copy("display".into())]
        );
        assert_eq!(
            act(&mut session, &item.id, "open"),
            [Effect::Open(
                "https://en.wiktionary.org/wiki/display#English".into()
            )]
        );
    }

    /// The previous word stays while the next is looked up, without its
    /// actions: Enter must not copy what is about to change.
    #[test]
    fn the_previous_word_stays_without_actions_while_the_next_is_looked_up() {
        let mut session = session();
        found(&mut session, "display");
        let list = list(&typed(&mut session, "displays"));
        assert_eq!(list.item(0).unwrap().title, "A show or spectacle.");
        assert!(list.items().all(|item| item.actions.is_empty()));
    }

    /// Going back to a word is answered from what the view already has.
    #[test]
    fn a_word_looked_up_before_is_not_asked_for_again() {
        let mut session = session();
        found(&mut session, "display");
        typed(&mut session, "displa");
        let effects = typed(&mut session, "display");
        assert!(sends(&effects).is_empty());
        assert!(!effects.iter().any(|e| matches!(e, Effect::Wait { .. })));
        assert_eq!(
            list(&effects).item(0).unwrap().title,
            "A show or spectacle."
        );
    }

    #[test]
    fn a_word_the_source_does_not_have_says_so() {
        let mut session = session();
        typed(&mut session, "qwzzx");
        let generation = session.generation;
        let list = list(&session.answer(generation, Ok(None)));
        assert!(list.is_empty());
        assert!(!list.showing_detail);
        let empty = list.empty.unwrap();
        assert_eq!(empty.title, "No definition for \u{201c}qwzzx\u{201d}");
        assert!(empty.description.unwrap().contains("English"));
    }

    #[test]
    fn offline_is_said_as_such_and_enter_tries_again() {
        let mut session = session();
        typed(&mut session, "display");
        let generation = session.generation;
        let list = list(&session.answer(generation, Err(Failure::Offline)));
        let item = list.item(0).unwrap();
        assert_eq!(item.title, "No internet connection");
        assert_eq!(item.actions[0].id, "retry");

        let effects = act(&mut session, "failure", "retry");
        assert_eq!(sends(&effects).len(), 1);
    }

    #[test]
    fn nothing_typed_or_too_much_asks_nothing() {
        let mut session = session();
        let list = list(&typed(&mut session, "   "));
        assert_eq!(list.empty.unwrap().title, "Type a word to define");

        let effects = typed(&mut session, &"word ".repeat(20));
        assert!(sends(&effects).is_empty());
        assert!(!effects.iter().any(|e| matches!(e, Effect::Wait { .. })));
        assert_eq!(
            list_after(effects).empty.unwrap().title,
            "That is longer than a word"
        );
    }

    fn list_after(effects: Vec<Effect>) -> List {
        list(&effects)
    }

    fn chose(session: &mut Session, option: &str) -> Vec<Effect> {
        session.event(Event::ScopeChanged {
            scope: "from".into(),
            option: option.into(),
        })
    }

    /// `significado` in Spanish, explained in English: the case F10 is for.
    #[test]
    fn choosing_a_language_looks_the_same_word_up_in_it_at_once() {
        let mut session = session();
        typed(&mut session, "significado");
        let generation = session.generation;
        session.answer(generation, Ok(None));

        let effects = chose(&mut session, "lang-es");
        let spanish = languages::find("es").unwrap();
        assert!(effects.contains(&Effect::Remember(spanish)));
        let requests = sends(&effects);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].word, "significado");
        assert_eq!(requests[0].language, spanish);
        assert_eq!(requests[0].explained_in, "en");

        let list = list(&effects);
        let from = &list.scopes[0];
        assert_eq!(from.current().unwrap().title, "Spanish");
        assert_eq!(from.options.len(), LANGUAGES.len());
        assert_eq!(
            list.placeholder.as_deref(),
            Some("Define a word in Spanish")
        );
    }

    /// An English entry must not stay up while the Spanish one is asked for.
    #[test]
    fn the_other_languages_entry_is_not_shown_while_waiting() {
        let mut session = session();
        found(&mut session, "display");
        let list = list(&chose(&mut session, "lang-fr"));
        assert!(list.is_empty());
        assert!(!list.showing_detail);
    }

    /// Each language's answer is kept apart: going back asks nothing.
    #[test]
    fn switching_back_to_a_language_uses_its_answer() {
        let mut session = session();
        found(&mut session, "display");
        chose(&mut session, "lang-fr");
        let effects = chose(&mut session, "lang-en");
        assert!(sends(&effects).is_empty());
        assert_eq!(
            list(&effects).item(0).unwrap().title,
            "A show or spectacle."
        );
    }

    #[test]
    fn an_unknown_or_current_language_changes_nothing() {
        let mut session = session();
        assert!(chose(&mut session, "lang-xx").is_empty());
        assert!(chose(&mut session, "lang-en").is_empty());
        let other = session.event(Event::ScopeChanged {
            scope: "to".into(),
            option: "lang-es".into(),
        });
        assert!(other.is_empty());
    }

    /// A definition that opens with its labels does not repeat them.
    #[test]
    fn labels_are_said_once() {
        let mostrar = free::parse(include_str!("fixtures/mostrar.json"), "es")
            .unwrap()
            .unwrap();
        let session = Session::new(languages::find("es").unwrap(), "en", "Wiktionary");
        let (part, sense) = (&mostrar.parts[0], &mostrar.parts[0].senses[0]);
        let markdown = session.detail(&mostrar, part, sense).markdown;
        assert!(markdown.contains("(transitive) to show"));
        assert!(!markdown.contains("*transitive*"));
    }
}
