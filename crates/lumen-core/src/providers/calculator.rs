//! The inline calculator.
//!
//! F3 lists this first, in order of value, and it is the cheapest thing in the
//! phase: a launcher is already a text field the user types into, so arithmetic
//! costs one parser and no new interface.
//!
//! This is the row; the engine, and the rules for when not to answer, are in
//! [`crate::calc`].

use crate::calc;
use crate::item::{Action, Item, Kind};
use crate::providers::Provider;

pub struct CalculatorProvider;

impl Provider for CalculatorProvider {
    fn id(&self) -> &'static str {
        "calculator"
    }

    // An answer, not a catalogue entry: `84` is the result of `12*7`, and
    // fuzzy-matching the query against it would discard every calculation.
    fn answer(&self, query: &str) -> Vec<Item> {
        let Some(value) = calc::evaluate(query) else {
            return Vec::new();
        };

        let formatted = calc::format_number(value);

        vec![Item {
            id: "calculator:result".into(),
            title: formatted.clone(),
            subtitle: Some(query.trim().to_string()),
            icon: None,
            keywords: Vec::new(),
            // Copying is the only sensible thing to do with a number, and it is
            // what the user reached for the calculator to get.
            actions: vec![Action::CopyToClipboard(formatted)],
            kind: Kind::Calculation,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_provider_offers_a_copyable_result() {
        let items = CalculatorProvider.answer("2+2");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "4");
        assert_eq!(items[0].subtitle.as_deref(), Some("2+2"));
        assert_eq!(items[0].primary(), Action::CopyToClipboard("4".to_string()));
    }

    #[test]
    fn the_provider_stays_quiet_for_ordinary_searches() {
        assert!(CalculatorProvider.answer("firefox").is_empty());
    }
}
