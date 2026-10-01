//! What the root list recognises as a question about time (F13, slice 3).
//!
//! Two shapes, and nothing else, because an answer leads the list and a
//! false one pushes down what was actually looked for:
//!
//! - **The time somewhere:** `time in vancouver`, `vancouver time`.
//! - **A time converted:** `3pm vancouver` is three in the afternoon in
//!   Vancouver, in the user's zone; `3pm in vancouver` is three in the
//!   user's afternoon, in Vancouver's; `15:00 vancouver in tokyo` is from
//!   one to the other. A time needs `am`, `pm` or a colon, so `3 vancouver`
//!   is not one.
//!
//! Rejected on the first word that does not fit, before anything is looked
//! up: the root list runs this on every keystroke.

/// A place as typed, at least this long, so that `time in v` does not
/// answer with whichever city starts with a V.
const SHORTEST_PLACE: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request<'a> {
    /// The time in `place` now.
    Now { place: &'a str },
    /// `hour:minute` in `from` (the user's zone when `None`), in `to` (the
    /// user's when `None`). Never both `None`.
    Convert {
        hour: u8,
        minute: u8,
        from: Option<&'a str>,
        to: Option<&'a str>,
    },
}

pub fn recognise(query: &str) -> Option<Request<'_>> {
    let query = query.trim();
    let lower = query.to_ascii_lowercase();

    if let Some(place) = lower.strip_prefix("time in ") {
        return place_at(query, query.len() - place.len()).map(|place| Request::Now { place });
    }
    if let Some(place) = lower.strip_suffix(" time") {
        return place_at(&query[..place.len()], 0).map(|place| Request::Now { place });
    }

    let (hour, minute, rest) = time(query)?;
    let rest = rest.trim_start();
    let lower_rest = rest.to_ascii_lowercase();
    // `3pm vancouver in`: the place it goes to is still being typed.
    if lower_rest.ends_with(" in") || lower_rest.ends_with(" to") {
        return None;
    }
    // `3pm in vancouver`: the user's own three o'clock.
    if let Some(to) = lower_rest
        .strip_prefix("in ")
        .or(lower_rest.strip_prefix("to "))
    {
        let to = place_at(rest, rest.len() - to.len())?;
        return Some(Request::Convert {
            hour,
            minute,
            from: None,
            to: Some(to),
        });
    }
    // `15:00 vancouver in tokyo`, or `3pm vancouver`.
    let split = [" in ", " to "]
        .iter()
        .filter_map(|word| lower_rest.rfind(word).map(|at| (at, word.len())))
        .max();
    let (from, to) = match split {
        Some((at, length)) => (
            place_at(&rest[..at], 0)?,
            Some(place_at(rest, at + length)?),
        ),
        None => (place_at(rest, 0)?, None),
    };
    Some(Request::Convert {
        hour,
        minute,
        from: Some(from),
        to,
    })
}

/// The place in `text` from byte `start`, trimmed, if it is long enough to
/// mean one.
fn place_at(text: &str, start: usize) -> Option<&str> {
    let place = text.get(start..)?.trim();
    (place.chars().count() >= SHORTEST_PLACE).then_some(place)
}

/// A time at the start of `text`: `3pm`, `3 pm`, `3:30pm`, `15:00`. The
/// hour, the minute, and what follows.
fn time(text: &str) -> Option<(u8, u8, &str)> {
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    if digits == 0 || digits > 2 {
        return None;
    }
    let mut hour: u8 = text[..digits].parse().ok()?;
    let mut rest = &text[digits..];

    let mut minute = 0;
    let mut colon = false;
    if let Some(after) = rest.strip_prefix(':') {
        let length = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        if length != 2 {
            return None;
        }
        minute = after[..2].parse().ok()?;
        rest = &after[2..];
        colon = true;
    }

    let trimmed = rest.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    let meridiem = ["am", "pm"].into_iter().find(|marker| {
        lower.starts_with(marker)
            && lower[marker.len()..]
                .chars()
                .next()
                .is_none_or(|next| next == ' ')
    });
    match meridiem {
        Some(marker) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            hour %= 12;
            if marker == "pm" {
                hour += 12;
            }
            rest = &trimmed[marker.len()..];
        }
        None if colon => {}
        None => return None,
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    // Something must separate the time from the place.
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((hour, minute, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(
        hour: u8,
        minute: u8,
        from: Option<&'static str>,
        to: Option<&'static str>,
    ) -> Option<Request<'static>> {
        Some(Request::Convert {
            hour,
            minute,
            from,
            to,
        })
    }

    #[test]
    fn the_time_somewhere() {
        assert_eq!(
            recognise("time in Vancouver"),
            Some(Request::Now { place: "Vancouver" })
        );
        assert_eq!(
            recognise("  Time in new york "),
            Some(Request::Now { place: "new york" })
        );
        assert_eq!(
            recognise("vancouver time"),
            Some(Request::Now { place: "vancouver" })
        );
        assert_eq!(recognise("time in va"), None, "too short to mean a place");
        assert_eq!(recognise("time"), None);
        assert_eq!(recognise("timers"), None);
    }

    #[test]
    fn a_time_from_a_place_to_here() {
        assert_eq!(
            recognise("3pm vancouver"),
            convert(15, 0, Some("vancouver"), None)
        );
        assert_eq!(
            recognise("3 PM Vancouver"),
            convert(15, 0, Some("Vancouver"), None)
        );
        assert_eq!(
            recognise("3:30pm tokyo"),
            convert(15, 30, Some("tokyo"), None)
        );
        assert_eq!(
            recognise("15:00 madrid"),
            convert(15, 0, Some("madrid"), None)
        );
        assert_eq!(recognise("12am seoul"), convert(0, 0, Some("seoul"), None));
        assert_eq!(recognise("12pm seoul"), convert(12, 0, Some("seoul"), None));
    }

    #[test]
    fn a_time_from_here_to_a_place_and_between_places() {
        assert_eq!(
            recognise("3pm in vancouver"),
            convert(15, 0, None, Some("vancouver"))
        );
        assert_eq!(
            recognise("9am to london"),
            convert(9, 0, None, Some("london"))
        );
        assert_eq!(
            recognise("15:00 vancouver in tokyo"),
            convert(15, 0, Some("vancouver"), Some("tokyo"))
        );
        assert_eq!(
            recognise("9am new york to sao paulo"),
            convert(9, 0, Some("new york"), Some("sao paulo"))
        );
    }

    #[test]
    fn what_is_not_a_time_is_left_alone() {
        for query in [
            "3 vancouver",
            "10 usd in cad",
            "25:00 madrid",
            "13pm madrid",
            "3pmvancouver",
            "3:3pm tokyo",
            "1+2",
            "spotify",
            "3pm",
            "3pm in",
            "3pm vancouver in",
        ] {
            assert_eq!(recognise(query), None, "{query}");
        }
    }
}
