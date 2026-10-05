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
//! load.
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
                let shortcut = CommandShortcut {
                    hotkey,
                    alias: entry.alias,
                };
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
        let source = serde_json::to_string_pretty(self)?;
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
}
