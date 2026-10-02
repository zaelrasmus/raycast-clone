//! The Dictionary view's state, as a plain value (F10).
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
//!
//! # Detect
//!
//! With no word language chosen, one request asks for every language in
//! the table, and the view chooses: the language with the most senses --
//! how much of a word it is there -- then, on a tie, a language the user
//! has looked words up in, then English. The others stay one key away as
//! *Show in ...*, and *From* still chooses one outright. Every language in
//! the answer is kept apart among the answers, so showing another asks
//! nothing.
//!
//! # Favourites and history
//!
//! What an empty search shows. A favourite keeps its whole entry and is put
//! among the answers when the view opens, so it opens with no request. A word
//! goes into history when it is *used* -- copied, opened, translated, saved,
//! or left on screen when the view closes -- not whenever a lookup answers,
//! which would fill history with `dis` and `disp` on the way to `display`.

use std::collections::HashMap;

use super::languages::{self, LANGUAGES, Language};
use super::store;
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
    /// This word language was chosen, or detection (`None`); open with it
    /// next time.
    Remember(Option<&'static Language>),
    /// Open Translate on this word (F10).
    Translate(String),
    /// Put this in the search field, as if typed: a favourite or a recent
    /// word was chosen.
    Type(String),
    /// Write favourites and history to `dictionary.json`.
    Save(store::State),
}

/// One lookup to make.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub generation: u64,
    pub word: String,
    /// `None` detects it.
    pub language: Option<&'static Language>,
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

/// A word, the language asked for -- a code, or [`DETECT`] -- and the
/// explanation language.
type Key = (String, &'static str, &'static str);

pub struct Session {
    /// `None` detects it.
    language: Option<&'static Language>,
    /// With detection, the other languages that have the word on screen, in
    /// the order they were ranked.
    also: Vec<&'static Language>,
    explained_in: &'static str,
    /// Who the definitions come from, said in every pane.
    attribution: &'static str,
    query: String,
    generation: u64,
    state: State,
    /// Every answer this view has had, favourites included: none, one, or
    /// with detection one per language.
    answers: HashMap<Key, Vec<Entry>>,
    /// Favourites and history, as `dictionary.json` holds them.
    saved: store::State,
    /// Whether history is kept (`dictionary.history`).
    keeping_history: bool,
}

// Ids. Opaque to the host; only this file reads them back.
const COPY: &str = "copy";
const COPY_WORD: &str = "copy-word";
const OPEN: &str = "open";
const TRANSLATE: &str = "translate";
const RETRY: &str = "retry";
const SENSE: &str = "sense-";
const FAILURE: &str = "failure";
const FAVOURITE: &str = "favourite";
const FAVOURITE_ROW: &str = "favourite-";
const RECENT_ROW: &str = "recent-";
const SHOW: &str = "show";
const UNFAVOURITE: &str = "unfavourite";
const FORGET: &str = "forget";
const CLEAR_HISTORY: &str = "clear-history";
/// The word language's scope, and the prefix of its options.
const FROM: &str = "from";
const LANGUAGE: &str = "lang-";
/// *Detect*, as an option of *From* and in the answers' keys.
const DETECT: &str = "detect";
/// *Show in ...*, by code.
const SHOW_IN: &str = "in-";
/// How many other languages *Show in ...* offers.
const MAX_ALSO: usize = 5;

impl Session {
    /// A session looking words up in `language`, explained in
    /// `explained_in`, with what `dictionary.json` holds.
    ///
    /// With history off, what it held is dropped, and the next save writes
    /// that: turning history off clears it.
    pub fn new(
        language: Option<&'static Language>,
        explained_in: &'static str,
        attribution: &'static str,
        mut saved: store::State,
        keeping_history: bool,
    ) -> Self {
        if !keeping_history {
            saved.history.clear();
        }
        let mut answers = HashMap::new();
        for entry in &saved.favourites {
            if let Some(language) = languages::find(&entry.language)
                && entry.explained_in == explained_in
            {
                answers.insert(
                    (entry.word.clone(), language.code, explained_in),
                    vec![entry.clone()],
                );
            }
        }
        Self {
            language,
            also: Vec::new(),
            explained_in,
            attribution,
            query: String::new(),
            generation: 0,
            state: State::Idle,
            answers,
            saved,
            keeping_history,
        }
    }

