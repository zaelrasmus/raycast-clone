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
//! or `7-zip` puts a wrong row above the right one. So [`evaluate`] is
//! deliberately reluctant: it rejects anything that does not look like
//! arithmetic before it tries to parse, and rejects the parse itself rather
//! than salvaging a prefix. `tests/calculator_corpus.rs` states who owns each
//! query the calculator must leave alone.
//!
//! A recursive-descent parser rather than a dependency: it has to reject far
//! more than it accepts, and owning the rejection rules is the entire point
//! (see F14 decisions).

mod format;
mod lexer;
mod parser;

pub use format::format_number;

/// Evaluate `input` as arithmetic, or decline.
///
/// Declining is the common case and is not an error.
pub fn evaluate(input: &str) -> Option<f64> {
    let trimmed = input.trim();
    if !looks_like_arithmetic(trimmed) {
        return None;
    }
    amount(trimmed)
}

/// Evaluate `input` as arithmetic, a bare number included.
///
/// [`evaluate`] without its gate. The gate keeps the root list quiet, and is
/// wrong where a number is what was asked for: in the currency view (F9),
/// `10` is an amount.
pub fn amount(input: &str) -> Option<f64> {
    let tokens = lexer::tokenize(input.trim())?;
    let value = parser::Parser::new(tokens).whole()?;
    value.is_finite().then_some(value)
}

/// A cheap gate applied before parsing.
///
/// Three conditions, each one earning its place by rejecting a real query the
/// user is likely to type:
///
/// - **A digit must appear.** Rejects `code`, `notepad`, every application name.
/// - **An operator must appear.** Rejects `7zip` and bare `2024`; a lone number
///   is not a calculation, and offering to copy it back is noise.
/// - **Only arithmetic characters may appear.** Rejects `7-zip`, which has both
///   a digit and an operator and is emphatically not arithmetic.
fn looks_like_arithmetic(input: &str) -> bool {
    if input.is_empty() {
        return false;
    }

    let mut has_digit = false;
    let mut has_operator = false;

    for character in input.chars() {
        match character {
            '0'..='9' | '.' => has_digit |= character.is_ascii_digit(),
            '+' | '-' | '*' | '/' | '%' | '^' => has_operator = true,
            '(' | ')' | ' ' | '\t' => {}
            // Anything else at all, and this is not arithmetic.
            _ => return false,
        }
    }

    has_digit && has_operator
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_the_four_operations() {
        assert_eq!(evaluate("2+2"), Some(4.0));
        assert_eq!(evaluate("10 - 3"), Some(7.0));
        assert_eq!(evaluate("6*7"), Some(42.0));
        assert_eq!(evaluate("9 / 2"), Some(4.5));
    }

    #[test]
    fn respects_precedence_and_parentheses() {
        assert_eq!(evaluate("2 + 3 * 4"), Some(14.0));
        assert_eq!(evaluate("(2 + 3) * 4"), Some(20.0));
        assert_eq!(evaluate("2 * (3 + 4) - 1"), Some(13.0));
    }

    #[test]
    fn handles_unary_minus_and_powers() {
        assert_eq!(evaluate("-5 + 8"), Some(3.0));
        assert_eq!(evaluate("2^10"), Some(1024.0));
        assert_eq!(evaluate("2^-1"), Some(0.5));
        // Right-associative: 2^(3^2), not (2^3)^2.
        assert_eq!(evaluate("2^3^2"), Some(512.0));
    }

    #[test]
    fn handles_the_modulo_operator() {
        assert_eq!(evaluate("10 % 3"), Some(1.0));
    }

    // The rejection rules matter more than the arithmetic. Each of these is a
    // query somebody will actually type into a launcher.

    #[test]
    fn declines_application_names() {
        assert_eq!(evaluate("code"), None);
        assert_eq!(evaluate("notepad"), None);
        assert_eq!(evaluate("visual studio"), None);
    }

    /// Has a digit and a minus sign, and is a program, not a subtraction.
    #[test]
    fn declines_seven_zip() {
        assert_eq!(evaluate("7-zip"), None);
    }

    #[test]
    fn declines_a_bare_number() {
        assert_eq!(evaluate("2024"), None);
        assert_eq!(evaluate("3.5"), None);
    }

    #[test]
    fn declines_incomplete_expressions() {
        assert_eq!(evaluate("2 +"), None);
        assert_eq!(evaluate("* 4"), None);
        assert_eq!(evaluate("(2 + 3"), None);
        assert_eq!(evaluate("2 + 3)"), None);
    }

    /// Accepting a prefix is how `3 + 4 apples` becomes `7`.
    #[test]
    fn declines_trailing_junk() {
        assert_eq!(evaluate("3 + 4 apples"), None);
    }

    #[test]
    fn declines_malformed_numbers() {
        assert_eq!(evaluate("1.2.3 + 1"), None);
    }

    #[test]
    fn declines_empty_and_whitespace() {
        assert_eq!(evaluate(""), None);
        assert_eq!(evaluate("   "), None);
    }

    /// Division by zero is infinity, which is not a result worth showing.
    #[test]
    fn declines_non_finite_results() {
        assert_eq!(evaluate("1/0"), None);
        assert_eq!(evaluate("0/0"), None);
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
