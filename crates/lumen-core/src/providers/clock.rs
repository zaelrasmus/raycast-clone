//! World Clock in the root list (F13).
//!
//! The *World Clock* row opens the view, and `time in vancouver` or `3pm
//! vancouver` is answered from the places in memory (slice 3). Nothing here
//! reads them from disk: the launcher has them read off the UI thread the
//! first time a question about time is typed, and the answer appears when
//! they land (I-3).

use jiff::Timestamp;
use jiff::tz::TimeZone;
use lumen_platform::LocalTime;

use crate::clock::query::Request;
use crate::clock::{self, DifferenceFormat};
use crate::item::{Action, Item, Kind, ViewSource};
use crate::places::{self, Catalogue, City, Within};
use crate::providers::Provider;

/// The root list's answer to `request`, at `now`, with times written by
/// `time`. `None` when no place matches what was typed.
pub fn answer(
    request: &Request,
    places: &Catalogue,
    here: &TimeZone,
    now: Timestamp,
    time: &dyn Fn(&LocalTime) -> String,
) -> Option<Item> {
    let here_now = now.to_zoned(here.clone());
    let write = |zoned: &jiff::Zoned| time(&clock::local_time(zoned));
    let (title, subtitle, copied) = match *request {
        Request::Now { place } => {
            let city = find(places, place)?;
            let there = now.to_zoned(clock::zone(places.zone_of(city)));
            let mut about = vec![places.whereabouts(city)];
            about.extend(clock::day(&here_now, &there).map(str::to_string));
            about.push(clock::describe(
                clock::difference(&here_now, &there),
                DifferenceFormat::Words,
            ));
            about.push(clock::offset(there.offset().seconds()));
            let shown = write(&there);
            (
                format!("{shown} in {}", city.name),
                about.join(" \u{00b7} "),
                shown,
            )
        }
        Request::Convert {
            hour,
            minute,
            from,
            to,
        } => {
            let side = |place: Option<&str>| -> Option<(String, TimeZone)> {
                match place {
                    None => Some(("your time".into(), here.clone())),
                    Some(place) => {
                        let city = find(places, place)?;
                        Some((
                            format!("in {}", city.name),
                            clock::zone(places.zone_of(city)),
                        ))
                    }
                }
            };
            let (from_name, from_zone) = side(from)?;
            let (to_name, to_zone) = side(to)?;
            // Today, as it is where the time was given.
            let start = now
                .to_zoned(from_zone.clone())
                .date()
                .at(hour as i8, minute as i8, 0, 0)
                .to_zoned(from_zone)
                .ok()?;
            let end = start.with_time_zone(to_zone);
            let day = |zoned: &jiff::Zoned| {
                if start.date() == end.date() {
                    String::new()
                } else {
                    format!(" {}", zoned.strftime("%a"))
                }
            };
            let shown = write(&end);
            (
                format!("{shown} {to_name}"),
                format!(
                    "{}{} {from_name} is {shown}{} {to_name}",
                    write(&start),
                    day(&start),
                    day(&end)
                ),
                shown,
            )
        }
    };
    Some(Item {
        id: "clock:answer".into(),
        title,
        subtitle: Some(subtitle),
        icon: None,
        keywords: Vec::new(),
        actions: vec![
            Action::CopyToClipboard(copied),
            Action::OpenView(ViewSource::Clock),
        ],
        kind: Kind::Clock,
    })
}

/// The place a few typed words mean: the city the list would put first.
fn find<'a>(places: &'a Catalogue, text: &str) -> Option<&'a City> {
    let found = places.find(&places::words(text), Within::World, 1);
    places.city(*found.cities.first()?)
}

pub struct ClockProvider;

impl Provider for ClockProvider {
    fn id(&self) -> &'static str {
        "clock"
    }

    fn catalogue(&self) -> Vec<Item> {
        vec![Item {
            id: "clock:open".into(),
            title: "World Clock".into(),
            subtitle: Some("The time anywhere, compared with yours".into()),
            icon: None,
            keywords: vec![
                "time".into(),
                "timezone".into(),
                "time zone".into(),
                "clock".into(),
                "utc".into(),
                "city".into(),
            ],
            actions: vec![Action::OpenView(ViewSource::Clock)],
            kind: Kind::Clock,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::query::recognise;

    fn ask(query: &str) -> Option<Item> {
        answer(
            &recognise(query).expect("a question about time"),
            places::catalogue(),
            &clock::zone("America/Santo_Domingo"),
            // 12:14 in Santo Domingo, 09:14 in Vancouver.
            "2026-09-30T16:14:00Z".parse().unwrap(),
            &|time| format!("{:02}:{:02}", time.hour, time.minute),
        )
    }

    #[test]
    fn the_command_opens_the_view_and_is_found_by_timezone() {
        let items = ClockProvider.catalogue();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].primary(), Action::OpenView(ViewSource::Clock));
        assert!(
            items[0]
                .keywords
                .iter()
                .any(|keyword| keyword == "timezone")
        );
    }

    #[test]
    fn the_time_somewhere_says_how_far_it_is() {
        let item = ask("time in vancouver").unwrap();
        assert_eq!(item.title, "09:14 in Vancouver");
        assert_eq!(
            item.subtitle.as_deref(),
            Some("British Columbia, Canada \u{00b7} 3 h behind \u{00b7} UTC\u{2212}7")
        );
        assert_eq!(item.primary(), Action::CopyToClipboard("09:14".into()));
        assert_eq!(item.kind, Kind::Clock);

        let item = ask("seoul time").unwrap();
        assert_eq!(item.title, "01:14 in Seoul");
        assert!(item.subtitle.unwrap().contains("Tomorrow"));
    }

    #[test]
    fn a_time_there_is_converted_to_here() {
        let item = ask("3pm vancouver").unwrap();
        assert_eq!(item.title, "18:00 your time");
        assert_eq!(
            item.subtitle.as_deref(),
            Some("15:00 in Vancouver is 18:00 your time")
        );
        assert_eq!(item.primary(), Action::CopyToClipboard("18:00".into()));
    }

    #[test]
    fn a_time_here_is_converted_to_there() {
        let item = ask("3pm in vancouver").unwrap();
        assert_eq!(item.title, "12:00 in Vancouver");
        assert_eq!(
            item.subtitle.as_deref(),
            Some("15:00 your time is 12:00 in Vancouver")
        );
    }

    #[test]
    fn between_two_places_the_days_are_named_when_they_differ() {
        let item = ask("15:00 vancouver in seoul").unwrap();
        assert_eq!(item.title, "07:00 in Seoul");
        assert_eq!(
            item.subtitle.as_deref(),
            Some("15:00 Wed in Vancouver is 07:00 Thu in Seoul")
        );
    }

    #[test]
    fn a_place_nobody_knows_is_no_answer() {
        assert_eq!(ask("time in zzzzqx"), None);
        assert_eq!(ask("3pm zzzzqx"), None);
    }
}
