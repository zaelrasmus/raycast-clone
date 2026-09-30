//! Dates as the lists show them: the section a row falls in and the moment
//! in its pane. Shared by *Search Snippets* (F11) and *Clipboard History*
//! (F12), so a *Yesterday* means the same thing in both.

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};

/// The section a date falls in, seen from `today`. A week starts on Monday;
/// a date in the future is *Today*, since a clock that was ahead is the
/// likely reason.
pub fn period(when: Option<Date>, today: Date) -> &'static str {
    let Some(when) = when else {
        return "Earlier";
    };
    if when >= today {
        return "Today";
    }
    if Some(when) == today.yesterday().ok() {
        return "Yesterday";
    }
    let into_week = i64::from(today.weekday().to_monday_zero_offset());
    let monday = today.checked_sub(jiff::Span::new().days(into_week)).ok();
    if monday.is_some_and(|monday| when >= monday) {
        return "This Week";
    }
    if when.year() == today.year() && when.month() == today.month() {
        return "This Month";
    }
    "Earlier"
}

/// The day `at` falls on, in `now`'s time zone.
pub fn day(at: Timestamp, now: &Zoned) -> Date {
    at.to_zoned(now.time_zone().clone()).date()
}

/// *Today, 9:00 PM*, *Yesterday, 9:00 PM*, or *Sep 3, 2026, 9:00 PM*.
pub fn moment(at: Timestamp, now: &Zoned) -> String {
    let zoned = at.to_zoned(now.time_zone().clone());
    let today = now.date();
    let time = zoned.strftime("%-I:%M %p");
    if zoned.date() == today {
        format!("Today, {time}")
    } else if Some(zoned.date()) == today.yesterday().ok() {
        format!("Yesterday, {time}")
    } else {
        format!("{}, {time}", zoned.strftime("%b %-d, %Y"))
    }
}
