//! The `files` section of `config.json`, as the settings window edits it
//! (F16, slice 4; `docs/FILE-SEARCH.md`, "The settings page").
//!
//! Through [`crate::section`], like the other pages: only this section
//! changes, in its own key order, a file that does not parse is never
//! written, and the write is one rename. The daemon's watcher on the file
//! does the rest; the index is walked again only when the rules' digest
//! moved.
//!
//! A list is written whole, from what the file means rather than what it
//! holds: a file with no `scopes` -- or F3's `roots` -- has `~` or the old
//! roots as its scopes, and adding one must keep them.

use std::path::{Component, Path};

use anyhow::{Result, bail};
use serde_json::{Map, Value};

use crate::config::{FileSearchConfig, InRoot, PreviewFont};
use crate::file_index::IndexDetail;
use crate::section;

const SECTION: &str = "files";

/// Which of the two path lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Scopes,
    Exclusions,
}

impl List {
    fn key(self) -> &'static str {
        match self {
            List::Scopes => "scopes",
            List::Exclusions => "exclusions",
        }
    }

    fn of(self, config: &FileSearchConfig) -> Vec<String> {
        let paths = match self {
            List::Scopes => &config.scopes,
            List::Exclusions => &config.exclusions,
        };
        paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }
}

/// One of the section's switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    Enabled,
    UseIgnoreFiles,
    IncludeHidden,
    SearchContents,
}

impl Flag {
    fn key(self) -> &'static str {
        match self {
            Flag::Enabled => "enabled",
            Flag::UseIgnoreFiles => "use-ignore-files",
            Flag::IncludeHidden => "include-hidden",
            Flag::SearchContents => "search-contents",
        }
    }
}

/// The section as the file has it, defaults for what it does not say.
pub fn read(path: &Path) -> Result<FileSearchConfig> {
    section::read(path, SECTION)
}

pub fn set_in_root(path: &Path, in_root: InRoot) -> Result<()> {
    let value = serde_json::to_value(in_root)?;
    section::edit(path, SECTION, |files| {
        files.insert("in-root".into(), value);
        Ok(())
    })
}

pub fn set_preview_font(path: &Path, font: PreviewFont) -> Result<()> {
    let value = serde_json::to_value(font)?;
    section::edit(path, SECTION, |files| {
        files.insert("preview-font".into(), value);
        Ok(())
    })
}

pub fn set_index_detail(path: &Path, detail: IndexDetail) -> Result<()> {
    let value = serde_json::to_value(detail)?;
    section::edit(path, SECTION, |files| {
        files.insert("index-detail".into(), value);
        Ok(())
    })
}

/// Write `files.hot-memory`, kept within [`crate::config::HOT_MEMORY`].
pub fn set_hot_memory(path: &Path, megabytes: u32) -> Result<()> {
    let range = crate::config::HOT_MEMORY;
    let value = Value::from(megabytes.clamp(*range.start(), *range.end()));
    section::edit(path, SECTION, |files| {
        files.insert("hot-memory".into(), value);
        Ok(())
    })
}

pub fn set_flag(path: &Path, flag: Flag, on: bool) -> Result<()> {
    section::edit(path, SECTION, |files| {
        files.insert(flag.key().into(), Value::Bool(on));
        Ok(())
    })
}

/// The patterns of the comma-separated field: each trimmed, the empty ones
/// dropped, each once.
pub fn parse_patterns(typed: &str) -> Vec<String> {
    let mut patterns: Vec<String> = Vec::new();
    for pattern in typed.split(',').map(str::trim) {
        if !pattern.is_empty() && !patterns.iter().any(|seen| seen == pattern) {
            patterns.push(pattern.to_string());
        }
    }
    patterns
}

pub fn set_ignore_patterns(path: &Path, patterns: &[String]) -> Result<()> {
    let value = Value::from(patterns.to_vec());
    section::edit(path, SECTION, |files| {
        files.insert("ignore-patterns".into(), value);
        Ok(())
    })
}

