//! Tokens into a value, by recursive descent.
//!
//! The grammar is small, it has to reject far more than it accepts, and
//! owning the rejection rules is the point: a parse that fails anywhere
//! fails whole, and a word that is not a function, a constant or a unit
//! fails it. That is what keeps `7-zip` and `python 3.12` out.
//!
//! ```text
//! expression := term (('+' | '-') term)*
//! term       := unary (('*' | '/' | '%') unary)*
//! unary      := ('-' | '+') unary | power
//! power      := postfix ('^' unary)?
//! postfix    := primary '!'*
//! primary    := NUMBER UNIT?
//!             | '(' expression ')' UNIT?
//!             | FUNCTION '(' expression ')'
//!             | CONSTANT
//! ```

use super::functions::{self, factorial};
use super::lexer::Token;
use super::units;
use super::value::Value;

/// How deep parentheses and signs may nest. Far past anything typed by
/// hand, and well short of the stack.
const MAX_DEPTH: u32 = 64;

/// What parsing found: the value, and how many operations produced it.
pub struct Parsed {
    pub value: Value,
    /// Zero for `2024`, `45 deg` or `pi`: a value restated is not a
    /// calculation, and the root list does not answer it.
    pub operations: u32,
}

pub struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    at: usize,
    depth: u32,
    operations: u32,
}

impl<'a> Parser<'a> {
    pub fn new(tokens: Vec<Token<'a>>) -> Self {
        Self {
            tokens,
            at: 0,
            depth: 0,
            operations: 0,
        }
    }

    /// Parse the whole input as one expression, or decline.
    ///
    /// Trailing junk means this was not an expression after all. Accepting a
    /// prefix is how `3 apples` becomes `3`.
    pub fn whole(mut self) -> Option<Parsed> {
        let value = self.expression()?;
        (self.at == self.tokens.len()).then_some(Parsed {
            value,
            operations: self.operations,
        })
    }

    fn peek(&self) -> Option<Token<'a>> {
        self.tokens.get(self.at).copied()
    }

    fn eat(&mut self, token: Token) -> bool {
        if self.peek() == Some(token) {
            self.at += 1;
            return true;
        }
        false
    }

    /// One more operation, whose result is `value`.
    fn counted(&mut self, value: Option<Value>) -> Option<Value> {
        self.operations += 1;
        value
    }

    /// Enter one level of nesting, or decline past [`MAX_DEPTH`].
    fn deeper(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_DEPTH).then_some(())
    }

    fn expression(&mut self) -> Option<Value> {
        self.deeper()?;
        let mut left = self.term()?;
        loop {
            let sign = if self.eat(Token::Plus) {
                1.0
            } else if self.eat(Token::Minus) {
                -1.0
            } else {
                break;
            };
            let right = self.term()?;
            left = self.counted(left.plus(right, sign))?;
        }
        self.depth -= 1;
        Some(left)
    }

    fn term(&mut self) -> Option<Value> {
        let mut left = self.unary()?;
        loop {
            left = if self.eat(Token::Star) {
                let right = self.unary()?;
                self.counted(left.times(right))?
            } else if self.eat(Token::Slash) {
                // Division by zero yields infinity in IEEE arithmetic, and
                // the caller rejects non-finite results -- so the launcher
                // shows nothing rather than a row saying `inf`.
                let right = self.unary()?;
                self.counted(left.over(right))?
            } else if self.eat(Token::Percent) {
                let right = self.unary()?;
                self.counted(left.numbers(right, |left, right| left % right))?
            } else {
                return Some(left);
            };
        }
    }

    fn unary(&mut self) -> Option<Value> {
        if self.eat(Token::Minus) {
            self.deeper()?;
            let value = self.unary()?;
            self.depth -= 1;
            return self.counted(Some(value.map(|amount| -amount)));
        }
        if self.eat(Token::Plus) {
            self.deeper()?;
            let value = self.unary()?;
            self.depth -= 1;
            return self.counted(Some(value));
        }
        self.power()
    }

    /// Right-associative, and the exponent is a `unary` so that `2^-1` parses.
    fn power(&mut self) -> Option<Value> {
        let base = self.postfix()?;
        if self.eat(Token::Caret) {
            let exponent = self.unary()?;
            return self.counted(base.numbers(exponent, f64::powf));
        }
        Some(base)
    }

    fn postfix(&mut self) -> Option<Value> {
        let mut value = self.primary()?;
        while self.eat(Token::Bang) {
            let Value::Number(n) = value else {
                return None;
            };
            value = self.counted(factorial(n).map(Value::Number))?;
        }
        Some(value)
    }

    fn primary(&mut self) -> Option<Value> {
        match self.peek()? {
            Token::Number(number) => {
                self.at += 1;
                Some(self.unit_after(Value::Number(number)))
            }
            Token::Open => {
                self.at += 1;
                let value = self.expression()?;
                self.eat(Token::Close).then(|| self.unit_after(value))
            }
            Token::Word(word) => {
                self.at += 1;
                if let Some(function) = functions::function(word) {
                    if !self.eat(Token::Open) {
                        return None;
                    }
                    let argument = self.expression()?;
                    if !self.eat(Token::Close) {
                        return None;
                    }
                    return self.counted(function.apply(argument));
                }
                functions::constant(word).map(Value::Number)
            }
            _ => None,
        }
    }

    /// `value` in the unit named next, when a plain number is followed by
    /// one: `45 deg`, `(1+2) rad`. Anything else is left for the caller,
    /// which declines a word it cannot place.
    fn unit_after(&mut self, value: Value) -> Value {
        let (Value::Number(amount), Some(Token::Word(word))) = (value, self.peek()) else {
            return value;
        };
        match units::find(word) {
            Some(unit) => {
                self.at += 1;
                Value::Quantity { amount, unit }
            }
            None => value,
        }
    }
}
