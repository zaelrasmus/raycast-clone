//! Units, as data.
//!
//! A unit is a row: what it measures, how many of the base unit one of it
//! is, and the words it is written with. A new unit is a row and a test, not
//! code. Temperatures alone need an offset as well as a factor.
//!
//! Three kinds of word, tried in this order:
//!
//! - **Symbols are case-sensitive**: `Mb` is a megabit and `MB` a megabyte,
//!   `m` a metre.
//! - **Names are not**, and include what people type in lower case:
//!   `Metre`, `metres`, and `gb` -- which, written in a launcher, means the
//!   size of a file, so bytes.
//! - **Loose words** are letters too short to stand alone: `c`, `f`, `k`.
//!   `100 f in c` is plainly a temperature, but `2f + 3` or `5k` is not, so
//!   the parser accepts them only in a conversion.

/// What a unit measures. Two quantities can be added, compared or converted
/// only when they measure the same thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    /// Base: the metre.
    Length,
    /// Base: the kilogram.
    Mass,
    /// Base: the second.
    Time,
    /// Base: the kelvin.
    Temperature,
    /// Base: the byte.
    Data,
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
    /// Case-insensitive, and only in a conversion: `c`, `f`.
    pub loose: &'static [&'static str],
}

/// A unit, and whether it was named by a loose word.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Found {
    pub unit: &'static Unit,
    pub loose: bool,
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
        loose: &[],
    }
}

/// A temperature: `offset` is where its zero sits, in kelvin.
const fn temperature(
    factor: f64,
    offset: f64,
    display: &'static str,
    symbols: &'static [&'static str],
    names: &'static [&'static str],
    loose: &'static [&'static str],
) -> Unit {
    Unit {
        dimension: Dimension::Temperature,
        factor,
        offset,
        display,
        symbols,
        names,
        loose,
    }
}

use Dimension::*;

/// A bit is an eighth of the base, the byte.
const BIT: f64 = 0.125;

