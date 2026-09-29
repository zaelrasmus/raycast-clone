//! Writing the `currency` section of `config.json` from the settings page
//! (F9, slice 6).
//!
//! Only the `currency` section changes, by the rules in [`crate::section`]:
//! the rest of the file is written back as it was read, a file that does not
//! parse is never overwritten, and nothing is left half written. The next
//! fetch asks the service chosen; the table in hand is from another service,
//! so the view fetches afresh rather than showing it under the new name.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::section;

use super::{CurrencyConfig, Service};

const SECTION: &str = "currency";

/// The section as written, for editing.
///
/// A missing file or section is the defaults. A file that does not parse is
/// an error: the loader forgives it, the writer must not.
pub fn read(path: &Path) -> Result<CurrencyConfig> {
    section::read(path, SECTION)
}

/// Choose the service, and set or keep the key.
///
/// `key` is `None` to keep the saved key, and `Some("")` to remove it: the
/// settings page never shows a saved key, so an untouched field must not
/// erase it.
pub fn set_service(path: &Path, service: Service, key: Option<&str>) -> Result<()> {
    section::edit(path, SECTION, |written| {
        written.insert("service".into(), serde_json::to_value(service)?);
        match key.map(str::trim) {
            None => {}
            Some("") => {
                written.remove("api-key");
            }
            Some(key) => {
                written.insert("api-key".into(), Value::String(key.to_string()));
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str, contents: Option<&str>) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("lumen-currency-store-{tag}-{}", std::process::id()));
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
  "future-setting": true
}"#;

    #[test]
    fn a_missing_file_reads_as_the_defaults() {
        let scratch = Scratch::new("missing", None);
        let config = read(&scratch.0).unwrap();
        assert_eq!(config.service, Service::Open);
        assert_eq!(config.key(), None);
    }

    #[test]
    fn the_service_and_key_are_written_and_the_rest_is_kept() {
        let scratch = Scratch::new("write", Some(CONFIG));
        set_service(&scratch.0, Service::Unirate, Some(" secret ")).unwrap();

        let config = read(&scratch.0).unwrap();
        assert_eq!(config.service, Service::Unirate);
        assert_eq!(config.key(), Some("secret"));
        let contents = scratch.contents();
        assert!(contents.contains("\"future-setting\": true"), "{contents}");
        assert!(contents.contains("\"theme\": \"daylight\""), "{contents}");
    }

    #[test]
    fn an_untouched_key_is_kept_and_an_empty_one_removed() {
        let scratch = Scratch::new("key", Some(CONFIG));
        set_service(&scratch.0, Service::ExchangerateApi, Some("secret")).unwrap();

        set_service(&scratch.0, Service::Unirate, None).unwrap();
        assert_eq!(read(&scratch.0).unwrap().key(), Some("secret"));

        set_service(&scratch.0, Service::Open, Some("")).unwrap();
        let config = read(&scratch.0).unwrap();
        assert_eq!(config.key(), None);
        assert_eq!(config.service, Service::Open);
    }

    #[test]
    fn a_file_that_does_not_parse_is_not_overwritten() {
        let scratch = Scratch::new("broken", Some("{ not json"));
        assert!(set_service(&scratch.0, Service::Unirate, None).is_err());
        assert_eq!(scratch.contents(), "{ not json");
    }
}
