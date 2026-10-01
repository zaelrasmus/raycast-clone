//! Tokens into a value, by recursive descent.
//!
//! The grammar is small, it has to reject far more than it accepts, and
//! owning the rejection rules is the point: a parse that fails anywhere
//! fails whole, and a word that is not a function, a constant or a unit
//! fails it. That is what keeps `7-zip` and `python 3.12` out.
//!
//! ```text
//! calculation := expression (('in' | 'to' | 'as') (UNIT | CURRENCY))?
//!              | expression ('as' '%' ('of' expression)?)?
//! expression  := term (('+' | '-') term)*
//! term        := unary (('*' | '/' | 'mod') unary)*
//! unary       := ('-' | '+') unary | power
//! power       := postfix ('^' unary)?
//! postfix     := primary '!'* ('%' (('of' | 'off' | 'on') unary)?)?
//! primary     := DATE
//!             | NUMBER (UNIT | CODE)?
//!             | '(' expression ')' UNIT?
//!             | FUNCTION '(' expression ')'
//!             | CONSTANT
//!             | SPAN? ('until' | 'since') unary
//! DATE        := NUMBER ORDINAL? MONTH YEAR? | MONTH NUMBER ORDINAL? YEAR?
//!             | WEEKDAY | 'today' | 'tomorrow' | 'yesterday'
//! ```

use jiff::civil::Date;

use super::dates::{self, Toward};
use super::functions::{self, factorial};
use super::lexer::Token;
use super::units;
use super::value::{Phrase, Value};
use crate::currency::{Currency, Rates, currencies};

/// How deep parentheses and signs may nest. Far past anything typed by
/// hand, and well short of the stack.
const MAX_DEPTH: u32 = 64;

/// What parsing found: the value, and how many operations produced it.
pub struct Parsed {
    pub value: Value,
    /// Zero for `2024`, `45 deg` or `pi`: a value restated is not a
    /// calculation, and the root list does not answer it.
    pub operations: u32,
    /// Whether money was named.
    pub money: bool,
    /// Whether one currency was converted into another: the table of rates
    /// was needed.
    pub converts: bool,
    /// Whether it was needed and there was none, so `value` is not the
    /// answer: every rate was taken as one, to finish the parse and know
    /// that the query is money.
    pub pending: bool,
}

pub struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    at: usize,
    depth: u32,
    operations: u32,
    /// Whether a unit was named by a loose word -- `c`, `f` -- which only
    /// a conversion may use.
    loose: bool,
    /// Asked for the first time a date needs it, and only then: most of what
    /// is typed has no date in it, and reading the time zone is not free.
    today: &'a dyn Fn() -> Date,
    today_read: Option<Date>,
    /// Which way a date with no year is looked for: back, inside `since`.
    toward: Toward,
    /// The table of exchange rates in hand, if any.
    rates: Option<&'a Rates>,
    money: bool,
    converts: bool,
    pending: bool,
}

impl<'a> Parser<'a> {
    pub fn new(
        tokens: Vec<Token<'a>>,
        today: &'a dyn Fn() -> Date,
        rates: Option<&'a Rates>,
    ) -> Self {
        Self {
            tokens,
            at: 0,
            depth: 0,
            operations: 0,
            loose: false,
            today,
            today_read: None,
            toward: Toward::Next,
            rates,
            money: false,
            converts: false,
            pending: false,
        }
    }

    fn today(&mut self) -> Date {
        *self.today_read.get_or_insert_with(|| (self.today)())
    }

