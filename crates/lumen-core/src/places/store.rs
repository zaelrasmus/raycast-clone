//! The `places` section of `config.json`: the home city, shared by World
//! Clock and Weather (F13).
//!
//! # One home, moved from `weather`
//!
//! The home city was first a Weather setting (`weather.home`). It is one
//! value for both views now, so it lives here; a home written in the old
//! place is still read, and moved here by the next save, so nobody loses
//! the city they chose.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::section;

const SECTION: &str = "places";
const LEGACY_SECTION: &str = "weather";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct PlacesConfig {
    /// The home city by its key (`CA/British Columbia/Vancouver`). `None`
    /// is the largest city in the system's time zone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
}

/// The home city as written: here, or where it was first kept.
pub fn home(config: &crate::Config) -> Option<String> {
    config
        .places
        .home
        .clone()
        .or_else(|| config.weather.home.clone())
}

/// The home city as written in the file, for editing.
pub fn read_home(path: &Path) -> Result<Option<String>> {
    let places: PlacesConfig = section::read(path, SECTION)?;
    if places.home.is_some() {
        return Ok(places.home);
    }
    let weather: crate::weather::WeatherConfig = section::read(path, LEGACY_SECTION)?;
    Ok(weather.home)
}

/// Write the home city; `None` goes back to the system's time zone. A home
/// still in the old place is removed, or it would come back as soon as this
/// one is cleared.
pub fn write_home(path: &Path, home: Option<&str>) -> Result<()> {
    section::edit(path, SECTION, |written| {
        match home {
            Some(home) => {
                written.insert("home".into(), Value::String(home.to_string()));
            }
            None => {
                written.remove("home");
            }
        }
        Ok(())
    })?;
    let legacy: crate::weather::WeatherConfig = section::read(path, LEGACY_SECTION)?;
    if legacy.home.is_some() {
        section::edit(path, LEGACY_SECTION, |written| {
            written.remove("home");
            Ok(())
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str, contents: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lumen-places-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_home_kept_by_weather_is_read_and_moved_by_the_next_save() {
        let path = scratch(
            "legacy",
            r#"{ "theme": "daylight", "weather": { "units": "imperial", "home": "KR/Seoul/Seoul" } }"#,
        );
        assert_eq!(read_home(&path).unwrap().as_deref(), Some("KR/Seoul/Seoul"));

        write_home(&path, Some("CA/British Columbia/Vancouver")).unwrap();
        let config = crate::Config::load(&path);
        assert_eq!(
            config.places.home.as_deref(),
            Some("CA/British Columbia/Vancouver")
        );
        assert_eq!(config.weather.home, None, "moved, not copied");
        assert_eq!(
            config.weather.units,
            crate::weather::Units::Imperial,
            "the rest kept"
        );

        // Cleared, it does not come back from the old place.
        write_home(&path, None).unwrap();
        assert_eq!(read_home(&path).unwrap(), None);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_shared_home_wins_over_the_old_one() {
        let mut config = crate::Config::default();
        config.weather.home = Some("old".into());
        assert_eq!(home(&config).as_deref(), Some("old"));
        config.places.home = Some("new".into());
        assert_eq!(home(&config).as_deref(), Some("new"));
    }
}