    /// The view is gone: a word left on screen counts as looked at.
    pub fn closed(&mut self) -> Vec<Effect> {
        self.record()
    }

    /// Put the word on screen into history, if it is kept; save if that
    /// changed anything.
    fn record(&mut self) -> Vec<Effect> {
        let State::Found(entry) = &self.state else {
            return Vec::new();
        };
        if !self.keeping_history {
            return Vec::new();
        }
        let before = self.saved.history.clone();
        self.saved.record(&entry.word, &entry.language);
        if self.saved.history == before {
            return Vec::new();
        }
        vec![Effect::Save(self.saved.clone())]
    }

    fn word(&self) -> Option<&str> {
        super::word(&self.query)
    }

    fn key(&self) -> Option<Key> {
        Some((
            self.word()?.to_string(),
            self.language.map_or(DETECT, |language| language.code),
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
            Event::Dismissed => self.closed(),
            Event::SelectionChanged { .. } | Event::Back => Vec::new(),
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
    pub fn answer(&mut self, generation: u64, result: Result<Vec<Entry>, Failure>) -> Vec<Effect> {
        if generation != self.generation {
            return Vec::new();
        }
        match result {
            Ok(entries) => {
                if let Some((word, _, explained_in)) = self.key() {
                    // Each language on its own too, so *Show in ...* and
                    // choosing it in *From* ask nothing.
                    if self.language.is_none() {
                        for entry in &entries {
                            if let Some(language) = languages::find(&entry.language) {
                                self.answers.insert(
                                    (word.clone(), language.code, explained_in),
                                    vec![entry.clone()],
                                );
                            }
                        }
                    }
                }
                if let Some(key) = self.key() {
                    self.answers.insert(key, entries.clone());
                }
                self.resolve(entries);
            }
            Err(failure) => {
                self.also.clear();
                self.state = State::Failed(failure);
            }
        }
        vec![Effect::Render(self.view())]
    }

    /// Show an answer: the one entry asked for, or with detection the best
    /// of them, the rest kept as *Show in ...*.
    fn resolve(&mut self, entries: Vec<Entry>) {
        let mut entries = entries;
        if self.language.is_none() {
            let preferred = self.preferred();
            rank(&mut entries, &preferred);
        }
        self.also = entries
            .iter()
            .skip(1)
            .filter_map(|entry| languages::find(&entry.language))
            .collect();
        if self.language.is_some() {
            self.also.clear();
        }
        self.state = match entries.into_iter().next() {
            Some(entry) => State::Found(entry),
            None => State::NotFound,
        };
    }

    /// The languages the user looks words up in, most recent first: a tie
    /// in detection goes to one of these.
    fn preferred(&self) -> Vec<String> {
        let mut preferred: Vec<String> = Vec::new();
        let saved = self
            .saved
            .history
            .iter()
            .map(|recent| &recent.language)
            .chain(self.saved.favourites.iter().map(|entry| &entry.language));
        for language in saved {
            if !preferred.contains(language) {
                preferred.push(language.clone());
            }
        }
        preferred
    }

    /// A word language was chosen in *From*: look the same text up in it, at
    /// once. What the view already had for the other language stays in its
    /// answers, so switching back costs nothing.
    fn choose(&mut self, scope: &str, option: &str) -> Vec<Effect> {
        if scope != FROM {
            return Vec::new();
        }
        let language = if option == DETECT {
            None
        } else {
            match option.strip_prefix(LANGUAGE).and_then(languages::find) {
                Some(language) => Some(language),
                None => return Vec::new(),
            }
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

    /// *From*: detection, then every language the source covers well.
    fn source_scope(&self) -> Scope {
        let detect = ScopeOption {
            id: DETECT.into(),
            title: "Detect language".into(),
            keywords: vec!["auto".into(), "any".into()],
        };
        Scope {
            id: FROM.into(),
            title: "From".into(),
            options: std::iter::once(detect)
                .chain(LANGUAGES.iter().map(|language| ScopeOption {
                    id: format!("{LANGUAGE}{}", language.code),
                    title: language.name.into(),
                    keywords: vec![language.code.into()],
                }))
                .collect(),
            selected: match self.language {
                Some(language) => format!("{LANGUAGE}{}", language.code),
                None => DETECT.into(),
            },
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
        if let Some(entries) = self.answers.get(&key).cloned() {
            self.resolve(entries);
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
        if let Some(effects) = self.act_on_saved(item, action) {
            return effects;
        }
        // Another language the word is in: shown from what the view has,
        // and not remembered -- the next opening still detects.
        if let Some(language) = action.strip_prefix(SHOW_IN).and_then(languages::find) {
            self.language = Some(language);
            return self.restart(true);
        }
        let State::Found(entry) = &self.state else {
            return match action {
                RETRY => self.restart(true),
                _ => Vec::new(),
            };
        };
        let entry = entry.clone();
        let done = match action {
            COPY => item
                .and_then(|item| sense(&entry, item))
                .map(|(_, sense)| Effect::Copy(sense.definition.clone())),
            COPY_WORD => Some(Effect::Copy(entry.word.clone())),
            OPEN if !entry.url.is_empty() => Some(Effect::Open(entry.url.clone())),
            TRANSLATE => Some(Effect::Translate(entry.word.clone())),
            FAVOURITE => {
                self.saved.toggle_favourite(&entry);
                let mut effects = self.record();
                // `record` saves only when history changed; a favourite
                // always has something to write.
                effects.retain(|effect| !matches!(effect, Effect::Save(_)));
                effects.push(Effect::Save(self.saved.clone()));
                effects.push(Effect::Render(self.view()));
                return effects;
            }
            _ => None,
        };
        let Some(done) = done else {
            return Vec::new();
        };
        let mut effects = self.record();
        effects.push(done);
        effects
    }

    /// An action on a favourite's or a recent word's row, when nothing is
    /// typed. `None` when the row is neither.
    fn act_on_saved(&mut self, item: Option<&str>, action: &str) -> Option<Vec<Effect>> {
        let item = item?;
        if let Some(index) = index(item, FAVOURITE_ROW) {
            let entry = self.saved.favourites.get(index)?.clone();
            return Some(match action {
                SHOW => self.show(&entry.word, &entry.language),
                UNFAVOURITE => {
                    self.saved.favourites.remove(index);
                    vec![
                        Effect::Save(self.saved.clone()),
                        Effect::Render(self.view()),
                    ]
                }
                _ => Vec::new(),
            });
        }
        if let Some(index) = index(item, RECENT_ROW) {
            let recent = self.saved.history.get(index)?.clone();
            return Some(match action {
                SHOW => self.show(&recent.word, &recent.language),
                FORGET => {
                    self.saved.history.remove(index);
                    vec![
                        Effect::Save(self.saved.clone()),
                        Effect::Render(self.view()),
                    ]
                }
                CLEAR_HISTORY => {
                    self.saved.history.clear();
                    vec![
                        Effect::Save(self.saved.clone()),
                        Effect::Render(self.view()),
                    ]
                }
                _ => Vec::new(),
            });
        }
        None
    }

    /// Look a saved word up again: its language, then the word, typed. A
    /// favourite is among the answers already and needs no request.
    fn show(&mut self, word: &str, language: &str) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some(language) = languages::find(language)
            && Some(language) != self.language
        {
            self.language = Some(language);
            effects.push(Effect::Remember(Some(language)));
        }
        effects.push(Effect::Type(word.to_string()));
        effects
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
                    "Type a word to look up".into(),
                    Some(format!(
                        "Looked up as you type, in {}. Tab chooses the language.",
                        self.attribution_short()
                    )),
                )),
                self.saved_sections(),
            ),
            (State::TooLong, _) => (
                Some(empty(
                    "That is longer than a word".into(),
                    Some(format!(
                        "The dictionary looks up words and short phrases, up to {} characters.",
                        super::MAX_CHARS
                    )),
                )),
                Vec::new(),
            ),
            (State::NotFound, _) => (
                Some(empty(
                    format!("No definition for {quoted}"),
                    Some(match self.language {
                        Some(language) => format!(
                            "Wiktionary has no {} entry for it. Check the spelling, or the language.",
                            language.name
                        ),
                        None => format!(
                            "Wiktionary has no entry for it in any of the {} languages. Check the spelling.",
                            LANGUAGES.len()
                        ),
                    }),
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
            placeholder: Some(format!("Look up a word in {}", self.language_name())),
            filtering: Filtering::Extension,
            scopes: vec![self.source_scope()],
            path: Vec::new(),
            empty,
            showing_detail: entry.is_some(),
            sections,
        })
    }

    /// *Favourites*, then *Recent*: what an empty search shows.
    fn saved_sections(&self) -> Vec<Section> {
        let language_name = |code: &str| -> String {
            languages::find(code).map_or_else(|| code.to_string(), |language| language.name.into())
        };
        let mut sections = Vec::new();
        if !self.saved.favourites.is_empty() {
            sections.push(Section {
                title: Some("Favourites".into()),
                subtitle: None,
                items: self
                    .saved
                    .favourites
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| Item {
                        id: format!("{FAVOURITE_ROW}{index}"),
                        title: entry.word.clone(),
                        subtitle: entry
                            .senses()
                            .next()
                            .map(|(_, sense)| one_line(&sense.definition)),
                        icon: None,
                        accessories: vec![Accessory::Text {
                            text: language_name(&entry.language),
                            strong: false,
                        }],
                        keywords: Vec::new(),
                        actions: vec![
                            action(SHOW, "Show Definition"),
                            action(UNFAVOURITE, "Remove from Favourites"),
                        ],
                        detail: None,
                    })
                    .collect(),
            });
        }
        if self.keeping_history && !self.saved.history.is_empty() {
            sections.push(Section {
                title: Some("Recent".into()),
                subtitle: None,
                items: self
                    .saved
                    .history
                    .iter()
                    .enumerate()
                    .map(|(index, recent)| Item {
                        id: format!("{RECENT_ROW}{index}"),
                        title: recent.word.clone(),
                        subtitle: None,
                        icon: None,
                        accessories: vec![Accessory::Text {
                            text: language_name(&recent.language),
                            strong: false,
                        }],
                        keywords: Vec::new(),
                        actions: vec![
                            action(SHOW, "Look Up"),
                            action(FORGET, "Remove from History"),
                            action(CLEAR_HISTORY, "Clear History"),
                        ],
                        detail: None,
                    })
                    .collect(),
            });
        }
        sections
    }

    /// The word language chosen, in words.
    fn language_name(&self) -> &'static str {
        self.language
            .map_or("any language", |language| language.name)
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
                // Detected, the language leads, so what was chosen is said.
                title: Some(match (self.language, languages::find(&entry.language)) {
                    (None, Some(language)) => {
                        format!("{} \u{00b7} {}", language.name, capitalised(&part.name))
                    }
                    _ => capitalised(&part.name),
                }),
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
                            // How it is said, one key away from what it
                            // means (F10).
                            actions.push(action(
                                TRANSLATE,
                                &format!("Translate \u{201c}{}\u{201d}", entry.word),
                            ));
                            actions.push(action(
                                FAVOURITE,
                                if self.saved.is_favourite(&entry.word, &entry.language) {
                                    "Remove from Favourites"
                                } else {
                                    "Add to Favourites"
                                },
                            ));
                            actions.extend(self.also.iter().take(MAX_ALSO).map(|language| {
                                action(
                                    &format!("{SHOW_IN}{}", language.code),
                                    &format!("Show in {}", language.name),
                                )
                            }));
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

        let language = languages::find(&entry.language)
            .map_or_else(|| entry.language.clone(), |language| language.name.into());
        let mut metadata = vec![MetadataRow {
            label: "Language".into(),
            value: match self.language {
                None => format!("{language} (detected)"),
                Some(_) => language,
            },
            tone: Tone::Neutral,
        }];
        if !self.also.is_empty() {
            let names: Vec<&str> = self.also.iter().map(|language| language.name).collect();
            metadata.push(MetadataRow {
                label: "Also in".into(),
                value: names.join(", "),
                tone: Tone::Neutral,
            });
        }
        metadata.push(MetadataRow {
            label: "Source".into(),
            value: self.attribution.into(),
            tone: Tone::Neutral,
        });
        ItemDetail {
            markdown,
            text: None,
            image: None,
            metadata,
        }
    }

    fn failure(&self, failure: &Failure) -> Section {
        let (title, subtitle, tone) = match failure {
            Failure::Offline => (
                "No internet connection".to_string(),
                "The dictionary needs the internet to reach Wiktionary. Enter tries again."
                    .to_string(),
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
                    text: self
                        .language
                        .map_or("Detect", |language| language.name)
                        .into(),
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
        icon: None,
    }
}

/// Order a detection's entries best first: the most senses, then a
/// language in `preferred` (earlier is better), then English, then the
/// source's order -- the sort is stable.
fn rank(entries: &mut [Entry], preferred: &[String]) {
    entries.sort_by_key(|entry| {
        let senses = entry.senses().count();
        let preference = preferred
            .iter()
            .position(|language| *language == entry.language)
            .unwrap_or(usize::MAX);
        let english = entry.language != "en";
        (std::cmp::Reverse(senses), preference, english)
    });
}

/// `"recent-2"` with prefix `"recent-"` is 2.
fn index(id: &str, prefix: &str) -> Option<usize> {
    id.strip_prefix(prefix)?.parse().ok()
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
        Session::new(
            Some(languages::english()),
            "en",
            "Wiktionary",
            store::State::default(),
            true,
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
        list(&session.answer(generation, Ok(vec![display()])))
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
                language: Some(languages::english()),
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
        assert!(session.answer(old, Ok(Vec::new())).is_empty());
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
                "Open in Wiktionary",
                "Translate \u{201c}display\u{201d}",
                "Add to Favourites"
            ]
        );

        // The first use also records the word; see the history tests.
        let effects = act(&mut session, &item.id, &item.actions[0].id);
        assert_eq!(effects.last(), Some(&Effect::Copy(item.title.clone())));
        assert_eq!(
            act(&mut session, &item.id, "translate"),
            [Effect::Translate("display".into())]
        );
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
        let list = list(&session.answer(generation, Ok(Vec::new())));
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
        assert_eq!(list.empty.unwrap().title, "Type a word to look up");

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
        session.answer(generation, Ok(Vec::new()));

        let effects = chose(&mut session, "lang-es");
        let spanish = languages::find("es").unwrap();
        assert!(effects.contains(&Effect::Remember(Some(spanish))));
        let requests = sends(&effects);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].word, "significado");
        assert_eq!(requests[0].language, Some(spanish));
        assert_eq!(requests[0].explained_in, "en");

        let list = list(&effects);
        let from = &list.scopes[0];
        assert_eq!(from.current().unwrap().title, "Spanish");
        assert_eq!(from.options.len(), LANGUAGES.len() + 1);
        assert_eq!(
            list.placeholder.as_deref(),
            Some("Look up a word in Spanish")
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

    fn saved(effects: &[Effect]) -> store::State {
        effects
            .iter()
            .rev()
            .find_map(|effect| match effect {
                Effect::Save(state) => Some(state.clone()),
                _ => None,
            })
            .expect("a save")
    }

    /// A lookup that answers is not history; using it is.
    #[test]
    fn a_word_goes_into_history_when_it_is_used() {
        let mut session = session();
        let list = found(&mut session, "display");
        assert!(session.saved.history.is_empty());

        let effects = act(&mut session, &list.item(0).unwrap().id, "copy");
        assert_eq!(saved(&effects).history[0].word, "display");
        assert!(matches!(effects.last(), Some(Effect::Copy(_))));

        // Used again, nothing changes and nothing is written.
        let effects = act(&mut session, &list.item(0).unwrap().id, "copy-word");
        assert!(!effects.iter().any(|e| matches!(e, Effect::Save(_))));
    }

    #[test]
    fn a_word_left_on_screen_counts_as_looked_at() {
        let mut session = session();
        found(&mut session, "display");
        let effects = session.event(Event::Dismissed);
        assert_eq!(saved(&effects).history[0].word, "display");
    }

    #[test]
    fn with_history_off_nothing_is_kept_and_what_was_is_dropped() {
        let mut kept = store::State::default();
        kept.record("old", "en");
        let mut session = Session::new(Some(languages::english()), "en", "Wiktionary", kept, false);
        assert!(session.saved.history.is_empty());
        found(&mut session, "display");
        assert!(session.closed().is_empty());
    }

    /// A favourite keeps the whole entry, and opens with no request.
    #[test]
    fn a_favourite_opens_from_what_was_saved() {
        let mut session = session();
        let shown = found(&mut session, "display");
        let item = shown.item(0).unwrap();
        assert!(item.actions.iter().any(|a| a.title == "Add to Favourites"));
        let effects = act(&mut session, &item.id, "favourite");
        let state = saved(&effects);
        assert_eq!(state.favourites[0].word, "display");
        assert!(!state.favourites[0].parts.is_empty());
        assert!(
            list_after(effects)
                .item(0)
                .unwrap()
                .actions
                .iter()
                .any(|a| a.title == "Remove from Favourites")
        );

        // A new view, from the file, offline.
        let mut session = Session::new(Some(languages::english()), "en", "Wiktionary", state, true);
        let empty = list(&[Effect::Render(session.view())]);
        assert_eq!(empty.sections[0].title.as_deref(), Some("Favourites"));
        let favourite = empty.item(0).unwrap();
        assert_eq!(favourite.title, "display");
        assert_eq!(favourite.subtitle.as_deref(), Some("A show or spectacle."));

        let effects = act(&mut session, &favourite.id, "show");
        assert_eq!(effects, [Effect::Type("display".into())]);
        let effects = typed(&mut session, "display");
        assert!(sends(&effects).is_empty());
        assert!(!effects.iter().any(|e| matches!(e, Effect::Wait { .. })));
        assert_eq!(
            list_after(effects).item(0).unwrap().title,
            "A show or spectacle."
        );
    }

    /// A recent word in another language switches to it, then is typed.
    #[test]
    fn a_recent_word_is_looked_up_in_its_language() {
        let mut kept = store::State::default();
        kept.record("mostrar", "es");
        kept.record("display", "en");
        let mut session = Session::new(Some(languages::english()), "en", "Wiktionary", kept, true);
        let list = list(&typed(&mut session, " "));
        let recent = &list.sections[0];
        assert_eq!(recent.title.as_deref(), Some("Recent"));
        assert_eq!(recent.items[1].title, "mostrar");

        let effects = act(&mut session, &recent.items[1].id, "show");
        let spanish = languages::find("es").unwrap();
        assert_eq!(
            effects,
            [
                Effect::Remember(Some(spanish)),
                Effect::Type("mostrar".into())
            ]
        );
        assert_eq!(sends(&typed(&mut session, "mostrar")).len(), 0);
        let requests = sends(&session.settle(session.generation));
        assert_eq!(requests[0].language, Some(spanish));
    }

    #[test]
    fn history_can_be_trimmed_and_cleared() {
        let mut kept = store::State::default();
        kept.record("a", "en");
        kept.record("b", "en");
        let mut session = Session::new(Some(languages::english()), "en", "Wiktionary", kept, true);

        let effects = act(&mut session, "recent-0", "forget");
        assert_eq!(saved(&effects).history.len(), 1);
        let effects = act(&mut session, "recent-0", "clear-history");
        assert!(saved(&effects).history.is_empty());
        assert!(list_after(effects).is_empty());
    }

    /// An entry with `senses` senses of one part of speech.
    fn entry_in(language: &str, senses: usize) -> Entry {
        Entry {
            word: "hola".into(),
            language: language.into(),
            explained_in: "en".into(),
            url: String::new(),
            parts: vec![Part {
                name: "interjection".into(),
                senses: (0..senses)
                    .map(|index| Sense {
                        definition: format!("{language} sense {index}"),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
        }
    }

    fn detecting(saved: store::State) -> Session {
        Session::new(None, "en", "Wiktionary", saved, true)
    }

    /// Detected, typed and answered.
    fn detected(session: &mut Session, entries: Vec<Entry>) -> List {
        typed(session, "hola");
        let generation = session.generation;
        let requests = sends(&session.settle(generation));
        assert_eq!(requests[0].language, None);
        list(&session.answer(generation, Ok(entries)))
    }

    #[test]
    fn detection_is_an_option_and_asks_every_language() {
        let session = detecting(store::State::default());
        let list = list(&[Effect::Render(session.view())]);
        let from = &list.scopes[0];
        assert_eq!(from.current().unwrap().title, "Detect language");
        assert_eq!(from.options[0].id, "detect");
        assert_eq!(
            list.placeholder.as_deref(),
            Some("Look up a word in any language")
        );
    }

    /// The language with the most senses wins, and says so.
    #[test]
    fn detection_shows_the_language_with_the_most_senses() {
        let mut session = detecting(store::State::default());
        let list = detected(
            &mut session,
            vec![entry_in("en", 1), entry_in("es", 3), entry_in("pt", 2)],
        );
        assert_eq!(
            list.sections[0].title.as_deref(),
            Some("Spanish \u{00b7} Interjection")
        );
        let item = list.item(0).unwrap();
        let detail = item.detail.as_ref().unwrap();
        let value = |label: &str| {
            detail
                .metadata
                .iter()
                .find(|row| row.label == label)
                .map(|row| row.value.clone())
        };
        assert_eq!(value("Language").as_deref(), Some("Spanish (detected)"));
        assert_eq!(value("Also in").as_deref(), Some("Portuguese, English"));
        let shows: Vec<&str> = item
            .actions
            .iter()
            .filter(|action| action.id.starts_with("in-"))
            .map(|action| action.title.as_str())
            .collect();
        assert_eq!(shows, ["Show in Portuguese", "Show in English"]);
    }

    /// A tie goes to a language the user uses, then to English.
    #[test]
    fn a_tie_goes_to_the_users_languages_then_english() {
        let tied = || vec![entry_in("ca", 1), entry_in("en", 1), entry_in("es", 1)];

        let mut session = detecting(store::State::default());
        let list = detected(&mut session, tied());
        assert!(
            list.sections[0]
                .title
                .as_deref()
                .unwrap()
                .starts_with("English")
        );

        let mut saved = store::State::default();
        saved.record("mostrar", "es");
        let mut session = detecting(saved);
        let list = detected(&mut session, tied());
        assert!(
            list.sections[0]
                .title
                .as_deref()
                .unwrap()
                .starts_with("Spanish")
        );
    }

    /// Another language is shown from what the view has, and the next
    /// opening still detects.
    #[test]
    fn showing_another_language_asks_nothing_and_is_not_remembered() {
        let mut session = detecting(store::State::default());
        detected(&mut session, vec![entry_in("es", 3), entry_in("pt", 2)]);

        let effects = act(&mut session, "sense-0-0", "in-pt");
        assert!(sends(&effects).is_empty());
        assert!(!effects.iter().any(|e| matches!(e, Effect::Remember(_))));
        let list = list(&effects);
        assert_eq!(list.item(0).unwrap().title, "pt sense 0");
        assert_eq!(list.scopes[0].current().unwrap().title, "Portuguese");
        // One language chosen: nothing else is offered.
        assert!(
            list.item(0)
                .unwrap()
                .actions
                .iter()
                .all(|a| !a.id.starts_with("in-"))
        );

        // Back to detection, from what the view already has.
        let effects = chose(&mut session, "detect");
        assert!(effects.contains(&Effect::Remember(None)));
        assert!(sends(&effects).is_empty());
        assert_eq!(list_after(effects).item(0).unwrap().title, "es sense 0");
    }

    #[test]
    fn a_word_in_no_language_says_so() {
        let mut session = detecting(store::State::default());
        let list = detected(&mut session, Vec::new());
        assert!(
            list.empty
                .unwrap()
                .description
                .unwrap()
                .contains("any of the 58 languages")
        );
    }

    /// A definition that opens with its labels does not repeat them.
    #[test]
    fn labels_are_said_once() {
        let mostrar = free::parse(include_str!("fixtures/mostrar.json"), "es")
            .unwrap()
            .unwrap();
        let session = Session::new(
            languages::find("es"),
            "en",
            "Wiktionary",
            store::State::default(),
            true,
        );
        let (part, sense) = (&mostrar.parts[0], &mostrar.parts[0].senses[0]);
        let markdown = session.detail(&mostrar, part, sense).markdown;
        assert!(markdown.contains("(transitive) to show"));
        assert!(!markdown.contains("*transitive*"));
    }
}
