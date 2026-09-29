//! The currency view's state, as a plain value.
//!
//! The daemon owns the thread, the client and the file; this owns every
//! decision. It is told what happened -- an event from the view, a table
//! that arrived or did not -- and replies with [`Effect`]s: what to draw,
//! whether to fetch, what to copy, what to keep. Nothing here blocks or
//! reads a clock: `now` is passed in, so every rule is a unit test.
//!
//! # A table, not a request per keystroke
//!
//! Typing converts against the table in hand. A fetch happens when the view
//! opens with no table, a table from another service, or one that is due --
//! and even then the table in hand is drawn at once, with its own date, and
//! replaced when the new one lands.

use super::{Currency, Failure, Rates, Service, State, currencies, format};
use crate::providers::calculator;
use crate::view::{
    Action, EmptyState, Event, Filtering, Icon, Item, List, Scope, ScopeOption, Section, Tone, View,
};

/// What the driver should do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Fetch the latest table on a thread, and report back with
    /// [`Session::fetched`].
    Fetch,
    /// Put this on the clipboard and close the launcher.
    Copy(String),
    /// Write this to `currency.json`.
    Save(State),
}

// Ids. Opaque to the host; only this file reads them back.
const ROW: &str = "cur-";
const COPY: &str = "copy";
const COPY_SHOWN: &str = "copy-shown";
const RETRY: &str = "retry";
const FAILED: &str = "failed";
/// The one scope, and the prefix of its options.
const FROM: &str = "from";

pub struct Session {
    service: Service,
    from: &'static Currency,
    rates: Option<Rates>,
    /// A fetch is out.
    fetching: bool,
    /// Why the last fetch failed, until one succeeds.
    failure: Option<Failure>,
    query: String,
}

impl Session {
    /// A session starting from what `currency.json` held.
    pub fn new(state: State, service: Service) -> Self {
        Self {
            service,
            from: state.from(),
            rates: state.rates,
            fetching: false,
            failure: None,
            query: String::new(),
        }
    }

    /// What to do on opening, at `now` (seconds since the epoch): draw, and
    /// fetch if the table in hand will not do.
    pub fn start(&mut self, now: i64) -> Vec<Effect> {
        let stale = match &self.rates {
            None => true,
            Some(rates) => rates.service != self.service || rates.is_due(now),
        };
        let mut effects = Vec::new();
        if stale {
            self.fetching = true;
            effects.push(Effect::Fetch);
        }
        effects.insert(0, Effect::Render(self.view()));
        effects
    }

