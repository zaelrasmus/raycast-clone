//! The `calculator` section of `config.json`, as the settings window edits
//! it (F14).
//!
//! Through [`crate::section`], like the other pages: only this section
//! changes, a file that does not parse is never written, and the write is one
//! rename. The daemon reads the limit when a calculation is kept, so it
//! applies to the next one without a restart.

use std::path::Path;

use anyhow::{Result, bail};
use serde_json::Value;

use super::history::{CalculatorConfig, MAX_LIMIT, NumberFormat};
use crate::section;

const SECTION: &str = "calculator";

/// The section as the file has it, defaults for what it does not say.
pub fn read(path: &Path) -> Result<CalculatorConfig> {
    section::read(path, SECTION)
}

/// Keep at most `limit` calculations, from 0 -- none -- to [`MAX_LIMIT`].
pub fn set_history_limit(path: &Path, limit: usize) -> Result<()> {
    if limit > MAX_LIMIT {
        bail!("Keep between 0 and {MAX_LIMIT} calculations");
    }
    section::edit(path, SECTION, |calculator| {
        calculator.insert("history-limit".into(), Value::from(limit));
        Ok(())
    })
}

/// Write and read numbers as `format` says.
pub fn set_number_format(path: &Path, format: NumberFormat) -> Result<()> {
    let value = serde_json::to_value(format)?;
    section::edit(path, SECTION, |calculator| {
        calculator.insert("number-format".into(), value);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, contents: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lumen-calc-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn the_limit_is_written_and_the_rest_is_kept() {
        let path = file("write", r#"{ "theme": "nocturne" }"#);
        assert_eq!(read(&path).unwrap().history_limit, 20);
        set_history_limit(&path, 50).unwrap();
        assert_eq!(read(&path).unwrap().history_limit, 50);
        set_history_limit(&path, 0).unwrap();
        assert_eq!(read(&path).unwrap().history_limit, 0);
        set_number_format(&path, NumberFormat::Plain).unwrap();
        let config = read(&path).unwrap();
        assert_eq!(
            (config.number_format, config.history_limit),
            (NumberFormat::Plain, 0)
        );
        assert!(std::fs::read_to_string(&path).unwrap().contains("nocturne"));
    }

    #[test]
    fn a_limit_out_of_range_is_refused() {
        let path = file("range", "{}");
        assert!(set_history_limit(&path, MAX_LIMIT + 1).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
    }
}
