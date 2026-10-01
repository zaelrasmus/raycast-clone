//! Text into tokens.
//!
//! Rejects rather than skips: a character this does not know means the input
//! is not a calculation, and the whole of it is declined.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Token {
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

pub fn tokenize(input: &str) -> Option<Vec<Token>> {
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
