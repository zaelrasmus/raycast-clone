//! Durations as people type them, and as the launcher writes them back.
//!
//! # What counts as a duration
//!
//! A number with a unit -- `25m`, `90 s`, `1.5h`, `1h30m`, `1h30` -- or a
//! clock reading, `1:30` (a minute and a half) or `1:02:03`. A leading
//! `timer` is allowed, and whatever follows the duration is the timer's name:
//! `25m write chapter 3`.
//!
//! A unit is required. `25` alone is a number, and the calculator's; guessing
//! minutes would put a timer above the answer to `25`.

use std::time::Duration;

/// The longest timer, so a typo such as `3000h` is not a timer for four months.
pub const MAX: Duration = Duration::from_secs(99 * 60 * 60);

/// A duration typed into the launcher, and the name typed after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typed {
    pub duration: Duration,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Unit {
    Seconds,
    Minutes,
    Hours,
}

impl Unit {
    fn millis(self) -> f64 {
        match self {
            Unit::Seconds => 1_000.0,
            Unit::Minutes => 60_000.0,
            Unit::Hours => 3_600_000.0,
        }
    }

    /// The unit a bare number attached to this one means: `1h30` is an hour
    /// and thirty minutes, `1m30` a minute and thirty seconds.
    fn next(self) -> Option<Unit> {
        match self {
            Unit::Hours => Some(Unit::Minutes),
            Unit::Minutes => Some(Unit::Seconds),
            Unit::Seconds => None,
        }
    }
}

/// Read `query` as a duration and a name, if it is one.
///
/// Instant and allocation-free until it is sure: it runs on every keystroke,
/// inside the frame budget (see `crate::providers`).
pub fn parse(query: &str) -> Option<Typed> {
    let mut rest = query.trim();
    if !rest.starts_with(|c: char| c.is_ascii_digit()) {
        rest = strip_word(rest, "timer")?.trim_start();
    }

    let (millis, rest) = clock(rest).or_else(|| units(rest))?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }

    let duration = Duration::from_millis(millis);
    if duration < Duration::from_secs(1) || duration > MAX {
        return None;
    }

    let name = rest.trim();
    Some(Typed {
        duration,
        name: (!name.is_empty()).then(|| name.to_string()),
    })
}

/// `text` without a leading `word` and the whitespace after it.
fn strip_word<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let head = text.get(..word.len())?;
    let tail = &text[word.len()..];
    (head.eq_ignore_ascii_case(word) && tail.starts_with(char::is_whitespace)).then_some(tail)
}

/// `m:ss` or `h:mm:ss`.
fn clock(text: &str) -> Option<(u64, &str)> {
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == ':'))
        .unwrap_or(text.len());
    let reading = &text[..end];
    let parts: Vec<&str> = reading.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    // The first part may be any length; the others are two digits and below
    // sixty, or it is not a clock reading.
    let mut total: u64 = 0;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() || part.len() > 6 {
            return None;
        }
        let value: u64 = part.parse().ok()?;
        if index > 0 && (part.len() != 2 || value >= 60) {
            return None;
        }
        total = total * 60 + value;
    }
    Some((total * 1_000, &text[end..]))
}

/// One or more `<number><unit>` parts, largest unit first.
fn units(text: &str) -> Option<(u64, &str)> {
    let mut total = 0.0;
    let mut rest = text;
    let mut last: Option<Unit> = None;

    loop {
        // Parts after the first may be separated by a space: `1h 30m`.
        let candidate = if last.is_some() {
            rest.trim_start()
        } else {
            rest
        };
        let Some((value, after_number)) = number(candidate) else {
            break;
        };
        let (unit, after) = match unit(after_number.trim_start()) {
            Some(found) => found,
            // A bare number counts only when it is attached to the part
            // before it: `1h30`, never `25m 3 things`.
            None => match last.and_then(Unit::next) {
                Some(unit) if candidate.len() == rest.len() && ends_word(after_number) => {
                    (unit, after_number)
                }
                _ => break,
            },
        };
        if last.is_some_and(|last| unit >= last) {
            break;
        }
        total += value * unit.millis();
        last = Some(unit);
        rest = after;
    }

    last.map(|_| (total.round() as u64, rest))
}

