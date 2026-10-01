//! Units, as data.
//!
//! A unit is a row: what it measures, how many of the base unit one of it
//! is, and the words it is written with. A new unit is a row and a test, not
//! code. Temperatures alone need an offset as well as a factor.
//!
//! **Symbols are case-sensitive** and names are not: `Mb` is not `MB`, and
//! `m` is not `M`, but `Metre` is `metre`.

/// What a unit measures. Two quantities can be added, compared or converted
/// only when they measure the same thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    /// Base: the radian.
    Angle,
}

#[derive(Debug, PartialEq)]
pub struct Unit {
    pub dimension: Dimension,
    /// How many of the dimension's base unit one of this is.
    pub factor: f64,
    /// Added after the factor, in the base unit. Zero for everything but
    /// temperatures.
    pub offset: f64,
    /// Written as a person would after a number: `45°`, `2 rad`.
    pub display: &'static str,
    /// Case-sensitive: `°`, `rad`.
    pub symbols: &'static [&'static str],
    /// Case-insensitive, plurals included: `degree`, `degrees`.
    pub names: &'static [&'static str],
}

impl Unit {
    /// `amount` of this unit, in the dimension's base unit.
    pub fn to_base(&self, amount: f64) -> f64 {
        amount * self.factor + self.offset
    }

    /// `amount` of the dimension's base unit, in this unit.
    pub fn of_base(&self, amount: f64) -> f64 {
        (amount - self.offset) / self.factor
    }

    /// Whether this is a temperature on a scale that does not start at zero,
    /// where `10 °C + 5 °F` has no single meaning.
    pub fn has_offset(&self) -> bool {
        self.offset != 0.0
    }

    /// `amount` written with this unit: `45°`, `2 rad`.
    pub fn write(&self, amount: &str) -> String {
        if self.display.starts_with('\u{b0}') {
            format!("{amount}{}", self.display)
        } else {
            format!("{amount} {}", self.display)
        }
    }
}

const fn unit(
    dimension: Dimension,
    factor: f64,
    display: &'static str,
    symbols: &'static [&'static str],
    names: &'static [&'static str],
) -> Unit {
    Unit {
        dimension,
        factor,
        offset: 0.0,
        display,
        symbols,
        names,
    }
}

use Dimension::*;

pub static UNITS: &[Unit] = &[
    unit(Angle, 1.0, "rad", &["rad"], &["radian", "radians"]),
    unit(
        Angle,
        std::f64::consts::PI / 180.0,
        "\u{b0}",
        &["\u{b0}", "deg"],
        &["degree", "degrees"],
    ),
];

/// The unit `word` names, if any.
pub fn find(word: &str) -> Option<&'static Unit> {
    UNITS
        .iter()
        .find(|unit| unit.symbols.contains(&word))
        .or_else(|| {
            UNITS.iter().find(|unit| {
                unit.names
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(word))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_match_exactly_and_names_in_any_case() {
        assert_eq!(find("deg").map(|unit| unit.display), Some("\u{b0}"));
        assert_eq!(find("Degrees").map(|unit| unit.display), Some("\u{b0}"));
        assert!(find("DEG").is_none());
        assert!(find("zip").is_none());
    }

    #[test]
    fn every_word_names_one_unit() {
        let mut seen = std::collections::HashSet::new();
        for unit in UNITS {
            for symbol in unit.symbols {
                assert!(seen.insert(symbol.to_string()), "symbol {symbol}");
            }
            for name in unit.names {
                assert!(seen.insert(name.to_lowercase()), "name {name}");
            }
        }
    }
}
