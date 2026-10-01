//! World Clock (F13): the time anywhere, compared with here.
//!
//! The places are [`crate::places`]; the zone rules are jiff's bundled IANA
//! database, the same one the user's own zone is read from, so the two
//! sides of a difference cannot disagree. Nothing here reads a clock: `now`
//! is passed in, so every rule is a unit test.

pub mod query;
pub mod session;
pub mod store;
mod sun;

use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use lumen_platform::LocalTime;
use serde::{Deserialize, Serialize};

pub use sun::{Daylight, daylight};

/// The `clock` section of `config.json` (F13).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ClockConfig {
    /// Which value leads a row.
    pub row: RowLayout,
    /// How the difference from here is written.
    pub difference_format: DifferenceFormat,
    /// When the user's working day and a place's overlap (slice 4).
    pub overlap: Overlap,
}

impl ClockConfig {
    /// The working hours to compare, when the overlap is to be shown: on,
    /// with *Difference first*, and hours that parse.
    pub fn working_hours(&self) -> Option<(u16, u16)> {
        if !self.overlap.enabled || self.row != RowLayout::Difference {
            return None;
        }
        self.overlap.hours()
    }
}

/// Working hours, compared with each place's (F13, slice 4).
///
/// The same hours are assumed there as here -- nine to five is nine to five
/// in Vancouver too -- which is the question a remote meeting asks: when
/// are both of us at work?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Overlap {
    pub enabled: bool,
    /// `HH:MM`, 24-hour, as written in the file whatever the system's clock.
    pub start: String,
    pub end: String,
}

impl Default for Overlap {
    fn default() -> Self {
        Self {
            enabled: false,
            start: "09:00".into(),
            end: "17:00".into(),
        }
    }
}

impl Overlap {
    /// The hours as minutes since midnight, when both parse and the day
    /// starts before it ends.
    pub fn hours(&self) -> Option<(u16, u16)> {
        let (start, end) = (minutes(&self.start)?, minutes(&self.end)?);
        (start < end).then_some((start, end))
    }
}

/// `09:00` as 540. Anything else is `None`.
pub fn minutes(text: &str) -> Option<u16> {
    let (hours, minutes) = text.trim().split_once(':')?;
    if hours.is_empty() || hours.len() > 2 || minutes.len() != 2 {
        return None;
    }
    let (hours, minutes): (u16, u16) = (hours.parse().ok()?, minutes.parse().ok()?);
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

/// When a working day `start..end` (minutes since midnight) here overlaps
/// the same working day at a place `difference` seconds ahead: a span of
/// the user's own day, or `None`. The day there is tried on either side of
/// midnight here, so Seoul's morning is found in Santo Domingo's evening
/// before.
pub fn overlap(start: u16, end: u16, difference: i32) -> Option<(u16, u16)> {
    let shift = difference.div_euclid(60);
    let (start, end) = (i32::from(start), i32::from(end));
    [-1440, 0, 1440]
        .into_iter()
        .map(|day| {
            let (theirs_start, theirs_end) = (start - shift + day, end - shift + day);
            (start.max(theirs_start), end.min(theirs_end))
        })
        .filter(|(from, to)| from < to)
        .max_by_key(|(from, to)| to - from)
        .map(|(from, to)| (from as u16, to as u16))
}

/// A minute of the user's day as a moment the platform writes, on `day`.
pub fn at_minute(day: &Zoned, minute: u16) -> LocalTime {
    LocalTime {
        hour: (minute / 60) as u8,
        minute: (minute % 60) as u8,
        second: 0,
        ..local_time(day)
    }
}

/// Which value leads a row: drawn like the title, at the right edge, with
/// the other dimmed beside it. Which reads better is a matter of feel, so
/// it is the user's to choose.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RowLayout {
    /// `09:14  2 h behind`: how far the place is from here.
    #[default]
    Difference,
    /// `2 h behind  09:14`: what time it is there.
    Time,
}

/// How a difference is written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DifferenceFormat {
    /// `2 h behind`, `Same time`: no sign to work out the meaning of.
    #[default]
    Words,
    /// `\u{2212}2 h`, `+5:30 h`.
    Signed,
}

/// Seconds since the epoch, for the driver to pass back as `now`.
pub fn now() -> Timestamp {
    Timestamp::now()
}

