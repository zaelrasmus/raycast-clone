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
    /// Base: the hertz.
    Frequency,
    /// Base: the watt.
    Power,
    /// Base: the joule.
    Energy,
    /// Base: the metre per second.
    Speed,
    /// Base: the square metre.
    Area,
    /// Base: the cubic metre.
    Volume,
    /// Base: the pascal.
    Pressure,
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
    /// What one of it is written as, for a unit written as a word: `1 day`
    /// beside `2 days`.
    pub singular: Option<&'static str>,
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

    /// `amount` written with this unit: `45°`, `2 rad`, `1 day`.
    pub fn write(&self, amount: &str) -> String {
        if let Some(singular) = self.singular
            && amount == "1"
        {
            return format!("{amount} {singular}");
        }
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
        singular: None,
    }
}

/// A span of the calendar, written as a word because dates are read in
/// words: `85 days`, `1 week`.
const fn calendar(
    factor: f64,
    singular: &'static str,
    display: &'static str,
    symbols: &'static [&'static str],
    names: &'static [&'static str],
) -> Unit {
    Unit {
        dimension: Dimension::Time,
        factor,
        offset: 0.0,
        display,
        symbols,
        names,
        loose: &[],
        singular: Some(singular),
    }
}

/// The calendar's spans, in seconds. A month and a year are the Gregorian
/// averages, which is right for a conversion; a date adds them by the
/// calendar instead (`Value::plus`).
pub const DAY: f64 = 86_400.0;
pub const WEEK: f64 = 7.0 * DAY;
pub const MONTH: f64 = 2_629_746.0;
pub const YEAR: f64 = 31_556_952.0;

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
        singular: None,
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
    calendar(DAY, "day", "days", &["d"], &["day", "days"]),
    calendar(WEEK, "week", "weeks", &["wk", "wks"], &["week", "weeks"]),
    calendar(MONTH, "month", "months", &[], &["month", "months"]),
    calendar(YEAR, "year", "years", &["yr", "yrs"], &["year", "years"]),
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
    // Frequency. Lower case is what is typed: `2.4 ghz`.
    unit(Frequency, 1.0, "Hz", &["Hz"], &["hertz", "hz"]),
    unit(Frequency, 1e3, "kHz", &["kHz"], &["kilohertz", "khz"]),
    unit(Frequency, 1e6, "MHz", &["MHz"], &["megahertz", "mhz"]),
    unit(Frequency, 1e9, "GHz", &["GHz"], &["gigahertz", "ghz"]),
    unit(Frequency, 1.0 / 60.0, "rpm", &["rpm"], &[]),
    // Power. No lower-case `mw`: a milliwatt and a megawatt are nine
    // orders apart.
    unit(Power, 1e-3, "mW", &["mW"], &["milliwatt", "milliwatts"]),
    unit(Power, 1.0, "W", &["W"], &["watt", "watts"]),
    unit(Power, 1e3, "kW", &["kW"], &["kilowatt", "kilowatts", "kw"]),
    unit(Power, 1e6, "MW", &["MW"], &["megawatt", "megawatts"]),
    unit(Power, 1e9, "GW", &["GW"], &["gigawatt", "gigawatts"]),
    // Mechanical horsepower.
    unit(Power, 745.699_871_582_270_2, "hp", &["hp"], &["horsepower"]),
    // Energy.
    unit(Energy, 1.0, "J", &["J"], &["joule", "joules"]),
    unit(
        Energy,
        1e3,
        "kJ",
        &["kJ"],
        &["kilojoule", "kilojoules", "kj"],
    ),
    unit(
        Energy,
        1e6,
        "MJ",
        &["MJ"],
        &["megajoule", "megajoules", "mj"],
    ),
    unit(Energy, 4.184, "cal", &["cal"], &["calorie", "calories"]),
    unit(
        Energy,
        4184.0,
        "kcal",
        &["kcal", "Cal"],
        &["kilocalorie", "kilocalories"],
    ),
    unit(Energy, 3600.0, "Wh", &["Wh"], &["wh"]),
    unit(Energy, 3.6e6, "kWh", &["kWh"], &["kwh"]),
    unit(Energy, 3.6e9, "MWh", &["MWh"], &[]),
    unit(
        Energy,
        1.602_176_634e-19,
        "eV",
        &["eV"],
        &["electronvolt", "electronvolts"],
    ),
    unit(Energy, 1055.05585262, "BTU", &["BTU", "Btu"], &["btu"]),
    // Speed. `km/h` and `m/s` are read whole when a word follows the
    // slash ([`find_per`]): `10 km / 2 h` is still a division.
    unit(Speed, 1.0, "m/s", &["m/s"], &[]),
    unit(Speed, 1.0 / 3.6, "km/h", &["km/h", "kmh", "kph"], &[]),
    unit(Speed, 0.3048, "ft/s", &["ft/s"], &[]),
    unit(Speed, 0.447_04, "mph", &["mph"], &[]),
    unit(
        Speed,
        1852.0 / 3600.0,
        "kn",
        &["kn", "kt"],
        &["knot", "knots"],
    ),
    // Area.
    unit(Area, 1e-6, "mm\u{b2}", &["mm\u{b2}", "mm2"], &[]),
    unit(Area, 1e-4, "cm\u{b2}", &["cm\u{b2}", "cm2"], &[]),
    unit(Area, 1.0, "m\u{b2}", &["m\u{b2}", "m2", "sqm"], &[]),
    unit(Area, 1e4, "ha", &["ha"], &["hectare", "hectares"]),
    unit(Area, 1e6, "km\u{b2}", &["km\u{b2}", "km2"], &[]),
    unit(
        Area,
        0.092_903_04,
        "ft\u{b2}",
        &["ft\u{b2}", "ft2", "sqft"],
        &[],
    ),
    unit(Area, 0.836_127_36, "yd\u{b2}", &["yd\u{b2}", "yd2"], &[]),
    unit(Area, 4046.8564224, "ac", &[], &["acre", "acres"]),
    unit(
        Area,
        2_589_988.110_336,
        "mi\u{b2}",
        &["mi\u{b2}", "mi2"],
        &[],
    ),
    // Volume. The cup, the pint and the gallon are the US ones.
    unit(
        Volume,
        1e-6,
        "mL",
        &["mL", "ml"],
        &["millilitre", "millilitres", "milliliter", "milliliters"],
    ),
    unit(
        Volume,
        1e-5,
        "cL",
        &["cL", "cl"],
        &["centilitre", "centilitres", "centiliter", "centiliters"],
    ),
    unit(
        Volume,
        1e-4,
        "dL",
        &["dL", "dl"],
        &["decilitre", "decilitres", "deciliter", "deciliters"],
    ),
    unit(
        Volume,
        1e-3,
        "L",
        &["L", "l"],
        &["litre", "litres", "liter", "liters"],
    ),
    unit(Volume, 1.0, "m\u{b3}", &["m\u{b3}", "m3"], &[]),
    unit(Volume, 1e-6, "cm\u{b3}", &["cm\u{b3}", "cm3", "cc"], &[]),
    unit(
        Volume,
        4.928_921_593_75e-6,
        "tsp",
        &["tsp"],
        &["teaspoon", "teaspoons"],
    ),
    unit(
        Volume,
        1.478_676_478_125e-5,
        "tbsp",
        &["tbsp"],
        &["tablespoon", "tablespoons"],
    ),
    unit(Volume, 2.957_352_956_25e-5, "fl oz", &["floz"], &[]),
    unit(Volume, 2.365_882_365e-4, "cup", &[], &["cup", "cups"]),
    unit(Volume, 4.731_764_73e-4, "pt", &["pt"], &["pint", "pints"]),
    unit(Volume, 9.463_529_46e-4, "qt", &["qt"], &["quart", "quarts"]),
    unit(
        Volume,
        3.785_411_784e-3,
        "gal",
        &["gal"],
        &["gallon", "gallons"],
    ),
    // Pressure.
    unit(Pressure, 1.0, "Pa", &["Pa"], &["pascal", "pascals"]),
    unit(
        Pressure,
        100.0,
        "hPa",
        &["hPa"],
        &["hectopascal", "hectopascals", "hpa"],
    ),
    unit(
        Pressure,
        1e3,
        "kPa",
        &["kPa"],
        &["kilopascal", "kilopascals", "kpa"],
    ),
    unit(
        Pressure,
        1e6,
        "MPa",
        &["MPa"],
        &["megapascal", "megapascals", "mpa"],
    ),
    unit(
        Pressure,
        100.0,
        "mbar",
        &["mbar"],
        &["millibar", "millibars"],
    ),
    unit(Pressure, 1e5, "bar", &["bar"], &["bars"]),
    unit(Pressure, 6894.757293168, "psi", &["psi"], &[]),
    unit(
        Pressure,
        101_325.0,
        "atm",
        &["atm"],
        &["atmosphere", "atmospheres"],
    ),
    unit(Pressure, 133.322_387_415, "mmHg", &["mmHg"], &["mmhg"]),
];

/// The unit a span of the calendar is written in: `days`.
pub fn calendar_unit(factor: f64) -> &'static Unit {
    UNITS
        .iter()
        .find(|unit| unit.dimension == Dimension::Time && unit.factor == factor)
        .expect("every span of the calendar is in the table")
}

/// The unit written `top/bottom`, if one is: `km/h`, `m/s`.
pub fn find_per(top: &str, bottom: &str) -> Option<&'static Unit> {
    UNITS.iter().find(|unit| {
        unit.symbols
            .iter()
            .any(|symbol| symbol.split_once('/') == Some((top, bottom)))
    })
}

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
    fn a_unit_over_another() {
        assert_eq!(find_per("km", "h").map(|unit| unit.display), Some("km/h"));
        assert_eq!(find_per("m", "s").map(|unit| unit.display), Some("m/s"));
        assert!(find_per("kg", "h").is_none());
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
