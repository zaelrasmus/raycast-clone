//! Writing the `translate` section of `config.json` from the settings page
//! (F7).
//!
//! Only the `translate` section changes, by the rules in [`crate::section`]:
//! the rest of the file is written back as it was read, a file that does not
//! parse is never overwritten, and nothing is left half written. The daemon's
//! watcher does the rest: the launcher re-indexes, and the next translate view
//! uses the new presets.

use std::path::Path;

use anyhow::{Result, bail};
use serde_json::{Map, Value};

use crate::section;

use super::{MAX_TARGETS, Preset, PresetConfig, Service, Targets, TranslateConfig, resolve};

const SECTION: &str = "translate";

/// The section as written, for editing.
///
/// A missing file or section is the defaults. A file that does not parse is
/// an error: the loader forgives it, the writer must not.
pub fn read(path: &Path) -> Result<TranslateConfig> {
    section::read(path, SECTION)
}

/// Choose the service, and set or keep DeepL's key.
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
                written.remove("deepl-key");
            }
            Some(key) => {
                written.insert("deepl-key".into(), Value::String(key.to_string()));
            }
        }
        Ok(())
    })
}

/// Whether `preset` may join `presets` in place of the one named `replacing`,
/// or alongside them when that is `None`.
///
/// What the loader checks, plus what only a writer can: a name is a preset's
/// identity -- its row in the root list and what usage is remembered by --
/// and an alias is a word the root list finds it by, so neither is shared.
/// More than [`MAX_TARGETS`] targets is refused rather than cut, because the
/// loader cutting a file is a warning and the page cutting a form is a loss.
pub fn check(
    presets: &[PresetConfig],
    preset: &PresetConfig,
    replacing: Option<&str>,
) -> Result<Preset> {
    let resolved = resolve(preset).map_err(anyhow::Error::msg)?;
    if resolved.to.len() > MAX_TARGETS {
        bail!("a preset translates into at most {MAX_TARGETS} languages");
    }

    let others = presets
        .iter()
        .filter(|other| Some(other.name.trim()) != replacing.map(str::trim));
    for other in others {
        if other.name.trim().eq_ignore_ascii_case(&resolved.name) {
            bail!("there is already a preset called {:?}", other.name.trim());
        }
        if let (Some(alias), Some(theirs)) = (&resolved.alias, &other.alias)
            && alias.eq_ignore_ascii_case(theirs.trim())
        {
            bail!("{:?} already uses the alias {alias:?}", other.name.trim());
        }
    }
    Ok(resolved)
}

/// Add `preset` after the others.
pub fn add(path: &Path, preset: PresetConfig) -> Result<()> {
    let existing = read(path)?.presets;
    check(&existing, &preset, None)?;
    section::edit(path, SECTION, |written| {
        section::list(written, "presets")?.push(Value::Object(fields(&preset, Map::new())));
        Ok(())
    })
}

/// Replace the preset called `name`, keeping its place and any fields this
/// build does not know.
pub fn replace(path: &Path, name: &str, preset: PresetConfig) -> Result<()> {
    let existing = read(path)?.presets;
    check(&existing, &preset, Some(name))?;
    section::edit(path, SECTION, |written| {
        let presets = section::list(written, "presets")?;
        let index = section::position(presets, name)?;
        let kept = match presets[index].take() {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        presets[index] = Value::Object(fields(&preset, kept));
        Ok(())
    })
}

/// Remove the preset called `name`.
pub fn remove(path: &Path, name: &str) -> Result<()> {
    section::edit(path, SECTION, |written| {
        let presets = section::list(written, "presets")?;
        let index = section::position(presets, name)?;
        presets.remove(index);
        Ok(())
    })
}

/// `preset`'s fields over `kept`, in the order a person would write them,
/// leaving out what is empty -- as the example in the README reads.
fn fields(preset: &PresetConfig, mut kept: Map<String, Value>) -> Map<String, Value> {
    let text = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Value::String(value.to_string()))
    };
    let from = preset
        .from
        .as_deref()
        .map(str::trim)
        .filter(|from| !from.is_empty() && !from.eq_ignore_ascii_case("auto"))
        .map(|from| Value::String(from.to_string()));
    let to: Vec<Value> = preset
        .to
        .codes()
        .into_iter()
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(|code| Value::String(code.to_string()))
        .collect();
    let to = match <[Value; 1]>::try_from(to) {
        Ok([one]) => one,
        Err(many) => Value::Array(many),
    };

    let mut map = Map::new();
    map.insert("name".into(), Value::String(preset.name.trim().to_string()));
    for (key, value) in [("alias", text(&preset.alias)), ("from", from)] {
        match value {
            Some(value) => {
                map.insert(key.into(), value);
            }
            None => {
                kept.remove(key);
            }
        }
    }
    map.insert("to".into(), to);
    for key in ["name", "alias", "from", "to"] {
        kept.remove(key);
    }
    map.extend(kept);
    map
}

