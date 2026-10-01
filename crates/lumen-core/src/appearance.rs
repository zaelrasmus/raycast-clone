//! The look and the root list's arrangement, as the settings window edits
//! them (F15).
//!
//! Three keys of `config.json` -- `theme`, `style` and `root` -- written
//! through [`crate::section`] like every page's: nothing else in the file
//! changes, a file that does not parse is never written, and each write is
//! one rename. The daemon's watch on the file applies them, so a choice made
//! here shows in the launcher at once.

use std::path::Path;

use anyhow::Result;
use serde::Deserialize;

use crate::root::{RootConfig, SectionConfig};
use crate::section;

/// What the page shows, as the file has it.
#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    pub theme: String,
    pub style: String,
    pub root: RootConfig,
}

/// Read the three keys, with the defaults for what the file does not say.
pub fn read(path: &Path) -> Result<Appearance> {
    let defaults = crate::Config::default();
    let or_default = |written: String, default: String| {
        if written.trim().is_empty() {
            default
        } else {
            written
        }
    };
    Ok(Appearance {
        theme: or_default(section::read(path, "theme")?, defaults.theme),
        style: or_default(section::read(path, "style")?, defaults.style),
        root: section::read(path, "root")?,
    })
}

pub fn set_theme(path: &Path, name: &str) -> Result<()> {
    section::set(path, "theme", name.into())
}

pub fn set_style(path: &Path, name: &str) -> Result<()> {
    section::set(path, "style", name.into())
}

pub fn set_root(path: &Path, root: &RootConfig) -> Result<()> {
    section::set(path, "root", serde_json::to_value(root)?)
}

/// The palettes or the styles in `dir`: each file's name without `.json`,
/// which is what `config.json` names, and the name written inside it, which
/// is what a person reads. Sorted by the latter. A file that does not parse
/// is listed by its file name, so it can still be chosen and its mistake
/// seen in the log.
pub fn installed(dir: &Path) -> Vec<(String, String)> {
    #[derive(Deserialize)]
    struct Named {
        name: String,
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, String)> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| {
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            let label = std::fs::read_to_string(&path)
                .ok()
                .and_then(|source| serde_json::from_str::<Named>(&source).ok())
                .map_or_else(|| stem.clone(), |named| named.name);
            Some((stem, label))
        })
        .collect();
    found.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    found
}

/// The ways of arranging the empty list the page offers by name (F15).
/// Anything else is still possible by hand, in `config.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrangement {
    /// Raycast's, and the default: favourites, commands, applications.
    Separate,
    /// Commands and applications in one section, under *Commands*.
    UnderCommands,
    /// The same, under *Applications*.
    UnderApplications,
    /// Commands alone; applications are found by typing.
    CommandsOnly,
}

impl Arrangement {
    pub const ALL: [Arrangement; 4] = [
        Arrangement::Separate,
        Arrangement::UnderCommands,
        Arrangement::UnderApplications,
        Arrangement::CommandsOnly,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Arrangement::Separate => "Favorites, Commands, Applications",
            Arrangement::UnderCommands => "Favorites, then Commands with applications",
            Arrangement::UnderApplications => "Favorites, then Applications with commands",
            Arrangement::CommandsOnly => "Favorites, then Commands only",
        }
    }

    /// The sections, as (title, shows).
    fn shape(self) -> Vec<(&'static str, Vec<&'static str>)> {
        let favorites = ("Favorites", vec!["favorites"]);
        match self {
            Arrangement::Separate => vec![
                favorites,
                ("Commands", vec!["commands"]),
                ("Applications", vec!["applications"]),
            ],
            Arrangement::UnderCommands => {
                vec![favorites, ("Commands", vec!["commands", "applications"])]
            }
            Arrangement::UnderApplications => vec![
                favorites,
                ("Applications", vec!["applications", "commands"]),
            ],
            Arrangement::CommandsOnly => vec![favorites, ("Commands", vec!["commands"])],
        }
    }

    /// Which of these `root` is, whatever its headers say. `None` for an
    /// arrangement written by hand.
    pub fn of(root: &RootConfig) -> Option<Self> {
        Self::ALL.into_iter().find(|arrangement| {
            let shape = arrangement.shape();
            shape.len() == root.sections.len()
                && shape
                    .iter()
                    .zip(&root.sections)
                    .all(|((title, shows), section)| {
                        section.title == *title && section.shows == *shows
                    })
        })
    }

    /// `root` arranged this way. A section that keeps its title keeps
    /// whether its header is drawn; a new one has it drawn.
    pub fn apply(self, root: &RootConfig) -> RootConfig {
        let header = |title: &str| {
            root.sections
                .iter()
                .find(|section| section.title == title)
                .is_none_or(|section| section.header)
        };
        RootConfig {
            sections: self
                .shape()
                .into_iter()
                .map(|(title, shows)| SectionConfig {
                    title: title.into(),
                    shows: shows.into_iter().map(Into::into).collect(),
                    header: header(title),
                })
                .collect(),
            results_header: root.results_header,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, contents: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lumen-appearance-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn reads_defaults_and_writes_each_key_alone() {
        let path = file("write", r#"{ "hotkey": "Alt+Space" }"#);
        let read_back = read(&path).unwrap();
        assert_eq!(read_back.theme, "nocturne");
        assert_eq!(read_back.style, "lumen");
        assert_eq!(read_back.root, RootConfig::default());

        set_theme(&path, "graphite").unwrap();
        set_style(&path, "ink").unwrap();
        let root = RootConfig {
            results_header: false,
            ..RootConfig::default()
        };
        set_root(&path, &root).unwrap();

        let read_back = read(&path).unwrap();
        assert_eq!(
            (read_back.theme.as_str(), read_back.style.as_str()),
            ("graphite", "ink")
        );
        assert!(!read_back.root.results_header);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("Alt+Space")
        );
    }

    #[test]
    fn the_default_is_the_separate_arrangement() {
        assert_eq!(
            Arrangement::of(&RootConfig::default()),
            Some(Arrangement::Separate)
        );
    }

    /// Changing arrangement keeps a hidden header hidden where the section
    /// survives, and every arrangement is recognised again once written.
    #[test]
    fn arranging_keeps_the_headers_that_survive() {
        let mut root = RootConfig::default();
        root.sections[1].header = false;
        let merged = Arrangement::UnderCommands.apply(&root);
        assert_eq!(merged.sections.len(), 2);
        assert!(!merged.sections[1].header);
        assert!(merged.sections[0].header);

        for arrangement in Arrangement::ALL {
            assert_eq!(
                Arrangement::of(&arrangement.apply(&root)),
                Some(arrangement)
            );
        }
    }

    #[test]
    fn a_hand_written_arrangement_is_not_one_of_these() {
        let mut root = RootConfig::default();
        root.sections.swap(1, 2);
        assert_eq!(Arrangement::of(&root), None);
    }

    #[test]
    fn lists_what_is_installed_by_its_written_name() {
        let dir = std::env::temp_dir().join(format!("lumen-installed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ink.json"), r#"{ "name": "Ink" }"#).unwrap();
        std::fs::write(dir.join("broken.json"), "{").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        assert_eq!(
            installed(&dir),
            [
                ("broken".to_string(), "broken".to_string()),
                ("ink".to_string(), "Ink".to_string())
            ]
        );
    }
}
