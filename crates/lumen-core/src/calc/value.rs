//! What an expression is worth: a number, an amount of a unit, a
//! percentage, or a day.
//!
//! Every operation answers `None` when it has no meaning -- `2 km * 3 km`,
//! `5 km + 2 kg`, `today * 2` -- and the whole query is declined. A
//! calculator that guesses here shows a confident wrong number.

use jiff::civil::Date;

use super::units::{self, Dimension, Unit};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Number(f64),
    Quantity {
        amount: f64,
        unit: &'static Unit,
    },
    /// `25%`, kept as written -- `25`, not `0.25` -- because what it means
    /// depends on what it meets: `399 + 25%` is `498.75`, `399 * 25%` is
    /// `99.75` (see 7.39).
    Percent(f64),
    /// A day of the calendar, with no time: `25 dec`, `today`.
    Date(Date),
}

use Value::{Date as Day, Number, Percent, Quantity};

impl Value {
    /// The number inside, whatever unit it is in. A day has none.
    pub fn amount(self) -> Option<f64> {
        match self {
            Number(amount) | Quantity { amount, .. } | Percent(amount) => Some(amount),
            Day(_) => None,
        }
    }

    pub fn is_finite(self) -> bool {
        self.amount().is_none_or(f64::is_finite)
    }

    /// The same value with its number changed and its unit kept. A day has
    /// no number to change: `-today` and `round(today)` are declined.
    pub fn map(self, f: impl FnOnce(f64) -> f64) -> Option<Value> {
        match self {
            Number(amount) => Some(Number(f(amount))),
            Quantity { amount, unit } => Some(Quantity {
                amount: f(amount),
                unit,
            }),
            Percent(percent) => Some(Percent(f(percent))),
            Day(_) => None,
        }
    }

    /// A percentage as the fraction it stands for: `25%` is `0.25`.
    fn fraction(percent: f64) -> f64 {
        percent / 100.0
    }

    /// `self` percent `of`, `off` or `on` `base`: `20% of 50` is `10`, `25%
    /// off 399` is `299.25`, `5% on 30` is `31.5`. The base keeps its unit.
    pub fn percent(self, phrase: Phrase, base: Value) -> Option<Value> {
        let (Percent(percent), Number(_) | Quantity { .. }) = (self, base) else {
            return None;
        };
        let share = Self::fraction(percent);
        base.map(|amount| match phrase {
            Phrase::Of => amount * share,
            Phrase::Off => amount * (1.0 - share),
            Phrase::On => amount * (1.0 + share),
        })
    }

    /// What share of `whole` this is: `5 as % of 20` is `25%`. With no
    /// whole, the number itself: `0.125 as %` is `12.5%`.
    pub fn share_of(self, whole: Option<Value>) -> Option<Value> {
        let ratio = match whole {
            Some(whole) => self.over(whole)?,
            None => self,
        };
        match ratio {
            Number(ratio) => Some(Percent(ratio * 100.0)),
            _ => None,
        }
    }

    /// This value in `unit`, when they measure the same thing.
    pub fn convert(self, to: &'static Unit) -> Option<Value> {
        let Quantity { amount, unit } = self else {
            return None;
        };
        (unit.dimension == to.dimension).then(|| Quantity {
            amount: to.of_base(unit.to_base(amount)),
            unit: to,
        })
    }

    /// The days from `earlier` to `later`, as a quantity: `85 days`.
    pub fn days_between(earlier: Date, later: Date) -> Option<Value> {
        let days = earlier.until(later).ok()?.get_days();
        Some(Quantity {
            amount: f64::from(days),
            unit: units::calendar_unit(units::DAY),
        })
    }

