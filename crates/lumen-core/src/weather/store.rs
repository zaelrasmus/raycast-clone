//! Writing the `weather` section of `config.json` from the settings page
//! (F13, slice 7), by the rules in [`crate::section`]: only this section
//! changes, a file that does not parse is never overwritten.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::section;

use super::WeatherConfig;

const SECTION: &str = "weather";

/// The section as written. A missing file or section is the defaults; a
/// file that does not parse is an error.
pub fn read(path: &Path) -> Result<WeatherConfig> {
    section::read(path, SECTION)
}

/// Write the whole section. `home: None` removes the key, so the view goes
/// back to the system's time zone.
pub fn write(path: &Path, config: &WeatherConfig) -> Result<()> {
    section::edit(path, SECTION, |written| {
        written.insert("units".into(), serde_json::to_value(config.units)?);
        written.insert("layout".into(), serde_json::to_value(config.layout)?);
        match &config.home {
            Some(home) => {
                written.insert("home".into(), Value::String(home.clone()));
            }
            None => {
                written.remove("home");
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::super::{Layout, Units};
    use super::*;

    #[test]
    fn the_section_is_written_and_the_rest_is_kept() {
        let dir = std::env::temp_dir().join(format!("lumen-weather-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        assert_eq!(read(&path).unwrap(), WeatherConfig::default());

        std::fs::write(
            &path,
            r#"{ "theme": "daylight", "weather": { "home": "x" } }"#,
        )
        .unwrap();
        let config = WeatherConfig {
            units: Units::Imperial,
            layout: Layout::Pane,
            home: None,
        };
        write(&path, &config).unwrap();
        assert_eq!(read(&path).unwrap(), config, "home removed");
        assert!(std::fs::read_to_string(&path).unwrap().contains("daylight"));

        std::fs::write(&path, "{ broken").unwrap();
        assert!(write(&path, &config).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
