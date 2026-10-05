//! Result sources.
//!
//! F3: "Everything that makes the launcher useful before an extension system
//! exists. Each one behind a `trait Provider` in `core`."
//!
//! # The contract, and why it is this shape
//!
//! Both methods run **on the UI thread, on every keystroke**, inside the 8 ms
//! per-frame budget. That is a deliberate constraint, not an oversight: a
//! provider that can block is a provider that can make the root list stutter,
//! and I-3 forbids root search waiting on anything.
//!
//! So a provider may only do work that is genuinely instant -- parsing a short
//! string, reading an in-memory table. Anything that touches the disk, the
//! network or another process does that work **elsewhere**, on a background
//! task, and keeps a snapshot that these methods read. The application index
//! already works exactly that way: `enumerate_applications` runs off-thread and
//! the result is handed to the view when it is ready.

use crate::item::Item;

pub mod arrange;
pub mod calculator;
pub mod clipboard;
pub mod clock;
pub mod commands;
pub mod currency;
pub mod dictionary;
pub mod extensions;
pub mod quicklinks;
pub mod scripts;
pub mod snippets;
pub mod timers;
pub mod translate;
pub mod weather;
pub mod web;
pub mod window_commands;

pub use arrange::ArrangeProvider;
pub use calculator::CalculatorProvider;
pub use clipboard::ClipboardProvider;
pub use clock::ClockProvider;
pub use commands::CommandsProvider;
pub use currency::CurrencyProvider;
pub use dictionary::DictionaryProvider;
pub use extensions::ExtensionProvider;
pub use quicklinks::{Quicklink, QuicklinkProvider};
pub use scripts::{Script, ScriptMode, ScriptProvider, ScriptRun};
pub use snippets::SnippetsProvider;
pub use timers::{TimerPresetsProvider, TimersProvider};
pub use translate::TranslateProvider;
pub use weather::WeatherProvider;
pub use window_commands::{CustomWindowCommand, WindowCommandsProvider};

/// A source of results.
///
/// # Two kinds of contribution, and why the distinction matters
///
/// A provider contributes in one of two ways, and confusing them produces a
/// launcher that silently drops results.
///
/// A **catalogue** is a fixed set of things that exist whether or not anyone
/// asked: Lumen's own commands, and later the snippets and window layouts of
/// F3. These join the application index and are fuzzy-matched exactly like
/// applications, because the user finds them by typing part of their name.
///
/// An **answer** is a result computed *from* the query and is already the thing
/// the user wants: `12*7` produces `84`. Answers must not be matched, because
/// matching compares the query against the result's title and `84` does not
/// look anything like `12*7` -- which silently discards every calculation.
pub trait Provider: Send + Sync {
    /// Stable identifier, used to prefix item ids so two providers cannot
    /// collide in the frecency store.
    fn id(&self) -> &'static str;

    /// Items that exist independently of the query, matched like applications.
    ///
    /// Built once and reused, so keep the count small: a root list padded with
    /// commands the user did not ask for is a root list they scroll past.
    fn catalogue(&self) -> Vec<Item> {
        Vec::new()
    }

    /// Items computed from the query, shown without further matching.
    ///
    /// Returning nothing is the common case and must stay cheap. A provider
    /// that answers only some queries should reject the rest on the first
    /// character it looks at, not after building a result.
    fn answer(&self, _query: &str) -> Vec<Item> {
        Vec::new()
    }
}

/// Everything registered, consulted in order.
#[derive(Default)]
pub struct Providers {
    providers: Vec<Box<dyn Provider>>,
}

impl Providers {
    pub fn new() -> Self {
        Self::default()
    }

    /// The set Lumen ships with.
    pub fn builtin() -> Self {
        let mut providers = Self::new();
        providers.register(Box::new(CommandsProvider));
        providers.register(Box::new(CalculatorProvider));
        providers.register(Box::new(ArrangeProvider));
        providers.register(Box::new(TimersProvider));
        providers.register(Box::new(CurrencyProvider));
        providers.register(Box::new(DictionaryProvider));
        providers.register(Box::new(SnippetsProvider));
        providers.register(Box::new(ClipboardProvider));
        providers.register(Box::new(ClockProvider));
        providers.register(Box::new(WeatherProvider));
        providers
    }

