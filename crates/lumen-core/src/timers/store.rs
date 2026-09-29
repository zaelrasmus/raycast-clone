//! Writing the `timers` section of `config.json` from the settings page
//! (F8, slice 2).
//!
//! Only that section changes, by the rules in [`crate::section`]. The
//! daemon's watcher does the rest: the launcher re-indexes, and an open
//! *Timers* view redraws with the new presets.

use std::path::Path;

use anyhow::{Result, bail};
use serde_json::{Map, Value};

use super::presets::{Preset, PresetConfig, Ring, TimersConfig, parse_duration, resolve, same};
use crate::section;

const SECTION: &str = "timers";

/// The section as written, for editing.
pub fn read(path: &Path) -> Result<TimersConfig> {
    section::read(path, SECTION)
}

/// Whether `preset` may join `presets` in place of the one named `replacing`,
/// or alongside them when that is `None`.
///
/// What the loader checks, plus what only a writer can: a name is a preset's
/// identity and an alias is a word the root list finds it by, so neither is
/// shared.
pub fn check(
    presets: &[PresetConfig],
    preset: &PresetConfig,
    replacing: Option<&str>,
) -> Result<Preset> {
    let resolved = resolve(preset).map_err(anyhow::Error::msg)?;
    let others = presets
        .iter()
        .filter(|other| !replacing.is_some_and(|replacing| same(&other.name, replacing)));
    for other in others {
        if same(&other.name, &resolved.name) {
            bail!("there is already a preset called {:?}", other.name.trim());
        }
        if let (Some(alias), Some(theirs)) = (&resolved.alias, &other.alias)
            && same(alias, theirs)
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
/// build does not know -- the sound and cycles of later slices among them.
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

/// Choose the alert sound, `None` for silence (slice 3).
pub fn set_alert_sound(path: &Path, sound: Option<&str>) -> Result<()> {
    section::edit(path, SECTION, |written| {
        written.insert("alert-sound".into(), sound_value(sound));
        Ok(())
    })
}

/// Save the General row's form: the sound, how long it rings, and the quick
/// durations, in one write so a refusal changes none of them.
pub fn set_general(
    path: &Path,
    sound: Option<&str>,
    ring: Ring,
    durations: &[String],
) -> Result<()> {
    let durations = checked(durations)?;
    section::edit(path, SECTION, |written| {
        written.insert("alert-sound".into(), sound_value(sound));
        written.insert("ring".into(), serde_json::to_value(ring)?);
        written.insert("quick-start".into(), durations);
        Ok(())
    })
}

fn sound_value(sound: Option<&str>) -> Value {
    sound.map_or(Value::Null, |sound| Value::String(sound.to_string()))
}

/// Replace the quick durations with `durations`, as typed.
///
/// Every one must read as a duration, or nothing is written: the loader would
/// skip it quietly, and the page is where a mistake can still be seen.
pub fn set_quick_start(path: &Path, durations: &[String]) -> Result<()> {
    let durations = checked(durations)?;
    section::edit(path, SECTION, |written| {
        written.insert("quick-start".into(), durations);
        Ok(())
    })
}

/// `durations` as the file's list, each read as a duration first.
fn checked(durations: &[String]) -> Result<Value> {
    let durations: Vec<&str> = durations
        .iter()
        .map(|written| written.trim())
        .filter(|written| !written.is_empty())
        .collect();
    if let Some(wrong) = durations
        .iter()
        .find(|written| parse_duration(written).is_none())
    {
        bail!("{wrong:?} is not a duration Lumen can read; try 25m, 1h30 or 1:30");
    }
    Ok(Value::Array(
        durations
            .iter()
            .map(|written| Value::String((*written).to_string()))
            .collect(),
    ))
}

/// `preset`'s fields over `kept`, in the order a person would write them,
/// leaving out an empty alias.
fn fields(preset: &PresetConfig, mut kept: Map<String, Value>) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("name".into(), Value::String(preset.name.trim().to_string()));
    match preset
        .alias
        .as_deref()
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
    {
        Some(alias) => {
            map.insert("alias".into(), Value::String(alias.to_string()));
        }
        None => {
            kept.remove("alias");
        }
    }
    map.insert(
        "duration".into(),
        Value::String(preset.duration.trim().to_string()),
    );
    let text = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Value::String(value.to_string()))
    };
    let cycle = [
        ("short-break", text(&preset.short_break)),
        ("long-break", text(&preset.long_break)),
        ("rounds", preset.rounds.map(Value::from)),
    ];
    for (key, value) in cycle {
        if let Some(value) = value {
            map.insert(key.into(), value);
        }
    }
    for key in [
        "name",
        "alias",
        "duration",
        "short-break",
        "long-break",
        "rounds",
    ] {
        kept.remove(key);
    }
    map.extend(kept);
    map
}

