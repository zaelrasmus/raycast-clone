//! Writing `quicklinks.json` from the interface (F6).
//!
//! The file stays the source of truth. Lumen's forms edit it the way a person
//! with an editor would, and the watcher reloads it exactly as it does after a
//! hand edit, so there is one path from the file to the list, not two.
//!
//! # What a write must not do
//!
//! - **Overwrite a file that does not parse.** Someone halfway through a hand
//!   edit has a file with a stray comma in it; saving a quicklink from the
//!   launcher at that moment must refuse, not replace their work with Lumen's
//!   idea of the file.
//! - **Drop what it does not understand.** Entries the loader skips and fields
//!   this build does not know are written back as they were read. A file
//!   written by a newer Lumen keeps its newer fields.
//! - **Leave half a file.** The new contents go to a sibling file that is then
//!   renamed over the old one, so a crash mid-write leaves the old file.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use super::{Entry, Quicklink, QuicklinkFile, SCHEMA_VERSION};

/// The file as written, for editing.
///
/// A missing file is an empty one. A malformed one is an error, which is the
/// whole point of reading it separately from [`super::load`]: the loader
/// forgives, the writer must not.
pub fn read(path: &Path) -> Result<QuicklinkFile> {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(QuicklinkFile {
                schema_version: SCHEMA_VERSION,
                quicklinks: Vec::new(),
                extra: Default::default(),
            });
        }
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };

    serde_json::from_str(&source).with_context(|| {
        format!(
            "{} has a mistake in it; fix it or remove it before saving from Lumen",
            path.display()
        )
    })
}

/// Replace the file with `file`, all at once.
pub fn write(path: &Path, file: &QuicklinkFile) -> Result<()> {
    let mut contents = serde_json::to_string_pretty(file)?;
    contents.push('\n');

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let staging = staging_path(path);
    std::fs::write(&staging, contents).with_context(|| format!("writing {}", staging.display()))?;
    // `rename` replaces an existing file on every platform Lumen targets.
    std::fs::rename(&staging, path).with_context(|| format!("replacing {}", path.display()))
}

fn staging_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".saving");
    path.with_file_name(name)
}

/// Whether `entry` may go into `entries` in place of the entry named
/// `replacing`, or alongside them when that is `None`.
///
/// The same checks the loader makes, plus the two only a writer can: a name
/// is the link's identity -- its frecency key and how a form finds it again --
/// and an alias is a word the query starts with, so neither can be shared.
pub fn check(entries: &[Entry], entry: &Entry, replacing: Option<&str>) -> Result<Quicklink> {
    let link = Quicklink::from_entry(entry.clone()).map_err(anyhow::Error::msg)?;

    let others = entries
        .iter()
        .filter(|other| Some(other.name.trim()) != replacing.map(str::trim));

    for other in others {
        if other.name.trim().eq_ignore_ascii_case(&link.name) {
            bail!("there is already a quicklink called {:?}", link.name);
        }
        if let (Some(alias), Some(theirs)) = (&link.alias, &other.alias)
            && alias.eq_ignore_ascii_case(theirs.trim())
        {
            bail!("{:?} already uses the alias {alias:?}", other.name);
        }
    }
    Ok(link)
}

/// Add `entry` at the end of the file.
pub fn add(path: &Path, entry: Entry) -> Result<()> {
    let mut file = read(path)?;
    check(&file.quicklinks, &entry, None)?;
    file.quicklinks.push(normalised(entry));
    write(path, &file)
}

/// Replace the entry called `name` with `entry`, keeping its place.
pub fn replace(path: &Path, name: &str, entry: Entry) -> Result<()> {
    let mut file = read(path)?;
    let index = position(&file, name)?;
    check(&file.quicklinks, &entry, Some(name))?;

    // Fields this build does not know stay with the entry they belong to.
    let mut entry = normalised(entry);
    if entry.extra.is_empty() {
        entry.extra = std::mem::take(&mut file.quicklinks[index].extra);
    }
    file.quicklinks[index] = entry;
    write(path, &file)
}

/// Remove the entry called `name`.
pub fn remove(path: &Path, name: &str) -> Result<()> {
    let mut file = read(path)?;
    let index = position(&file, name)?;
    file.quicklinks.remove(index);
    write(path, &file)
}

/// Where the entry called `name` is.
///
/// By name rather than by position: the file can be edited by hand while a
/// form is open, and a position read before that edit points at a different
/// link after it.
fn position(file: &QuicklinkFile, name: &str) -> Result<usize> {
    file.quicklinks
        .iter()
        .position(|entry| entry.name.trim() == name.trim())
        .with_context(|| format!("there is no quicklink called {name:?} any more"))
}

