//! The `clipboard` section of `config.json`, as the settings window edits it
//! (F12).
//!
//! Through [`crate::section`], like the other pages: only this section
//! changes, a file that does not parse is never written, and the write is one
//! rename. The daemon's watcher on the file does the rest -- the limit applies
//! at once, without a restart.

use std::path::Path;

use anyhow::{Result, bail};
use serde_json::{Map, Value};

use super::store::MAX_ENTRIES_LIMIT;
use crate::config::ClipboardConfig;
use crate::section;

const SECTION: &str = "clipboard";

/// The section as the file has it, defaults for what it does not say.
pub fn read(path: &Path) -> Result<ClipboardConfig> {
    section::read(path, SECTION)
}

/// Keep at most `max_entries`, from 1 to [`MAX_ENTRIES_LIMIT`].
pub fn set_max_entries(path: &Path, max_entries: usize) -> Result<()> {
    if !(1..=MAX_ENTRIES_LIMIT).contains(&max_entries) {
        bail!("Keep between 1 and {MAX_ENTRIES_LIMIT} entries");
    }
    section::edit(path, SECTION, |clipboard| {
        let retention = clipboard
            .entry("retention")
            .or_insert_with(|| Value::Object(Map::new()));
        let Value::Object(retention) = retention else {
            bail!("the clipboard retention in config.json is not an object");
        };
        retention.insert("max-entries".into(), Value::from(max_entries));
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, contents: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lumen-clip-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn the_limit_is_written_and_the_rest_is_kept() {
        let path = file(
            "write",
            r#"{ "theme": "nocturne", "clipboard": { "enabled": true, "retention": { "max-age-days": 7 } } }"#,
        );
        set_max_entries(&path, 120).unwrap();

        let config = read(&path).unwrap();
        assert_eq!(config.retention.max_entries, 120);
        assert_eq!(config.retention.max_age_days, 7);
        assert!(std::fs::read_to_string(&path).unwrap().contains("nocturne"));
    }

    #[test]
    fn a_limit_out_of_range_is_refused() {
        let path = file("range", "{}");
        assert!(set_max_entries(&path, 0).is_err());
        assert!(set_max_entries(&path, MAX_ENTRIES_LIMIT + 1).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
    }
}
