//! Writing the `weather` section of `config.json` from the settings page
//! (F13, slice 7), by the rules in [`crate::section`]: only this section
//! changes, a file that does not parse is never overwritten.

use std::path::Path;

use anyhow::Result;

use crate::section;

use super::WeatherConfig;

const SECTION: &str = "weather";

/// The section as written. A missing file or section is the defaults; a
/// file that does not parse is an error.
pub fn read(path: &Path) -> Result<WeatherConfig> {
    section::read(path, SECTION)
}

/// Write the units and the layout. The home city is the `places`
/// section's, shared with World Clock (`places::store`).
pub fn write(path: &Path, config: &WeatherConfig) -> Result<()> {
    section::edit(path, SECTION, |written| {
        written.insert("units".into(), serde_json::to_value(config.units)?);
        written.insert("layout".into(), serde_json::to_value(config.layout)?);
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
        let written = read(&path).unwrap();
        assert_eq!(
            (written.units, written.layout),
            (Units::Imperial, Layout::Pane)
        );
        assert_eq!(
            written.home.as_deref(),
            Some("x"),
            "the home is not this writer's"
        );
        assert!(std::fs::read_to_string(&path).unwrap().contains("daylight"));

        std::fs::write(&path, "{ broken").unwrap();
        assert!(write(&path, &config).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
