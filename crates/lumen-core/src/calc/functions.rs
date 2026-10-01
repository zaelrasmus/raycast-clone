//! Functions and constants, by name.
//!
//! Names are case-insensitive. A function is always called with
//! parentheses -- `sqrt(16)`, never `sqrt 16` -- so a function name alone is
//! a word like any other, and declines.

use super::units::Dimension;
use super::value::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Function {
    Sqrt,
    Cbrt,
    Abs,
    Round,
    Floor,
    Ceil,
    Ln,
    Log10,
    Log2,
    Exp,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
}

use Function::*;

const FUNCTIONS: &[(&str, Function)] = &[
    ("sqrt", Sqrt),
    ("cbrt", Cbrt),
    ("abs", Abs),
    ("round", Round),
    ("floor", Floor),
    ("ceil", Ceil),
    ("ln", Ln),
    // `log` is base ten, as on a calculator's key; `ln` is the natural one.
    ("log", Log10),
    ("log10", Log10),
    ("log2", Log2),
    ("exp", Exp),
    ("sin", Sin),
    ("cos", Cos),
    ("tan", Tan),
    ("asin", Asin),
    ("arcsin", Asin),
    ("acos", Acos),
    ("arccos", Acos),
    ("atan", Atan),
    ("arctan", Atan),
    ("sinh", Sinh),
    ("cosh", Cosh),
    ("tanh", Tanh),
];

const CONSTANTS: &[(&str, f64)] = &[
    ("pi", std::f64::consts::PI),
    ("\u{3c0}", std::f64::consts::PI),
    ("tau", std::f64::consts::TAU),
    ("\u{3c4}", std::f64::consts::TAU),
    ("e", std::f64::consts::E),
];

/// What is left of a trigonometric zero after rounding π.
const NOISE: f64 = 1e-12;

pub fn function(word: &str) -> Option<Function> {
    FUNCTIONS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(word))
        .map(|&(_, function)| function)
}

pub fn constant(word: &str) -> Option<f64> {
    CONSTANTS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(word) || *name == word)
        .map(|&(_, value)| value)
}

impl Function {
    /// `self` applied to `argument`, or `None` when it has no meaning there.
    ///
    /// Trigonometry takes an angle -- `sin(45 deg)` -- or a plain number in
    /// radians. Rounding keeps a quantity's unit: `round(5.6 km)` is `6 km`.
    /// Everything else takes plain numbers only.
    pub fn apply(self, argument: Value) -> Option<Value> {
        if matches!(self, Sin | Cos | Tan) {
            let radians = match argument {
                Value::Number(radians) => radians,
                Value::Quantity { amount, unit } if unit.dimension == Dimension::Angle => {
                    unit.to_base(amount)
                }
                Value::Quantity { .. } | Value::Percent(_) => return None,
            };
            let result = match self {
                Sin => radians.sin(),
                Cos => radians.cos(),
                _ => radians.tan(),
            };
            // `sin(180 deg)` is zero, not the 1.2e-16 that π's rounding
            // leaves: the noise is snapped where it is made, so a tiny
            // number typed on purpose is still written as one.
            return Some(Value::Number(if result.abs() < NOISE {
                0.0
            } else {
                result
            }));
        }
        if matches!(self, Abs | Round | Floor | Ceil) {
            return Some(argument.map(match self {
                Abs => f64::abs,
                Round => f64::round,
                Floor => f64::floor,
                _ => f64::ceil,
            }));
        }
        let Value::Number(x) = argument else {
            return None;
        };
        Some(Value::Number(match self {
            Sqrt => x.sqrt(),
            Cbrt => x.cbrt(),
            Ln => x.ln(),
            Log10 => x.log10(),
            Log2 => x.log2(),
            Exp => x.exp(),
            Asin => x.asin(),
            Acos => x.acos(),
            Atan => x.atan(),
            Sinh => x.sinh(),
            Cosh => x.cosh(),
            Tanh => x.tanh(),
            Sin | Cos | Tan | Abs | Round | Floor | Ceil => unreachable!("handled above"),
        }))
    }
}

/// `n!` for a whole `n` from 0 to 170; past that it is not a finite `f64`.
pub fn factorial(n: f64) -> Option<f64> {
    (n.fract() == 0.0 && (0.0..=170.0).contains(&n))
        .then(|| (1..=n as u32).map(f64::from).product())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_any_case() {
        assert_eq!(function("SQRT"), Some(Sqrt));
        assert_eq!(function("log"), Some(Log10));
        assert_eq!(constant("Pi"), Some(std::f64::consts::PI));
        assert_eq!(function("sqr"), None);
        assert_eq!(constant("p"), None);
    }

    #[test]
    fn factorials_are_whole_and_bounded() {
        assert_eq!(factorial(0.0), Some(1.0));
        assert_eq!(factorial(5.0), Some(120.0));
        assert!(factorial(170.0).is_some_and(f64::is_finite));
        assert_eq!(factorial(171.0), None);
        assert_eq!(factorial(2.5), None);
        assert_eq!(factorial(-1.0), None);
    }
}
