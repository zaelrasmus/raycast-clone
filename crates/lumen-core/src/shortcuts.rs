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

    pub fn alias_of(&self, id: &str) -> Option<&str> {
        self.commands.get(id)?.alias.as_deref()
    }

    pub fn hotkey_of(&self, id: &str) -> Option<&Chord> {
        self.commands.get(id)?.hotkey.as_ref()
    }

    /// Give `id` this alias, or none. An entry left with neither half goes,
    /// so the file holds only what binds something.
    pub fn set_alias(&mut self, id: &str, alias: Option<String>) {
        self.commands.entry(id.to_string()).or_default().alias = alias;
        self.tidy(id);
    }

    /// The command other than `except` that `chord` is bound to: what the
    /// recorder names before offering to take it.
    pub fn hotkey_owner(&self, chord: &Chord, except: &str) -> Option<&str> {
        self.commands
            .iter()
            .find(|(id, shortcut)| *id != except && shortcut.hotkey.as_ref() == Some(chord))
            .map(|(id, _)| id.as_str())
    }

    /// Bind `chord` to `id`, or unbind it. A chord another command had is
    /// taken from it -- settings asks first -- and that command is returned.
    /// One chord, one command: the registrations are slots, and two slots
    /// for one chord would leave the second `Taken` by the first.
    pub fn set_hotkey(&mut self, id: &str, chord: Option<Chord>) -> Option<String> {
        let taken = chord
            .as_ref()
            .and_then(|chord| self.hotkey_owner(chord, id))
            .map(str::to_string);
        if let Some(other) = &taken {
            if let Some(shortcut) = self.commands.get_mut(other) {
                shortcut.hotkey = None;
            }
            self.tidy(other);
        }
        self.commands.entry(id.to_string()).or_default().hotkey = chord;
        self.tidy(id);
        taken
    }

    /// What the launcher's action panel offers to change about `id`'s
    /// shortcuts, in its order: each half set or changed, and deleted when
    /// it is set.
    pub fn configure(&self, id: &str) -> Vec<Configure> {
        let (alias, hotkey) = (self.alias_of(id).is_some(), self.hotkey_of(id).is_some());
        let mut offered = vec![if alias {
            Configure::ChangeAlias
        } else {
            Configure::SetAlias
        }];
        offered.extend(alias.then_some(Configure::DeleteAlias));
        offered.push(if hotkey {
            Configure::ChangeHotkey
        } else {
            Configure::SetHotkey
        });
        offered.extend(hotkey.then_some(Configure::DeleteHotkey));
        offered
    }

    fn tidy(&mut self, id: &str) {
        if self
            .commands
            .get(id)
            .is_some_and(|shortcut| shortcut.hotkey.is_none() && shortcut.alias.is_none())
        {
            self.commands.remove(id);
        }
    }
}

/// A change to a command's shortcuts from the launcher's action panel:
/// Raycast's *Configure Command* (F17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Configure {
    SetAlias,
    ChangeAlias,
    DeleteAlias,
    SetHotkey,
    ChangeHotkey,
    DeleteHotkey,
}

impl Configure {
    pub fn verb(self) -> &'static str {
        match self {
            Configure::SetAlias => "Set Alias",
            Configure::ChangeAlias => "Change Alias",
            Configure::DeleteAlias => "Delete Alias",
            Configure::SetHotkey => "Set Hotkey",
            Configure::ChangeHotkey => "Change Hotkey",
            Configure::DeleteHotkey => "Delete Hotkey",
        }
    }

    /// Whether it asks for something first -- an alias typed, a chord
    /// pressed -- rather than saving at once.
    pub fn asks(self) -> bool {
        !matches!(self, Configure::DeleteAlias | Configure::DeleteHotkey)
    }

    /// Whether it is about the alias rather than the hotkey.
    pub fn is_alias(self) -> bool {
        matches!(
            self,
            Configure::SetAlias | Configure::ChangeAlias | Configure::DeleteAlias
        )
    }
}

/// What settings says of an alias it will not save.
pub const ALIAS_RULE: &str = "Use letters, digits and single spaces";

/// An alias as typed into settings: nothing typed clears it, anything else
/// is stored as [`normalise_alias`] stores it, or refused with the rule.
pub fn alias_input(text: &str) -> Result<Option<String>, &'static str> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    normalise_alias(text).map(Some).ok_or(ALIAS_RULE)
}

