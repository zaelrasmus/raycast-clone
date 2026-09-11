//! Index, matching, ranking and theming for Lumen.
//!
//! # Invariant I-1
//!
//! This crate contains no `#[cfg(windows)]`, no `#[cfg(unix)]` and no direct
//! operating-system call. Everything that touches the system goes through
//! `trait Platform`. That is not tidiness: it is the reason the F5 port is
//! estimated at three to six weeks of implementing an already-defined trait
//! rather than a rewrite (R-4).
//!
//! It also contains nothing that depends on GPUI. If GPUI turns out to be
//! unworkable, the presentation layer is replaced without losing the project
//! (AD-03).

pub mod config;
pub mod frecency;
pub mod item;
pub mod layout;
pub mod paths;
pub mod search;
pub mod sheets;
pub mod theme;
pub mod watcher;

pub use config::Config;
pub use frecency::Frecency;
pub use item::{Action, Item};
pub use paths::Paths;
pub use search::{Candidate, Match, Search};
pub use sheets::{Sheet, SheetLibrary};
pub use theme::{Appearance, Color, Theme};

use std::path::Path;

use anyhow::Result;

/// Write the in-house themes into the themes folder if they are not there.
///
/// Two themes, one light and one dark, exist to validate the schema before it is
/// opened to third parties (F2, R-11). A format proven against exactly one theme
/// is a format that encodes that theme's assumptions.
pub fn install_builtin_themes(themes_dir: &Path) -> Result<()> {
    const BUILTIN: [(&str, &str); 2] = [
        (
            "nocturne",
            include_str!("../../../assets/themes/nocturne.json"),
        ),
        (
            "daylight",
            include_str!("../../../assets/themes/daylight.json"),
        ),
    ];

    std::fs::create_dir_all(themes_dir)?;

    for (name, source) in BUILTIN {
        let path = themes_dir.join(format!("{name}.json"));
        // Never overwrite: the user may well have edited the shipped themes,
        // and silently reverting their work on every launch would be the most
        // annoying possible behaviour.
        if !path.exists() {
            std::fs::write(&path, source)?;
        }
    }

    Ok(())
}

/// Write the starter shortcut sheet into the sheets folder if it is not there.
///
/// Only the global sheet ships. Per-application sheets are deliberately left to
/// the user: F3's whole distinction from the PowerToys equivalent is that the
/// shortcuts are *theirs*, annotated by them, rather than a fixed set someone
/// else chose. Shipping a pile of guessed sheets would quietly turn this back
/// into the thing it is trying not to be.
pub fn install_builtin_sheets(sheets_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(sheets_dir)?;

    let path = sheets_dir.join("global.json");
    if !path.exists() {
        std::fs::write(&path, include_str!("../../../assets/sheets/global.json"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped themes are compiled into the binary, so a malformed one is a
    /// build-time fact rather than something to discover on a user's machine.
    #[test]
    fn the_builtin_themes_parse_and_resolve() {
        for source in [
            include_str!("../../../assets/themes/nocturne.json"),
            include_str!("../../../assets/themes/daylight.json"),
        ] {
            let file: theme::ThemeFile =
                serde_json::from_str(source).expect("a shipped theme must parse");
            assert_eq!(file.schema_version, theme::SCHEMA_VERSION);
            let _ = Theme::resolve(&file);
        }
    }

    /// I-11, as a test: no shipped theme may carry a geometric property, because
    /// the schema must not grow one by accident.
    #[test]
    fn no_shipped_theme_declares_geometry() {
        for source in [
            include_str!("../../../assets/themes/nocturne.json"),
            include_str!("../../../assets/themes/daylight.json"),
        ] {
            let value: serde_json::Value = serde_json::from_str(source).unwrap();
            let object = value.as_object().unwrap();
            for forbidden in ["spacing", "radius", "row_height", "sizes", "geometry"] {
                assert!(
                    !object.contains_key(forbidden),
                    "a theme may not define {forbidden:?} (I-11)"
                );
            }
        }
    }
}
