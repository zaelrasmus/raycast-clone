//! Global hotkeys and aliases for any command (F17).
//!
//! One file, `shortcuts.json` beside `config.json`, keyed by item id:
//!
//! ```json
//! { "commands": { "window:left-half": { "hotkey": "Ctrl+Alt+Left", "alias": "lh" } } }
//! ```
//!
//! Configuration rather than state, unlike the favourites: people edit it by
//! hand, so a hotkey is written as it is shown, and one that does not parse
//! costs only itself -- the rest of the file, and that command's alias, still
//! load. An alias is stored in lowercase, letters, digits and single spaces;
//! one that is not is likewise reported and left out alone.
//!
//! An id whose item is gone -- an application uninstalled, a quicklink
//! deleted -- is kept, not dropped, for the favourites' reason: the binding
//! simply has nothing to run until the item comes back, and an update that
//! briefly removes a shortcut must not cost the user their hotkey.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use lumen_platform::Chord;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Shortcuts {
    pub commands: BTreeMap<String, CommandShortcut>,
}

/// What one command is bound to. Either half may be missing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CommandShortcut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<Chord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// The file as written, before any hotkey is parsed: a hotkey is read as a
/// plain string so one that is not a chord fails alone.
#[derive(Deserialize)]
struct RawShortcuts {
    #[serde(default)]
    commands: BTreeMap<String, RawCommandShortcut>,
}

#[derive(Deserialize)]
struct RawCommandShortcut {
    #[serde(default)]
    hotkey: Option<String>,
    #[serde(default)]
    alias: Option<String>,
}

impl Shortcuts {
    /// Load from disk, falling back to none. Like the favourites, an
    /// unreadable file is reported and survived, not fatal; a hotkey that is
    /// not a chord is reported and left out, keeping the rest of its entry.
    pub fn load(path: &Path) -> Self {
        let Ok(source) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let raw: RawShortcuts = match serde_json::from_str(&source) {
            Ok(raw) => raw,
            Err(error) => {
                log::warn!(
                    "ignoring unreadable shortcuts at {}: {error}",
                    path.display()
                );
                return Self::default();
            }
        };
        let commands = raw
            .commands
            .into_iter()
            .map(|(id, entry)| {
                let hotkey = entry.hotkey.and_then(|text| match text.parse::<Chord>() {
                    Ok(chord) => Some(chord),
                    Err(error) => {
                        log::warn!(
                            "ignoring the hotkey for {id:?} in {}: {error}",
                            path.display()
                        );
                        None
                    }
                });
                let alias = checked_alias(entry.alias, &id, path);
                let shortcut = CommandShortcut { hotkey, alias };
                (id, shortcut)
            })
            .collect();
        Self { commands }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        // Aliases as they will be read back, so the file shows what is
        // matched.
        let mut stored = self.clone();
        for (id, shortcut) in &mut stored.commands {
            shortcut.alias = checked_alias(shortcut.alias.take(), id, path);
        }
        let source = serde_json::to_string_pretty(&stored)?;
        // Beside and renamed over, as the favourites are written.
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, source)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
    }

    /// The commands that have a hotkey, with it, in id order: what the
    /// daemon registers, slot `i` being entry `i`.
    pub fn hotkeys(&self) -> Vec<(&str, &Chord)> {
        self.commands
            .iter()
            .filter_map(|(id, shortcut)| Some((id.as_str(), shortcut.hotkey.as_ref()?)))
            .collect()
    }

    /// The commands that have an alias, with it, in id order.
    pub fn aliases(&self) -> impl Iterator<Item = (&str, &str)> {
        self.commands
            .iter()
            .filter_map(|(id, shortcut)| Some((id.as_str(), shortcut.alias.as_deref()?)))
    }

    /// Each alias two or more commands share, with their ids in order: what
    /// settings flags. Both still lead when it is typed.
    pub fn conflicts(&self) -> BTreeMap<String, Vec<String>> {
        let mut by_alias: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (id, alias) in self.aliases() {
            by_alias
                .entry(alias.to_string())
                .or_default()
                .push(id.to_string());
        }
        by_alias.retain(|_, ids| ids.len() > 1);
        by_alias
    }
}

