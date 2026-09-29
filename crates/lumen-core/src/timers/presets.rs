//! Saved timers and quick durations: the `timers` section of `config.json`
//! (F8, slice 2).
//!
//! A **preset** is a timer with a name and, optionally, an alias -- *Deep
//! work*, `dw`, 90 minutes -- and each one is a row in the root list. The
//! **quick durations** are the plain lengths people start most, 2, 5, 10 and
//! 30 minutes by default, offered only inside the *Timers* view so that they
//! never crowd the root list.
//!
//! Read leniently, like translate presets: one preset with no name or a
//! duration that does not parse is skipped with a log line, and the rest of
//! the file -- the hotkey, the theme -- still loads.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::duration;

/// The `timers` section of `config.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct TimersConfig {
    pub presets: Vec<PresetConfig>,
    /// Durations as typed in the launcher: `"5m"`, `"1h30"`.
    pub quick_start: Vec<String>,
}

impl Default for TimersConfig {
    fn default() -> Self {
        Self {
            presets: Vec::new(),
            quick_start: ["2m", "5m", "10m", "30m"].map(String::from).to_vec(),
        }
    }
}

/// A preset as written, before it is checked.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct PresetConfig {
    pub name: String,
    pub alias: Option<String>,
    /// As typed in the launcher: `"25m"`, `"1h30"`, `"1:30"`.
    pub duration: String,
}

/// A preset that makes sense.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub name: String,
    pub alias: Option<String>,
    pub duration: Duration,
}

impl TimersConfig {
    /// The presets that make sense, in the order they were written; a second
    /// preset with a name already taken is skipped.
    pub fn presets(&self) -> Vec<Preset> {
        let mut presets: Vec<Preset> = Vec::new();
        for written in &self.presets {
            match resolve(written) {
                Ok(preset) if presets.iter().any(|other| same(&other.name, &preset.name)) => {
                    log::warn!("skipping a second timer preset named {:?}", preset.name);
                }
                Ok(preset) => presets.push(preset),
                Err(problem) => log::warn!("skipping a timer preset: {problem}"),
            }
        }
        presets
    }

    /// The quick durations that parse, shortest first, each once.
    pub fn quick_start(&self) -> Vec<Duration> {
        let mut durations: Vec<Duration> = Vec::new();
        for written in &self.quick_start {
            match parse_duration(written) {
                Some(duration) => durations.push(duration),
                None => log::warn!("skipping a quick-start duration, {written:?}"),
            }
        }
        durations.sort();
        durations.dedup();
        durations
    }
}

/// Names and aliases are compared as the root list matches them: ignoring
/// case and the spaces around them.
pub fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// A duration on its own, with nothing after it: what a preset's field and a
/// quick duration must be.
pub fn parse_duration(written: &str) -> Option<Duration> {
    duration::parse(written)
        .filter(|typed| typed.name.is_none())
        .map(|typed| typed.duration)
}

/// One preset, checked on its own. What the loader skips and the settings
/// page refuses are the same thing, so they are decided here once.
pub fn resolve(written: &PresetConfig) -> Result<Preset, String> {
    let name = written.name.trim();
    if name.is_empty() {
        return Err("a preset needs a name".into());
    }
    let duration = parse_duration(&written.duration).ok_or_else(|| {
        format!(
            "{name:?} has no duration Lumen can read; try 25m, 1h30 or 1:30 (it has {:?})",
            written.duration.trim()
        )
    })?;
    let alias = written
        .alias
        .as_deref()
        .map(str::trim)
        .filter(|alias| !alias.is_empty());
    if alias.is_some_and(|alias| alias.contains(char::is_whitespace)) {
        return Err(format!("{name:?} has an alias with a space in it"));
    }
    Ok(Preset {
        name: name.to_string(),
        alias: alias.map(str::to_string),
        duration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(source: &str) -> TimersConfig {
        serde_json::from_str(source).unwrap()
    }

    #[test]
    fn a_preset_has_a_name_an_alias_and_a_duration() {
        let presets = config(
            r#"{ "presets": [ { "name": " Deep work ", "alias": "dw", "duration": "1h30" } ] }"#,
        )
        .presets();
        assert_eq!(
            presets,
            [Preset {
                name: "Deep work".into(),
                alias: Some("dw".into()),
                duration: Duration::from_secs(90 * 60),
            }]
        );
    }

    #[test]
    fn a_preset_that_makes_no_sense_is_skipped_and_the_rest_load() {
        let presets = config(
            r#"{ "presets": [
                { "name": "", "duration": "5m" },
                { "name": "Tea", "duration": "soon" },
                { "name": "Named", "duration": "5m eggs" },
                { "name": "Eggs", "duration": "7m" },
                { "name": "eggs", "duration": "9m" }
            ] }"#,
        )
        .presets();
        let names: Vec<_> = presets.iter().map(|preset| preset.name.as_str()).collect();
        assert_eq!(names, ["Eggs"]);
    }

    #[test]
    fn an_alias_is_one_word() {
        let error = resolve(&PresetConfig {
            name: "Deep work".into(),
            alias: Some("deep work".into()),
            duration: "90m".into(),
        })
        .unwrap_err();
        assert!(error.contains("space"), "{error}");
    }

    /// Nobody has to configure anything to have the common lengths.
    #[test]
    fn the_quick_durations_default_to_two_five_ten_and_thirty_minutes() {
        let minutes: Vec<u64> = TimersConfig::default()
            .quick_start()
            .iter()
            .map(|duration| duration.as_secs() / 60)
            .collect();
        assert_eq!(minutes, [2, 5, 10, 30]);
        // Absent from the file is the defaults, too.
        assert_eq!(config("{}").quick_start().len(), 4);
    }

    #[test]
    fn quick_durations_are_sorted_once_each_and_nonsense_is_skipped() {
        let quick =
            config(r#"{ "quick-start": ["10m", "1:30", "soon", "10 min", "45s"] }"#).quick_start();
        let seconds: Vec<u64> = quick.iter().map(Duration::as_secs).collect();
        assert_eq!(seconds, [45, 90, 600]);
    }

    /// Emptied on purpose is empty, not the defaults.
    #[test]
    fn an_empty_quick_start_list_stays_empty() {
        assert!(config(r#"{ "quick-start": [] }"#).quick_start().is_empty());
    }
}
