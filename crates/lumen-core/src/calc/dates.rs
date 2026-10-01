//! The calendar's words: months, weekdays, `today`.
//!
//! A date is written as people write it in English -- `25 dec`, `dec 25`,
//! `25th december 2027`, `friday`, `tomorrow` -- and never as numbers alone:
//! `12/05` is the fifth of December to some readers and the twelfth of May
//! to others, and it is a division to the calculator.
//!
//! A date with no year is the one meant: in `days until 25 dec` the next
//! one, in `days since 25 dec` the last. Today is passed in, never read
//! here, so a test does not depend on the day it runs.

use jiff::civil::{Date, Weekday};

/// Which way a date with no year is looked for from today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toward {
    /// Today or after: `until 25 dec`, `25 dec - today`.
    Next,
    /// Today or before: `since 1 jan`.
    Last,
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("monday", Weekday::Monday),
    ("tuesday", Weekday::Tuesday),
    ("wednesday", Weekday::Wednesday),
    ("thursday", Weekday::Thursday),
    ("friday", Weekday::Friday),
    ("saturday", Weekday::Saturday),
    ("sunday", Weekday::Sunday),
];

/// The month `word` names, 1 to 12: its full name or its first three
/// letters, and `sept`.
pub fn month(word: &str) -> Option<i8> {
    if word.eq_ignore_ascii_case("sept") {
        return Some(9);
    }
    let at = MONTHS.iter().position(|name| names(name, word))?;
    i8::try_from(at + 1).ok()
}

/// The weekday `word` names: its full name or its first three letters.
pub fn weekday(word: &str) -> Option<Weekday> {
    WEEKDAYS
        .iter()
        .find(|(name, _)| names(name, word))
        .map(|&(_, weekday)| weekday)
}

/// Whether `word` is `name` or its first three letters, in any case.
fn names(name: &str, word: &str) -> bool {
    name.eq_ignore_ascii_case(word) || (word.len() == 3 && name[..3].eq_ignore_ascii_case(word))
}

/// How many days from today `word` is: `yesterday`, `today`, `tomorrow`.
pub fn relative(word: &str) -> Option<i64> {
    [("yesterday", -1), ("today", 0), ("tomorrow", 1)]
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(word))
        .map(|(_, days)| days)
}

/// Whether `word` is what follows a day's number: `25th`, `1st`.
pub fn is_ordinal(word: &str) -> bool {
    ["st", "nd", "rd", "th"]
        .iter()
        .any(|suffix| suffix.eq_ignore_ascii_case(word))
}

/// The year a number written after a date is, if it can be one.
pub fn year(number: f64) -> Option<i16> {
    (number.fract() == 0.0 && (1000.0..=9999.0).contains(&number)).then_some(number as i16)
}

/// The day of the month a number is, if it can be one.
pub fn day(number: f64) -> Option<i8> {
    (number.fract() == 0.0 && (1.0..=31.0).contains(&number)).then_some(number as i8)
}

/// `day` of `month`, in `year` when one was written, or in the year
/// `toward` finds from `today`.
///
/// A day that does not exist in a year -- the 29th of February -- is
/// looked for in the years either side, as far as the next leap year.
pub fn on(month: i8, day: i8, year: Option<i16>, today: Date, toward: Toward) -> Option<Date> {
    if let Some(year) = year {
        return Date::new(year, month, day).ok();
    }
    let step: i16 = match toward {
        Toward::Next => 1,
        Toward::Last => -1,
    };
    (0..=8)
        .filter_map(|years| Date::new(today.year() + step * years, month, day).ok())
        .find(|date| match toward {
            Toward::Next => *date >= today,
            Toward::Last => *date <= today,
        })
}

/// The `weekday` after today, or before it: never today itself, since
/// `until friday` on a Friday means the next one.
pub fn weekday_from(weekday: Weekday, today: Date, toward: Toward) -> Option<Date> {
    match toward {
        Toward::Next => today.nth_weekday(1, weekday).ok(),
        Toward::Last => today.nth_weekday(-1, weekday).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    /// A Thursday.
    const TODAY: Date = date(2026, 10, 1);

    #[test]
    fn months_and_weekdays_by_name() {
        assert_eq!(month("Dec"), Some(12));
        assert_eq!(month("december"), Some(12));
        assert_eq!(month("sept"), Some(9));
        assert_eq!(month("may"), Some(5));
        assert_eq!(month("de"), None);
        assert_eq!(month("decem"), None);
        assert_eq!(weekday("fri"), Some(Weekday::Friday));
        assert_eq!(weekday("Friday"), Some(Weekday::Friday));
        assert_eq!(weekday("fr"), None);
    }

    #[test]
    fn a_date_with_no_year_is_the_one_meant() {
        assert_eq!(
            on(12, 25, None, TODAY, Toward::Next),
            Some(date(2026, 12, 25))
        );
        assert_eq!(on(1, 1, None, TODAY, Toward::Next), Some(date(2027, 1, 1)));
        assert_eq!(on(1, 1, None, TODAY, Toward::Last), Some(date(2026, 1, 1)));
        assert_eq!(
            on(12, 25, None, TODAY, Toward::Last),
            Some(date(2025, 12, 25))
        );
        assert_eq!(on(10, 1, None, TODAY, Toward::Next), Some(TODAY));
        assert_eq!(
            on(2, 29, None, TODAY, Toward::Next),
            Some(date(2028, 2, 29))
        );
        assert_eq!(
            on(2, 29, None, TODAY, Toward::Last),
            Some(date(2024, 2, 29))
        );
        assert_eq!(on(2, 30, None, TODAY, Toward::Next), None);
        assert_eq!(
            on(12, 25, Some(2020), TODAY, Toward::Next),
            Some(date(2020, 12, 25))
        );
    }

    #[test]
    fn a_weekday_is_never_today() {
        assert_eq!(
            weekday_from(Weekday::Friday, TODAY, Toward::Next),
            Some(date(2026, 10, 2))
        );
        assert_eq!(
            weekday_from(Weekday::Thursday, TODAY, Toward::Next),
            Some(date(2026, 10, 8))
        );
        assert_eq!(
            weekday_from(Weekday::Thursday, TODAY, Toward::Last),
            Some(date(2026, 9, 24))
        );
    }
}
