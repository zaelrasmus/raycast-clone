//! The calculator's engine (F3, grown in F14).
//!
//! What the root list's calculator answers, the *Currency* view's amount and
//! the snippets' `{calculator}` all come from here. The provider in
//! `providers::calculator` only turns a value into a row.
//!
//! # The hard part is not the arithmetic
//!
//! It is deciding when *not* to answer. The launcher's search field is mostly
//! used for application names, and a calculator that offers a result for `code`
//! or `7-zip` puts a wrong row above the right one. F3 refused every letter;
//! F14 needs `sqrt`, `pi` and `deg`, so the gate is three rules instead:
//!
//! - **A digit or a parenthesis must appear.** A scan, before anything is
//!   allocated: most of what is typed is an application's name, and this
//!   is what it costs to turn one away.
//! - **Every word must be known** -- a function, a constant or a unit. One
//!   that is not declines the whole query: `7-zip`, `python 3.12`, `obs 28`.
//!   A parse is never salvaged from a prefix.
//! - **Something must be calculated.** `2024`, `45 deg` and `pi` restate a
//!   value; offering to copy it back is noise.
//!
//! `tests/calculator_corpus.rs` states who owns each query the calculator
//! must leave alone.
//!
//! A recursive-descent parser rather than a dependency: it has to reject far
//! more than it accepts, and owning the rejection rules is the entire point
//! (see F14 decisions).

mod format;
mod functions;
mod lexer;
mod parser;
mod units;
mod value;

pub use format::{format_number, format_value};
pub use value::Value;

/// The longest input read, in bytes. Past a line, it is not being typed.
const MAX_INPUT: usize = 256;

/// Evaluate `input` as a calculation, or decline.
///
/// Declining is the common case and is not an error.
pub fn evaluate(input: &str) -> Option<Value> {
    let trimmed = input.trim();
    if !trimmed.contains(|c: char| c.is_ascii_digit() || c == '(') {
        return None;
    }
    let parsed = parse(trimmed)?;
    (parsed.operations > 0).then_some(parsed.value)
}

/// Evaluate `input` as arithmetic, a bare number included.
///
/// [`evaluate`] without its gate. The gate keeps the root list quiet, and is
/// wrong where a number is what was asked for: in the currency view (F9),
/// `10` is an amount. A plain number, or a percentage as the fraction it
/// stands for -- `50%` is `0.5` -- but not a quantity: `10 deg` is not an
/// amount of money.
pub fn amount(input: &str) -> Option<f64> {
    match parse(input.trim())?.value {
        Value::Number(number) => Some(number),
        Value::Percent(percent) => Some(percent / 100.0),
        Value::Quantity { .. } => None,
    }
}

