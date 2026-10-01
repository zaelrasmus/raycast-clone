//! Results written the way a person would write them.

/// Render a result the way a person would write it.
///
/// Floating point prints badly by default: `0.1 + 0.2` is `0.30000000000000004`
/// and `6 / 2` is `3`. Rounding to ten significant decimals kills the first
/// without visibly harming anything the user typed, and trimming the trailing
/// zeros keeps whole numbers looking whole.
pub fn format_number(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }

    let mut text = format!("{value:.10}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
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
    }
}
