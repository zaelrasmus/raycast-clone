//! The inline calculator.
//!
//! F3 lists this first, in order of value, and it is the cheapest thing in the
//! phase: a launcher is already a text field the user types into, so arithmetic
//! costs one parser and no new interface.
//!
//! # The hard part is not the arithmetic
//!
//! It is deciding when *not* to answer. The launcher's search field is mostly
//! used for application names, and a calculator that offers a result for `code`
//! or `7-zip` puts a wrong row above the right one. So [`evaluate`] is
//! deliberately reluctant: it rejects anything that does not look like
//! arithmetic before it tries to parse, and rejects the parse itself rather
//! than salvaging a prefix.
//!
//! A recursive-descent parser rather than a dependency. The grammar is six
//! productions, it has to reject far more than it accepts, and owning the
//! rejection rules is the entire point.

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
        let Some(value) = evaluate(query) else {
            return Vec::new();
        };

        let formatted = format_number(value);

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
    let trimmed = input.trim();
    let tokens = tokenize(trimmed)?;
    let mut parser = Parser { tokens, at: 0 };
    let value = parser.expression()?;

    // Trailing junk means this was not an expression after all. Accepting a
    // prefix is how `3 apples` becomes `3`.
    if parser.at != parser.tokens.len() {
        return None;
    }

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

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Number(f64),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Open,
    Close,
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let characters: Vec<char> = input.chars().collect();
    let mut at = 0;

    while at < characters.len() {
        let character = characters[at];
        match character {
            ' ' | '\t' => at += 1,
            '+' => {
                tokens.push(Token::Plus);
                at += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                at += 1;
            }
            '*' => {
                tokens.push(Token::Star);
                at += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
                at += 1;
            }
            '%' => {
                tokens.push(Token::Percent);
                at += 1;
            }
            '^' => {
                tokens.push(Token::Caret);
                at += 1;
            }
            '(' => {
                tokens.push(Token::Open);
                at += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                at += 1;
            }
            '0'..='9' | '.' => {
                let start = at;
                let mut seen_point = false;
                while at < characters.len() {
                    match characters[at] {
                        '0'..='9' => at += 1,
                        // A second point means this is not a number. `1.2.3`
                        // must be rejected, not read as `1.2`.
                        '.' if !seen_point => {
                            seen_point = true;
                            at += 1;
                        }
                        _ => break,
                    }
                }
                let text: String = characters[start..at].iter().collect();
                tokens.push(Token::Number(text.parse().ok()?));
            }
            _ => return None,
        }
    }

    (!tokens.is_empty()).then_some(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.at).copied()
    }

    fn eat(&mut self, token: Token) -> bool {
        if self.peek() == Some(token) {
            self.at += 1;
            return true;
        }
        false
    }

    /// `expression := term (('+' | '-') term)*`
    fn expression(&mut self) -> Option<f64> {
        let mut left = self.term()?;
        loop {
            if self.eat(Token::Plus) {
                left += self.term()?;
            } else if self.eat(Token::Minus) {
                left -= self.term()?;
            } else {
                return Some(left);
            }
        }
    }

    /// `term := unary (('*' | '/' | '%') unary)*`
    fn term(&mut self) -> Option<f64> {
        let mut left = self.unary()?;
        loop {
            if self.eat(Token::Star) {
                left *= self.unary()?;
            } else if self.eat(Token::Slash) {
                let right = self.unary()?;
                // Division by zero yields infinity in IEEE arithmetic, and
                // `evaluate` rejects non-finite results -- so the launcher shows
                // nothing rather than a row saying `inf`.
                left /= right;
            } else if self.eat(Token::Percent) {
                let right = self.unary()?;
                left %= right;
            } else {
                return Some(left);
            }
        }
    }

    /// `unary := ('-' | '+')* power`
    fn unary(&mut self) -> Option<f64> {
        if self.eat(Token::Minus) {
            return Some(-self.unary()?);
        }
        if self.eat(Token::Plus) {
            return self.unary();
        }
        self.power()
    }

    /// `power := primary ('^' unary)?`
    ///
    /// Right-associative, and the exponent is a `unary` so that `2^-1` parses.
    fn power(&mut self) -> Option<f64> {
        let base = self.primary()?;
        if self.eat(Token::Caret) {
            let exponent = self.unary()?;
            return Some(base.powf(exponent));
        }
        Some(base)
    }

    /// `primary := NUMBER | '(' expression ')'`
    fn primary(&mut self) -> Option<f64> {
        match self.peek()? {
            Token::Number(value) => {
                self.at += 1;
                Some(value)
            }
            Token::Open => {
                self.at += 1;
                let value = self.expression()?;
                self.eat(Token::Close).then_some(value)
            }
            _ => None,
        }
    }
}

/// Render a result the way a person would write it.
///
/// Floating point prints badly by default: `0.1 + 0.2` is `0.30000000000000004`
/// and `6 / 2` is `3`. Rounding to ten significant decimals kills the first
/// without visibly harming anything the user typed, and trimming the trailing
/// zeros keeps whole numbers looking whole.
pub fn format_number(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }

    let mut text = format!("{value:.10}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
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

    #[test]
    fn formats_whole_numbers_without_a_decimal_point() {
        assert_eq!(format_number(4.0), "4");
        assert_eq!(format_number(-7.0), "-7");
        assert_eq!(format_number(1024.0), "1024");
    }

    #[test]
    fn formats_fractions_without_floating_point_noise() {
        assert_eq!(format_number(4.5), "4.5");
        // The canonical embarrassment.
        assert_eq!(format_number(0.1 + 0.2), "0.3");
    }

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