/// A decimal number at the start of `text`.
fn number(text: &str) -> Option<(f64, &str)> {
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let digits = &text[..end];
    if digits.is_empty() || digits.len() > 8 || !digits.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    Some((digits.parse().ok()?, &text[end..]))
}

/// A unit word at the start of `text`, ending where the letters do.
fn unit(text: &str) -> Option<(Unit, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(text.len());
    let unit = match text[..end].to_ascii_lowercase().as_str() {
        "h" | "hr" | "hrs" | "hour" | "hours" => Unit::Hours,
        "m" | "min" | "mins" | "minute" | "minutes" => Unit::Minutes,
        "s" | "sec" | "secs" | "second" | "seconds" => Unit::Seconds,
        _ => return None,
    };
    Some((unit, &text[end..]))
}

fn ends_word(text: &str) -> bool {
    text.is_empty() || text.starts_with(char::is_whitespace)
}

/// A duration in words: `25 min`, `1 h 30 min`, `45 s`.
pub fn describe(duration: Duration) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    let parts: Vec<String> = [(hours, "h"), (minutes, "min"), (seconds, "s")]
        .into_iter()
        .filter(|(value, _)| *value > 0)
        .map(|(value, unit)| format!("{value} {unit}"))
        .collect();
    if parts.is_empty() {
        "0 s".into()
    } else {
        parts.join(" ")
    }
}

