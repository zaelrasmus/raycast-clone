//! Global command hotkeys (F17).
//!
//! A [`Chord`] is what a command is bound to: modifiers plus one key, written
//! the way it is shown (`Ctrl+Alt+Shift+F11`). Key *names* only -- no
//! virtual-key or keysym values cross this boundary; each platform maps the
//! names to its own codes.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Hotkey, Modifiers};

/// A global hotkey bound to a command: modifiers plus exactly one key.
///
/// Unlike [`Hotkey`] it has no hold gesture; that belongs to the launcher.
/// Written and read as a string, which is also its serde form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Chord {
    pub modifiers: Modifiers,
    /// The key's canonical name: see [`canonical_key`].
    pub key: String,
}

/// Why a string is not a [`Chord`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseChordError(String);

impl fmt::Display for ParseChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseChordError {}

/// Why one command hotkey could not be registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    /// Another application, or Lumen itself, already holds the combination.
    Taken,
    /// The key has no mapping on this platform.
    Unsupported(String),
    /// Anything else the system reported.
    Failed(String),
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Taken => f.write_str("the combination is already in use"),
            Self::Unsupported(why) => write!(f, "not supported here: {why}"),
            Self::Failed(why) => write!(f, "could not be registered: {why}"),
        }
    }
}

impl std::error::Error for HotkeyError {}

/// Every key a chord may name, in its canonical spelling.
///
/// Letters and digits are the uppercase character (`K`, `7`). Function keys
/// are `F1` to `F24`. Punctuation is spelled as a word -- `Backtick`, `Minus`,
/// `Equals`, `LeftBracket`, `RightBracket`, `Backslash`, `Semicolon`, `Quote`,
/// `Comma`, `Period`, `Slash` -- so a chord never has to escape a character
/// in a configuration file; the character itself is accepted on input. The
/// names describe the US layout's key positions, which is what every
/// accelerator API means by them.
pub const KEY_NAMES: &[&str] = &[
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "F13",
    "F14",
    "F15",
    "F16",
    "F17",
    "F18",
    "F19",
    "F20",
    "F21",
    "F22",
    "F23",
    "F24",
    "Space",
    "Enter",
    "Tab",
    "Escape",
    "Backspace",
    "Delete",
    "Insert",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Left",
    "Right",
    "Up",
    "Down",
    "Backtick",
    "Minus",
    "Equals",
    "LeftBracket",
    "RightBracket",
    "Backslash",
    "Semicolon",
    "Quote",
    "Comma",
    "Period",
    "Slash",
];

/// Other spellings accepted on input, lowercase, and the key each one means.
const ALIASES: &[(&str, &str)] = &[
    ("esc", "Escape"),
    ("return", "Enter"),
    ("del", "Delete"),
    ("ins", "Insert"),
    ("pgup", "PageUp"),
    ("pgdn", "PageDown"),
    ("grave", "Backtick"),
    ("`", "Backtick"),
    ("-", "Minus"),
    ("=", "Equals"),
    ("equal", "Equals"),
    ("[", "LeftBracket"),
    ("]", "RightBracket"),
    ("\\", "Backslash"),
    (";", "Semicolon"),
    ("'", "Quote"),
    ("apostrophe", "Quote"),
    (",", "Comma"),
    (".", "Period"),
    ("dot", "Period"),
    ("/", "Slash"),
];

/// The canonical spelling of a key name, or `None` if it is not one.
///
/// Case-insensitive, surrounding space ignored. Also used for the launcher's
/// [`Hotkey::key`], so that `space` and `Space` are the same key everywhere.
pub fn canonical_key(name: &str) -> Option<&'static str> {
    let name = name.trim();
    KEY_NAMES
        .iter()
        .copied()
        .find(|key| key.eq_ignore_ascii_case(name))
        .or_else(|| {
            ALIASES
                .iter()
                .find(|(alias, _)| alias.eq_ignore_ascii_case(name))
                .map(|&(_, key)| key)
        })
}

/// `F13` to `F24`: keys no application types with, so a chord of one alone is
/// a deliberate binding rather than a stolen keystroke.
fn stands_alone(key: &str) -> bool {
    key.strip_prefix('F')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (13..=24).contains(&number))
}

impl Chord {
    /// Whether this chord is the same combination as `hotkey`'s accelerator.
    ///
    /// Compared by canonical key, so a launcher hotkey written `alt+space`
    /// still collides with a command bound to `Alt+Space`.
    pub fn is_hotkey(&self, hotkey: &Hotkey) -> bool {
        self.modifiers == hotkey.modifiers
            && canonical_key(&hotkey.key).is_some_and(|key| key == self.key)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The launcher hotkey's order, so both read the same in settings.
        let m = &self.modifiers;
        for (active, name) in [
            (m.control, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
            (m.super_key, "Win"),
        ] {
            if active {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&self.key)
    }
}

impl FromStr for Chord {
    type Err = ParseChordError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let fail = |why: &str| ParseChordError(format!("{text:?} is not a hotkey: {why}"));

        let mut modifiers = Modifiers::default();
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            if part.is_empty() {
                return Err(fail("a part is empty"));
            }
            let modifier = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut modifiers.control,
                "alt" => &mut modifiers.alt,
                "shift" => &mut modifiers.shift,
                "win" | "super" => &mut modifiers.super_key,
                _ => {
                    let Some(name) = canonical_key(part) else {
                        return Err(fail(&format!("{part:?} is not a key name")));
                    };
                    if key.replace(name).is_some() {
                        return Err(fail("it names more than one key"));
                    }
                    continue;
                }
            };
            *modifier = true;
        }

