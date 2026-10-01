//! What is typed in the currency view: `<amount> [<code>] [in|to <filter>]`.
//!
//! - The amount is the calculator's arithmetic, a bare number included:
//!   `10`, `1+2`, `(3*4)/2`.
//! - A code after it is this query's source, over the one picked in *From*:
//!   `10 cad`. Written apart or not: `10cad` is the same.
//! - `in` or `to` starts a filter on the rows, matched as root search matches
//!   (`crate::search`), against each currency's code, name and countries.
//!
//! Lenient while typing, because the view redraws on every key: an
//! unfinished word after the amount -- the `i` of `in`, the `ca` of `cad` --
//! is left out rather than turning the list into "Not an amount", and `10 in`
//! with nothing after it filters nothing.

use super::{Currency, currencies};
use crate::calc;

/// A query taken apart.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub amount: f64,
    /// A code written after the amount.
    pub from: Option<&'static Currency>,
    /// What follows `in` or `to`, trimmed. Empty when nothing does yet.
    pub filter: String,
}

/// Take `text` apart, or `None` when there is no amount in it.
pub fn parse(text: &str) -> Option<Query> {
    let (left, filter) = split_filter(text);
    let (amount, from) = amount_and_code(left)?;
    Some(Query {
        amount,
        from,
        filter: filter.trim().to_string(),
    })
}

/// A conversion asked in the root list: `10 usd in cad`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conversion {
    pub amount: f64,
    pub from: &'static Currency,
    pub to: &'static Currency,
}

/// Recognise a conversion in root search's query, or decline.
///
/// Strict where [`parse`] is lenient, because the root list is for
/// everything and a guess there is noise: an amount, a code, `in` or `to`,
/// and a target that names exactly one currency -- `cad`, `canadian
/// dollar`, `canada`. Declines on the first thing missing, so the common
/// case -- a query that is not money -- costs a scan for a space.
pub fn recognise(text: &str) -> Option<Conversion> {
    let (left, target) = split_filter(text);
    if target.trim().is_empty() {
        return None;
    }
    let (rest, code) = last_word(left.trim())?;
    let from = currencies::find(code)?;
    let to = currencies::exactly(target)?;
    if to == from {
        return None;
    }
    Some(Conversion {
        amount: calc::amount(rest)?,
        from,
        to,
    })
}

/// Split at the first `in` or `to` that is a word of its own and not the
/// first: `10 usd in cad` is `10 usd` and `cad`.
fn split_filter(text: &str) -> (&str, &str) {
    let mut words = word_spans(text);
    words.next();
    for (start, end) in words {
        let word = &text[start..end];
        if word.eq_ignore_ascii_case("in") || word.eq_ignore_ascii_case("to") {
            return (&text[..start], &text[end..]);
        }
    }
    (text, "")
}

/// The byte ranges of the whitespace-separated words of `text`.
fn word_spans(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    text.split_whitespace().map(move |word| {
        let start = word.as_ptr() as usize - text.as_ptr() as usize;
        (start, start + word.len())
    })
}

/// The amount, and the code written after it if there is one.
fn amount_and_code(text: &str) -> Option<(f64, Option<&'static Currency>)> {
    let text = text.trim();
    if let Some(amount) = calc::amount(text) {
        return Some((amount, None));
    }

    let (rest, word) = last_word(text)?;
    let written = code(word);
    if let Some(amount) = calc::amount(rest) {
        // A word that is not a code is one still being typed: left out.
        return Some((amount, written));
    }
    // Or a code, and then a word still being typed: `10 usd i`.
    if written.is_some() {
        return None;
    }
    let (rest, word) = last_word(rest)?;
    let written = code(word)?;
    Some((calc::amount(rest)?, Some(written)))
}

/// The letters at the end of `text`, written apart from what is before them
/// or not, and what is before them.
fn last_word(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_end();
    let at = text
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_alphabetic())
        .last()
        .map(|(at, _)| at)?;
    Some(text.split_at(at))
}

fn code(word: &str) -> Option<&'static Currency> {
    currencies::find(&word.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> (f64, Option<&'static str>, String) {
        let query = parse(text).unwrap_or_else(|| panic!("{text:?} did not parse"));
        (query.amount, query.from.map(|from| from.code), query.filter)
    }

    #[test]
    fn an_amount_alone() {
        assert_eq!(parsed("10"), (10.0, None, String::new()));
        assert_eq!(parsed(" (3*4)/2 "), (6.0, None, String::new()));
    }

    #[test]
    fn a_code_after_the_amount_is_the_source() {
        assert_eq!(parsed("10 cad"), (10.0, Some("CAD"), String::new()));
        assert_eq!(parsed("10CAD"), (10.0, Some("CAD"), String::new()));
        assert_eq!(parsed("1 + 2 eur"), (3.0, Some("EUR"), String::new()));
    }

    #[test]
    fn in_or_to_starts_a_filter() {
        assert_eq!(parsed("10 usd in cad"), (10.0, Some("USD"), "cad".into()));
        assert_eq!(parsed("10 to japan"), (10.0, None, "japan".into()));
        assert_eq!(
            parsed("10 IN united states"),
            (10.0, None, "united states".into())
        );
    }

    /// Each key on the way to `10 usd in cad` still converts.
    #[test]
    fn an_unfinished_word_is_left_out() {
        assert_eq!(parsed("10 u"), (10.0, None, String::new()));
        assert_eq!(parsed("10 us"), (10.0, None, String::new()));
        assert_eq!(parsed("10 usd i"), (10.0, Some("USD"), String::new()));
        assert_eq!(parsed("10 usd in"), (10.0, Some("USD"), String::new()));
        assert_eq!(parsed("10 usd in c"), (10.0, Some("USD"), "c".into()));
    }

    fn recognised(text: &str) -> Option<(f64, &'static str, &'static str)> {
        recognise(text)
            .map(|conversion| (conversion.amount, conversion.from.code, conversion.to.code))
    }

    #[test]
    fn the_root_list_recognises_a_whole_conversion() {
        assert_eq!(recognised("10 usd in cad"), Some((10.0, "USD", "CAD")));
        assert_eq!(recognised("10usd to japan"), Some((10.0, "USD", "JPY")));
        assert_eq!(
            recognised("2*5 EUR in canadian dollar"),
            Some((10.0, "EUR", "CAD"))
        );
    }

    /// Anything less is not money, and the root list stays quiet.
    #[test]
    fn the_root_list_declines_anything_less() {
        for text in [
            "10 usd",
            "10 usd in",
            "10 usd in ca",
            "10 in cad",
            "usd in cad",
            "10 usd in usd",
            "10 usd in zimbabwe",
            "log in to github",
        ] {
            assert_eq!(recognise(text), None, "{text}");
        }
    }

    #[test]
    fn no_amount_is_no_query() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("usd"), None);
        assert_eq!(parse("in cad"), None);
        assert_eq!(parse("ten usd"), None);
    }
}
