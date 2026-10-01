//! Text into tokens.
//!
//! Rejects rather than skips: a character this does not know means the input
//! is not a calculation, and the whole of it is declined. Words are sliced
//! from the input, not copied: this runs on every keystroke.
//!
//! Numbers are read as the user's regional settings write them (F14):
//! `2,000.5` in the United States, `2.000,5` in Spain. A grouping separator
//! is only one between groups of three digits, so `1,5` is not one and a
//! half to an American -- it is declined, not guessed.

use lumen_platform::NumberSeparators;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Token<'a> {
    Number(f64),
    /// A run of letters: a function, a constant or a unit. What it is, if
    /// anything, is the parser's to decide.
    Word(&'a str),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Bang,
    Open,
    Close,
}

pub fn tokenize(input: &str, separators: NumberSeparators) -> Option<Vec<Token<'_>>> {
    let mut tokens = Vec::new();
    let mut at = 0;

    while let Some(character) = input[at..].chars().next() {
        let rest = &input[at..];
        let symbol = match character {
            '+' => Some(Token::Plus),
            // What a pasted minus, times or divide sign looks like.
            '-' | '\u{2212}' => Some(Token::Minus),
            '*' | '\u{d7}' => Some(Token::Star),
            '/' | '\u{f7}' => Some(Token::Slash),
            '%' => Some(Token::Percent),
            '^' => Some(Token::Caret),
            '!' => Some(Token::Bang),
            '(' => Some(Token::Open),
            ')' => Some(Token::Close),
            _ => None,
        };
        if let Some(token) = symbol {
            tokens.push(token);
            at += character.len_utf8();
        } else if character == ' ' || character == '\t' {
            at += 1;
        } else if character.is_ascii_digit() || character == separators.decimal {
            let (number, length) = number(rest, separators)?;
            tokens.push(Token::Number(number));
            at += length;
        } else if starts_word(character) {
            let length = rest
                .char_indices()
                .find(|&(_, next)| !continues_word(next))
                .map_or(rest.len(), |(end, _)| end);
            tokens.push(Token::Word(&rest[..length]));
            at += length;
        } else {
            return None;
        }
    }

    (!tokens.is_empty()).then_some(tokens)
}

/// The number at the start of `text`, and how many bytes it took: digits,
/// groups of three after a grouping separator, the decimal mark and more
/// digits, and an exponent when one follows -- `1,500.25`, `1.5e-3`.
///
/// What does not fit is left for the tokenizer, which declines a separator
/// it finds on its own: `1,5` in the United States, and `1.2.3` anywhere --
/// a second decimal mark starts another number, which nothing accepts.
fn number(text: &str, separators: NumberSeparators) -> Option<(f64, usize)> {
    let digits = |from: usize| {
        text.as_bytes()[from..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count()
    };
    // Rewritten as Rust reads a number, then parsed -- but only when it was
    // written otherwise: most numbers are read straight from `text`, with
    // nothing allocated on the keystroke.
    let mut plain = String::new();
    let mut rewritten = false;

    let first = digits(0);
    let mut end = first;
    plain.push_str(&text[..end]);
    if let Some(grouping) = separators.grouping
        && (1..=3).contains(&first)
    {
        while text[end..].starts_with(grouping) {
            let group = end + grouping.len_utf8();
            if digits(group) != 3 {
                break;
            }
            plain.push_str(&text[group..group + 3]);
            end = group + 3;
            rewritten = true;
        }
    }
    if text[end..].starts_with(separators.decimal) {
        let fraction = end + separators.decimal.len_utf8();
        let length = digits(fraction);
        rewritten |= separators.decimal != '.';
        plain.push('.');
        plain.push_str(&text[fraction..fraction + length]);
        end = fraction + length;
    }
    // `2e3` is two thousand; `2em` is a two and a word.
    let bytes = text.as_bytes();
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
        let exponent = digits(end + 1 + sign);
        if exponent > 0 {
            plain.push_str(&text[end..end + 1 + sign + exponent]);
            end += 1 + sign + exponent;
        }
    }
    let number = if rewritten {
        plain.as_str()
    } else {
        &text[..end]
    };
    Some((number.parse().ok()?, end))
}

fn starts_word(character: char) -> bool {
    character.is_alphabetic() || character == '\u{b0}'
}

/// Digits continue a word, so `log10` and `km2` are one: they never start
/// one. So do the superscripts of `m²` and `m³`.
fn continues_word(character: char) -> bool {
    starts_word(character) || character.is_ascii_digit() || matches!(character, '\u{b2}' | '\u{b3}')
}

#[cfg(test)]
mod tests {
    use super::*;
    use Token::*;

    fn plain(input: &str) -> Option<Vec<Token<'_>>> {
        tokenize(input, NumberSeparators::PLAIN)
    }

    const US: NumberSeparators = NumberSeparators {
        decimal: '.',
        grouping: Some(','),
    };
    const SPAIN: NumberSeparators = NumberSeparators {
        decimal: ',',
        grouping: Some('.'),
    };

    #[test]
    fn numbers_as_the_region_writes_them() {
        assert_eq!(tokenize("2,000", US), Some(vec![Number(2000.0)]));
        assert_eq!(tokenize("1,234,567.5", US), Some(vec![Number(1_234_567.5)]));
        assert_eq!(tokenize("2.000", SPAIN), Some(vec![Number(2000.0)]));
        assert_eq!(tokenize("1.234,5", SPAIN), Some(vec![Number(1234.5)]));
        assert_eq!(tokenize("1,5", SPAIN), Some(vec![Number(1.5)]));
        assert_eq!(
            tokenize("2000", US),
            Some(vec![Number(2000.0)]),
            "grouping is optional"
        );
    }

    /// A separator that is not between groups of three is not guessed at.
    #[test]
    fn a_separator_out_of_place_declines() {
        assert_eq!(tokenize("1,5", US), None);
        assert_eq!(tokenize("1,50", US), None);
        assert_eq!(tokenize("1,5000", US), None);
        assert_eq!(tokenize("1234,567", US), None);
        assert_eq!(tokenize("1.5", SPAIN), None);
        assert_eq!(tokenize("2,000", NumberSeparators::PLAIN), None);
    }

    #[test]
    fn numbers_words_and_symbols() {
        assert_eq!(
            plain("sqrt(2)^pi"),
            Some(vec![
                Word("sqrt"),
                Open,
                Number(2.0),
                Close,
                Caret,
                Word("pi")
            ])
        );
        assert_eq!(
            plain("45\u{b0} \u{d7} 2"),
            Some(vec![Number(45.0), Word("\u{b0}"), Star, Number(2.0)])
        );
        assert_eq!(plain("log10"), Some(vec![Word("log10")]));
    }

    #[test]
    fn exponents_only_when_digits_follow() {
        assert_eq!(plain("1.5e-3"), Some(vec![Number(0.0015)]));
        assert_eq!(plain("2E3"), Some(vec![Number(2000.0)]));
        assert_eq!(plain("2em"), Some(vec![Number(2.0), Word("em")]));
        assert_eq!(plain("2e"), Some(vec![Number(2.0), Word("e")]));
    }

    #[test]
    fn a_second_point_starts_another_number() {
        assert_eq!(plain("1.2.3"), Some(vec![Number(1.2), Number(0.3)]));
        assert_eq!(plain("."), None);
    }

    #[test]
    fn unknown_characters_decline_everything() {
        assert_eq!(plain("localhost:8080"), None);
        assert_eq!(plain("c#"), None);
        assert_eq!(plain(""), None);
    }
}