/// An alias as it is stored: trimmed and in lowercase, letters `a` to `z`,
/// digits and single spaces. `None` for anything else.
///
/// Narrow on purpose: an alias is typed at the start of a query, so it is
/// something a keyboard types the same way in every layout, and a run of
/// spaces is a typo nobody could see.
pub fn normalise_alias(alias: &str) -> Option<String> {
    let alias = alias.trim().to_lowercase();
    let allowed = alias
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ');
    (allowed && !alias.is_empty() && !alias.contains("  ")).then_some(alias)
}

/// `alias` as stored, or nothing with a warning naming `id` and `path`.
fn checked_alias(alias: Option<String>, id: &str, path: &Path) -> Option<String> {
    let alias = alias?;
    let normalised = normalise_alias(&alias);
    if normalised.is_none() {
        log::warn!(
            "ignoring the alias {alias:?} for {id:?} in {}: use letters, digits and single spaces",
            path.display()
        );
    }
    normalised
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lumen-shortcuts-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("shortcuts.json")
    }

    fn chord(text: &str) -> Chord {
        text.parse().unwrap()
    }

    #[test]
    fn a_missing_file_is_no_shortcuts() {
        let path = temporary("missing");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Shortcuts::load(&path), Shortcuts::default());
    }

    #[test]
    fn an_unreadable_file_is_no_shortcuts() {
        let path = temporary("unreadable");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Shortcuts::load(&path), Shortcuts::default());
    }

    #[test]
    fn round_trips() {
        let path = temporary("round-trip");
        let mut shortcuts = Shortcuts::default();
        shortcuts.commands.insert(
            "window:left-half".into(),
            CommandShortcut {
                hotkey: Some(chord("Ctrl+Alt+Left")),
                alias: Some("lh".into()),
            },
        );
        shortcuts.commands.insert(
            "app:code".into(),
            CommandShortcut {
                hotkey: None,
                alias: Some("c".into()),
            },
        );
        shortcuts.save(&path).unwrap();
        assert_eq!(Shortcuts::load(&path), shortcuts);
    }

    #[test]
    fn missing_halves_are_left_out_of_the_file() {
        let path = temporary("omitted");
        let mut shortcuts = Shortcuts::default();
        shortcuts.commands.insert(
            "window:center".into(),
            CommandShortcut {
                hotkey: Some(chord("Ctrl+Alt+C")),
                alias: None,
            },
        );
        shortcuts.commands.insert(
            "app:code".into(),
            CommandShortcut {
                hotkey: None,
                alias: Some("c".into()),
            },
        );
        shortcuts.save(&path).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "commands": {
                "window:center": { "hotkey": "Ctrl+Alt+C" },
                "app:code": { "alias": "c" },
            }})
        );
    }

    #[test]
    fn one_bad_hotkey_costs_only_itself() {
        let path = temporary("bad-hotkey");
        std::fs::write(
            &path,
            r#"{ "commands": {
                "window:center": { "hotkey": "Ctrl+Nope", "alias": "ce" },
                "window:left-half": { "hotkey": "Ctrl+Alt+Left" }
            } }"#,
        )
        .unwrap();
        let shortcuts = Shortcuts::load(&path);
        assert_eq!(
            shortcuts.commands["window:center"],
            CommandShortcut {
                hotkey: None,
                alias: Some("ce".into()),
            }
        );
        assert_eq!(
            shortcuts.commands["window:left-half"].hotkey,
            Some(chord("Ctrl+Alt+Left"))
        );
    }

    #[test]
    fn ids_whose_item_is_gone_are_kept() {
        // Nothing here knows which items exist, and nothing prunes: a
        // binding for an uninstalled application survives a save.
        let path = temporary("kept");
        std::fs::write(
            &path,
            r#"{ "commands": { "app:uninstalled": { "hotkey": "Ctrl+Alt+U" } } }"#,
        )
        .unwrap();
        let shortcuts = Shortcuts::load(&path);
        shortcuts.save(&path).unwrap();
        assert!(
            Shortcuts::load(&path)
                .commands
                .contains_key("app:uninstalled")
        );
    }

    #[test]
    fn the_hotkeys_are_listed_in_id_order_without_the_unbound() {
        let mut shortcuts = Shortcuts::default();
        for (id, hotkey) in [
            ("window:right-half", Some("Ctrl+Alt+Right")),
            ("app:code", None),
            ("window:left-half", Some("Ctrl+Alt+Left")),
        ] {
            shortcuts.commands.insert(
                id.into(),
                CommandShortcut {
                    hotkey: hotkey.map(chord),
                    alias: Some("x".into()),
                },
            );
        }
        let left = chord("Ctrl+Alt+Left");
        let right = chord("Ctrl+Alt+Right");
        assert_eq!(
            shortcuts.hotkeys(),
            [("window:left-half", &left), ("window:right-half", &right)]
        );
    }

    #[test]
    fn an_alias_is_stored_in_lowercase_and_trimmed() {
        assert_eq!(normalise_alias("  LH "), Some("lh".into()));
        assert_eq!(normalise_alias("Left Half 2"), Some("left half 2".into()));
    }

    #[test]
    fn an_alias_is_letters_digits_and_single_spaces() {
        for invalid in [
            "",
            "   ",
            "left  half",
            "l-h",
            "lh!",
            "\u{e9}t\u{e9}",
            "a\tb",
        ] {
            assert_eq!(normalise_alias(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn one_bad_alias_costs_only_itself() {
        let path = temporary("bad-alias");
        std::fs::write(
            &path,
            r#"{ "commands": {
                "window:center": { "hotkey": "Ctrl+Alt+C", "alias": "c-e" },
                "window:left-half": { "alias": "LH" }
            } }"#,
        )
        .unwrap();
        let shortcuts = Shortcuts::load(&path);
        assert_eq!(
            shortcuts.commands["window:center"],
            CommandShortcut {
                hotkey: Some(chord("Ctrl+Alt+C")),
                alias: None,
            }
        );
        assert_eq!(
            shortcuts.commands["window:left-half"].alias.as_deref(),
            Some("lh")
        );
    }

    #[test]
    fn an_alias_is_saved_as_it_is_stored() {
        let path = temporary("saved-alias");
        let mut shortcuts = Shortcuts::default();
        shortcuts.commands.insert(
            "window:left-half".into(),
            CommandShortcut {
                hotkey: None,
                alias: Some(" Left Half ".into()),
            },
        );
        shortcuts.save(&path).unwrap();
        assert_eq!(
            Shortcuts::load(&path).commands["window:left-half"]
                .alias
                .as_deref(),
            Some("left half")
        );
    }

    #[test]
    fn a_shared_alias_is_a_conflict_between_its_commands() {
        let mut shortcuts = Shortcuts::default();
        for (id, alias) in [
            ("window:right-half", Some("h")),
            ("window:left-half", Some("h")),
            ("app:code", Some("c")),
            ("app:slack", None),
        ] {
            shortcuts.commands.insert(
                id.into(),
                CommandShortcut {
                    hotkey: None,
                    alias: alias.map(Into::into),
                },
            );
        }
        let expected: BTreeMap<String, Vec<String>> = [(
            "h".to_string(),
            vec![
                "window:left-half".to_string(),
                "window:right-half".to_string(),
            ],
        )]
        .into();
        assert_eq!(shortcuts.conflicts(), expected);
        assert_eq!(
            shortcuts.aliases().collect::<Vec<_>>(),
            [
                ("app:code", "c"),
                ("window:left-half", "h"),
                ("window:right-half", "h"),
            ]
        );
    }
}
