//! Open-Meteo (F13): keyless, by coordinates, current and daily in one
//! request, several places at once.
//!
//! One place answers with an object and several with a list of them, in
//! the order asked; a refusal is `{"error": true, "reason": ...}` with a
//! 400. Times are asked for as seconds since the epoch, in each place's own
//! zone (`timezone=auto`), so a day's date is its local midnight.

use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::{Current, Day, Forecast};

const TIMEOUT: Duration = Duration::from_secs(10);
const ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";
const CURRENT: &str = "temperature_2m,relative_humidity_2m,apparent_temperature,is_day,\
precipitation,weather_code,cloud_cover,pressure_msl,wind_speed_10m,wind_direction_10m,\
uv_index,visibility";
const DAILY: &str = "weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset,\
precipitation_sum,precipitation_probability_max,sunshine_duration,uv_index_max";

/// Fetch the forecasts for `places` (latitude, longitude), in their order.
/// Blocking: call it off the UI thread.
pub fn fetch(places: &[(f32, f32)], now: i64) -> Result<Vec<Forecast>> {
    if places.is_empty() {
        return Ok(Vec::new());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("Lumen/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let join = |pick: fn(&(f32, f32)) -> f32| {
        places
            .iter()
            .map(|place| format!("{:.3}", pick(place)))
            .collect::<Vec<_>>()
            .join(",")
    };
    let url = format!(
        "{ENDPOINT}?latitude={}&longitude={}&current={CURRENT}&daily={DAILY}\
         &timezone=auto&forecast_days=7&timeformat=unixtime",
        join(|place| place.0),
        join(|place| place.1),
    );
    let response = client
        .get(url)
        .send()
        .map_err(reqwest::Error::without_url)
        .context("reaching Open-Meteo")?;
    let status = response.status();
    let body = response
        .text()
        .map_err(reqwest::Error::without_url)
        .context("reading Open-Meteo's answer")?;
    let forecasts = parse(&body, now).with_context(|| format!("Open-Meteo answered {status}"))?;
    if forecasts.len() != places.len() {
        bail!(
            "Open-Meteo answered for {} places of {}",
            forecasts.len(),
            places.len()
        );
    }
    Ok(forecasts)
}

/// Read an answer: one place's object, several in a list, or a refusal.
pub fn parse(body: &str, now: i64) -> Result<Vec<Forecast>> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Answer {
        Refused { reason: String, error: bool },
        Many(Vec<Place>),
        One(Box<Place>),
    }
    match serde_json::from_str::<Answer>(body).context("not a forecast")? {
        Answer::Refused {
            reason,
            error: true,
        } => bail!("Open-Meteo refused: {reason}"),
        Answer::Refused { .. } => bail!("Open-Meteo answered without a forecast"),
        Answer::Many(places) => places.into_iter().map(|place| place.read(now)).collect(),
        Answer::One(place) => Ok(vec![place.read(now)?]),
    }
}

#[derive(Deserialize)]
struct Place {
    current: Option<RawCurrent>,
    daily: Option<RawDaily>,
}

#[derive(Deserialize)]
struct RawCurrent {
    temperature_2m: f64,
    relative_humidity_2m: f64,
    apparent_temperature: f64,
    is_day: u8,
    precipitation: f64,
    weather_code: u8,
    cloud_cover: f64,
    pressure_msl: f64,
    wind_speed_10m: f64,
    wind_direction_10m: f64,
    #[serde(default)]
    uv_index: Option<f64>,
    #[serde(default)]
    visibility: Option<f64>,
}

#[derive(Deserialize)]
struct RawDaily {
    time: Vec<i64>,
    weather_code: Vec<Option<u8>>,
    temperature_2m_max: Vec<Option<f64>>,
    temperature_2m_min: Vec<Option<f64>>,
    sunrise: Vec<i64>,
    sunset: Vec<i64>,
    precipitation_sum: Vec<Option<f64>>,
    precipitation_probability_max: Vec<Option<f64>>,
    sunshine_duration: Vec<Option<f64>>,
    uv_index_max: Vec<Option<f64>>,
}

