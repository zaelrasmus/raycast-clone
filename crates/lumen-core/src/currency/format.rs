//! Amounts as they are shown and as they are copied.
//!
//! Shown: in the currency's own units, grouped by thousands, with its symbol
//! where one is common -- `$7.06`, `AED 25.92`, `¥1,570`. Copied: the same
//! digits with nothing around them, because a copied amount is pasted into
//! a form, and a form does not want `$` or `,`.

use super::Currency;

/// The most decimals ever shown, however small the amount.
const MAX_DECIMALS: usize = 10;

/// Significant digits kept for an amount below one, so ten dong in dollars
/// is `$0.000379` and not `$0.00`.
const SIGNIFICANT: i32 = 3;

/// How many decimals `value` is written with, in `currency`.
fn decimals(value: f64, currency: &Currency) -> usize {
    let own = usize::from(currency.decimals);
    let magnitude = value.abs();
    if magnitude == 0.0 || magnitude >= 1.0 {
        return own;
    }
    // 0.000379 has its first significant digit at the fourth decimal.
    let first = (-magnitude.log10().floor()) as i32;
    let needed = (first + SIGNIFICANT - 1).max(0) as usize;
    needed.max(own).min(MAX_DECIMALS)
}

/// The digits of `value` in `currency`, ungrouped: `7.06`, `1570`.
pub fn plain(value: f64, currency: &Currency) -> String {
    let mut text = format!("{:.*}", decimals(value, currency), value);
    // The decimals a small amount needed beyond the currency's own are kept
    // only while they say something: 0.50 dollars, not 0.500.
    if let Some(point) = text.find('.') {
        let own = point + 1 + usize::from(currency.decimals);
        while text.len() > own && text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    // `-0.00` is zero.
    if text.starts_with('-') && text[1..].bytes().all(|b| b == b'0' || b == b'.') {
        return text[1..].to_string();
    }
    text
}

/// `value` as shown: `$7.06`, `AED 25.92`, `-€1,234.50`.
pub fn shown(value: f64, currency: &Currency) -> String {
    let digits = plain(value, currency);
    let (sign, digits) = match digits.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", digits.as_str()),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (digits, None),
    };

    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if let Some(fraction) = fraction {
        grouped.push('.');
        grouped.push_str(fraction);
    }

    match currency.symbol() {
        Some(symbol) => format!("{sign}{symbol}{grouped}"),
        None => format!("{sign}{} {grouped}", currency.code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::currency::currencies::find;

    fn currency(code: &str) -> &'static Currency {
        find(code).unwrap()
    }

    #[test]
    fn an_amount_is_written_in_its_currencys_units() {
        assert_eq!(shown(7.05823, currency("USD")), "$7.06");
        assert_eq!(shown(1570.4, currency("JPY")), "\u{a5}1,570");
        assert_eq!(shown(1.23456, currency("BHD")), "BHD 1.235");
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(shown(2565.5597, currency("AMD")), "AMD 2,565.56");
        assert_eq!(shown(1_234_567.0, currency("EUR")), "\u{20ac}1,234,567.00");
        assert_eq!(shown(100.0, currency("EUR")), "\u{20ac}100.00");
    }

    #[test]
    fn a_small_amount_keeps_three_significant_digits() {
        assert_eq!(shown(0.000_379_12, currency("USD")), "$0.000379");
        assert_eq!(shown(0.5, currency("USD")), "$0.50");
        assert_eq!(shown(0.123_456, currency("USD")), "$0.123");
    }

    #[test]
    fn a_negative_amount_keeps_its_sign_before_the_symbol() {
        assert_eq!(shown(-1234.5, currency("EUR")), "-\u{20ac}1,234.50");
        assert_eq!(plain(-0.001, currency("JPY")), "-0.001");
        assert_eq!(plain(-0.0, currency("USD")), "0.00");
    }

    #[test]
    fn what_is_copied_has_no_symbol_and_no_grouping() {
        assert_eq!(plain(2565.5597, currency("AMD")), "2565.56");
        assert_eq!(plain(1570.4, currency("JPY")), "1570");
    }
}