/// Which of Lumen's own hotkeys `chord` is, if one: those stay registered
/// while a command hotkey is recorded, and a command bound to one would be
/// `Taken` (7.53).
pub fn reserved_by(
    chord: &Chord,
    launcher: &lumen_platform::Hotkey,
    timers: Option<&lumen_platform::Hotkey>,
) -> Option<&'static str> {
    if chord.is_hotkey(launcher) {
        return Some("Used by the Lumen launcher");
    }
    timers
        .filter(|timers| chord.is_hotkey(timers))
        .map(|_| "Used by Lumen's Timers hotkey")
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

    fn bound(entries: &[(&str, Option<&str>, Option<&str>)]) -> Shortcuts {
        let mut shortcuts = Shortcuts::default();
        for (id, hotkey, alias) in entries {
            shortcuts.commands.insert(
                id.to_string(),
                CommandShortcut {
                    hotkey: hotkey.map(chord),
                    alias: alias.map(Into::into),
                },
            );
        }
        shortcuts
    }

    /// What the settings field does with what was typed: empty clears,
    /// anything else is stored as it will be matched, or refused.
    #[test]
    fn an_alias_typed_in_settings_is_cleared_stored_or_refused() {
        assert_eq!(alias_input("  "), Ok(None));
        assert_eq!(alias_input(" LH "), Ok(Some("lh".into())));
        assert_eq!(alias_input("l-h"), Err(ALIAS_RULE));
        assert_eq!(alias_input("left  half"), Err(ALIAS_RULE));
    }

    #[test]
    fn setting_an_alias_keeps_the_hotkey_and_clearing_both_drops_the_entry() {
        let mut shortcuts = bound(&[("window:center", Some("Ctrl+Alt+C"), None)]);
        shortcuts.set_alias("window:center", Some("ce".into()));
        assert_eq!(shortcuts.alias_of("window:center"), Some("ce"));
        assert_eq!(
            shortcuts.hotkey_of("window:center"),
            Some(&chord("Ctrl+Alt+C"))
        );

        shortcuts.set_hotkey("window:center", None);
        shortcuts.set_alias("window:center", None);
        assert!(shortcuts.commands.is_empty(), "{shortcuts:?}");

        shortcuts.set_alias("app:code", Some("c".into()));
        assert_eq!(shortcuts.alias_of("app:code"), Some("c"));
    }

    #[test]
    fn a_chord_is_owned_by_the_command_bound_to_it_other_than_the_one_asking() {
        let shortcuts = bound(&[
            ("window:center", Some("Ctrl+Alt+C"), None),
            ("app:code", Some("Ctrl+Alt+V"), Some("c")),
        ]);
        let center = chord("ctrl+alt+c");
        assert_eq!(
            shortcuts.hotkey_owner(&center, "app:code"),
            Some("window:center")
        );
        assert_eq!(shortcuts.hotkey_owner(&center, "window:center"), None);
        assert_eq!(
            shortcuts.hotkey_owner(&chord("Ctrl+Alt+X"), "app:code"),
            None
        );
    }

    /// Overwrite: the chord moves, and the command it moved from keeps its
    /// alias -- or goes, if the hotkey was all it had.
    #[test]
    fn binding_a_taken_chord_takes_it_from_the_other_command() {
        let mut shortcuts = bound(&[
            ("window:center", Some("Ctrl+Alt+C"), Some("ce")),
            ("window:left-half", Some("Ctrl+Alt+L"), None),
        ]);
        let taken = shortcuts.set_hotkey("app:code", Some(chord("Ctrl+Alt+C")));
        assert_eq!(taken.as_deref(), Some("window:center"));
        assert_eq!(shortcuts.hotkey_of("app:code"), Some(&chord("Ctrl+Alt+C")));
        assert_eq!(shortcuts.hotkey_of("window:center"), None);
        assert_eq!(shortcuts.alias_of("window:center"), Some("ce"));

        let taken = shortcuts.set_hotkey("app:code", Some(chord("Ctrl+Alt+L")));
        assert_eq!(taken.as_deref(), Some("window:left-half"));
        assert!(!shortcuts.commands.contains_key("window:left-half"));
        assert_eq!(
            shortcuts
                .hotkeys()
                .iter()
                .filter(|(_, bound)| **bound == chord("Ctrl+Alt+C"))
                .count(),
            0
        );
    }

    #[test]
    fn an_edit_made_in_settings_survives_a_save() {
        let path = temporary("settings-edit");
        let mut shortcuts = bound(&[("window:center", Some("Ctrl+Alt+C"), None)]);
        shortcuts.set_alias("app:code", Some("c".into()));
        shortcuts.set_hotkey("app:code", Some(chord("Ctrl+Alt+C")));
        shortcuts.save(&path).unwrap();
        assert_eq!(Shortcuts::load(&path), shortcuts);
        assert_eq!(
            Shortcuts::load(&path).hotkey_of("app:code"),
            Some(&chord("Ctrl+Alt+C"))
        );
    }

    /// The launcher's and the timers' keys stay registered while a hotkey
    /// is recorded, so pressing one never reaches the recorder; typed or
    /// recorded any other way, it is refused by name before it is saved.
    #[test]
    fn a_chord_equal_to_lumens_own_hotkeys_is_reserved() {
        let launcher = crate::config::Config::default().hotkey;
        let timers = crate::timers::presets::default_hotkey();
        assert_eq!(
            reserved_by(&chord("Alt+Space"), &launcher, Some(&timers)),
            Some("Used by the Lumen launcher")
        );
        assert_eq!(
            reserved_by(&chord("Alt+Shift+Space"), &launcher, Some(&timers)),
            Some("Used by Lumen's Timers hotkey")
        );
        assert_eq!(
            reserved_by(&chord("Alt+Shift+Space"), &launcher, None),
            None
        );
        assert_eq!(
            reserved_by(&chord("Ctrl+Alt+Space"), &launcher, Some(&timers)),
            None
        );
    }

    /// What the launcher's action panel offers for a command: set or
    /// change each half, and delete one only when it is set.
    #[test]
    fn configuring_a_command_offers_what_its_shortcuts_allow() {
        use Configure::*;
        let shortcuts = bound(&[
            ("window:center", Some("Ctrl+Alt+C"), Some("ce")),
            ("app:code", None, Some("c")),
            ("app:slack", Some("Ctrl+Alt+S"), None),
        ]);
        assert_eq!(shortcuts.configure("app:none"), [SetAlias, SetHotkey]);
        assert_eq!(
            shortcuts.configure("app:code"),
            [ChangeAlias, DeleteAlias, SetHotkey]
        );
        assert_eq!(
            shortcuts.configure("app:slack"),
            [SetAlias, ChangeHotkey, DeleteHotkey]
        );
        assert_eq!(
            shortcuts.configure("window:center"),
            [ChangeAlias, DeleteAlias, ChangeHotkey, DeleteHotkey]
        );
        assert_eq!(ChangeAlias.verb(), "Change Alias");
        assert!(SetHotkey.asks() && !DeleteHotkey.asks());
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
