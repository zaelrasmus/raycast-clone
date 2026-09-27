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

pub mod clipboard;
pub mod config;
pub mod extension;
pub mod frecency;
pub mod item;
pub mod layout;
pub mod paths;
pub mod protocol;
pub mod providers;
pub mod root;
pub mod search;
pub mod sheets;
pub mod theme;
pub mod view;
pub mod watcher;

pub use config::Config;
pub use frecency::Frecency;
pub use item::{Action, Builtin, Item, Kind, ViewSource};
pub use paths::Paths;
pub use providers::{Provider, Providers};
pub use root::RootSearch;
pub use search::{Candidate, Match, Search};
pub use sheets::{Sheet, SheetEntry, SheetLibrary};
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
/// Write the example script command into the scripts folder if it is not there.
///
/// One example, heavily commented, because the header format is the only thing
/// about tier T1 that has to be learned -- and a working file to copy teaches it
/// faster than a section of the README nobody opens.
pub fn install_example_script(scripts_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(scripts_dir)?;

    let path = scripts_dir.join("example.ps1");
    if !path.exists() {
        std::fs::write(&path, providers::scripts::EXAMPLE)?;
    }

    Ok(())
}

/// Write the two example extensions out, if they are not already there.
///
/// They are compiled in and written to the extensions folder rather than run
/// from inside the binary, because the point of an example is that it can be
/// opened, read and changed. An extension that only exists as bytes in an
/// executable teaches nobody how to write the next one.
///
/// Never overwrites: an example the user has edited is theirs.
pub fn install_example_extensions(extensions_dir: &Path) -> Result<()> {
    // Only ever on a first run. Deleting the folder is how somebody says they
    // do not want the examples, and an installer that puts them back is an
    // installer arguing with the user.
    if extensions_dir.exists() {
        return Ok(());
    }

    const FILES: &[(&str, &str, &str)] = &[
        (
            "lumen-clock",
            include_str!("../../../assets/extensions/lumen-clock/extension.json"),
            include_str!("../../../assets/extensions/lumen-clock/main.ps1"),
        ),
        (
            "lumen-unicode",
            include_str!("../../../assets/extensions/lumen-unicode/extension.json"),
            include_str!("../../../assets/extensions/lumen-unicode/main.ps1"),
        ),
    ];

    for (folder, manifest, script) in FILES {
        let directory = extensions_dir.join(folder);
        std::fs::create_dir_all(&directory)?;

        let manifest_path = directory.join(extension::MANIFEST);
        if !manifest_path.exists() {
            std::fs::write(&manifest_path, manifest)?;
        }

        let script_path = directory.join("main.ps1");
        if !script_path.exists() {
            // Written with a byte-order mark, and it is not optional.
            //
            // Windows PowerShell 5.1 reads a `.ps1` without one as the
            // machine's ANSI codepage, so every multi-byte character in the
            // file becomes mojibake -- and mojibake inside a quoted string is
            // a parse error, several lines away from anything that looks
            // wrong. The first run of this extension failed exactly that way:
            // "The string is missing the terminator", pointing at a line
            // whose only crime was containing a clock emoji.
            //
            // A fact about the file format rather than about the platform, so
            // it needs no `cfg` and breaks no invariant: a `.ps1` is UTF-8
            // only if it says so.
            let mut bytes = Vec::with_capacity(script.len() + 3);
            bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            bytes.extend_from_slice(script.as_bytes());
            std::fs::write(&script_path, bytes)?;
        }
    }

    Ok(())
}

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

    /// The shipped extensions are compiled in, so a manifest that will not
    /// parse is a failing build rather than a thing to discover on somebody's
    /// machine. The scripts are only checked for existence -- whether
    /// PowerShell likes them is not something a unit test can know.
    #[test]
    fn the_example_extensions_have_valid_manifests() {
        for source in [
            include_str!("../../../assets/extensions/lumen-clock/extension.json"),
            include_str!("../../../assets/extensions/lumen-unicode/extension.json"),
        ] {
            let manifest: extension::Manifest =
                serde_json::from_str(source).expect("a shipped manifest must parse");

            assert_eq!(manifest.schema_version, extension::SCHEMA_VERSION);
            assert!(!manifest.run.is_empty());
            assert!(!manifest.commands.is_empty());
            assert!(manifest.commands.iter().all(|c| !c.id.is_empty()));
        }
    }

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