fn parse(input: &str) -> Option<parser::Parsed> {
    if input.len() > MAX_INPUT {
        return None;
    }
    let parsed = parser::Parser::new(lexer::tokenize(input)?).whole()?;
    parsed.value.is_finite().then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the root list answers for `input`, when it is a plain number.
    fn number(input: &str) -> Option<f64> {
        match evaluate(input)? {
            Value::Number(number) => Some(number),
            Value::Quantity { .. } | Value::Percent(_) => None,
        }
    }

    /// What the root list shows for `input`.
    fn shown(input: &str) -> Option<String> {
        evaluate(input).map(format_value)
    }

    /// The screenshot F14 was designed from, to the digit Raycast shows.
    #[test]
    fn the_screenshot() {
        assert_eq!(
            shown("sqrt(2)^pi / log(1000) * sin(45 deg)").as_deref(),
            Some("0.700197505")
        );
    }

    #[test]
    fn functions_take_parentheses() {
        assert_eq!(number("sqrt(16)"), Some(4.0));
        assert_eq!(number("SQRT(16) + 1"), Some(5.0));
        assert_eq!(number("cbrt(27)"), Some(3.0));
        assert_eq!(number("abs(-3)"), Some(3.0));
        assert_eq!(number("round(2.5) + floor(1.9) + ceil(1.1)"), Some(6.0));
        assert_eq!(number("log(1000)"), Some(3.0));
        assert_eq!(number("log2(8)"), Some(3.0));
        assert_eq!(number("ln(e)"), Some(1.0));
        assert_eq!(number("exp(0)"), Some(1.0));
        assert_eq!(number("sqrt 16"), None, "a function needs parentheses");
        assert_eq!(number("sqrt(-1)"), None, "not a real number");
        assert_eq!(number("ln(0)"), None);
    }

    #[test]
    fn trigonometry_in_radians_or_an_angle() {
        assert_eq!(number("sin(90 deg)"), Some(1.0));
        assert_eq!(number("sin(90\u{b0})"), Some(1.0));
        assert_eq!(number("cos(0)"), Some(1.0));
        assert_eq!(shown("sin(180 degrees)").as_deref(), Some("0"));
        assert_eq!(shown("cos(pi/2)").as_deref(), Some("0"));
        assert_eq!(shown("tan(45 deg)").as_deref(), Some("1"));
        assert_eq!(
            shown("sin(pi rad)").as_deref(),
            None,
            "pi is not a number before a unit"
        );
        assert_eq!(shown("asin(1) * 2").as_deref(), Some("3.1415926536"));
    }

    #[test]
    fn constants_need_something_done_with_them() {
        assert_eq!(number("pi * 2"), Some(std::f64::consts::TAU));
        assert_eq!(number("2 * \u{3c0}"), Some(std::f64::consts::TAU));
        assert_eq!(number("tau / 2"), Some(std::f64::consts::PI));
        assert_eq!(number("pi"), None, "a constant restated");
        assert_eq!(number("2pi"), None, "no multiplication without a sign");
    }

    #[test]
    fn factorials() {
        assert_eq!(number("5!"), Some(120.0));
        assert_eq!(number("3!!"), Some(720.0));
        assert_eq!(number("2^3!"), Some(64.0));
        assert_eq!(number("171!"), None);
        assert_eq!(number("2.5!"), None);
    }

    #[test]
    fn scientific_notation() {
        assert_eq!(number("1e3 * 2"), Some(2000.0));
        assert_eq!(number("1.5E-3 * 2"), Some(0.003));
        assert_eq!(number("1e3"), None, "a lone number");
        assert_eq!(shown("2^100").as_deref(), Some("1.2676506e30"));
    }

    #[test]
    fn pasted_signs() {
        assert_eq!(number("6 \u{d7} 7"), Some(42.0));
        assert_eq!(number("9 \u{f7} 2"), Some(4.5));
        assert_eq!(number("5 \u{2212} 8"), Some(-3.0));
    }

    #[test]
    fn an_angle_is_written_with_its_unit() {
        assert_eq!(shown("45 deg * 2").as_deref(), Some("90\u{b0}"));
        assert_eq!(shown("1 rad + 1").as_deref(), Some("2 rad"));
        assert_eq!(shown("45 deg"), None, "a value restated");
        assert_eq!(shown("2 deg * 3 deg"), None);
    }

    /// The worst that can be typed without meaning harm costs microseconds
    /// and answers nothing, rather than a stack or a stall.
    #[test]
    fn hostile_input_is_declined_cheaply() {
        let deep = format!("{}1{}", "(".repeat(100), ")".repeat(100));
        assert_eq!(number(&format!("{deep}+1")), None);
        assert_eq!(number(&format!("{}1", "-".repeat(100))), None);
        let shallow = format!("{}1{}", "(".repeat(30), ")".repeat(30));
        assert_eq!(number(&format!("{shallow}+1")), Some(2.0));
        assert_eq!(number("2^2^2^2^2^2^2"), None, "not finite");
        assert_eq!(number(&vec!["1"; 200].join("+")), None, "too long");
    }

    #[test]
    fn evaluates_the_four_operations() {
        assert_eq!(number("2+2"), Some(4.0));
        assert_eq!(number("10 - 3"), Some(7.0));
        assert_eq!(number("6*7"), Some(42.0));
        assert_eq!(number("9 / 2"), Some(4.5));
    }

    #[test]
    fn respects_precedence_and_parentheses() {
        assert_eq!(number("2 + 3 * 4"), Some(14.0));
        assert_eq!(number("(2 + 3) * 4"), Some(20.0));
        assert_eq!(number("2 * (3 + 4) - 1"), Some(13.0));
    }

    #[test]
    fn handles_unary_minus_and_powers() {
        assert_eq!(number("-5 + 8"), Some(3.0));
        assert_eq!(number("2^10"), Some(1024.0));
        assert_eq!(number("2^-1"), Some(0.5));
        // Right-associative: 2^(3^2), not (2^3)^2.
        assert_eq!(number("2^3^2"), Some(512.0));
    }

    /// `%` was the remainder until F14 (7.39); the remainder is `mod` now.
    #[test]
    fn handles_the_modulo_operator() {
        assert_eq!(number("10 mod 3"), Some(1.0));
        assert_eq!(number("10 MOD 3 + 1"), Some(2.0));
        assert_eq!(number("10 % 3"), None, "a percentage, then junk");
        assert_eq!(number("mod 3"), None);
    }

    #[test]
    fn percentages_of_off_and_on() {
        assert_eq!(number("20% of 50"), Some(10.0));
        assert_eq!(number("25% off 399"), Some(299.25));
        assert_eq!(number("5% on 30"), Some(31.5));
        assert_eq!(number("20% OF 50 + 1"), Some(11.0));
        assert_eq!(number("10% of 20% of 1000"), Some(20.0));
    }

    #[test]
    fn a_percentage_added_is_a_share_of_the_left() {
        assert_eq!(number("399 + 25%"), Some(498.75));
        assert_eq!(number("399 - 25%"), Some(299.25));
        assert_eq!(number("200 * 15%"), Some(30.0));
        assert_eq!(number("30 / 50%"), Some(60.0));
        assert_eq!(shown("10% + 5%").as_deref(), Some("15%"));
        assert_eq!(number("25% + 399"), None, "which one is the share?");
    }

    #[test]
    fn what_share_one_is_of_another() {
        assert_eq!(shown("5 as % of 20").as_deref(), Some("25%"));
        assert_eq!(shown("1/8 as %").as_deref(), Some("12.5%"));
        assert_eq!(shown("90 deg as % of 360 deg").as_deref(), Some("25%"));
        assert_eq!(shown("5 as"), None);
        assert_eq!(shown("5 as % of"), None);
    }

    #[test]
    fn a_percentage_alone_is_a_value_not_a_calculation() {
        assert_eq!(shown("50%"), None);
        assert_eq!(amount("50%"), Some(0.5));
        assert_eq!(amount("10%3"), None);
        assert_eq!(shown("sqrt(25%)"), None);
        assert_eq!(shown("45 deg %"), None);
    }

    // The rejection rules matter more than the arithmetic. Each of these is a
    // query somebody will actually type into a launcher.

    #[test]
    fn declines_application_names() {
        assert_eq!(number("code"), None);
        assert_eq!(number("notepad"), None);
        assert_eq!(number("visual studio"), None);
    }

    /// Has a digit and a minus sign, and is a program, not a subtraction.
    #[test]
    fn declines_seven_zip() {
        assert_eq!(number("7-zip"), None);
    }

    #[test]
    fn declines_a_bare_number() {
        assert_eq!(number("2024"), None);
        assert_eq!(number("3.5"), None);
    }

    #[test]
    fn declines_incomplete_expressions() {
        assert_eq!(number("2 +"), None);
        assert_eq!(number("* 4"), None);
        assert_eq!(number("(2 + 3"), None);
        assert_eq!(number("2 + 3)"), None);
    }

    /// Accepting a prefix is how `3 + 4 apples` becomes `7`.
    #[test]
    fn declines_trailing_junk() {
        assert_eq!(number("3 + 4 apples"), None);
    }

    #[test]
    fn declines_malformed_numbers() {
        assert_eq!(number("1.2.3 + 1"), None);
    }

    #[test]
    fn declines_empty_and_whitespace() {
        assert_eq!(number(""), None);
        assert_eq!(number("   "), None);
    }

    /// Division by zero is infinity, which is not a result worth showing.
    #[test]
    fn declines_non_finite_results() {
        assert_eq!(number("1/0"), None);
        assert_eq!(number("0/0"), None);
    }

    #[test]
    fn an_amount_may_be_a_bare_number_but_not_junk() {
        assert_eq!(amount("10"), Some(10.0));
        assert_eq!(amount(" 1+2 "), Some(3.0));
        assert_eq!(amount("10 usd"), None);
        assert_eq!(amount(""), None);
        assert_eq!(amount("1/0"), None);
    }
}