/// The user's own zone, as the system has it. Reads the system, so it is
/// called off the UI thread.
pub fn here() -> TimeZone {
    TimeZone::system()
}

/// How long until the minute turns over after `now`, when a clock showing
/// minutes next has something new to say.
pub fn until_next_minute(now: Timestamp) -> std::time::Duration {
    let into = now.as_millisecond().rem_euclid(60_000);
    std::time::Duration::from_millis((60_000 - into) as u64)
}

/// The zone called `name`, or UTC when the database does not know it -- a
/// zone the catalogue names but this jiff does not, which a refresh of one
/// without the other could make.
pub fn zone(name: &str) -> TimeZone {
    TimeZone::get(name).unwrap_or_else(|error| {
        log::warn!("no time zone {name:?}: {error}");
        TimeZone::UTC
    })
}

/// A moment as the platform writes it (`Platform::format_date`).
pub fn local_time(zoned: &Zoned) -> LocalTime {
    LocalTime {
        year: zoned.year(),
        month: zoned.month() as u8,
        day: zoned.day() as u8,
        weekday: zoned.weekday().to_sunday_zero_offset() as u8,
        hour: zoned.hour() as u8,
        minute: zoned.minute() as u8,
        second: zoned.second() as u8,
    }
}

/// `UTC\u{2212}7`, `UTC+5:30`, `UTC+0`.
pub fn offset(seconds: i32) -> String {
    format!("UTC{}", signed(seconds))
}

/// `\u{2212}7`, `+5:30`, `+0`: an offset or a difference with its sign, in
/// hours and, when there are any, minutes.
fn signed(seconds: i32) -> String {
    let sign = if seconds < 0 { '\u{2212}' } else { '+' };
    let minutes = seconds.unsigned_abs() / 60;
    match minutes % 60 {
        0 => format!("{sign}{}", minutes / 60),
        rest => format!("{sign}{}:{rest:02}", minutes / 60),
    }
}

/// The range of offsets a country or a continent spans:
/// `UTC\u{2212}10 \u{2026} \u{2212}4`. One offset is just that offset.
pub fn offset_range(lowest: i32, highest: i32) -> String {
    if lowest == highest {
        offset(lowest)
    } else {
        format!("{} \u{2026} {}", offset(lowest), signed(highest))
    }
}

/// How far `there` is from `here`, in seconds: positive is ahead.
pub fn difference(here: &Zoned, there: &Zoned) -> i32 {
    there.offset().seconds() - here.offset().seconds()
}

/// A difference written for a row: `2 h behind`, `5 h 30 min ahead`,
/// `Same time`; or `\u{2212}2 h`, `+5:30 h`, `\u{b1}0 h`.
pub fn describe(seconds: i32, format: DifferenceFormat) -> String {
    let minutes = seconds.unsigned_abs() / 60;
    let (hours, minutes) = (minutes / 60, minutes % 60);
    match format {
        DifferenceFormat::Words => {
            if seconds == 0 {
                return "Same time".into();
            }
            let amount = match (hours, minutes) {
                (0, minutes) => format!("{minutes} min"),
                (hours, 0) => format!("{hours} h"),
                (hours, minutes) => format!("{hours} h {minutes} min"),
            };
            let side = if seconds < 0 { "behind" } else { "ahead" };
            format!("{amount} {side}")
        }
        DifferenceFormat::Signed if seconds == 0 => "\u{b1}0 h".into(),
        DifferenceFormat::Signed => format!("{} h", signed(seconds)),
    }
}

/// Whether `there`'s date is not `here`'s: *Tomorrow* or *Yesterday*.
pub fn day(here: &Zoned, there: &Zoned) -> Option<&'static str> {
    match there.date().cmp(&here.date()) {
        std::cmp::Ordering::Greater => Some("Tomorrow"),
        std::cmp::Ordering::Less => Some("Yesterday"),
        std::cmp::Ordering::Equal => None,
    }
}

