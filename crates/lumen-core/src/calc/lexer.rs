//! Text into tokens.
//!
//! Rejects rather than skips: a character this does not know means the input
//! is not a calculation, and the whole of it is declined. Words are sliced
//! from the input, not copied: this runs on every keystroke.

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

pub fn tokenize(input: &str) -> Option<Vec<Token<'_>>> {
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
        } else if character.is_ascii_digit() || character == '.' {
            let length = number_length(rest);
            tokens.push(Token::Number(rest[..length].parse().ok()?));
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

/// How much of `text` is one number: digits, one point, more digits, and an
/// exponent when one follows -- `1.5e-3`.
///
/// A second point ends the number and starts another, which nothing in the
/// grammar accepts: `1.2.3` is declined, not read as `1.2`.
fn number_length(text: &str) -> usize {
    let bytes = text.as_bytes();
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count()
    };

    let mut end = digits(0);
    if bytes.get(end) == Some(&b'.') {
        end += 1 + digits(end + 1);
    }
    // `2e3` is two thousand; `2em` is a two and a word.
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
        let exponent = digits(end + 1 + sign);
        if exponent > 0 {
            end += 1 + sign + exponent;
        }
    }
    end
}

fn starts_word(character: char) -> bool {
    character.is_alphabetic() || character == '\u{b0}'
}

/// Digits continue a word, so `log10` is one: they never start one.
fn continues_word(character: char) -> bool {
    starts_word(character) || character.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Token::*;

    #[test]
    fn numbers_words_and_symbols() {
        assert_eq!(
            tokenize("sqrt(2)^pi"),
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
            tokenize("45\u{b0} \u{d7} 2"),
            Some(vec![Number(45.0), Word("\u{b0}"), Star, Number(2.0)])
        );
        assert_eq!(tokenize("log10"), Some(vec![Word("log10")]));
    }

    #[test]
    fn exponents_only_when_digits_follow() {
        assert_eq!(tokenize("1.5e-3"), Some(vec![Number(0.0015)]));
        assert_eq!(tokenize("2E3"), Some(vec![Number(2000.0)]));
        assert_eq!(tokenize("2em"), Some(vec![Number(2.0), Word("em")]));
        assert_eq!(tokenize("2e"), Some(vec![Number(2.0), Word("e")]));
    }

    #[test]
    fn a_second_point_starts_another_number() {
        assert_eq!(tokenize("1.2.3"), Some(vec![Number(1.2), Number(0.3)]));
        assert_eq!(tokenize("."), None);
    }

    #[test]
    fn unknown_characters_decline_everything() {
        assert_eq!(tokenize("localhost:8080"), None);
        assert_eq!(tokenize("c#"), None);
        assert_eq!(tokenize(""), None);
    }
}
