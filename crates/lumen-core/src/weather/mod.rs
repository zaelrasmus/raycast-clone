//! Weather (F13, slices 5 to 7): the forecast for any place in the
//! catalogue, from Open-Meteo.
//!
//! # Fetched only when asked, kept for a quarter of an hour
//!
//! A forecast is fetched by the place's coordinates, which the catalogue
//! already has, so no geocoding service is asked anything. It is kept in
//! `weather.json` in the data folder and is fresh for [`FRESH`]; an older
//! one is still shown, with its age, while a new one is fetched. Nothing is
//! scheduled: a daemon nobody asks about the weather never fetches.
//!
//! # Metric inside, the user's units outside
//!
//! Forecasts are fetched and kept in Celsius, km/h and millimetres, and
//! converted only when written, so changing units needs no new request and
//! a cache from before the change still reads.

pub mod query;
pub mod session;
pub mod source;
pub mod store;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

pub use crate::net::{Failure, failure};

/// How long a forecast is fresh, in seconds.
pub const FRESH: i64 = 15 * 60;

/// How many places' forecasts are kept; the oldest go first.
pub const KEPT: usize = 64;

/// The `weather` section of `config.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct WeatherConfig {
    pub units: Units,
    pub layout: Layout,
    /// Where the home city was first kept, before World Clock shared it:
    /// read so a city chosen then is not lost, and moved to `places.home`
    /// by the next save (`places::store`). Not written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Units {
    /// \u{b0}C, km/h, mm, km.
    #[default]
    Metric,
    /// \u{b0}F, mph, in, mi.
    Imperial,
}

/// How a place's report is shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    /// `Enter` opens the report at full width; `Escape` returns.
    #[default]
    Report,
    /// The report beside the list, following the selection.
    Pane,
}

/// The conditions now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Current {
    /// \u{b0}C.
    pub temperature: f64,
    pub feels_like: f64,
    /// Per cent.
    pub humidity: f64,
    /// mm in the last quarter hour.
    pub precipitation: f64,
    /// WMO weather interpretation code.
    pub code: u8,
    pub is_day: bool,
    /// Per cent.
    pub cloud_cover: f64,
    /// hPa, at sea level.
    pub pressure: f64,
    /// km/h.
    pub wind_speed: f64,
    /// Degrees, where the wind comes from.
    pub wind_direction: f64,
    pub uv_index: f64,
    /// Metres.
    pub visibility: f64,
}

/// One day of the forecast.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Day {
    /// The day's local midnight, as seconds since the epoch.
    pub date: i64,
    pub code: u8,
    pub high: f64,
    pub low: f64,
    pub sunrise: i64,
    pub sunset: i64,
    /// mm.
    pub precipitation: f64,
    /// Per cent, the day's highest chance.
    pub precipitation_chance: Option<f64>,
    /// Seconds of sunshine.
    pub sunshine: f64,
    pub uv_max: f64,
}

/// A place's forecast, as fetched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Forecast {
    /// When it was fetched, seconds since the epoch.
    pub fetched: i64,
    pub current: Current,
    pub days: Vec<Day>,
}

impl Forecast {
    pub fn is_fresh(&self, now: i64) -> bool {
        now - self.fetched < FRESH
    }

    pub fn condition(&self) -> Condition {
        Condition::from_code(self.current.code, self.current.is_day)
    }

    /// Today's, as the forecast has it: the first day.
    pub fn today(&self) -> Option<&Day> {
        self.days.first()
    }
}

/// The forecasts kept, by place key.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cache {
    pub forecasts: BTreeMap<String, Forecast>,
}

impl Cache {
    /// What `weather.json` holds, or nothing: a lost cache is one request.
    pub fn load(path: &Path) -> Self {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                log::warn!("could not read {}: {error}", path.display());
                return Self::default();
            }
        };
        serde_json::from_str(&source).unwrap_or_else(|error| {
            log::warn!("{} is not a weather cache: {error}", path.display());
            Self::default()
        })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let source = serde_json::to_string(self)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, source)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
    }

    pub fn get(&self, key: &str) -> Option<&Forecast> {
        self.forecasts.get(key)
    }

    /// Keep `forecast` for `key`, forgetting the oldest past [`KEPT`].
    pub fn insert(&mut self, key: String, forecast: Forecast) {
        self.forecasts.insert(key, forecast);
        while self.forecasts.len() > KEPT {
            let oldest = self
                .forecasts
                .iter()
                .min_by_key(|(_, forecast)| forecast.fetched)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(key) => self.forecasts.remove(&key),
                None => break,
            };
        }
    }
}

/// What the sky is doing, as a glyph can draw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    ClearDay,
    ClearNight,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Storm,
}

impl Condition {
    /// From a WMO weather interpretation code, as Open-Meteo sends.
    pub fn from_code(code: u8, is_day: bool) -> Self {
        match code {
            0 | 1 if is_day => Condition::ClearDay,
            0 | 1 => Condition::ClearNight,
            2 if is_day => Condition::PartlyCloudy,
            2 | 3 => Condition::Cloudy,
            45 | 48 => Condition::Fog,
            51..=57 => Condition::Drizzle,
            61..=67 | 80..=82 => Condition::Rain,
            71..=77 | 85 | 86 => Condition::Snow,
            95..=99 => Condition::Storm,
            _ => Condition::Cloudy,
        }
    }