/// The next time `zone`'s clocks change after `now`, within a year, and by
/// how much: the next daylight-saving change, or a government's.
pub fn next_change(zone: &TimeZone, now: Timestamp) -> Option<(Timestamp, i32)> {
    let current = zone.to_offset(now).seconds();
    let horizon = now
        .checked_add(jiff::SignedDuration::from_hours(24 * 366))
        .ok()?;
    zone.following(now)
        .take_while(|transition| transition.timestamp() < horizon)
        // A change of name alone moves no clock.
        .find(|transition| transition.offset().seconds() != current)
        .map(|transition| {
            (
                transition.timestamp(),
                transition.offset().seconds() - current,
            )
        })
}

/// A clock change, for the pane: `Clocks go back 1 h on Sun 1 Nov 2026`.
pub fn describe_change(change: Option<(Timestamp, i32)>, zone: &TimeZone) -> String {
    let Some((when, by)) = change else {
        return "No change in the next year".into();
    };
    let way = if by > 0 { "forward" } else { "back" };
    let amount = describe(by.abs(), DifferenceFormat::Words)
        .trim_end_matches(" ahead")
        .to_string();
    let date = when.to_zoned(zone.clone()).strftime("%a %-d %b %Y");
    format!("Clocks go {way} {amount} on {date}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn zoned(when: &str, name: &str) -> Zoned {
        at(when).to_zoned(zone(name))
    }

    #[test]
    fn a_difference_is_written_in_words_or_signed() {
        use DifferenceFormat::*;
        assert_eq!(describe(0, Words), "Same time");
        assert_eq!(describe(-2 * 3600, Words), "2 h behind");
        assert_eq!(describe(5 * 3600 + 1800, Words), "5 h 30 min ahead");
        assert_eq!(describe(45 * 60, Words), "45 min ahead");
        assert_eq!(describe(0, Signed), "\u{b1}0 h");
        assert_eq!(describe(-2 * 3600, Signed), "\u{2212}2 h");
        assert_eq!(describe(5 * 3600 + 1800, Signed), "+5:30 h");
    }

    #[test]
    fn offsets_and_their_ranges() {
        assert_eq!(offset(-7 * 3600), "UTC\u{2212}7");
        assert_eq!(offset(5 * 3600 + 1800), "UTC+5:30");
        assert_eq!(offset(0), "UTC+0");
        assert_eq!(
            offset_range(-10 * 3600, -4 * 3600),
            "UTC\u{2212}10 \u{2026} \u{2212}4"
        );
        assert_eq!(offset_range(3600, 3600), "UTC+1");
    }

    /// The difference follows each side's daylight saving: Los Angeles is
    /// three hours behind Santo Domingo in summer and four in winter.
    /// Vancouver, an hour north, no longer is: British Columbia keeps
    /// UTC\u{2212}7 from November 2026, which the database knows.
    #[test]
    fn the_difference_is_the_offsets_at_that_moment() {
        let summer = "2026-07-01T12:00:00Z";
        let here = zoned(summer, "America/Santo_Domingo");
        assert_eq!(
            difference(&here, &zoned(summer, "America/Los_Angeles")),
            -3 * 3600
        );
        let winter = "2026-12-01T12:00:00Z";
        let here = zoned(winter, "America/Santo_Domingo");
        assert_eq!(
            difference(&here, &zoned(winter, "America/Los_Angeles")),
            -4 * 3600
        );
        assert_eq!(
            difference(&here, &zoned(winter, "America/Vancouver")),
            -3 * 3600
        );
        assert_eq!(
            difference(&here, &zoned(winter, "Asia/Kolkata")),
            9 * 3600 + 1800
        );
    }

    #[test]
    fn a_date_other_than_here_is_named() {
        // 22:00 in Santo Domingo is 11:00 tomorrow in Seoul.
        let when = "2026-10-01T02:00:00Z";
        let here = zoned(when, "America/Santo_Domingo");
        assert_eq!(day(&here, &zoned(when, "Asia/Seoul")), Some("Tomorrow"));
        assert_eq!(day(&here, &zoned(when, "America/Vancouver")), None);
        let here = zoned(when, "Asia/Seoul");
        assert_eq!(
            day(&here, &zoned(when, "America/Vancouver")),
            Some("Yesterday")
        );
    }

    #[test]
    fn the_next_clock_change_is_found_and_described() {
        let los_angeles = zone("America/Los_Angeles");
        let change = next_change(&los_angeles, at("2026-09-30T12:00:00Z"));
        let (when, by) = change.unwrap();
        assert_eq!(by, -3600);
        assert_eq!(when, at("2026-11-01T09:00:00Z"));
        assert_eq!(
            describe_change(change, &los_angeles),
            "Clocks go back 1 h on Sun 1 Nov 2026"
        );
        assert_eq!(
            describe_change(
                next_change(&los_angeles, at("2026-12-01T12:00:00Z")),
                &los_angeles
            ),
            "Clocks go forward 1 h on Sun 14 Mar 2027"
        );

        // A change of name is not a change of clocks: Vancouver's PDT
        // becomes MST on 1 Nov 2026 at the same UTC\u{2212}7.
        let vancouver = zone("America/Vancouver");
        assert_eq!(next_change(&vancouver, at("2026-09-30T12:00:00Z")), None);

        let santo_domingo = zone("America/Santo_Domingo");
        let change = next_change(&santo_domingo, at("2026-09-30T12:00:00Z"));
        assert_eq!(change, None);
        assert_eq!(
            describe_change(change, &santo_domingo),
            "No change in the next year"
        );
    }

    #[test]
    fn the_minute_turns_over_when_expected() {
        let now = at("2026-09-30T12:00:45.250Z");
        assert_eq!(
            until_next_minute(now),
            std::time::Duration::from_millis(14_750)
        );
        assert_eq!(
            until_next_minute(at("2026-09-30T12:01:00Z")),
            std::time::Duration::from_secs(60)
        );
    }

    #[test]
    fn working_hours_are_read_strictly() {
        assert_eq!(minutes("09:00"), Some(540));
        assert_eq!(minutes("9:30"), Some(570));
        assert_eq!(minutes("23:59"), Some(1439));
        for text in ["24:00", "9", "9:5", "nine", "", "12:60"] {
            assert_eq!(minutes(text), None, "{text}");
        }
        let backwards = Overlap {
            enabled: true,
            start: "17:00".into(),
            end: "09:00".into(),
        };
        assert_eq!(backwards.hours(), None);
    }

    #[test]
    fn the_overlap_is_a_span_of_the_users_own_day() {
        let (nine, five) = (9 * 60, 17 * 60);
        // Vancouver, three hours behind: their nine is our noon.
        assert_eq!(overlap(nine, five, -3 * 3600), Some((12 * 60, five)));
        // Madrid, six ahead: their five is our eleven.
        assert_eq!(overlap(nine, five, 6 * 3600), Some((nine, 11 * 60)));
        // India, nine and a half ahead: their day is our 23:30 to 07:30.
        assert_eq!(overlap(nine, five, 9 * 3600 + 1800), None);
        // Half an hour counts: five and a half ahead ends at 11:30.
        assert_eq!(
            overlap(nine, five, 5 * 3600 + 1800),
            Some((nine, 11 * 60 + 30))
        );
        assert_eq!(overlap(nine, five, 0), Some((nine, five)));
        // Seoul, thirteen ahead: their working day is our night.
        assert_eq!(overlap(nine, five, 13 * 3600), None);
        // A long day reaches across midnight: the larger of this morning
        // (their evening, 06:00-09:00) and this evening (their next
        // morning, 17:00-22:00).
        assert_eq!(
            overlap(6 * 60, 22 * 60, 13 * 3600),
            Some((17 * 60, 22 * 60))
        );
    }

    /// Shown only on, with *Difference first*, and with hours that parse.
    #[test]
    fn the_overlap_is_shown_only_where_it_belongs() {
        let mut config = ClockConfig::default();
        assert_eq!(config.working_hours(), None, "off by default");
        config.overlap.enabled = true;
        assert_eq!(config.working_hours(), Some((540, 1020)));
        config.row = RowLayout::Time;
        assert_eq!(config.working_hours(), None);
    }

    #[test]
    fn an_unknown_zone_is_utc_not_a_failure() {
        assert_eq!(zone("Mars/Olympus_Mons"), TimeZone::UTC);
    }

    #[test]
    fn a_moment_is_handed_to_the_platform_in_its_own_zone() {
        let local = local_time(&zoned("2026-10-01T02:05:00Z", "Asia/Seoul"));
        assert_eq!(
            (local.year, local.month, local.day, local.hour, local.minute),
            (2026, 10, 1, 11, 5)
        );
        assert_eq!(local.weekday, 4, "a Thursday, counted from Sunday");
    }
}