/// A preset as the settings form describes it.
pub fn preset(name: &str, alias: &str, from: Option<&str>, to: &[&str]) -> PresetConfig {
    PresetConfig {
        name: name.to_string(),
        alias: Some(alias.to_string()),
        from: from.map(str::to_string),
        to: Targets::Many(to.iter().map(|code| (*code).to_string()).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str, contents: Option<&str>) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "lumen-translate-store-{tag}-{}",
                std::process::id()
            ));
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
  "window": { "width": 800.0 },
  "future-setting": true
}"#;

    #[test]
    fn a_preset_is_added_and_the_rest_of_the_file_is_kept_in_order() {
        let file = Scratch::new("add", Some(CONFIG));

        add(
            &file.0,
            preset("Spanish → English", "esen", Some("es"), &["en"]),
        )
        .unwrap();

        let written = file.contents();
        let theme = written.find("\"theme\"").unwrap();
        let window = written.find("\"window\"").unwrap();
        let future = written.find("\"future-setting\"").unwrap();
        assert!(theme < window && window < future, "{written}");

        let config: crate::Config = serde_json::from_str(&written).unwrap();
        assert_eq!(config.theme, "daylight");
        let presets = config.translate.presets();
        assert_eq!(presets[0].name, "Spanish → English");
        assert_eq!(presets[0].alias.as_deref(), Some("esen"));
        assert_eq!(presets[0].to[0].code, "en");
        // One target is written as one, as a person would.
        assert!(written.contains("\"to\": \"en\""), "{written}");
    }

    #[test]
    fn a_missing_file_is_created_with_just_the_section() {
        let file = Scratch::new("missing", None);
        add(&file.0, preset("To French", "", None, &["fr"])).unwrap();

        let read = read(&file.0).unwrap();
        assert_eq!(read.presets.len(), 1);
        // No source means detect it, and is left out of the file.
        assert!(!file.contents().contains("\"from\""));
        assert!(!file.contents().contains("\"alias\""));
    }

    #[test]
    fn a_file_that_does_not_parse_is_never_overwritten() {
        let file = Scratch::new("broken", Some("{ \"theme\": oops"));
        let error = add(&file.0, preset("X", "", None, &["en"])).unwrap_err();
        assert!(format!("{error:#}").contains("mistake"), "{error:#}");
        assert_eq!(file.contents(), "{ \"theme\": oops");
    }

    #[test]
    fn replacing_keeps_the_place_and_unknown_fields() {
        let file = Scratch::new(
            "replace",
            Some(
                r#"{ "translate": { "presets": [
                    { "name": "A", "to": "en", "colour": "red" },
                    { "name": "B", "to": "fr" }
                ] } }"#,
            ),
        );

        replace(&file.0, "A", preset("A2", "", Some("es"), &["en", "de"])).unwrap();

        let written = file.contents();
        assert!(written.contains("\"colour\""), "{written}");
        let read = read(&file.0).unwrap();
        assert_eq!(read.presets[0].name, "A2");
        assert_eq!(read.presets[1].name, "B");
        assert_eq!(read.presets[0].from.as_deref(), Some("es"));
    }

    #[test]
    fn a_name_or_an_alias_cannot_be_shared() {
        let existing = vec![preset("Work", "wk", None, &["en"])];

        let error = check(&existing, &preset("work", "", None, &["fr"]), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("\"Work\""), "{error}");
        assert!(check(&existing, &preset("Other", "WK", None, &["fr"]), None).is_err());

        // Editing a preset may keep its own name and alias.
        assert!(
            check(
                &existing,
                &preset("Work", "wk", None, &["de"]),
                Some("Work")
            )
            .is_ok()
        );
    }

    #[test]
    fn what_the_loader_skips_the_page_refuses() {
        assert!(check(&[], &preset("", "", None, &["en"]), None).is_err());
        assert!(check(&[], &preset("X", "", Some("xx"), &["en"]), None).is_err());
        assert!(check(&[], &preset("X", "", None, &[]), None).is_err());
        assert!(
            check(
                &[],
                &preset("X", "", None, &["en", "fr", "de", "it", "pt"]),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn removing_a_preset_that_is_gone_says_so() {
        let file = Scratch::new("remove", Some(CONFIG));
        add(&file.0, preset("A", "", None, &["en"])).unwrap();

        remove(&file.0, "A").unwrap();
        assert!(read(&file.0).unwrap().presets.is_empty());
        assert!(
            remove(&file.0, "A")
                .unwrap_err()
                .to_string()
                .contains("no preset called")
        );
    }

    /// The page never shows a saved key, so leaving the field alone keeps it.
    #[test]
    fn the_key_is_kept_unless_replaced_or_removed() {
        let file = Scratch::new("key", Some(CONFIG));

        set_service(&file.0, Service::Deepl, Some(" secret:fx ")).unwrap();
        assert_eq!(
            read(&file.0).unwrap().deepl_key.as_deref(),
            Some("secret:fx")
        );

        set_service(&file.0, Service::Google, None).unwrap();
        let read_back = read(&file.0).unwrap();
        assert_eq!(read_back.service, Service::Google);
        assert_eq!(read_back.deepl_key.as_deref(), Some("secret:fx"));

        set_service(&file.0, Service::Deepl, Some("")).unwrap();
        assert_eq!(read(&file.0).unwrap().deepl_key, None);
        assert!(!section::staging_path(&file.0).exists());
    }
}