    /// `self + right`, or `self - right` when `sign` is `-1`.
    ///
    /// A number joins a quantity in its unit -- `5 km + 3` is `8 km` -- and a
    /// quantity joins another in the left one's. Temperatures on different
    /// scales are not added: `10 °C + 5 °F` is two different questions.
    ///
    /// A percentage added to anything else is a share of it: `399 + 25%` is
    /// `498.75`. Two percentages add as numbers do; a percentage first and a
    /// number second has no one meaning, and is declined.
    ///
    /// A day moves by whole days, weeks, months or years, by the calendar:
    /// `31 jan + 1 month` is the end of February. One day less another is
    /// the days between them.
    pub fn plus(self, right: Value, sign: f64) -> Option<Value> {
        match (self, right) {
            (Day(day), Quantity { amount, unit }) => shift(day, sign * amount, unit).map(Day),
            (Quantity { amount, unit }, Day(day)) if sign > 0.0 => {
                shift(day, amount, unit).map(Day)
            }
            (Day(later), Day(earlier)) if sign < 0.0 => Self::days_between(earlier, later),
            (Day(_), _) | (_, Day(_)) => None,
            (Number(left), Number(right)) => Some(Number(left + sign * right)),
            (Percent(left), Percent(right)) => Some(Percent(left + sign * right)),
            (left @ (Number(_) | Quantity { .. }), Percent(right)) => {
                left.map(|amount| amount * (1.0 + sign * Self::fraction(right)))
            }
            (Percent(_), _) => None,
            (Quantity { amount, unit }, Number(right)) => Some(Quantity {
                amount: amount + sign * right,
                unit,
            }),
            (Number(left), Quantity { amount, unit }) => Some(Quantity {
                amount: left + sign * amount,
                unit,
            }),
            (Quantity { amount, unit }, right @ Quantity { unit: other, .. }) => {
                if (unit.has_offset() || other.has_offset()) && !std::ptr::eq(unit, other) {
                    return None;
                }
                let right = right.convert(unit)?.amount()?;
                Some(Quantity {
                    amount: amount + sign * right,
                    unit,
                })
            }
        }
    }

    /// `self * right`. A quantity times a quantity would be a new unit,
    /// which is declined rather than half-supported.
    pub fn times(self, right: Value) -> Option<Value> {
        match (self, right) {
            (Day(_), _) | (_, Day(_)) => None,
            (Number(left), Number(right)) => Some(Number(left * right)),
            (quantity @ Quantity { .. }, Number(right))
            | (Number(right), quantity @ Quantity { .. }) => quantity.map(|amount| amount * right),
            (Quantity { .. }, Quantity { .. }) => None,
            (Percent(percent), other) | (other, Percent(percent)) => match other {
                Number(_) | Quantity { .. } => other.map(|amount| amount * Self::fraction(percent)),
                Percent(_) | Day(_) => None,
            },
        }
    }

    /// `self / right`. A quantity over one of the same kind is a ratio:
    /// `1 km / 1 m` is `1000`.
    pub fn over(self, right: Value) -> Option<Value> {
        match (self, right) {
            (Day(_), _) | (_, Day(_)) => None,
            (Number(left), Number(right)) => Some(Number(left / right)),
            (quantity @ Quantity { .. }, Number(right)) => quantity.map(|amount| amount / right),
            (
                Quantity { amount, unit },
                Quantity {
                    amount: below,
                    unit: other,
                },
            ) => (unit.dimension == other.dimension && !unit.has_offset() && !other.has_offset())
                .then(|| Number(unit.to_base(amount) / other.to_base(below))),
            (left @ (Number(_) | Quantity { .. }), Percent(percent)) => {
                left.map(|amount| amount / Self::fraction(percent))
            }
            (Percent(percent), Number(right)) => Some(Percent(percent / right)),
            (Number(_), Quantity { .. }) | (Percent(_), _) => None,
        }
    }

    /// The operations only a plain number has: powers and remainders.
    pub fn numbers(self, right: Value, f: impl FnOnce(f64, f64) -> f64) -> Option<Value> {
        match (self, right) {
            (Number(left), Number(right)) => Some(Number(f(left, right))),
            _ => None,
        }
    }
}

