//! Tokens into a value, by recursive descent.
//!
//! The grammar is small, it has to reject far more than it accepts, and
//! owning the rejection rules is the point: a parse that fails anywhere
//! fails whole.

use super::lexer::Token;

pub struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, at: 0 }
    }

    /// Parse the whole input as one expression, or decline.
    ///
    /// Trailing junk means this was not an expression after all. Accepting a
    /// prefix is how `3 apples` becomes `3`.
    pub fn whole(mut self) -> Option<f64> {
        let value = self.expression()?;
        (self.at == self.tokens.len()).then_some(value)
    }

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
                // the caller rejects non-finite results -- so the launcher
                // shows nothing rather than a row saying `inf`.
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