    pub fn register(&mut self, provider: Box<dyn Provider>) {
        self.providers.push(provider);
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Everything the providers offer independently of any query.
    ///
    /// Built once, at startup, and folded into the searchable index.
    pub fn catalogue(&self) -> Vec<Item> {
        self.providers
            .iter()
            .flat_map(|provider| provider.catalogue())
            .collect()
    }

    /// Everything the providers compute *from* `query`.
    ///
    /// An empty query has no answers by definition: there is nothing to answer.
    /// That is what keeps a stray `0` out of the root list.
    pub fn answers(&self, query: &str) -> Vec<Item> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }

        self.providers
            .iter()
            .flat_map(|provider| provider.answer(query))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Kind;

    struct Noisy;

    impl Provider for Noisy {
        fn id(&self) -> &'static str {
            "noisy"
        }

        fn catalogue(&self) -> Vec<Item> {
            vec![Item {
                id: "noisy:root".into(),
                title: "Root".into(),
                subtitle: None,
                icon: None,
                keywords: Vec::new(),
                actions: Vec::new(),
                kind: Kind::Command,
            }]
        }

        fn answer(&self, query: &str) -> Vec<Item> {
            vec![Item {
                id: "noisy:query".into(),
                title: query.into(),
                subtitle: None,
                icon: None,
                keywords: Vec::new(),
                actions: Vec::new(),
                kind: Kind::Command,
            }]
        }
    }

    #[test]
    fn the_catalogue_is_offered_regardless_of_the_query() {
        let mut providers = Providers::new();
        providers.register(Box::new(Noisy));

        let items = providers.catalogue();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "noisy:root");
    }

    #[test]
    fn answers_are_computed_from_the_query() {
        let mut providers = Providers::new();
        providers.register(Box::new(Noisy));

        let items = providers.answers("hello");
        assert_eq!(items[0].id, "noisy:query");
        assert_eq!(items[0].title, "hello");
    }

    /// Whitespace is not a query. Without the trim, pressing space in an empty
    /// field would produce answers about nothing.
    #[test]
    fn whitespace_yields_no_answers() {
        let mut providers = Providers::new();
        providers.register(Box::new(Noisy));

        assert!(providers.answers("   ").is_empty());
        assert!(providers.answers("").is_empty());
    }

    #[test]
    fn the_builtin_catalogue_is_commands_window_layouts_timers_and_currency() {
        let items = Providers::builtin().catalogue();
        assert!(!items.is_empty());
        assert!(items.iter().all(|item| matches!(
            item.kind,
            Kind::Command
                | Kind::Window
                | Kind::Timer
                | Kind::Currency
                | Kind::Clipboard
                | Kind::Clock
                | Kind::Weather
                | Kind::Calculation
        )));
    }

    /// F8: a typed duration is an answer, and leads like a calculation.
    #[test]
    fn a_duration_is_answered_with_a_timer() {
        let answers = Providers::builtin().answers("25m");
        assert!(answers.iter().any(|item| item.kind == Kind::Timer));
    }

    /// The calculator must not contribute a result to the catalogue, or every
    /// launch starts with a stray arithmetic row. Its one entry is the
    /// command that opens its history (F14).
    #[test]
    fn the_calculator_contributes_no_result_to_the_catalogue() {
        let items = Providers::builtin().catalogue();
        let calculator: Vec<_> = items
            .iter()
            .filter(|item| item.kind == Kind::Calculation)
            .map(|item| item.primary())
            .collect();
        assert_eq!(
            calculator,
            [crate::item::Action::OpenView(
                crate::item::ViewSource::CalculatorHistory
            )]
        );
    }

    /// The bug this split exists to prevent: a calculation must survive being
    /// offered for a query that looks nothing like its own title.
    #[test]
    fn a_calculation_is_answered_for_a_query_it_does_not_resemble() {
        let answers = Providers::builtin().answers("12*7");
        assert!(answers.iter().any(|item| item.title == "84"));
    }
}