/// A preset as the settings form describes it.
pub fn preset(name: &str, alias: &str, duration: &str) -> PresetConfig {
    PresetConfig {
        name: name.to_string(),
        alias: Some(alias.to_string()),
        duration: duration.to_string(),
        ..PresetConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str, contents: Option<&str>) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("lumen-timers-store-{tag}-{}", std::process::id()));
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
  "translate": { "service": "google" },
  "future-setting": true
}"#;

    #[test]
    fn a_preset_is_added_and_the_rest_of_the_file_is_kept_in_order() {
        let file = Scratch::new("add", Some(CONFIG));

        add(&file.0, preset("Deep work", "dw", "1h30")).unwrap();

        let written = file.contents();
        let theme = written.find("\"theme\"").unwrap();
        let future = written.find("\"future-setting\"").unwrap();
        let timers = written.find("\"timers\"").unwrap();
        assert!(theme < future && future < timers, "{written}");

        let config: crate::Config = serde_json::from_str(&written).unwrap();
        assert_eq!(config.theme, "daylight");
        let presets = config.timers.presets();
        assert_eq!(presets[0].name, "Deep work");
        assert_eq!(presets[0].duration, Duration::from_secs(90 * 60));
    }

    #[test]
    fn an_empty_alias_is_left_out() {
        let file = Scratch::new("alias", None);
        add(&file.0, preset("Tea", " ", "3m")).unwrap();
        assert!(!file.contents().contains("\"alias\""));
    }

    #[test]
    fn replacing_keeps_the_place_and_unknown_fields() {
        let file = Scratch::new(
            "replace",
            Some(
                r#"{ "timers": { "presets": [
                    { "name": "A", "duration": "5m", "sound": "Alarm03" },
                    { "name": "B", "duration": "10m" }
                ] } }"#,
            ),
        );

        replace(&file.0, "A", preset("A2", "a", "6m")).unwrap();

        assert!(file.contents().contains("\"sound\""));
        let read = read(&file.0).unwrap();
        assert_eq!(read.presets[0].name, "A2");
        assert_eq!(read.presets[1].name, "B");
    }

    #[test]
    fn a_cycle_is_written_and_unwritten() {
        let file = Scratch::new("cycle", Some(CONFIG));
        let pomodoro = PresetConfig {
            short_break: Some("5m".into()),
            long_break: Some("15m".into()),
            rounds: Some(4),
            ..preset("Pomodoro", "pom", "25m")
        };
        add(&file.0, pomodoro).unwrap();
        let written = file.contents();
        assert!(written.contains("\"short-break\": \"5m\""), "{written}");
        assert!(written.contains("\"rounds\": 4"), "{written}");

        // Emptying the break makes it a plain timer again, with no leftovers.
        replace(&file.0, "Pomodoro", preset("Pomodoro", "pom", "25m")).unwrap();
        let written = file.contents();
        assert!(
            !written.contains("break") && !written.contains("rounds"),
            "{written}"
        );
    }

    #[test]
    fn a_name_or_an_alias_cannot_be_shared() {
        let existing = vec![preset("Deep work", "dw", "90m")];

        assert!(check(&existing, &preset("deep WORK", "", "5m"), None).is_err());
        assert!(check(&existing, &preset("Other", "DW", "5m"), None).is_err());
        // Editing a preset may keep its own name and alias.
        assert!(
            check(
                &existing,
                &preset("Deep work", "dw", "2h"),
                Some("Deep work")
            )
            .is_ok()
        );
    }

    #[test]
    fn what_the_loader_skips_the_page_refuses() {
        assert!(check(&[], &preset("", "", "5m"), None).is_err());
        assert!(check(&[], &preset("X", "", "soon"), None).is_err());
        assert!(check(&[], &preset("X", "", "5m tea"), None).is_err());
        assert!(check(&[], &preset("X", "two words", "5m"), None).is_err());
    }

    #[test]
    fn removing_a_preset_that_is_gone_says_so() {
        let file = Scratch::new("remove", Some(CONFIG));
        add(&file.0, preset("A", "", "5m")).unwrap();

        remove(&file.0, "A").unwrap();
        assert!(read(&file.0).unwrap().presets.is_empty());
        assert!(remove(&file.0, "A").is_err());
    }

    #[test]
    fn the_general_row_is_saved_whole_or_not_at_all() {
        let file = Scratch::new("general", Some(CONFIG));

        set_general(&file.0, None, Ring::UntilDismissed, &["5m".into()]).unwrap();
        let read_back = read(&file.0).unwrap();
        assert_eq!(read_back.alert_sound, None);
        assert_eq!(read_back.ring, Ring::UntilDismissed);
        assert!(file.contents().contains("\"alert-sound\": null"));

        assert!(set_general(&file.0, Some("Ring02"), Ring::Once, &["later".into()]).is_err());
        assert_eq!(read(&file.0).unwrap().ring, Ring::UntilDismissed);

        set_alert_sound(&file.0, Some("Ring02")).unwrap();
        assert_eq!(
            read(&file.0).unwrap().alert_sound.as_deref(),
            Some("Ring02")
        );
    }

    #[test]
    fn quick_durations_are_written_as_typed_or_not_at_all() {
        let file = Scratch::new("quick", Some(CONFIG));

        set_quick_start(&file.0, &["1m".into(), " 1:30 ".into(), String::new()]).unwrap();
        assert_eq!(read(&file.0).unwrap().quick_start, ["1m", "1:30"]);

        let error = set_quick_start(&file.0, &["5m".into(), "later".into()]).unwrap_err();
        assert!(error.to_string().contains("\"later\""), "{error}");
        assert_eq!(read(&file.0).unwrap().quick_start, ["1m", "1:30"]);
    }
}