pub static UNITS: &[Unit] = &[
    // Length. No `in` for the inch: it is the word conversions are written
    // with, and `5 in in cm` reads as a typo.
    unit(
        Length,
        1e-9,
        "nm",
        &["nm"],
        &["nanometre", "nanometres", "nanometer", "nanometers"],
    ),
    unit(
        Length,
        1e-6,
        "\u{b5}m",
        &["\u{b5}m", "\u{3bc}m", "um"],
        &[
            "micrometre",
            "micrometres",
            "micrometer",
            "micrometers",
            "micron",
            "microns",
        ],
    ),
    unit(
        Length,
        1e-3,
        "mm",
        &["mm"],
        &["millimetre", "millimetres", "millimeter", "millimeters"],
    ),
    unit(
        Length,
        1e-2,
        "cm",
        &["cm"],
        &["centimetre", "centimetres", "centimeter", "centimeters"],
    ),
    unit(
        Length,
        1.0,
        "m",
        &["m"],
        &["metre", "metres", "meter", "meters"],
    ),
    unit(
        Length,
        1e3,
        "km",
        &["km"],
        &["kilometre", "kilometres", "kilometer", "kilometers"],
    ),
    unit(Length, 0.0254, "in", &[], &["inch", "inches"]),
    unit(Length, 0.3048, "ft", &["ft"], &["foot", "feet"]),
    unit(Length, 0.9144, "yd", &["yd"], &["yard", "yards"]),
    unit(Length, 1609.344, "mi", &["mi"], &["mile", "miles"]),
    unit(Length, 1852.0, "nmi", &["nmi"], &[]),
    // Mass.
    unit(Mass, 1e-6, "mg", &["mg"], &["milligram", "milligrams"]),
    unit(Mass, 1e-3, "g", &["g"], &["gram", "grams"]),
    unit(
        Mass,
        1.0,
        "kg",
        &["kg"],
        &["kilogram", "kilograms", "kilo", "kilos"],
    ),
    // The metric tonne. `ton` is left out: it is a different weight on
    // either side of the Atlantic.
    unit(Mass, 1e3, "t", &["t"], &["tonne", "tonnes"]),
    unit(Mass, 0.028_349_523_125, "oz", &["oz"], &["ounce", "ounces"]),
    unit(
        Mass,
        0.453_592_37,
        "lb",
        &["lb", "lbs"],
        &["pound", "pounds"],
    ),
    unit(Mass, 6.350_293_18, "st", &[], &["stone", "stones"]),
    // Time. A month and a year are the Gregorian averages; a date is the
    // place for the exact length of a particular one.
    unit(Time, 1e-9, "ns", &["ns"], &["nanosecond", "nanoseconds"]),
    unit(
        Time,
        1e-6,
        "\u{b5}s",
        &["\u{b5}s", "\u{3bc}s", "us"],
        &["microsecond", "microseconds"],
    ),
    unit(Time, 1e-3, "ms", &["ms"], &["millisecond", "milliseconds"]),
    unit(
        Time,
        1.0,
        "s",
        &["s", "sec", "secs"],
        &["second", "seconds"],
    ),
    unit(Time, 60.0, "min", &["min", "mins"], &["minute", "minutes"]),
    unit(Time, 3600.0, "h", &["h", "hr", "hrs"], &["hour", "hours"]),
    unit(Time, 86_400.0, "d", &["d"], &["day", "days"]),
    unit(Time, 604_800.0, "wk", &["wk", "wks"], &["week", "weeks"]),
    unit(Time, 2_629_746.0, "mo", &[], &["month", "months"]),
    unit(Time, 31_556_952.0, "yr", &["yr", "yrs"], &["year", "years"]),
    // Temperature.
    temperature(1.0, 0.0, "K", &["K"], &["kelvin", "kelvins"], &["k"]),
    temperature(
        1.0,
        273.15,
        "\u{b0}C",
        &["\u{b0}C", "\u{b0}c"],
        &["celsius", "centigrade"],
        &["c"],
    ),
    temperature(
        5.0 / 9.0,
        273.15 - 32.0 * 5.0 / 9.0,
        "\u{b0}F",
        &["\u{b0}F", "\u{b0}f"],
        &["fahrenheit"],
        &["f"],
    ),
    // Data. SI and IEC apart: a gigabyte is 10⁹ bytes, a gibibyte 2³⁰.
    unit(Data, BIT, "bit", &["b"], &["bit", "bits"]),
    unit(Data, 1.0, "B", &["B"], &["byte", "bytes"]),
    unit(
        Data,
        1e3,
        "kB",
        &["kB", "KB"],
        &["kilobyte", "kilobytes", "kb"],
    ),
    unit(Data, 1e6, "MB", &["MB"], &["megabyte", "megabytes", "mb"]),
    unit(Data, 1e9, "GB", &["GB"], &["gigabyte", "gigabytes", "gb"]),
    unit(Data, 1e12, "TB", &["TB"], &["terabyte", "terabytes", "tb"]),
    unit(Data, 1e15, "PB", &["PB"], &["petabyte", "petabytes", "pb"]),
    unit(
        Data,
        1024.0,
        "KiB",
        &["KiB"],
        &["kibibyte", "kibibytes", "kib"],
    ),
    unit(
        Data,
        1_048_576.0,
        "MiB",
        &["MiB"],
        &["mebibyte", "mebibytes", "mib"],
    ),
    unit(
        Data,
        1_073_741_824.0,
        "GiB",
        &["GiB"],
        &["gibibyte", "gibibytes", "gib"],
    ),
    unit(
        Data,
        1_099_511_627_776.0,
        "TiB",
        &["TiB"],
        &["tebibyte", "tebibytes", "tib"],
    ),
    unit(
        Data,
        1e3 * BIT,
        "kbit",
        &["Kb", "kbit"],
        &["kilobit", "kilobits"],
    ),
    unit(
        Data,
        1e6 * BIT,
        "Mbit",
        &["Mb", "Mbit"],
        &["megabit", "megabits"],
    ),
    unit(
        Data,
        1e9 * BIT,
        "Gbit",
        &["Gb", "Gbit"],
        &["gigabit", "gigabits"],
    ),
    unit(
        Data,
        1e12 * BIT,
        "Tbit",
        &["Tb", "Tbit"],
        &["terabit", "terabits"],
    ),
    // Angle.
    unit(Angle, 1.0, "rad", &["rad"], &["radian", "radians"]),
    unit(
        Angle,
        std::f64::consts::PI / 180.0,
        "\u{b0}",
        &["\u{b0}", "deg"],
        &["degree", "degrees"],
    ),
];