    /// The bundled glyph's name.
    pub fn glyph(self) -> &'static str {
        match self {
            Condition::ClearDay => "weather-clear-day",
            Condition::ClearNight => "weather-clear-night",
            Condition::PartlyCloudy => "weather-partly-cloudy",
            Condition::Cloudy => "weather-cloudy",
            Condition::Fog => "weather-fog",
            Condition::Drizzle => "weather-drizzle",
            Condition::Rain => "weather-rain",
            Condition::Snow => "weather-snow",
            Condition::Storm => "weather-storm",
        }
    }
}

/// A WMO code in words, as Open-Meteo documents them.
pub fn describe(code: u8) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 => "Fog",
        48 => "Rime fog",
        51 => "Light drizzle",
        53 => "Drizzle",
        55 => "Dense drizzle",
        56 => "Light freezing drizzle",
        57 => "Freezing drizzle",
        61 => "Light rain",
        63 => "Rain",
        65 => "Heavy rain",
        66 => "Light freezing rain",
        67 => "Freezing rain",
        71 => "Light snow",
        73 => "Snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 => "Light showers",
        81 => "Showers",
        82 => "Violent showers",
        85 => "Light snow showers",
        86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => "Unknown",
    }
}

impl Units {
    fn temperature(self, celsius: f64) -> f64 {
        match self {
            Units::Metric => celsius,
            Units::Imperial => celsius * 9.0 / 5.0 + 32.0,
        }
    }

    /// `14\u{b0}`, for a row.
    pub fn degrees(self, celsius: f64) -> String {
        format!("{}\u{b0}", whole(self.temperature(celsius)))
    }

    /// `14 \u{b0}C`, for a heading.
    pub fn temperature_with_unit(self, celsius: f64) -> String {
        let unit = match self {
            Units::Metric => "C",
            Units::Imperial => "F",
        };
        format!("{} \u{b0}{unit}", whole(self.temperature(celsius)))
    }

    /// `7 km/h W`.
    pub fn wind(self, kmh: f64, from_degrees: f64) -> String {
        let (speed, unit) = match self {
            Units::Metric => (kmh, "km/h"),
            Units::Imperial => (kmh / 1.609_344, "mph"),
        };
        format!("{} {unit} {}", whole(speed), compass(from_degrees))
    }

    /// `0.4 mm`, `0.02 in`.
    pub fn precipitation(self, mm: f64) -> String {
        match self {
            Units::Metric => format!("{mm:.1} mm"),
            Units::Imperial => format!("{:.2} in", mm / 25.4),
        }
    }

    /// `17 km`, `11 mi`.
    pub fn distance(self, metres: f64) -> String {
        match self {
            Units::Metric => format!("{} km", whole(metres / 1000.0)),
            Units::Imperial => format!("{} mi", whole(metres / 1609.344)),
        }
    }

    /// `1020 hPa`, `30.11 inHg`.
    pub fn pressure(self, hpa: f64) -> String {
        match self {
            Units::Metric => format!("{} hPa", whole(hpa)),
            Units::Imperial => format!("{:.2} inHg", hpa * 0.029_53),
        }
    }
}

/// Rounded, with no `-0`.
fn whole(value: f64) -> i64 {
    let rounded = value.round() as i64;
    if rounded == 0 { 0 } else { rounded }
}

/// Where a wind comes from, on sixteen points.
pub fn compass(degrees: f64) -> &'static str {
    const POINTS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let index = ((degrees.rem_euclid(360.0) / 22.5).round() as usize) % 16;
    POINTS[index]
}

/// How long ago something was, for a cached forecast: `just now`,
/// `5 min ago`, `2 h ago`, `3 days ago`.
pub fn age(seconds: i64) -> String {
    match seconds.max(0) {
        0..60 => "just now".into(),
        seconds @ 60..3600 => format!("{} min ago", seconds / 60),
        seconds @ 3600..86_400 => format!("{} h ago", seconds / 3600),
        seconds => match seconds / 86_400 {
            1 => "a day ago".into(),
            days => format!("{days} days ago"),
        },
    }
}

