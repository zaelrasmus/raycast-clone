//! Weather in the root list (F13, slice 5).
//!
//! The *Weather* row opens the view. `weather in vancouver` is answered from
//! the forecasts in memory -- at once when one is kept, with its age when it
//! is old -- and otherwise with a row that says it is loading, its height
//! the same, while the launcher has one request made in the background. The
//! keystroke is never held (I-3), and a city is named whole before anything
//! is asked (`places::Catalogue::named_exactly`).

use crate::item::{Action, Item, Kind, ViewSource};
use crate::places::Catalogue;
use crate::providers::Provider;
use crate::weather::{Failure, Forecast, Units, age, describe};

/// Where a city's forecast stands, for its answer.
pub enum Standing<'a> {
    Kept(&'a Forecast),
    Loading,
    Failed(&'a Failure),
}

/// The root list's answer about `city`, at `now` (seconds).
pub fn answer(
    places: &Catalogue,
    city: u32,
    standing: Standing,
    units: Units,
    now: i64,
) -> Option<Item> {
    let place = places.city(city)?;
    let key = places.key(place);
    let whereabouts = places.whereabouts(place);
    let open = Action::OpenView(ViewSource::Weather {
        city: Some(key.clone()),
    });
    let (title, subtitle, actions) = match standing {
        Standing::Kept(forecast) => {
            let mut about = vec![
                whereabouts,
                format!("Feels like {}", units.degrees(forecast.current.feels_like)),
            ];
            if let Some(today) = forecast.today() {
                about.push(format!(
                    "\u{2191}{} \u{2193}{}",
                    units.degrees(today.high),
                    units.degrees(today.low)
                ));
            }
            if !forecast.is_fresh(now) {
                about.push(age(now - forecast.fetched));
            }
            let title = format!(
                "{} {} in {}",
                units.degrees(forecast.current.temperature),
                describe(forecast.current.code),
                place.name
            );
            let copied = format!(
                "{}: {}, {}",
                place.name,
                units.temperature_with_unit(forecast.current.temperature),
                describe(forecast.current.code)
            );
            (
                title,
                about.join(" \u{00b7} "),
                vec![open, Action::CopyToClipboard(copied)],
            )
        }
        Standing::Loading => (
            format!("Weather in {}", place.name),
            format!("{whereabouts} \u{00b7} Loading\u{2026}"),
            vec![open],
        ),
        Standing::Failed(failure) => (
            format!("Weather in {}", place.name),
            match failure {
                Failure::Offline => format!("{whereabouts} \u{00b7} No internet connection"),
                Failure::Other(_) => format!("{whereabouts} \u{00b7} Could not load the weather"),
            },
            vec![open],
        ),
    };
    Some(Item {
        id: "weather:answer".into(),
        title,
        subtitle: Some(subtitle),
        icon: None,
        keywords: Vec::new(),
        actions,
        kind: Kind::Weather,
    })
}

pub struct WeatherProvider;

impl Provider for WeatherProvider {
    fn id(&self) -> &'static str {
        "weather"
    }

    fn catalogue(&self) -> Vec<Item> {
        vec![Item {
            id: "weather:open".into(),
            title: "Weather".into(),
            subtitle: Some("The forecast anywhere".into()),
            icon: None,
            keywords: vec![
                "weather".into(),
                "forecast".into(),
                "temperature".into(),
                "rain".into(),
            ],
            actions: vec![Action::OpenView(ViewSource::Weather { city: None })],
            kind: Kind::Weather,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::places;
    use crate::weather::tests::forecast;

    const NOW: i64 = 1_790_825_000;

    fn vancouver() -> u32 {
        places::catalogue().named_exactly("vancouver").unwrap()
    }

    #[test]
    fn the_command_opens_the_list() {
        let items = WeatherProvider.catalogue();
        assert_eq!(
            items[0].primary(),
            Action::OpenView(ViewSource::Weather { city: None })
        );
    }

    #[test]
    fn a_kept_forecast_is_answered_and_opens_its_report() {
        let kept = forecast(12.0, 3, NOW - 60);
        let item = answer(
            places::catalogue(),
            vancouver(),
            Standing::Kept(&kept),
            Units::Metric,
            NOW,
        )
        .unwrap();
        assert_eq!(item.title, "12\u{b0} Overcast in Vancouver");
        assert_eq!(
            item.subtitle.as_deref(),
            Some(
                "British Columbia, Canada \u{00b7} Feels like 10\u{b0} \u{00b7} \u{2191}14\u{b0} \u{2193}7\u{b0}"
            )
        );
        assert_eq!(
            item.primary(),
            Action::OpenView(ViewSource::Weather {
                city: Some("CA/British Columbia/Vancouver".into())
            })
        );
    }

    #[test]
    fn an_old_forecast_says_its_age_and_loading_says_so() {
        let kept = forecast(12.0, 3, NOW - 2 * 3600);
        let item = answer(
            places::catalogue(),
            vancouver(),
            Standing::Kept(&kept),
            Units::Imperial,
            NOW,
        )
        .unwrap();
        assert!(item.title.starts_with("54\u{b0}"));
        assert!(item.subtitle.unwrap().ends_with("2 h ago"));

        let item = answer(
            places::catalogue(),
            vancouver(),
            Standing::Loading,
            Units::Metric,
            NOW,
        )
        .unwrap();
        assert_eq!(item.title, "Weather in Vancouver");
        assert!(item.subtitle.unwrap().ends_with("Loading\u{2026}"));
    }
}