        let Some(key) = key else {
            return Err(fail("it names no key"));
        };
        if modifiers == Modifiers::default() && !stands_alone(key) {
            return Err(fail("only F13 to F24 may be bound without a modifier"));
        }
        Ok(Self {
            modifiers,
            key: key.to_string(),
        })
    }
}

impl Serialize for Chord {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::IntoDeserializer;
    use serde::de::value::{Error as ValueError, StrDeserializer};

    fn chord(text: &str) -> Chord {
        text.parse()
            .unwrap_or_else(|error| panic!("{text:?}: {error}"))
    }

    #[test]
    fn round_trips_canonical_chords() {
        for text in [
            "Ctrl+Alt+Shift+F11",
            "Ctrl+Alt+Left",
            "Shift+Win+M",
            "Ctrl+Win+Space",
            "Alt+0",
            "Ctrl+Shift+PageDown",
            "Ctrl+Alt+Backtick",
            "Ctrl+Comma",
            "Alt+Backslash",
            "F13",
            "F24",
            "Shift+F24",
            "Ctrl+Escape",
            "Ctrl+Alt+Delete",
        ] {
            assert_eq!(chord(text).to_string(), text);
        }
    }

    #[test]
    fn writes_modifiers_in_the_launcher_hotkeys_order() {
        // Same order as `Hotkey`'s Display: Ctrl, Alt, Shift, Win.
        assert_eq!(chord("Win+Shift+M").to_string(), "Shift+Win+M");
        assert_eq!(chord("Shift+Alt+Ctrl+K").to_string(), "Ctrl+Alt+Shift+K");
    }

    #[test]
    fn parses_case_insensitively_with_aliases() {
        assert_eq!(chord("control+alt+k"), chord("Ctrl+Alt+K"));
        assert_eq!(chord("SUPER+shift+m"), chord("Win+Shift+M"));
        assert_eq!(chord("ctrl + alt + left"), chord("Ctrl+Alt+Left"));
        assert_eq!(chord("Ctrl+Esc").to_string(), "Ctrl+Escape");
        assert_eq!(chord("Ctrl+Return").to_string(), "Ctrl+Enter");
        assert_eq!(chord("Ctrl+pgup").to_string(), "Ctrl+PageUp");
        assert_eq!(chord("f13").to_string(), "F13");
    }

    #[test]
    fn accepts_punctuation_as_characters_or_names() {
        for (character, name) in [
            ("`", "Backtick"),
            ("-", "Minus"),
            ("=", "Equals"),
            ("[", "LeftBracket"),
            ("]", "RightBracket"),
            ("\\", "Backslash"),
            (";", "Semicolon"),
            ("'", "Quote"),
            (",", "Comma"),
            (".", "Period"),
            ("/", "Slash"),
        ] {
            assert_eq!(chord(&format!("Ctrl+{character}")).key, name);
            assert_eq!(chord(&format!("ctrl+{}", name.to_lowercase())).key, name);
        }
    }

    #[test]
    fn knows_every_documented_key() {
        for name in [
            "A",
            "Z",
            "0",
            "9",
            "F1",
            "F12",
            "F24",
            "Space",
            "Enter",
            "Tab",
            "Escape",
            "Backspace",
            "Delete",
            "Insert",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "Left",
            "Right",
            "Up",
            "Down",
        ] {
            assert_eq!(canonical_key(name), Some(name), "{name}");
        }
        assert_eq!(canonical_key("F25"), None);
        assert_eq!(canonical_key("F0"), None);
        assert_eq!(canonical_key("Nonsense"), None);
    }

    #[test]
    fn rejects_malformed_chords() {
        for text in [
            "",
            "Ctrl+Alt",
            "Ctrl+",
            "Ctrl+A+B",
            "K",
            "Space",
            "F12",
            "Ctrl+Nonsense",
            "Hyper+K",
            "Ctrl++",
        ] {
            assert!(text.parse::<Chord>().is_err(), "{text:?} should not parse");
        }
    }

    #[test]
    fn f13_to_f24_stand_alone() {
        assert_eq!(chord("F13").modifiers, Modifiers::default());
        assert_eq!(chord("F24").key, "F24");
    }

    #[test]
    fn deserializes_from_a_string() {
        let deserializer: StrDeserializer<'_, ValueError> = "ctrl+alt+left".into_deserializer();
        let parsed = Chord::deserialize(deserializer).unwrap();
        assert_eq!(parsed.to_string(), "Ctrl+Alt+Left");

        let deserializer: StrDeserializer<'_, ValueError> = "Ctrl+Alt".into_deserializer();
        assert!(Chord::deserialize(deserializer).is_err());
    }

    #[test]
    fn serializes_as_its_display_string() {
        // `Serialize` is `collect_str(self)`; `serde_json` is not a dependency of
        // this crate, so the string it writes is checked through `Display`,
        // which is the same code path.
        assert_eq!(chord("win+shift+m").to_string(), "Shift+Win+M");
    }

    #[test]
    fn matches_the_launcher_hotkey_regardless_of_spelling() {
        let launcher = Hotkey {
            modifiers: Modifiers {
                alt: true,
                ..Default::default()
            },
            key: "space".into(),
            hold: None,
        };
        assert!(chord("Alt+Space").is_hotkey(&launcher));
        assert!(!chord("Ctrl+Alt+Space").is_hotkey(&launcher));
        assert!(!chord("Alt+K").is_hotkey(&launcher));
    }

    #[test]
    fn hotkey_errors_read_as_sentences() {
        assert!(HotkeyError::Taken.to_string().contains("already"));
        assert!(
            HotkeyError::Unsupported("Linux".into())
                .to_string()
                .contains("Linux")
        );
        assert!(
            HotkeyError::Failed("boom".into())
                .to_string()
                .contains("boom")
        );
    }
}