impl Place {
    fn read(self, now: i64) -> Result<Forecast> {
        let (Some(current), Some(daily)) = (self.current, self.daily) else {
            bail!("Open-Meteo answered without current conditions or days");
        };
        let at = |values: &[Option<f64>], day: usize| values.get(day).copied().flatten();
        let days = (0..daily.time.len())
            .filter_map(|day| {
                Some(Day {
                    date: daily.time[day],
                    code: daily.weather_code.get(day).copied().flatten()?,
                    high: at(&daily.temperature_2m_max, day)?,
                    low: at(&daily.temperature_2m_min, day)?,
                    sunrise: *daily.sunrise.get(day)?,
                    sunset: *daily.sunset.get(day)?,
                    precipitation: at(&daily.precipitation_sum, day).unwrap_or(0.0),
                    precipitation_chance: at(&daily.precipitation_probability_max, day),
                    sunshine: at(&daily.sunshine_duration, day).unwrap_or(0.0),
                    uv_max: at(&daily.uv_index_max, day).unwrap_or(0.0),
                })
            })
            .collect();
        Ok(Forecast {
            fetched: now,
            current: Current {
                temperature: current.temperature_2m,
                feels_like: current.apparent_temperature,
                humidity: current.relative_humidity_2m,
                precipitation: current.precipitation,
                code: current.weather_code,
                is_day: current.is_day == 1,
                cloud_cover: current.cloud_cover,
                pressure: current.pressure_msl,
                wind_speed: current.wind_speed_10m,
                wind_direction: current.wind_direction_10m,
                uv_index: current.uv_index.unwrap_or(0.0),
                visibility: current.visibility.unwrap_or(0.0),
            },
            days,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real answer for Vancouver and Santo Domingo, kept as fetched.
    const TWO_PLACES: &str = include_str!("open-meteo.json");

    #[test]
    fn several_places_are_read_in_order() {
        let forecasts = parse(TWO_PLACES, 42).unwrap();
        assert_eq!(forecasts.len(), 2);
        let vancouver = &forecasts[0];
        assert_eq!(vancouver.fetched, 42);
        assert_eq!(vancouver.days.len(), 7);
        assert!((-30.0..45.0).contains(&vancouver.current.temperature));
        assert!(vancouver.days[0].sunrise < vancouver.days[0].sunset);
        assert!(vancouver.days[0].high >= vancouver.days[0].low);
        // Santo Domingo is tropical.
        assert!(forecasts[1].current.temperature > 15.0);
    }

    #[test]
    fn one_place_is_an_object() {
        let one = r#"{"latitude":49.2,"current":{"temperature_2m":11.8,"relative_humidity_2m":86,
            "apparent_temperature":10.9,"is_day":0,"precipitation":0.0,"weather_code":3,
            "cloud_cover":100,"pressure_msl":1019.5,"wind_speed_10m":5.8,"wind_direction_10m":274},
            "daily":{"time":[1790751600],"weather_code":[3],"temperature_2m_max":[14.8],
            "temperature_2m_min":[8.4],"sunrise":[1790777459],"sunset":[1790819560],
            "precipitation_sum":[0.0],"precipitation_probability_max":[null],
            "sunshine_duration":[35163.5],"uv_index_max":[4.2]}}"#;
        let forecasts = parse(one, 0).unwrap();
        assert_eq!(forecasts.len(), 1);
        assert!(!forecasts[0].current.is_day);
        assert_eq!(forecasts[0].days[0].precipitation_chance, None);
        assert_eq!(forecasts[0].current.uv_index, 0.0, "absent is zero");
    }

    /// The request as built, against the service. Ignored: it needs the
    /// network. `cargo test -p lumen-core live_open_meteo -- --ignored`.
    #[test]
    #[ignore]
    fn live_open_meteo() {
        let forecasts = fetch(&[(49.25, -123.119), (-33.869, 151.207)], 0).unwrap();
        assert_eq!(forecasts.len(), 2);
        assert_eq!(forecasts[1].days.len(), 7);
        let one = fetch(&[(18.472, -69.892)], 0).unwrap();
        assert_eq!(one.len(), 1);
    }

    #[test]
    fn a_refusal_says_why() {
        let error = parse(
            r#"{"reason":"Latitude must be in range of -90 to 90°. Given: 999.0.","error":true}"#,
            0,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("Latitude must be in range"));
        assert!(parse("<html>", 0).is_err());
    }
}