/// The unit `word` names, if any: by symbol, then by name, then loosely.
pub fn find(word: &str) -> Option<Found> {
    let strict = |unit: &'static Unit| Found { unit, loose: false };
    let any_case = |words: &[&str]| words.iter().any(|name| name.eq_ignore_ascii_case(word));
    UNITS
        .iter()
        .find(|unit| unit.symbols.contains(&word))
        .map(strict)
        .or_else(|| UNITS.iter().find(|unit| any_case(unit.names)).map(strict))
        .or_else(|| {
            UNITS
                .iter()
                .find(|unit| any_case(unit.loose))
                .map(|unit| Found { unit, loose: true })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(word: &str) -> Option<&'static str> {
        find(word).map(|found| found.unit.display)
    }

    #[test]
    fn symbols_match_exactly_and_names_in_any_case() {
        assert_eq!(display("deg"), Some("\u{b0}"));
        assert_eq!(display("Degrees"), Some("\u{b0}"));
        assert_eq!(display("KILOMETRES"), Some("km"));
        assert_eq!(display("DEG"), None);
        assert_eq!(display("zip"), None);
    }

    /// The case of a data unit is the difference between bits and bytes, and
    /// a lower-case one typed in a launcher is the size of a file.
    #[test]
    fn bits_and_bytes() {
        assert_eq!(display("MB"), Some("MB"));
        assert_eq!(display("Mb"), Some("Mbit"));
        assert_eq!(display("mb"), Some("MB"));
        assert_eq!(display("Gb"), Some("Gbit"));
        assert_eq!(display("gb"), Some("GB"));
        assert_eq!(display("GiB"), Some("GiB"));
        assert_eq!(display("gib"), Some("GiB"));
        assert_eq!(display("b"), Some("bit"));
        assert_eq!(display("B"), Some("B"));
    }

    #[test]
    fn loose_words_are_marked() {
        let found = find("c").unwrap();
        assert_eq!((found.unit.display, found.loose), ("\u{b0}C", true));
        assert!(find("F").unwrap().loose);
        assert!(!find("K").unwrap().loose, "the kelvin's own symbol");
        assert!(!find("\u{b0}C").unwrap().loose);
    }

    #[test]
    fn every_word_names_one_unit() {
        let mut symbols = std::collections::HashSet::new();
        let mut words = std::collections::HashSet::new();
        for unit in UNITS {
            for symbol in unit.symbols {
                assert!(symbols.insert(*symbol), "symbol {symbol}");
            }
            for word in unit.names.iter().chain(unit.loose) {
                assert!(words.insert(word.to_lowercase()), "word {word}");
            }
        }
    }

    #[test]
    fn temperatures_meet_where_they_should() {
        let celsius = find("celsius").unwrap().unit;
        let fahrenheit = find("fahrenheit").unwrap().unit;
        let kelvin = find("K").unwrap().unit;
        assert!((fahrenheit.of_base(celsius.to_base(100.0)) - 212.0).abs() < 1e-9);
        assert!((fahrenheit.of_base(celsius.to_base(-40.0)) + 40.0).abs() < 1e-9);
        assert!((kelvin.of_base(celsius.to_base(0.0)) - 273.15).abs() < 1e-9);
    }
}
