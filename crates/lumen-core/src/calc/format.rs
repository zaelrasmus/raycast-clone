//! Results written the way a person would write them.

use super::value::Value;

/// At or past this, a number is written in scientific notation: its digits
/// beyond the sixteenth are noise that only looks precise.
const LARGE: f64 = 1e15;
/// Below this, but not zero, too: `0.0000000123` hides its digits in zeros.
const SMALL: f64 = 1e-6;

/// Render a result the way a person would write it.
///
/// Floating point prints badly by default: `0.1 + 0.2` is `0.30000000000000004`
/// and `6 / 2` is `3`. Rounding to ten significant decimals kills the first
/// without visibly harming anything the user typed, and trimming the trailing
/// zeros keeps whole numbers looking whole. Very large and very small numbers
/// are written as `1.2676506e30`, which another calculator reads back.
pub fn format_number(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude >= LARGE || (magnitude < SMALL && value != 0.0) {
        let text = format!("{value:.9e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        return format!("{}e{exponent}", trimmed(mantissa));
    }
    if value == value.trunc() {
        return format!("{}", value as i64);
    }

    trimmed(&format!("{value:.10}")).to_string()
}

/// A value written with its unit, if it has one: `45°`, `2 rad`, `25%`.
pub fn format_value(value: Value) -> String {
    match value {
        Value::Number(number) => format_number(number),
        Value::Quantity { amount, unit } => unit.write(&format_number(amount)),
        Value::Percent(percent) => format!("{}%", format_number(percent)),
    }
}

/// `text` without the zeros a decimal point leaves behind, nor the point.
fn trimmed(text: &str) -> &str {
    if !text.contains('.') {
        return text;
    }
    text.trim_end_matches('0').trim_end_matches('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_whole_numbers_without_a_decimal_point() {
        assert_eq!(format_number(4.0), "4");
        assert_eq!(format_number(-7.0), "-7");
        assert_eq!(format_number(1024.0), "1024");
    }

    #[test]
    fn formats_fractions_without_floating_point_noise() {
        assert_eq!(format_number(4.5), "4.5");
        // The canonical embarrassment.
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(-0.000_000_000_01 * 0.5), "-5e-12");
    }

    #[test]
    fn very_large_and_very_small_in_scientific_notation() {
        assert_eq!(format_number(2f64.powi(100)), "1.2676506e30");
        assert_eq!(format_number(1e15), "1e15");
        assert_eq!(format_number(999_999_999_999_999.0), "999999999999999");
        assert_eq!(format_number(0.000_000_012_345), "1.2345e-8");
        assert_eq!(format_number(-1e-7), "-1e-7");
        assert_eq!(format_number(0.000_001), "0.000001");
        assert_eq!(format_number(0.0), "0");
    }
}
