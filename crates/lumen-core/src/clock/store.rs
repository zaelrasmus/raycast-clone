//! Writing the `clock` section of `config.json` from the settings page
//! (F13, slice 4).
//!
//! Only the `clock` section changes, by the rules in [`crate::section`]:
//! the rest of the file is written back as it was read, a file that does not
//! parse is never overwritten, and nothing is left half written. The next
//! World Clock opened reads what was saved (7.33).

use std::path::Path;

use anyhow::{Result, bail};

use crate::section;

use super::ClockConfig;

const SECTION: &str = "clock";

/// The section as written, for editing. A missing file or section is the
/// defaults; a file that does not parse is an error.
pub fn read(path: &Path) -> Result<ClockConfig> {
    section::read(path, SECTION)
}

/// Write the whole section. Working hours that do not read as a day are
/// refused, with why, rather than saved to be ignored.
pub fn write(path: &Path, config: &ClockConfig) -> Result<()> {
    if super::minutes(&config.overlap.start).is_none() {
        bail!("Start is not a time: write it as 09:00");
    }
    if super::minutes(&config.overlap.end).is_none() {
        bail!("End is not a time: write it as 17:00");
    }
    if config.overlap.hours().is_none() {
        bail!("The working day must start before it ends");
    }
    section::edit(path, SECTION, |written| {
        written.insert("row".into(), serde_json::to_value(config.row)?);
        written.insert(
            "difference-format".into(),
            serde_json::to_value(config.difference_format)?,
        );
        written.insert("overlap".into(), serde_json::to_value(&config.overlap)?);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::super::{DifferenceFormat, RowLayout};
    use super::*;
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str, contents: Option<&str>) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("lumen-clock-store-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("config.json");
            if let Some(contents) = contents {
                std::fs::write(&path, contents).unwrap();
            }
            Self(path)
        }

        fn contents(&self) -> String {
            std::fs::read_to_string(&self.0).unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Some(dir) = self.0.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    const CONFIG: &str = r#"{
  "theme": "daylight",
  "clock": { "row": "time", "future-setting": 1 }
}"#;

    #[test]
    fn a_missing_file_reads_as_the_defaults() {
        let scratch = Scratch::new("missing", None);
        assert_eq!(read(&scratch.0).unwrap(), ClockConfig::default());
    }

    #[test]
    fn the_section_is_written_and_the_rest_is_kept() {
        let scratch = Scratch::new("write", Some(CONFIG));
        assert_eq!(read(&scratch.0).unwrap().row, RowLayout::Time);

        let mut config = ClockConfig {
            row: RowLayout::Difference,
            difference_format: DifferenceFormat::Signed,
            ..ClockConfig::default()
        };
        config.overlap.enabled = true;
        config.overlap.start = "08:30".into();
        write(&scratch.0, &config).unwrap();

        assert_eq!(read(&scratch.0).unwrap(), config);
        let contents = scratch.contents();
        assert!(contents.contains("\"theme\": \"daylight\""), "{contents}");
        assert!(contents.contains("\"future-setting\": 1"), "{contents}");
    }

    #[test]
    fn hours_that_are_not_a_day_are_refused() {
        let scratch = Scratch::new("hours", Some(CONFIG));
        let mut config = ClockConfig::default();
        config.overlap.start = "nine".into();
        assert!(
            write(&scratch.0, &config)
                .unwrap_err()
                .to_string()
                .contains("Start")
        );
        config.overlap.start = "18:00".into();
        assert!(
            write(&scratch.0, &config)
                .unwrap_err()
                .to_string()
                .contains("before it ends")
        );
        assert_eq!(scratch.contents(), CONFIG, "nothing written");
    }

    #[test]
    fn a_file_that_does_not_parse_is_not_overwritten() {
        let scratch = Scratch::new("broken", Some("{ not json"));
        assert!(write(&scratch.0, &ClockConfig::default()).is_err());
        assert_eq!(scratch.contents(), "{ not json");
    }
}
