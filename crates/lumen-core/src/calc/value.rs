//! What an expression is worth: a number, or an amount of a unit.
//!
//! Every operation answers `None` when it has no meaning -- `2 km * 3 km`,
//! `5 km + 2 kg` -- and the whole query is declined. A calculator that
//! guesses here shows a confident wrong number.

use super::units::Unit;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Number(f64),
    Quantity { amount: f64, unit: &'static Unit },
}

use Value::{Number, Quantity};

impl Value {
    /// The number inside, whatever unit it is in.
    pub fn amount(self) -> f64 {
        match self {
            Number(amount) | Quantity { amount, .. } => amount,
        }
    }

    pub fn is_finite(self) -> bool {
        self.amount().is_finite()
    }

    /// The same value with `amount` changed and the unit kept.
    pub fn map(self, f: impl FnOnce(f64) -> f64) -> Value {
        match self {
            Number(amount) => Number(f(amount)),
            Quantity { amount, unit } => Quantity {
                amount: f(amount),
                unit,
            },
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

    /// `self + right`, or `self - right` when `sign` is `-1`.
    ///
    /// A number joins a quantity in its unit -- `5 km + 3` is `8 km` -- and a
    /// quantity joins another in the left one's. Temperatures on different
    /// scales are not added: `10 °C + 5 °F` is two different questions.
    pub fn plus(self, right: Value, sign: f64) -> Option<Value> {
        match (self, right) {
            (Number(left), Number(right)) => Some(Number(left + sign * right)),
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
                let right = right.convert(unit)?.amount();
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
            (Number(left), Number(right)) => Some(Number(left * right)),
            (quantity @ Quantity { .. }, Number(right))
            | (Number(right), quantity @ Quantity { .. }) => {
                Some(quantity.map(|amount| amount * right))
            }
            (Quantity { .. }, Quantity { .. }) => None,
        }
    }

    /// `self / right`. A quantity over one of the same kind is a ratio:
    /// `1 km / 1 m` is `1000`.
    pub fn over(self, right: Value) -> Option<Value> {
        match (self, right) {
            (Number(left), Number(right)) => Some(Number(left / right)),
            (quantity @ Quantity { .. }, Number(right)) => {
                Some(quantity.map(|amount| amount / right))
            }
            (
                Quantity { amount, unit },
                Quantity {
                    amount: below,
                    unit: other,
                },
            ) => (unit.dimension == other.dimension && !unit.has_offset() && !other.has_offset())
                .then(|| Number(unit.to_base(amount) / other.to_base(below))),
            (Number(_), Quantity { .. }) => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calc::units::find;

    fn deg(amount: f64) -> Value {
        Quantity {
            amount,
            unit: find("deg").unwrap(),
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
        let rad = find("rad").unwrap();
        let sum = deg(180.0)
            .plus(
                Value::Quantity {
                    amount: std::f64::consts::PI,
                    unit: rad,
                },
                1.0,
            )
            .unwrap();
        assert!((sum.amount() - 360.0).abs() < 1e-9);
        let ratio = deg(180.0).over(Value::Quantity {
            amount: std::f64::consts::PI,
            unit: rad,
        });
        assert!((ratio.unwrap().amount() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn what_has_no_meaning_is_declined() {
        assert_eq!(deg(1.0).times(deg(1.0)), None);
        assert_eq!(Number(1.0).over(deg(1.0)), None);
        assert_eq!(deg(2.0).numbers(Number(2.0), f64::powf), None);
        assert_eq!(Number(1.0).convert(find("rad").unwrap()), None);
    }
}