    /// Something happened in the view.
    pub fn event(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::QueryChanged { query } => {
                if query == self.query {
                    return Vec::new();
                }
                self.query = query;
                vec![Effect::Render(self.view())]
            }
            Event::ScopeChanged { scope, option } => self.choose(&scope, &option),
            Event::ActionInvoked { item, action } => self.act(item.as_deref(), &action),
            Event::SelectionChanged { .. } | Event::Dismissed => Vec::new(),
        }
    }

    /// A fetch came back.
    pub fn fetched(&mut self, result: Result<Rates, Failure>) -> Vec<Effect> {
        self.fetching = false;
        match result {
            Ok(mut rates) => {
                rates.retain_known();
                self.rates = Some(rates);
                self.failure = None;
                vec![Effect::Render(self.view()), Effect::Save(self.state())]
            }
            Err(failure) => {
                self.failure = Some(failure);
                vec![Effect::Render(self.view())]
            }
        }
    }

    /// What `currency.json` should hold now.
    pub fn state(&self) -> State {
        State {
            from: Some(self.from.code.to_string()),
            rates: self.rates.clone(),
        }
    }

    fn choose(&mut self, scope: &str, option: &str) -> Vec<Effect> {
        if scope != FROM {
            return Vec::new();
        }
        let Some(currency) = option.strip_prefix(ROW).and_then(currencies::find) else {
            return Vec::new();
        };
        if currency == self.from {
            return Vec::new();
        }
        self.from = currency;
        vec![Effect::Render(self.view()), Effect::Save(self.state())]
    }

    fn act(&mut self, item: Option<&str>, action: &str) -> Vec<Effect> {
        if action == RETRY {
            if self.fetching {
                return Vec::new();
            }
            self.fetching = true;
            return vec![Effect::Render(self.view()), Effect::Fetch];
        }
        let Some(to) = item
            .and_then(|item| item.strip_prefix(ROW))
            .and_then(currencies::find)
        else {
            return Vec::new();
        };
        let Some(value) = self.convert(to) else {
            return Vec::new();
        };
        match action {
            COPY => vec![Effect::Copy(format::plain(value, to))],
            COPY_SHOWN => vec![Effect::Copy(format::shown(value, to))],
            _ => Vec::new(),
        }
    }

    fn amount(&self) -> Option<f64> {
        calculator::amount(&self.query)
    }

    fn convert(&self, to: &Currency) -> Option<f64> {
        self.rates
            .as_ref()?
            .convert(self.amount()?, self.from.code, to.code)
    }

    /// What the view should show now.
    pub fn view(&self) -> View {
        let typed = !self.query.trim().is_empty();
        let amount = self.amount();

        let (empty, sections) = match (&self.rates, amount) {
            (_, None) if typed => (
                EmptyState {
                    title: "Not an amount".into(),
                    description: Some("Type a number or arithmetic: 10, 1+2, (3*4)/2".into()),
                    icon: None,
                },
                Vec::new(),
            ),
            (_, None) => (
                EmptyState {
                    title: format!("Type an amount of {}", self.from.name),
                    description: Some(
                        "10, 1+2 or (3*4)/2, converted into every currency. Tab chooses the currency."
                            .into(),
                    ),
                    icon: None,
                },
                Vec::new(),
            ),
            (Some(rates), Some(amount)) => (self.waiting(), vec![self.section(rates, amount)]),
            (None, Some(_)) if self.fetching => (self.waiting(), Vec::new()),
            // No table, and none coming: a row, so Enter can try again.
            (None, Some(_)) => (self.waiting(), vec![self.failed()]),
        };

        View::List(List {
            placeholder: Some(format!("Convert from {}", self.from.name)),
            filtering: Filtering::Extension,
            scopes: vec![self.source_scope()],
            empty: Some(empty),
            sections,
        })
    }

    fn waiting(&self) -> EmptyState {
        EmptyState {
            title: "Fetching today's rates\u{2026}".into(),
            description: Some(format!("From {}", self.service.label())),
            icon: None,
        }
    }

    fn section(&self, rates: &Rates, amount: f64) -> Section {
        let mut subtitle = format!(
            "Rates of {} \u{00b7} {}",
            rates.date(),
            rates.service.label()
        );
        match (&self.failure, self.fetching) {
            (_, true) => subtitle.push_str(" \u{00b7} updating\u{2026}"),
            (Some(Failure::Offline), false) => subtitle.push_str(" \u{00b7} offline"),
            (Some(Failure::Other(_)), false) => subtitle.push_str(" \u{00b7} could not update"),
            (None, false) => {}
        }

        Section {
            title: Some(format!(
                "Exchange from {} {}",
                format::shown(amount, self.from),
                self.from.name
            )),
            subtitle: Some(subtitle),
            items: currencies::CURRENCIES
                .iter()
                .filter(|currency| *currency != self.from)
                .filter_map(|to| Some(row(to, rates.convert(amount, self.from.code, to.code)?)))
                .collect(),
        }
    }

    fn failed(&self) -> Section {
        let (title, subtitle) = match &self.failure {
            Some(Failure::Other(message)) => ("Could not fetch rates".to_string(), message.clone()),
            Some(Failure::Offline) | None => (
                "No internet connection".to_string(),
                format!(
                    "Currency needs the internet once a day to reach {}. Enter tries again.",
                    self.service.label()
                ),
            ),
        };
        let tone = match &self.failure {
            Some(Failure::Other(_)) => Tone::Danger,
            _ => Tone::Caution,
        };
        Section {
            title: Some(self.service.label().into()),
            subtitle: None,
            items: vec![Item {
                id: FAILED.into(),
                title,
                subtitle: Some(subtitle),
                icon: None,
                accessories: vec![crate::view::Accessory::Tag {
                    text: "Rates".into(),
                    tone,
                }],
                keywords: Vec::new(),
                actions: vec![action(RETRY, "Try Again")],
            }],
        }
    }

    /// *From*: every currency.
    fn source_scope(&self) -> Scope {
        Scope {
            id: FROM.into(),
            title: "From".into(),
            options: currencies::CURRENCIES
                .iter()
                .map(|currency| ScopeOption {
                    id: format!("{ROW}{}", currency.code),
                    title: format!("{} {}", currency.code, currency.name),
                    keywords: currency
                        .countries
                        .iter()
                        .map(|country| (*country).into())
                        .collect(),
                })
                .collect(),
            selected: format!("{ROW}{}", self.from.code),
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

/// One currency's row: `US  USD  $7.06          United States Dollar`.
fn row(to: &'static Currency, value: f64) -> Item {
    Item {
        id: format!("{ROW}{}", to.code),
        title: to.code.into(),
        subtitle: Some(format::shown(value, to)),
        icon: Some(Icon::Text {
            text: to.region().into(),
        }),
        accessories: vec![crate::view::Accessory::Text {
            text: to.name.into(),
        }],
        keywords: Vec::new(),
        actions: vec![
            action(COPY, &format!("Copy {}", format::plain(value, to))),
            action(COPY_SHOWN, &format!("Copy {}", format::shown(value, to))),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::currency::tests::rates;

    fn table() -> Rates {
        rates(&[("USD", 1.0), ("CAD", 1.4), ("EUR", 0.9), ("JPY", 150.0)])
    }

    /// Opened with a table that is not due.
    fn session() -> Session {
        let mut session = Session::new(
            State {
                from: None,
                rates: Some(table()),
            },
            Service::Open,
        );
        let now = table().next - 1;
        assert_eq!(
            session.start(now).len(),
            1,
            "a fresh table asks for nothing"
        );
        session
    }

    fn typed(session: &mut Session, text: &str) -> Vec<Effect> {
        session.event(Event::QueryChanged { query: text.into() })
    }

    fn list(effects: &[Effect]) -> List {
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Render(View::List(list)) => Some(list.clone()),
                _ => None,
            })
            .expect("a render")
    }

    fn row<'a>(list: &'a List, code: &str) -> &'a Item {
        list.items()
            .find(|item| item.title == code)
            .unwrap_or_else(|| panic!("no {code} row"))
    }

    #[test]
    fn an_expression_is_converted_into_every_other_currency() {
        let mut session = session();
        let list = list(&typed(&mut session, "1+2"));

        let section = &list.sections[0];
        assert_eq!(
            section.title.as_deref(),
            Some("Exchange from $3.00 United States Dollar")
        );
        assert_eq!(
            section.subtitle.as_deref(),
            Some("Rates of 28 Sep 2026 \u{00b7} ExchangeRate-API")
        );
        // The source is not a row of its own.
        let codes: Vec<&str> = list.items().map(|item| item.title.as_str()).collect();
        assert_eq!(codes, ["CAD", "EUR", "JPY"]);
        assert_eq!(row(&list, "CAD").subtitle.as_deref(), Some("CA$4.20"));
        assert_eq!(row(&list, "JPY").subtitle.as_deref(), Some("\u{a5}450"));
        assert_eq!(
            row(&list, "CAD").icon,
            Some(Icon::Text { text: "CA".into() })
        );
    }

    #[test]
    fn enter_copies_the_plain_number() {
        let mut session = session();
        typed(&mut session, "1000");
        let effects = session.event(Event::ActionInvoked {
            item: Some("cur-JPY".into()),
            action: "copy".into(),
        });
        assert_eq!(effects, vec![Effect::Copy("150000".into())]);

        let effects = session.event(Event::ActionInvoked {
            item: Some("cur-JPY".into()),
            action: "copy-shown".into(),
        });
        assert_eq!(effects, vec![Effect::Copy("\u{a5}150,000".into())]);
    }

    #[test]
    fn nothing_typed_or_not_an_amount_shows_no_rows() {
        let mut session = session();
        typed(&mut session, "5");
        let list = list(&typed(&mut session, ""));
        assert!(list.is_empty());
        assert!(list.empty.unwrap().title.contains("United States Dollar"));

        let list = list_of(typed(&mut session, "ten"));
        assert!(list.is_empty());
        assert_eq!(list.empty.unwrap().title, "Not an amount");
    }

    fn list_of(effects: Vec<Effect>) -> List {
        list(&effects)
    }

    #[test]
    fn choosing_the_source_converts_from_it_and_is_kept() {
        let mut session = session();
        typed(&mut session, "14");
        let effects = session.event(Event::ScopeChanged {
            scope: "from".into(),
            option: "cur-CAD".into(),
        });

        let list = list(&effects);
        assert_eq!(row(&list, "EUR").subtitle.as_deref(), Some("\u{20ac}9.00"));
        assert_eq!(row(&list, "USD").subtitle.as_deref(), Some("$10.00"));
        assert!(list.items().all(|item| item.title != "CAD"));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::Save(State { from: Some(code), .. }) if code == "CAD"
        )));
    }

    #[test]
    fn a_due_table_is_drawn_at_once_and_refreshed() {
        let mut session = Session::new(
            State {
                from: None,
                rates: Some(table()),
            },
            Service::Open,
        );
        let effects = session.start(table().next);
        assert_eq!(effects.last(), Some(&Effect::Fetch));

        let list = list_of(typed(&mut session, "10"));
        assert_eq!(
            list.len(),
            3,
            "the old table converts while the new one comes"
        );
        assert!(
            list.sections[0]
                .subtitle
                .as_deref()
                .unwrap()
                .ends_with("updating\u{2026}")
        );
    }

    /// A table from another service is replaced, not presented as the new
    /// service's.
    #[test]
    fn another_services_table_is_refetched() {
        let mut session = Session::new(
            State {
                from: None,
                rates: Some(table()),
            },
            Service::Unirate,
        );
        assert_eq!(session.start(0).last(), Some(&Effect::Fetch));
    }

    #[test]
    fn with_no_table_the_view_waits_then_converts_and_keeps_it() {
        let mut session = Session::new(State::default(), Service::Open);
        assert_eq!(session.start(0).last(), Some(&Effect::Fetch));

        let list = list_of(typed(&mut session, "10"));
        assert!(list.is_empty());
        assert!(list.empty.unwrap().title.starts_with("Fetching"));

        let effects = session.fetched(Ok(table()));
        assert_eq!(list_of(effects.clone()).len(), 3);
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Save(State { rates: Some(_), .. })))
        );
    }

    #[test]
    fn offline_with_no_table_is_a_row_that_retries() {
        let mut session = Session::new(State::default(), Service::Open);
        session.start(0);
        typed(&mut session, "10");
        let list = list_of(session.fetched(Err(Failure::Offline)));

        let item = list.item(0).unwrap();
        assert_eq!(item.title, "No internet connection");
        assert_eq!(item.actions[0].id, "retry");

        let effects = session.event(Event::ActionInvoked {
            item: Some(item.id.clone()),
            action: "retry".into(),
        });
        assert_eq!(effects.last(), Some(&Effect::Fetch));
        // Asking twice while one fetch is out asks once.
        assert!(
            session
                .event(Event::ActionInvoked {
                    item: None,
                    action: "retry".into(),
                })
                .is_empty()
        );
    }

    /// Offline with a table is not a failure: the rates still convert, and
    /// the date says how old they are.
    #[test]
    fn offline_with_a_table_still_converts() {
        let mut session = Session::new(
            State {
                from: None,
                rates: Some(table()),
            },
            Service::Open,
        );
        session.start(table().next);
        typed(&mut session, "10");
        let list = list_of(session.fetched(Err(Failure::Offline)));

        assert_eq!(list.len(), 3);
        assert!(
            list.sections[0]
                .subtitle
                .as_deref()
                .unwrap()
                .ends_with("offline")
        );
    }

    /// The picker is searched with the root list's matcher, over the code,
    /// the name and every country (F9, slice 2).
    #[test]
    fn the_source_picker_finds_a_currency_by_name_code_or_country() {
        use crate::frecency::Frecency;
        use crate::search::{Candidate, Search};

        let scope = session().source_scope();
        let candidates: Vec<Candidate> = scope
            .options
            .iter()
            .map(|option| {
                Candidate::new(crate::item::Item {
                    id: option.id.clone(),
                    title: option.title.clone(),
                    subtitle: None,
                    icon: None,
                    keywords: option.keywords.clone(),
                    actions: Vec::new(),
                    kind: crate::item::Kind::Command,
                })
            })
            .collect();
        let mut search = Search::new();
        let mut best = |query: &str| {
            search
                .rank(query, &candidates, &Frecency::new(), 1)
                .first()
                .map(|matched| matched.item.id.clone())
        };

        assert_eq!(best("cad").as_deref(), Some("cur-CAD"));
        assert_eq!(best("canadian").as_deref(), Some("cur-CAD"));
        assert_eq!(best("japan").as_deref(), Some("cur-JPY"));
        assert_eq!(best("spain").as_deref(), Some("cur-EUR"));
    }

    #[test]
    fn every_currency_is_a_source_option() {
        let list = list(&[Effect::Render(session().view())]);
        let scope = &list.scopes[0];
        assert_eq!(scope.options.len(), currencies::CURRENCIES.len());
        assert_eq!(scope.current().unwrap().title, "USD United States Dollar");
    }
}