    /// The token `ahead` places past the next one.
    fn peek_at(&self, ahead: usize) -> Option<Token<'a>> {
        self.tokens.get(self.at + ahead).copied()
    }

    /// Parse the whole input as one expression, or decline.
    ///
    /// Trailing junk means this was not an expression after all. Accepting a
    /// prefix is how `3 apples` becomes `3`.
    pub fn whole(mut self) -> Option<Parsed> {
        let value = self.calculation()?;
        (self.at == self.tokens.len()).then_some(Parsed {
            value,
            operations: self.operations,
            money: self.money,
            converts: self.converts,
            pending: self.pending,
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

    /// Take the word `name` if it is next, in any case.
    fn eat_word(&mut self, name: &str) -> bool {
        if let Some(Token::Word(word)) = self.peek()
            && word.eq_ignore_ascii_case(name)
        {
            self.at += 1;
            return true;
        }
        false
    }

    /// What the whole expression is turned into, last: `5 km in mi`, `5 as
    /// % of 20`.
    fn calculation(&mut self) -> Option<Value> {
        let value = self.expression()?;
        let into = ["in", "to", "as"]
            .into_iter()
            .find(|&word| self.eat_word(word));
        let Some(into) = into else {
            // `2f + 3` is not a temperature; `100 f in c` is.
            return (!self.loose).then_some(value);
        };
        if !self.eat(Token::Percent) {
            if let Value::Money { amount, currency } = value {
                let to = self.currency_named()?;
                let value = self.exchange(amount, currency, to)?;
                // `10 usd in usd` restates a value, as `5 km in km` does.
                return if std::ptr::eq(currency, to) {
                    Some(value)
                } else {
                    self.counted(Some(value))
                };
            }
            let target = self.unit()?;
            if let Value::Quantity { unit, .. } = value
                && std::ptr::eq(unit, target.unit)
            {
                return Some(value);
            }
            return self.counted(value.convert(target.unit));
        }
        if into != "as" || self.loose {
            return None;
        }
        let whole = if self.eat_word("of") {
            Some(self.expression()?)
        } else {
            None
        };
        self.counted(value.share_of(whole))
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
            let right = self.in_currency_of(left, right)?;
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
                let right = self.in_currency_of(left, right)?;
                self.counted(left.over(right))?
            } else if self.eat_word("mod") {
                // `%` is a percentage (7.39); the remainder has a name.
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
            return self.counted(value.map(|amount| -amount));
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
        if !self.eat(Token::Percent) {
            return Some(value);
        }
        // A percentage is a value written another way, not a calculation:
        // `50%` alone is not answered.
        let Value::Number(percent) = value else {
            return None;
        };
        let percent = Value::Percent(percent);
        let Some(phrase) = self.phrase() else {
            return Some(percent);
        };
        let base = self.unary()?;
        self.counted(percent.percent(phrase, base))
    }

    /// `of`, `off` or `on`, taken if next.
    fn phrase(&mut self) -> Option<Phrase> {
        let Some(Token::Word(word)) = self.peek() else {
            return None;
        };
        let phrase = Phrase::named(word)?;
        self.at += 1;
        Some(phrase)
    }

    fn primary(&mut self) -> Option<Value> {
        match self.peek()? {
            Token::Number(number) => {
                self.at += 1;
                if let Some(day) = self.day_then_month(number) {
                    return day;
                }
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
                if let Some(constant) = functions::constant(word) {
                    return Some(Value::Number(constant));
                }
                self.date_or_span(word)
            }
            _ => None,
        }
    }

    /// A date written day first, when `number` is followed by a month:
    /// `25 dec`, `25th december 2027`. `None` when it is not one, and the
    /// number is left as a number; `Some(None)` when it began like one and
    /// is not one, which declines.
    fn day_then_month(&mut self, number: f64) -> Option<Option<Value>> {
        let ordinal = matches!(self.peek(), Some(Token::Word(word)) if dates::is_ordinal(word));
        let Some(Token::Word(word)) = self.peek_at(usize::from(ordinal)) else {
            return None;
        };
        let month = dates::month(word)?;
        self.at += 1 + usize::from(ordinal);
        Some(self.date(month, number))
    }

    /// A date written month first, after its month: `dec 25`, `dec 25th
    /// 2027`.
    fn month_then_day(&mut self, month: i8) -> Option<Value> {
        let Token::Number(number) = self.peek()? else {
            return None;
        };
        self.at += 1;
        if matches!(self.peek(), Some(Token::Word(word)) if dates::is_ordinal(word)) {
            self.at += 1;
        }
        self.date(month, number)
    }

    /// `day` of `month`, and the year written after them, if one is.
    fn date(&mut self, month: i8, day: f64) -> Option<Value> {
        let day = dates::day(day)?;
        let year = match self.peek() {
            Some(Token::Number(number)) => {
                let year = dates::year(number)?;
                self.at += 1;
                Some(year)
            }
            _ => None,
        };
        let today = self.today();
        dates::on(month, day, year, today, self.toward).map(Value::Date)
    }

    /// What a word that is not a function or a constant may still be: a
    /// date, or how long until or since one.
    fn date_or_span(&mut self, word: &str) -> Option<Value> {
        if let Some(days) = dates::relative(word) {
            let today = self.today();
            return today
                .checked_add(jiff::Span::new().days(days))
                .ok()
                .map(Value::Date);
        }
        if let Some(weekday) = dates::weekday(word) {
            let today = self.today();
            return dates::weekday_from(weekday, today, self.toward).map(Value::Date);
        }
        if let Some(month) = dates::month(word) {
            return self.month_then_day(month);
        }
        // `until 25 dec` is in days; `weeks until 25 dec` says so.
        let span = if is_until_or_since(word) {
            self.at -= 1;
            units::calendar_unit(units::DAY)
        } else {
            let span = units::find(word).map(|found| found.unit).filter(|unit| {
                [units::DAY, units::WEEK, units::MONTH, units::YEAR].contains(&unit.factor)
            })?;
            if !matches!(self.peek(), Some(Token::Word(next)) if is_until_or_since(next)) {
                return None;
            }
            span
        };
        self.until_or_since(span)
    }

    /// `until <date>` or `since <date>`, in `span`: `weeks until 25 dec`.
    fn until_or_since(&mut self, span: &'static units::Unit) -> Option<Value> {
        let since = self.eat_word("since");
        if !since && !self.eat_word("until") {
            return None;
        }
        let looking = std::mem::replace(
            &mut self.toward,
            if since { Toward::Last } else { Toward::Next },
        );
        let target = self.unary();
        self.toward = looking;
        let Value::Date(target) = target? else {
            return None;
        };
        let today = self.today();
        let between = if since {
            Value::days_between(target, today)
        } else {
            Value::days_between(today, target)
        };
        self.counted(between?.convert(span))
    }

    /// `value` in the unit named next, when a plain number is followed by
    /// one: `45 deg`, `(1+2) rad`. Anything else is left for the caller,
    /// which declines a word it cannot place.
    fn unit_after(&mut self, value: Value) -> Value {
        let Value::Number(amount) = value else {
            return value;
        };
        if let Some(found) = self.unit() {
            self.loose |= found.loose;
            return Value::Quantity {
                amount,
                unit: found.unit,
            };
        }
        // A currency's code, after the units: `10 usd`, `10usd`.
        if let Some(Token::Word(word)) = self.peek()
            && let Some(currency) = currencies::find(word)
        {
            self.at += 1;
            self.money = true;
            return Value::Money { amount, currency };
        }
        value
    }

    /// The currency the rest of the query names: a code, a name or a
    /// country, as many words as it takes -- `cad`, `canadian dollar`,
    /// `japan` -- and only one currency: Zimbabwe has two, and the root list
    /// does not guess.
    fn currency_named(&mut self) -> Option<&'static Currency> {
        let words: Vec<&str> = self.tokens[self.at..]
            .iter()
            .map(|token| match token {
                Token::Word(word) => Some(*word),
                _ => None,
            })
            .collect::<Option<_>>()?;
        let currency = currencies::exactly(&words.join(" "))?;
        self.at = self.tokens.len();
        Some(currency)
    }

    /// `amount` of `from` in `to`, from the table in hand. With no table,
    /// one for one and marked pending, so the parse can finish. A currency
    /// the table has no rate for declines the query.
    fn exchange(
        &mut self,
        amount: f64,
        from: &'static Currency,
        to: &'static Currency,
    ) -> Option<Value> {
        if std::ptr::eq(from, to) {
            return Some(Value::Money {
                amount,
                currency: to,
            });
        }
        self.converts = true;
        let amount = match self.rates {
            Some(rates) => rates.convert(amount, from.code, to.code)?,
            None => {
                self.pending = true;
                amount
            }
        };
        Some(Value::Money {
            amount,
            currency: to,
        })
    }

    /// `right` in `left`'s currency, when both are money: `100 usd + 20 eur`
    /// adds dollars.
    fn in_currency_of(&mut self, left: Value, right: Value) -> Option<Value> {
        match (left, right) {
            (
                Value::Money { currency, .. },
                Value::Money {
                    amount,
                    currency: from,
                },
            ) => self.exchange(amount, from, currency),
            _ => Some(right),
        }
    }

    /// The unit named next, taken if there is one: a word, or two around a
    /// slash that name one together -- `km/h`. A slash before a number is
    /// a division, and is left alone.
    fn unit(&mut self) -> Option<units::Found> {
        let Some(Token::Word(word)) = self.peek() else {
            return None;
        };
        if let (Some(Token::Slash), Some(Token::Word(below))) = (
            self.tokens.get(self.at + 1).copied(),
            self.tokens.get(self.at + 2).copied(),
        ) && let Some(unit) = units::find_per(word, below)
        {
            self.at += 3;
            return Some(units::Found { unit, loose: false });
        }
        let found = units::find(word)?;
        self.at += 1;
        Some(found)
    }
}

fn is_until_or_since(word: &str) -> bool {
    word.eq_ignore_ascii_case("until") || word.eq_ignore_ascii_case("since")
}
