//! The inline calculator.
//!
//! F3 lists this first, in order of value, and it is the cheapest thing in the
//! phase: a launcher is already a text field the user types into, so arithmetic
//! costs one parser and no new interface.
//!
//! This is the row; the engine, and the rules for when not to answer, are in
//! [`crate::calc`].

use crate::calc::{self, Value};
use crate::currency::{Rates, format};
use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;

/// Whether `item` is the calculator's answer -- a result, or money -- and
/// not the command that opens its history: the launcher draws an answer as
/// a card (F14).
pub fn is_answer(item: &Item) -> bool {
    item.id == RESULT || item.id == MONEY
}

const RESULT: &str = "calculator:result";
const MONEY: &str = "calculator:money";

/// How the table of exchange rates stands, for an answer in money.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    /// The table is due: a newer one is being asked for.
    pub due: bool,
    /// The last time one was asked for, it could not be fetched.
    pub failed: bool,
}

/// The root list's answer to a calculation with money in it (F14, slice
/// 12), against the table in hand.
///
/// The rates' date is always said when they were used, so a table a week
/// old is never passed off as today's. With no table, the row holds its
/// place and says why there is no number yet.
pub fn money(
    expression: &str,
    answer: &calc::Answer,
    rates: Option<&Rates>,
    standing: Standing,
) -> Item {
    let row = |title: String, subtitle: String, actions: Vec<Action>| Item {
        id: MONEY.into(),
        title,
        subtitle: Some(subtitle),
        icon: None,
        keywords: Vec::new(),
        actions,
        kind: Kind::Calculation,
    };
    let currency = vec![Action::OpenView(ViewSource::Currency)];
    if answer.pending {
        let title = if standing.failed {
            "Couldn\u{2019}t fetch exchange rates"
        } else {
            "Fetching exchange rates\u{2026}"
        };
        return row(title.into(), expression.into(), currency);
    }

    // One currency throughout used no rates, and says none.
    let subtitle = match rates.filter(|_| answer.converts) {
        Some(rates) => {
            let state = match (standing.due, standing.failed) {
                (true, true) => " \u{b7} couldn\u{2019}t update",
                (true, false) => " \u{b7} updating\u{2026}",
                _ => "",
            };
            format!("{expression} \u{b7} rates of {}{state}", rates.date())
        }
        None => expression.into(),
    };
    // What is pasted into a form: the number alone, as F9 copies it.
    let (title, copied) = match answer.value {
        Value::Money { amount, currency } => (
            format::with_code(amount, currency),
            format::plain(amount, currency),
        ),
        value => (calc::format_value(value), calc::copied_value(value)),
    };
    let mut actions = vec![Action::CopyToClipboard(copied)];
    actions.extend(currency);
    row(title, subtitle, actions)
}

pub struct CalculatorProvider;

impl Provider for CalculatorProvider {
    fn id(&self) -> &'static str {
        "calculator"
    }

    /// The one row that opens what was calculated before (F14).
    fn catalogue(&self) -> Vec<Item> {
        vec![Item {
            id: "calculator:history".into(),
            title: "Calculator History".into(),
            subtitle: Some("The answers you copied, to use again".into()),
            icon: None,
            keywords: vec![
                "calculator".into(),
                "calculation".into(),
                "history".into(),
                "math".into(),
            ],
            actions: vec![Action::OpenView(ViewSource::CalculatorHistory)],
            kind: Kind::Calculation,
        }]
    }

    // An answer, not a catalogue entry: `84` is the result of `12*7`, and
    // fuzzy-matching the query against it would discard every calculation.
    fn answer(&self, query: &str) -> Vec<Item> {
        let Some(value) = calc::evaluate(query) else {
            return Vec::new();
        };

        vec![Item {
            id: RESULT.into(),
            // Shown with the regional settings' grouping, copied without it,
            // so a number field the result is pasted into reads it (F14).
            title: calc::format_value(value),
            subtitle: Some(query.trim().to_string()),
            icon: None,
            keywords: Vec::new(),
            // Copying is the only sensible thing to do with a number, and it is
            // what the user reached for the calculator to get.
            actions: vec![Action::CopyToClipboard(calc::copied_value(value))],
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

    fn money_row(input: &str, rates: Option<&Rates>, due: bool, failed: bool) -> Item {
        let context = calc::Context {
            today: &calc::today,
            rates,
            separators: calc::separators(),
        };
        let answer = calc::answer(input, &context).unwrap();
        money(input, &answer, rates, Standing { due, failed })
    }

    /// The four ways the table can stand, each said in the row (F14).
    #[test]
    fn money_dates_its_rates_and_says_when_there_are_none() {
        let rates = crate::currency::tests::rates(&[("USD", 1.0), ("CAD", 1.41649)]);

        let fresh = money_row("10 usd in cad", Some(&rates), false, false);
        assert_eq!(fresh.title, "14.16 CAD");
        assert_eq!(
            fresh.subtitle.as_deref(),
            Some("10 usd in cad \u{b7} rates of 28 Sep 2026")
        );
        // What is pasted into a form: the number alone, as F9 copied it.
        assert_eq!(fresh.primary(), Action::CopyToClipboard("14.16".into()));

        let due = money_row("10 usd in cad", Some(&rates), true, false);
        assert_eq!(due.title, "14.16 CAD", "the old table, at once");
        assert!(
            due.subtitle
                .unwrap()
                .ends_with("28 Sep 2026 \u{b7} updating\u{2026}")
        );

        let stale = money_row("10 usd in cad", Some(&rates), true, true);
        assert!(stale.subtitle.unwrap().ends_with("couldn\u{2019}t update"));

        let fetching = money_row("10 usd in cad", None, false, false);
        assert_eq!(fetching.title, "Fetching exchange rates\u{2026}");
        assert_eq!(fetching.primary(), Action::OpenView(ViewSource::Currency));

        let failed = money_row("10 usd in cad", None, false, true);
        assert_eq!(failed.title, "Couldn\u{2019}t fetch exchange rates");

        // One currency throughout needs no table, and says none.
        let plain = money_row("25% off 399 usd", None, false, false);
        assert_eq!(plain.title, "299.25 USD");
        assert_eq!(plain.subtitle.as_deref(), Some("25% off 399 usd"));
    }

    #[test]
    fn money_is_the_root_lists_not_the_providers() {
        assert!(CalculatorProvider.answer("10 usd in cad").is_empty());
        assert!(CalculatorProvider.answer("25% off 399 usd").is_empty());
    }

    #[test]
    fn the_history_is_a_command() {
        let items = CalculatorProvider.catalogue();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].primary(),
            Action::OpenView(ViewSource::CalculatorHistory)
        );
    }

    #[test]
    fn the_provider_stays_quiet_for_ordinary_searches() {
        assert!(CalculatorProvider.answer("firefox").is_empty());
    }
}