/// Time left, as a clock reads it: `24:59`, `1:02:03`, `0:05`.
///
/// Rounded up, so a timer with half a second left says `0:01` rather than
/// `0:00` while it is still running.
pub fn clock_face(remaining: Duration) -> String {
    let total = remaining.as_millis().div_ceil(1_000) as u64;
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Whole hours, minutes and seconds: what the settings form's three fields
/// show.
pub fn split(duration: Duration) -> (u64, u64, u64) {
    let total = duration.as_secs();
    (total / 3600, total / 60 % 60, total % 60)
}

/// A duration as the launcher's grammar writes it, for `config.json`:
/// `25m`, `1h30m`, `1m30s`, `45s`. The form picks the numbers; the file
/// keeps a string a person can still read and edit.
pub fn written(duration: Duration) -> String {
    let (hours, minutes, seconds) = split(duration);
    let written: String = [(hours, "h"), (minutes, "m"), (seconds, "s")]
        .into_iter()
        .filter(|(value, _)| *value > 0)
        .map(|(value, unit)| format!("{value}{unit}"))
        .collect();
    if written.is_empty() {
        "0s".into()
    } else {
        written
    }
}

/// Time counted, as a stopwatch reads it: `0:00`, `12:34`, `1:02:03`.
///
/// Rounded down, the other way from [`clock_face`]: a stopwatch started half a
/// second ago has not counted a second yet.
pub fn stopwatch_face(elapsed: Duration) -> String {
    clock_face(Duration::from_secs(elapsed.as_secs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(query: &str) -> Option<u64> {
        parse(query).map(|typed| typed.duration.as_secs())
    }

    #[test]
    fn a_number_with_a_unit_is_a_duration() {
        assert_eq!(secs("25m"), Some(25 * 60));
        assert_eq!(secs("25 min"), Some(25 * 60));
        assert_eq!(secs("90s"), Some(90));
        assert_eq!(secs("2 hours"), Some(2 * 3600));
        assert_eq!(secs("1.5h"), Some(90 * 60));
        assert_eq!(secs("25M"), Some(25 * 60));
    }

    #[test]
    fn parts_add_up_largest_first() {
        assert_eq!(secs("1h30m"), Some(90 * 60));
        assert_eq!(secs("1h 30m"), Some(90 * 60));
        assert_eq!(secs("1h30"), Some(90 * 60));
        assert_eq!(secs("1m30"), Some(90));
        assert_eq!(secs("2m 5s"), Some(125));
    }

    #[test]
    fn a_clock_reading_is_minutes_and_seconds() {
        assert_eq!(secs("1:30"), Some(90));
        assert_eq!(secs("25:00"), Some(25 * 60));
        assert_eq!(secs("1:02:03"), Some(3723));
        assert_eq!(secs("1:5"), None);
        assert_eq!(secs("1:75"), None);
    }

    /// `25` is the calculator's. Guessing minutes would put a timer above
    /// the answer to a sum.
    #[test]
    fn a_bare_number_is_not_a_duration() {
        assert_eq!(parse("25"), None);
        assert_eq!(parse("12*7"), None);
    }

    #[test]
    fn words_that_start_like_a_unit_are_not_one() {
        assert_eq!(parse("25 minions"), None);
        assert_eq!(parse("3d printer"), None);
        assert_eq!(parse("2mx"), None);
        assert_eq!(parse("minutes"), None);
    }

    #[test]
    fn what_follows_the_duration_names_it() {
        assert_eq!(
            parse("25m write chapter 3"),
            Some(Typed {
                duration: Duration::from_secs(25 * 60),
                name: Some("write chapter 3".into()),
            })
        );
        // A bare number after a space is part of the name, not seconds.
        assert_eq!(
            parse("25m 3 things").unwrap().name.as_deref(),
            Some("3 things")
        );
    }

    #[test]
    fn the_word_timer_may_come_first() {
        assert_eq!(secs("timer 10m"), Some(600));
        assert_eq!(secs("Timer 10m tea"), Some(600));
        assert_eq!(parse("timer"), None);
        assert_eq!(parse("timers 10m"), None);
    }

    #[test]
    fn nothing_and_forever_are_refused() {
        assert_eq!(parse("0m"), None);
        assert_eq!(parse("0.1s"), None);
        assert_eq!(parse("3000h"), None);
        assert_eq!(secs("99h"), Some(99 * 3600));
    }

    #[test]
    fn units_out_of_order_end_the_duration() {
        // `30m 1h` is thirty minutes named "1h", not ninety minutes.
        assert_eq!(parse("30m 1h").unwrap().name.as_deref(), Some("1h"));
    }

    #[test]
    fn a_duration_is_described_in_words() {
        assert_eq!(describe(Duration::from_secs(25 * 60)), "25 min");
        assert_eq!(describe(Duration::from_secs(90 * 60)), "1 h 30 min");
        assert_eq!(describe(Duration::from_secs(45)), "45 s");
        assert_eq!(describe(Duration::from_secs(3601)), "1 h 1 s");
    }

    #[test]
    fn a_duration_is_written_back_so_the_parser_reads_it_the_same() {
        for (secs, text) in [
            (1500, "25m"),
            (5400, "1h30m"),
            (90, "1m30s"),
            (45, "45s"),
            (3661, "1h1m1s"),
        ] {
            let duration = Duration::from_secs(secs);
            assert_eq!(written(duration), text);
            assert_eq!(parse(text).unwrap().duration, duration, "{text}");
        }
        assert_eq!(split(Duration::from_secs(3661)), (1, 1, 1));
    }

    #[test]
    fn time_counted_rounds_down() {
        assert_eq!(stopwatch_face(Duration::from_millis(999)), "0:00");
        assert_eq!(stopwatch_face(Duration::from_millis(61_500)), "1:01");
        assert_eq!(stopwatch_face(Duration::from_secs(3723)), "1:02:03");
    }

    #[test]
    fn time_left_reads_like_a_clock_and_rounds_up() {
        assert_eq!(clock_face(Duration::from_secs(25 * 60)), "25:00");
        assert_eq!(clock_face(Duration::from_millis(1_499_001)), "25:00");
        assert_eq!(clock_face(Duration::from_millis(1_499_000)), "24:59");
        assert_eq!(clock_face(Duration::from_millis(500)), "0:01");
        assert_eq!(clock_face(Duration::ZERO), "0:00");
        assert_eq!(clock_face(Duration::from_secs(3723)), "1:02:03");
    }
}