/// `folder` as `config.json` writes it: under `home` as `~` or `~/...`, and
/// with forward slashes, so the file reads the same on every machine.
///
/// By its components, never by replacing characters: a backslash is a
/// character of a name on Linux, not a separator (I-1).
pub fn written(folder: &Path, home: Option<&Path>) -> String {
    if let Some(rest) = home.and_then(|home| folder.strip_prefix(home).ok()) {
        let rest = joined(rest);
        return if rest.is_empty() {
            "~".into()
        } else {
            format!("~/{rest}")
        };
    }
    let mut written = String::new();
    let mut names = Vec::new();
    for component in folder.components() {
        match component {
            Component::Prefix(prefix) => {
                written.push_str(&prefix.as_os_str().to_string_lossy());
            }
            Component::RootDir => written.push('/'),
            Component::Normal(name) => names.push(name.to_string_lossy().into_owned()),
            Component::CurDir | Component::ParentDir => {
                names.push(component.as_os_str().to_string_lossy().into_owned());
            }
        }
    }
    written.push_str(&names.join("/"));
    written
}

/// The names of a relative path, joined by `/`.
fn joined(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether two written paths name the same folder: by their spelling once
/// written, in any case -- the picker spells a folder as the disk does, and
/// the file as the user typed it.
fn same(a: &str, b: &str) -> bool {
    let trim = |path: &str| {
        let path = path.trim();
        let trimmed = path.trim_end_matches(['/', '\\']);
        // `E:/` and `/` keep the separator that makes them a root.
        if trimmed.is_empty() || trimmed.ends_with(':') {
            path.to_string()
        } else {
            trimmed.to_string()
        }
    };
    trim(a).replace('\\', "/").to_lowercase() == trim(b).replace('\\', "/").to_lowercase()
}

/// Add `folder`, already [`written`], to `list`. `false`, and nothing
/// written, when it is already there.
pub fn add(path: &Path, list: List, folder: &str) -> Result<bool> {
    let mut paths = list.of(&read(path)?);
    if paths.iter().any(|each| same(each, folder)) {
        return Ok(false);
    }
    paths.push(folder.to_string());
    write_list(path, list, paths)?;
    Ok(true)
}

/// Remove `folder` from `list`, as it is written there.
pub fn remove(path: &Path, list: List, folder: &str) -> Result<()> {
    let mut paths = list.of(&read(path)?);
    let before = paths.len();
    paths.retain(|each| !same(each, folder));
    if paths.len() == before {
        bail!("{folder} is not in the list any more");
    }
    write_list(path, list, paths)
}

fn write_list(path: &Path, list: List, paths: Vec<String>) -> Result<()> {
    section::edit(path, SECTION, |files| {
        files.insert(list.key().into(), Value::from(paths));
        Ok(())
    })
}

/// Remember *Not now* for the drive at `root` (slice 5): appended to
/// `declined-drives` as written, `D:/`. `false`, and nothing written, when
/// it is already there in any spelling.
pub fn decline_drive(path: &Path, root: &Path) -> Result<bool> {
    let root = written(root, None);
    let mut declined: Vec<String> = read(path)?
        .declined_drives
        .iter()
        .map(|each| each.to_string_lossy().into_owned())
        .collect();
    if declined.iter().any(|each| same(each, &root)) {
        return Ok(false);
    }
    declined.push(root);
    section::edit(path, SECTION, |files| {
        files.insert("declined-drives".into(), Value::from(declined));
        Ok(())
    })?;
    Ok(true)
}

/// Whether the default skip called `name` is excluded: on the list and not
/// turned back on.
pub fn is_excluded(config: &FileSearchConfig, name: &str) -> bool {
    !config
        .default_skips_off
        .iter()
        .any(|off| off.eq_ignore_ascii_case(name))
}

/// Exclude the default skip called `name` again, or turn it back on: only
/// what was turned on is written (`default-skips-off`), as the list spells
/// it, and the key goes once it is empty.
pub fn set_default_skip(path: &Path, name: &str, excluded: bool) -> Result<()> {
    let Some(name) = crate::file_index::default_skips()
        .iter()
        .find(|skip| skip.eq_ignore_ascii_case(name))
    else {
        bail!("{name} is not on the default skip list");
    };
    section::edit(path, SECTION, |files| {
        let mut off = strings(files, "default-skips-off");
        off.retain(|each| !each.eq_ignore_ascii_case(name));
        if !excluded {
            off.push((*name).to_string());
        }
        if off.is_empty() {
            files.remove("default-skips-off");
        } else {
            files.insert("default-skips-off".into(), Value::from(off));
        }
        Ok(())
    })
}

/// The strings of the list at `key`, whatever else it holds left out.
fn strings(files: &Map<String, Value>, key: &str) -> Vec<String> {
    files
        .get(key)
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn file(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lumen-files-settings-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn contents(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    /// Where each of `keys` is in the file, in order.
    fn order(path: &Path, keys: &[&str]) -> Vec<usize> {
        let text = contents(path);
        keys.iter()
            .map(|key| text.find(&format!("\"{key}\"")).unwrap())
            .collect()
    }

    const WRITTEN: &str = r#"{
  "theme": "nocturne",
  "files": {
    "include-hidden": false,
    "scopes": ["~", "D:/Dev"],
    "use-ignore-files": true,
    "future-key": 7
  },
  "window": { "width": 640 }
}"#;

    #[test]
    fn every_key_is_written_and_the_rest_is_kept_in_order() {
        let path = file("every", WRITTEN);
        set_in_root(&path, InRoot::Folders).unwrap();
        set_flag(&path, Flag::IncludeHidden, true).unwrap();
        set_flag(&path, Flag::UseIgnoreFiles, false).unwrap();
        set_flag(&path, Flag::Enabled, false).unwrap();
        set_flag(&path, Flag::SearchContents, false).unwrap();
        set_ignore_patterns(&path, &["*.log".into()]).unwrap();
        set_preview_font(&path, PreviewFont::Interface).unwrap();
        set_index_detail(&path, IndexDetail::Complete).unwrap();
        assert!(add(&path, List::Exclusions, "~/Videos").unwrap());
        set_default_skip(&path, "node_modules", false).unwrap();

        let config = read(&path).unwrap();
        assert_eq!(config.in_root, InRoot::Folders);
        assert!(config.include_hidden);
        assert!(!config.use_ignore_files);
        assert!(!config.enabled);
        assert!(!config.search_contents);
        assert_eq!(config.ignore_patterns, ["*.log"]);
        assert_eq!(config.preview_font, PreviewFont::Interface);
        assert_eq!(config.index_detail, IndexDetail::Complete);
        assert!(contents(&path).contains("\"index-detail\": \"complete\""));
        assert_eq!(config.exclusions, [PathBuf::from("~/Videos")]);
        assert_eq!(config.default_skips_off, ["node_modules"]);
        assert_eq!(config.scopes, [PathBuf::from("~"), PathBuf::from("D:/Dev")]);

        let text = contents(&path);
        assert!(text.contains("\"future-key\": 7"));
        assert!(text.contains("\"width\": 640"));
        let at = order(
            &path,
            &[
                "theme",
                "files",
                "include-hidden",
                "scopes",
                "use-ignore-files",
                "future-key",
                "in-root",
                "window",
            ],
        );
        assert!(at.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    }

    /// Written as the megabytes the file holds, within the range the
    /// configuration reads.
    #[test]
    fn the_hot_memory_is_written_in_megabytes_within_its_range() {
        let path = file("hot-memory", WRITTEN);
        set_hot_memory(&path, 16).unwrap();
        assert_eq!(read(&path).unwrap().hot_memory, 16);
        assert!(contents(&path).contains("\"hot-memory\": 16"));
        set_hot_memory(&path, 64).unwrap();
        assert_eq!(read(&path).unwrap().hot_memory, 32);
        assert!(contents(&path).contains("\"hot-memory\": 32"));
        assert!(contents(&path).contains("\"future-key\": 7"));
    }

    #[test]
    fn a_file_without_the_section_gets_it() {
        let path = file("missing", r#"{ "theme": "daylight" }"#);
        set_in_root(&path, InRoot::Off).unwrap();
        assert_eq!(read(&path).unwrap().in_root, InRoot::Off);
        assert!(contents(&path).contains("daylight"));
    }

    #[test]
    fn a_picked_folder_under_home_is_written_with_a_tilde() {
        let home = Path::new("C:/Users/ana");
        assert_eq!(written(Path::new("C:/Users/ana"), Some(home)), "~");
        assert_eq!(
            written(Path::new("C:/Users/ana/Documents/Work"), Some(home)),
            "~/Documents/Work"
        );
        assert_eq!(
            written(Path::new("C:/Users/anabel"), Some(home)),
            "C:/Users/anabel"
        );
        assert_eq!(written(Path::new("/srv/data"), None), "/srv/data");
    }

    #[cfg(windows)]
    #[test]
    fn a_picked_windows_folder_is_written_with_forward_slashes() {
        let home = Path::new(r"C:\Users\ana");
        assert_eq!(
            written(Path::new(r"C:\Users\ana\Videos\Clips"), Some(home)),
            "~/Videos/Clips"
        );
        assert_eq!(
            written(Path::new(r"D:\Dev\lumen"), Some(home)),
            "D:/Dev/lumen"
        );
        assert_eq!(written(Path::new(r"E:\"), Some(home)), "E:/");
    }

    #[test]
    fn adding_a_folder_already_there_writes_nothing() {
        let path = file("duplicate", WRITTEN);
        let before = contents(&path);
        assert!(!add(&path, List::Scopes, "~").unwrap());
        assert!(!add(&path, List::Scopes, "d:/dev/").unwrap());
        assert_eq!(contents(&path), before);

        assert!(add(&path, List::Exclusions, "~/Videos").unwrap());
        assert!(!add(&path, List::Exclusions, "~/videos").unwrap());
        assert_eq!(read(&path).unwrap().exclusions.len(), 1);
    }

    #[test]
    fn a_scope_added_to_a_file_without_scopes_keeps_the_ones_it_meant() {
        let path = file("implicit", r#"{ "files": {} }"#);
        assert!(add(&path, List::Scopes, "D:/Dev").unwrap());
        assert_eq!(
            read(&path).unwrap().scopes,
            [PathBuf::from("~"), PathBuf::from("D:/Dev")]
        );

        let legacy = file("legacy", r#"{ "files": { "roots": ["E:/work"] } }"#);
        assert!(add(&legacy, List::Scopes, "D:/Dev").unwrap());
        assert_eq!(
            read(&legacy).unwrap().scopes,
            [PathBuf::from("E:/work"), PathBuf::from("D:/Dev")]
        );
    }

    #[test]
    fn a_folder_is_removed_by_how_it_is_written() {
        let path = file("remove", WRITTEN);
        remove(&path, List::Scopes, "D:/Dev").unwrap();
        assert_eq!(read(&path).unwrap().scopes, [PathBuf::from("~")]);
        assert!(remove(&path, List::Scopes, "D:/Dev").is_err());
        remove(&path, List::Scopes, "~").unwrap();
        // Empty is nothing indexed, not the defaults.
        assert!(read(&path).unwrap().scopes.is_empty());
    }

    #[test]
    fn turning_a_default_skip_back_on_writes_its_name_and_off_removes_it() {
        let path = file("skips", WRITTEN);
        let config = read(&path).unwrap();
        assert!(is_excluded(&config, "node_modules"));

        set_default_skip(&path, "NODE_MODULES", false).unwrap();
        set_default_skip(&path, "node_modules", false).unwrap();
        let config = read(&path).unwrap();
        assert_eq!(config.default_skips_off, ["node_modules"]);
        assert!(!is_excluded(&config, "node_modules"));

        set_default_skip(&path, "node_modules", true).unwrap();
        let config = read(&path).unwrap();
        assert!(config.default_skips_off.is_empty());
        assert!(!contents(&path).contains("default-skips-off"));

        assert!(set_default_skip(&path, "not-a-skip", false).is_err());
    }

    #[test]
    fn a_declined_drive_is_appended_once_with_forward_slashes() {
        let path = file("declined", WRITTEN);
        assert!(decline_drive(&path, Path::new("E:/")).unwrap());
        assert!(!decline_drive(&path, Path::new("e:/")).unwrap());
        assert!(decline_drive(&path, Path::new("D:/")).unwrap());
        let config = read(&path).unwrap();
        assert_eq!(
            config.declined_drives,
            [PathBuf::from("E:/"), PathBuf::from("D:/")]
        );
        // The rest of the section is untouched.
        assert_eq!(config.scopes, [PathBuf::from("~"), PathBuf::from("D:/Dev")]);
        assert!(contents(&path).contains("\"future-key\": 7"));
    }

    #[cfg(windows)]
    #[test]
    fn a_declined_windows_drive_is_written_as_the_file_spells_roots() {
        let path = file("declined-windows", WRITTEN);
        assert!(decline_drive(&path, Path::new(r"D:\")).unwrap());
        assert!(!decline_drive(&path, Path::new("D:/")).unwrap());
        assert!(contents(&path).contains("\"D:/\""));
    }

    #[test]
    fn patterns_are_read_from_the_comma_separated_field() {
        assert_eq!(
            parse_patterns(" *.log, **/build/** ,, *.tmp,*.log, "),
            ["*.log", "**/build/**", "*.tmp"]
        );
        assert!(parse_patterns(" , ").is_empty());
    }

    #[test]
    fn a_file_with_a_mistake_is_never_written() {
        let path = file("mistake", "{ \"files\": ");
        assert!(set_in_root(&path, InRoot::Off).is_err());
        assert!(add(&path, List::Scopes, "D:/Dev").is_err());
        assert_eq!(contents(&path), "{ \"files\": ");
    }
}