/// Trimmed and without empty optional fields, as a person would have written
/// it.
fn normalised(mut entry: Entry) -> Entry {
    entry.name = entry.name.trim().to_string();
    entry.link = entry.link.trim().to_string();
    for field in [&mut entry.alias, &mut entry.open_with] {
        *field = field
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
    entry.keywords.retain(|keyword| !keyword.trim().is_empty());
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("lumen-links-store-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir.join("quicklinks.json"))
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

    fn entry(name: &str, link: &str) -> Entry {
        Entry {
            name: name.into(),
            link: link.into(),
            ..Entry::default()
        }
    }

    #[test]
    fn adding_to_a_missing_file_creates_it() {
        let file = Scratch::new("create");

        add(&file.0, entry("Channel", "https://youtube.com/@x")).unwrap();

        let links = super::super::load(&file.0);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].name, "Channel");
    }

    /// The case the whole module is careful about: someone halfway through a
    /// hand edit must not lose it to a save from the launcher.
    #[test]
    fn a_file_that_does_not_parse_is_never_overwritten() {
        let file = Scratch::new("broken");
        std::fs::write(&file.0, "{ \"schema_version\": 1, oops").unwrap();

        let error = add(&file.0, entry("New", "https://x.com")).unwrap_err();

        assert!(format!("{error:#}").contains("mistake"), "{error:#}");
        assert_eq!(file.contents(), "{ \"schema_version\": 1, oops");
    }

    #[test]
    fn entries_and_fields_this_build_skips_survive_a_write() {
        let file = Scratch::new("keep");
        std::fs::write(
            &file.0,
            r#"{
                "schema_version": 1,
                "future_setting": true,
                "quicklinks": [
                    { "name": "Broken", "link": "no-scheme" },
                    { "name": "Kept", "link": "https://x.com", "colour": "red" }
                ]
            }"#,
        )
        .unwrap();

        add(&file.0, entry("New", "https://y.com")).unwrap();

        let written = file.contents();
        assert!(written.contains("\"future_setting\""), "{written}");
        assert!(written.contains("\"no-scheme\""), "{written}");
        assert!(written.contains("\"colour\""), "{written}");
        assert!(written.contains("\"New\""), "{written}");
    }

    #[test]
    fn a_name_or_an_alias_cannot_be_shared() {
        let existing = vec![Entry {
            alias: Some("g".into()),
            ..entry("Google", "https://google.com/?q={query}")
        }];

        let same_name = entry("google", "https://x.com");
        assert!(check(&existing, &same_name, None).is_err());

        let same_alias = Entry {
            alias: Some("G".into()),
            ..entry("Other", "https://x.com/{query}")
        };
        assert!(check(&existing, &same_alias, None).is_err());

        // Replacing a link may keep its own name and alias.
        let edited = Entry {
            alias: Some("g".into()),
            ..entry("Google", "https://www.google.com/search?q={query}")
        };
        assert!(check(&existing, &edited, Some("Google")).is_ok());
    }

    #[test]
    fn what_the_loader_refuses_the_writer_refuses() {
        assert!(check(&[], &entry("Typo", "https://x.com/{qeury}"), None).is_err());
        assert!(check(&[], &entry("No scheme", "x.com"), None).is_err());
    }

    #[test]
    fn replacing_keeps_the_links_place_and_its_unknown_fields() {
        let file = Scratch::new("replace");
        std::fs::write(
            &file.0,
            r#"{ "schema_version": 1, "quicklinks": [
                { "name": "A", "link": "https://a.com", "colour": "red" },
                { "name": "B", "link": "https://b.com" }
            ] }"#,
        )
        .unwrap();

        replace(&file.0, "A", entry("A2", "https://a2.com")).unwrap();

        let read = read(&file.0).unwrap();
        assert_eq!(read.quicklinks[0].name, "A2");
        assert_eq!(read.quicklinks[1].name, "B");
        assert!(read.quicklinks[0].extra.contains_key("colour"));
    }

    #[test]
    fn removing_a_link_that_is_gone_says_so() {
        let file = Scratch::new("remove");
        add(&file.0, entry("A", "https://a.com")).unwrap();

        remove(&file.0, "A").unwrap();
        assert!(read(&file.0).unwrap().quicklinks.is_empty());

        let error = remove(&file.0, "A").unwrap_err();
        assert!(error.to_string().contains("no quicklink called"), "{error}");
    }

    #[test]
    fn empty_optional_fields_are_left_out_of_the_file() {
        let file = Scratch::new("tidy");
        add(
            &file.0,
            Entry {
                alias: Some("  ".into()),
                open_with: Some(String::new()),
                ..entry("  Tidy  ", " https://x.com ")
            },
        )
        .unwrap();

        let written = file.contents();
        assert!(!written.contains("alias"), "{written}");
        assert!(!written.contains("open_with"), "{written}");
        assert!(written.contains("\"Tidy\""), "{written}");
        assert!(!staging_path(&file.0).exists());
    }
}