/// The moon's phase at `when` (seconds since the epoch): its name and how
/// much of it is lit, computed from the mean synodic month -- good to a
/// day, which is what a name needs.
pub fn moon(when: i64) -> (&'static str, u8) {
    const SYNODIC: f64 = 29.530_588_853;
    // A new moon: 6 January 2000, 18:14 UTC.
    const KNOWN_NEW: i64 = 947_182_440;
    let age = ((when - KNOWN_NEW) as f64 / 86_400.0).rem_euclid(SYNODIC);
    let lit = (1.0 - (2.0 * std::f64::consts::PI * age / SYNODIC).cos()) / 2.0;
    let eighth = SYNODIC / 8.0;
    let name = match (age / eighth + 0.5) as u8 % 8 {
        0 => "New Moon",
        1 => "Waxing Crescent",
        2 => "First Quarter",
        3 => "Waxing Gibbous",
        4 => "Full Moon",
        5 => "Waning Gibbous",
        6 => "Last Quarter",
        _ => "Waning Crescent",
    };
    (name, (lit * 100.0).round() as u8)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A forecast with `temperature` and `code`, fetched at `fetched`.
    pub fn forecast(temperature: f64, code: u8, fetched: i64) -> Forecast {
        Forecast {
            fetched,
            current: Current {
                temperature,
                feels_like: temperature - 2.0,
                humidity: 80.0,
                precipitation: 0.0,
                code,
                is_day: true,
                cloud_cover: 100.0,
                pressure: 1019.5,
                wind_speed: 5.8,
                wind_direction: 274.0,
                uv_index: 3.0,
                visibility: 17_000.0,
            },
            days: (0..7)
                .map(|day| Day {
                    // 2026-09-30, local midnight in Vancouver.
                    date: 1_790_751_600 + day * 86_400,
                    code: if day == 1 { 61 } else { code },
                    high: temperature + 2.0,
                    low: temperature - 5.0,
                    sunrise: 1_790_777_459 + day * 86_400,
                    sunset: 1_790_819_560 + day * 86_400,
                    precipitation: if day == 1 { 4.0 } else { 0.0 },
                    precipitation_chance: Some(if day == 1 { 80.0 } else { 5.0 }),
                    sunshine: 30_000.0,
                    uv_max: 4.2,
                })
                .collect(),
        }
    }

    #[test]
    fn a_code_is_a_condition_and_words() {
        assert_eq!(Condition::from_code(0, true), Condition::ClearDay);
        assert_eq!(Condition::from_code(1, false), Condition::ClearNight);
        assert_eq!(Condition::from_code(2, false), Condition::Cloudy);
        assert_eq!(Condition::from_code(81, true), Condition::Rain);
        assert_eq!(Condition::from_code(86, true), Condition::Snow);
        assert_eq!(Condition::from_code(99, true), Condition::Storm);
        assert_eq!(describe(51), "Light drizzle");
        assert_eq!(describe(200), "Unknown");
    }

    #[test]
    fn units_are_converted_only_when_written() {
        assert_eq!(Units::Metric.degrees(11.8), "12\u{b0}");
        assert_eq!(Units::Imperial.degrees(11.8), "53\u{b0}");
        assert_eq!(Units::Metric.degrees(-0.4), "0\u{b0}", "no minus zero");
        assert_eq!(Units::Metric.temperature_with_unit(28.0), "28 \u{b0}C");
        assert_eq!(Units::Metric.wind(7.0, 112.0), "7 km/h ESE");
        assert_eq!(Units::Imperial.wind(16.1, 0.0), "10 mph N");
        assert_eq!(Units::Metric.precipitation(0.1), "0.1 mm");
        assert_eq!(Units::Imperial.precipitation(25.4), "1.00 in");
        assert_eq!(Units::Metric.distance(10_000.0), "10 km");
        assert_eq!(Units::Metric.pressure(1012.4), "1012 hPa");
        assert_eq!(Units::Imperial.pressure(1013.25), "29.92 inHg");
    }

    #[test]
    fn ages_read_as_words() {
        assert_eq!(age(10), "just now");
        assert_eq!(age(5 * 60), "5 min ago");
        assert_eq!(age(2 * 3600), "2 h ago");
        assert_eq!(age(86_400), "a day ago");
        assert_eq!(age(3 * 86_400), "3 days ago");
    }

    /// Against published phases (timeanddate.com).
    #[test]
    fn the_moon_has_its_published_phase() {
        // Full moon, 7 September 2025 18:09 UTC.
        assert_eq!(moon(1_757_268_540).0, "Full Moon");
        // New moon, 21 September 2025 19:54 UTC.
        assert_eq!(moon(1_758_484_440).0, "New Moon");
        // A week after the full moon.
        let (name, lit) = moon(1_757_268_540 + 7 * 86_400);
        assert_eq!(name, "Last Quarter");
        assert!((40..=60).contains(&lit), "{lit}");
    }

    #[test]
    fn the_cache_keeps_the_newest() {
        let mut cache = Cache::default();
        for index in 0..(KEPT as i64 + 5) {
            cache.insert(format!("place {index}"), forecast(10.0, 0, index));
        }
        assert_eq!(cache.forecasts.len(), KEPT);
        assert!(cache.get("place 0").is_none());
        assert!(cache.get(&format!("place {}", KEPT + 4)).is_some());
        assert!(forecast(10.0, 0, 0).is_fresh(FRESH - 1));
        assert!(!forecast(10.0, 0, 0).is_fresh(FRESH));
    }

    #[test]
    fn the_cache_file_round_trips() {
        let dir = std::env::temp_dir().join(format!("lumen-weather-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("weather.json");
        assert_eq!(Cache::load(&path), Cache::default());
        let mut cache = Cache::default();
        cache.insert(
            "CA/British Columbia/Vancouver".into(),
            forecast(12.0, 3, 100),
        );
        cache.save(&path).unwrap();
        assert_eq!(Cache::load(&path), cache);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