/// `day` moved by `amount` of `unit`, by the calendar.
///
/// Only whole days, weeks, months and years: a day has no time to move by
/// hours, and half a month has no one answer.
fn shift(day: Date, amount: f64, unit: &Unit) -> Option<Date> {
    if unit.dimension != Dimension::Time || amount.fract() != 0.0 || amount.abs() > 1e6 {
        return None;
    }
    let whole = amount as i64;
    let span = match unit.factor {
        units::DAY => jiff::Span::new().try_days(whole),
        units::WEEK => jiff::Span::new().try_weeks(whole),
        units::MONTH => jiff::Span::new().try_months(whole),
        units::YEAR => jiff::Span::new().try_years(whole),
        _ => return None,
    };
    day.checked_add(span.ok()?).ok()
}

/// The words that follow a percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phrase {
    Of,
    Off,
    On,
}

impl Phrase {
    pub fn named(word: &str) -> Option<Phrase> {
        [("of", Phrase::Of), ("off", Phrase::Off), ("on", Phrase::On)]
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(word))
            .map(|(_, phrase)| phrase)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calc::units::find;
    use jiff::civil::date;

    fn deg(amount: f64) -> Value {
        Quantity {
            amount,
            unit: find("deg").unwrap().unit,
        }
    }

    fn of(amount: f64, unit: &str) -> Value {
        Quantity {
            amount,
            unit: find(unit).unwrap().unit,
        }
    }

    #[test]
    fn numbers_join_quantities_in_their_unit() {
        assert_eq!(deg(90.0).plus(Number(1.0), 1.0), Some(deg(91.0)));
        assert_eq!(Number(2.0).times(deg(45.0)), Some(deg(90.0)));
        assert_eq!(deg(90.0).over(Number(2.0)), Some(deg(45.0)));
    }

    #[test]
    fn quantities_of_one_kind_convert_into_the_left() {
        let rad = find("rad").unwrap().unit;
        let sum = deg(180.0)
            .plus(
                Value::Quantity {
                    amount: std::f64::consts::PI,
                    unit: rad,
                },
                1.0,
            )
            .unwrap();
        assert!((sum.amount().unwrap() - 360.0).abs() < 1e-9);
        let ratio = deg(180.0).over(Value::Quantity {
            amount: std::f64::consts::PI,
            unit: rad,
        });
        assert!((ratio.unwrap().amount().unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn what_has_no_meaning_is_declined() {
        assert_eq!(deg(1.0).times(deg(1.0)), None);
        assert_eq!(Number(1.0).over(deg(1.0)), None);
        assert_eq!(deg(2.0).numbers(Number(2.0), f64::powf), None);
        assert_eq!(Number(1.0).convert(find("rad").unwrap().unit), None);
    }

    #[test]
    fn a_day_moves_by_the_calendar() {
        let day = Day(date(2026, 1, 31));
        assert_eq!(
            day.plus(of(1.0, "month"), 1.0),
            Some(Day(date(2026, 2, 28)))
        );
        assert_eq!(
            day.plus(of(2.0, "weeks"), 1.0),
            Some(Day(date(2026, 2, 14)))
        );
        assert_eq!(day.plus(of(1.0, "day"), -1.0), Some(Day(date(2026, 1, 30))));
        assert_eq!(of(1.0, "year").plus(day, 1.0), Some(Day(date(2027, 1, 31))));
        assert_eq!(day.plus(of(36.0, "h"), 1.0), None, "a day has no hours");
        assert_eq!(day.plus(of(1.5, "days"), 1.0), None);
        assert_eq!(day.plus(Number(1.0), 1.0), None);
        assert_eq!(of(1.0, "day").plus(day, -1.0), None);
    }

    #[test]
    fn one_day_less_another_is_the_days_between() {
        let christmas = Day(date(2026, 12, 25));
        let shown = christmas.plus(Day(date(2026, 10, 1)), -1.0);
        assert_eq!(shown, Some(of(85.0, "days")));
        assert_eq!(christmas.plus(christmas, 1.0), None);
        assert_eq!(christmas.times(Number(2.0)), None);
        assert_eq!(christmas.map(|amount| -amount), None);
    }
}
