//! Writing `snippets.json` from the interface (F11, slice 6).
//!
//! `quicklinks::store`'s rules, for the same reasons: the file stays the
//! source of truth and is edited the way a person with an editor would.
//!
//! - **A file that does not parse is refused,** not replaced: someone may be
//!   halfway through a hand edit.
//! - **Nothing is dropped:** entries the loader skips and fields this build
//!   does not know are written back as they were read.
//! - **No half files:** the new contents go to a sibling that is renamed over
//!   the old one.
//!
//! A snippet is found again by its name, which is unique among snippets --
//! as its keyword is -- so an edit made by hand while the form was open
//! cannot redirect a save to a different snippet.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use super::{SCHEMA_VERSION, Snippet, SnippetFile};

/// The file as written, for editing. Missing is empty; malformed is an error.
pub fn read(path: &Path) -> Result<SnippetFile> {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SnippetFile {
                schema_version: SCHEMA_VERSION,
                snippets: Vec::new(),
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
pub fn write(path: &Path, file: &SnippetFile) -> Result<()> {
    let mut contents = serde_json::to_string_pretty(file)?;
    contents.push('\n');
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let staging = staging_path(path);
    std::fs::write(&staging, contents).with_context(|| format!("writing {}", staging.display()))?;
    std::fs::rename(&staging, path).with_context(|| format!("replacing {}", path.display()))
}

fn staging_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".saving");
    path.with_file_name(name)
}

/// Whether `snippet` may join `snippets`, in place of the one called
/// `replacing` when that is set: a name and text, and a name and keyword no
/// other snippet has.
pub fn check(snippets: &[Snippet], snippet: &Snippet, replacing: Option<&str>) -> Result<()> {
    if snippet.name.trim().is_empty() {
        bail!("a snippet needs a name");
    }
    if snippet.text.is_empty() {
        bail!("a snippet needs some text");
    }
    let others = snippets
        .iter()
        .filter(|other| Some(other.name.trim()) != replacing.map(str::trim));
    for other in others {
        if other.name.trim().eq_ignore_ascii_case(snippet.name.trim()) {
            bail!("there is already a snippet called {:?}", other.name.trim());
        }
        if let (Some(keyword), Some(theirs)) = (snippet.keyword(), other.keyword())
            && keyword.eq_ignore_ascii_case(theirs)
        {
            bail!(
                "{:?} already uses the keyword {keyword:?}",
                other.name.trim()
            );
        }
    }
    Ok(())
}

/// Trimmed, without empty optional fields, and stamped: `created` kept or
/// set to `now`, `modified` set to `now`.
pub fn normalised(mut snippet: Snippet, now: jiff::Timestamp) -> Snippet {
    snippet.name = snippet.name.trim().to_string();
    snippet.keyword = snippet
        .keyword
        .take()
        .map(|keyword| keyword.trim().to_string())
        .filter(|keyword| !keyword.is_empty());
    let mut tags: Vec<String> = Vec::new();
    for tag in snippet.tags.drain(..) {
        let tag = tag.trim().to_string();
        if !tag.is_empty() && !tags.iter().any(|seen| seen.eq_ignore_ascii_case(&tag)) {
            tags.push(tag);
        }
    }
    snippet.tags = tags;
    snippet
        .keywords
        .retain(|keyword| !keyword.trim().is_empty());
    // Whole seconds: the file is read by people.
    let stamp = jiff::Timestamp::from_second(now.as_second())
        .unwrap_or(now)
        .to_string();
    if snippet.created.is_none() {
        snippet.created = Some(stamp.clone());
    }
    snippet.modified = Some(stamp);
    snippet
}

/// Add `snippet` at the end of the file.
pub fn add(path: &Path, snippet: Snippet, now: jiff::Timestamp) -> Result<()> {
    let mut file = read(path)?;
    // New now, whatever it was a copy of.
    let snippet = normalised(
        Snippet {
            created: None,
            ..snippet
        },
        now,
    );
    check(&file.snippets, &snippet, None)?;
    file.schema_version = file.schema_version.max(SCHEMA_VERSION);
    file.snippets.push(snippet);
    write(path, &file)
}

/// Replace the snippet called `name` with `snippet`, keeping its place, its
/// creation date and the fields this build does not know.
pub fn replace(path: &Path, name: &str, snippet: Snippet, now: jiff::Timestamp) -> Result<()> {
    let mut file = read(path)?;
    let index = position(&file, name)?;
    let old = &file.snippets[index];
    let mut snippet = snippet;
    snippet.created = old.created.clone();
    if snippet.extra.is_empty() {
        snippet.extra = old.extra.clone();
    }
    let snippet = normalised(snippet, now);
    check(&file.snippets, &snippet, Some(name))?;
    file.schema_version = file.schema_version.max(SCHEMA_VERSION);
    file.snippets[index] = snippet;
    write(path, &file)
}

/// Remove the snippet called `name`.
pub fn remove(path: &Path, name: &str) -> Result<()> {
    let mut file = read(path)?;
    let index = position(&file, name)?;
    file.snippets.remove(index);
    write(path, &file)
}

/// A name for a copy of `name` that no snippet has: *Name copy*, then
/// *Name copy 2*.
pub fn copy_name(snippets: &[Snippet], name: &str) -> String {
    let taken = |candidate: &str| {
        snippets
            .iter()
            .any(|snippet| snippet.name.trim().eq_ignore_ascii_case(candidate))
    };
    let base = format!("{} copy", name.trim());
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !taken(candidate))
        .expect("an unbounded range finds one")
}

/// Where the snippet called `name` is.
fn position(file: &SnippetFile, name: &str) -> Result<usize> {
    file.snippets
        .iter()
        .position(|snippet| snippet.name.trim() == name.trim())
        .with_context(|| format!("there is no snippet called {name:?} any more"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> jiff::Timestamp {
        "2026-09-30T21:00:19.5Z".parse().unwrap()
    }

    fn temp(name: &str, contents: Option<&str>) -> (PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("lumen-snip-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("snippets.json");
        if let Some(contents) = contents {
            std::fs::write(&path, contents).unwrap();
        }
        (dir, path)
    }

    fn snippet(name: &str, text: &str) -> Snippet {
        Snippet {
            name: name.into(),
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn adding_to_no_file_creates_it_stamped_and_tidied() {
        let (dir, path) = temp("add", None);
        let new = Snippet {
            keyword: Some("  ".into()),
            tags: vec![" work ".into(), "Work".into(), "".into()],
            format: super::super::Format::Markdown,
            ..snippet("  Sig ", "**Best**")
        };
        add(&path, new, now()).unwrap();

        let file = read(&path).unwrap();
        assert_eq!(file.schema_version, SCHEMA_VERSION);
        let saved = &file.snippets[0];
        assert_eq!(saved.name, "Sig");
        assert_eq!(saved.keyword, None);
        assert_eq!(saved.tags, ["work"]);
        assert_eq!(saved.created.as_deref(), Some("2026-09-30T21:00:19Z"));
        assert_eq!(saved.modified, saved.created);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"format\": \"markdown\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_malformed_file_is_refused_and_left_alone() {
        let (dir, path) = temp("bad", Some("{ half an edit"));
        assert!(add(&path, snippet("A", "a"), now()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ half an edit");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn names_and_keywords_are_unique() {
        let existing = vec![Snippet {
            keyword: Some(";sig".into()),
            ..snippet("Signature", "x")
        }];
        assert!(check(&existing, &snippet("signature", "y"), None).is_err());
        let same_keyword = Snippet {
            keyword: Some(";SIG".into()),
            ..snippet("Other", "y")
        };
        assert!(check(&existing, &same_keyword, None).is_err());
        // Saving a snippet over itself is not a clash.
        assert!(check(&existing, &existing[0], Some("Signature")).is_ok());
        assert!(check(&existing, &snippet("", "y"), None).is_err());
        assert!(check(&existing, &snippet("Empty", ""), None).is_err());
    }

    #[test]
    fn replacing_keeps_the_place_the_creation_date_and_unknown_fields() {
        let (dir, path) = temp(
            "replace",
            Some(
                r#"{
                    "schema_version": 1,
                    "future": true,
                    "snippets": [
                        { "name": "First", "text": "1" },
                        { "name": "Second", "text": "2", "created": "2020-01-01T00:00:00Z", "color": "red" },
                        { "name": "Third", "text": "3" }
                    ]
                }"#,
            ),
        );
        replace(&path, "Second", snippet("Second, edited", "two"), now()).unwrap();

        let file = read(&path).unwrap();
        assert_eq!(
            file.extra.get("future"),
            Some(&serde_json::Value::Bool(true))
        );
        let names: Vec<&str> = file.snippets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["First", "Second, edited", "Third"]);
        let edited = &file.snippets[1];
        assert_eq!(edited.created.as_deref(), Some("2020-01-01T00:00:00Z"));
        assert_eq!(edited.modified.as_deref(), Some("2026-09-30T21:00:19Z"));
        assert_eq!(edited.extra.get("color"), Some(&serde_json::json!("red")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn removing_and_a_snippet_that_is_gone() {
        let (dir, path) = temp(
            "remove",
            Some(r#"{ "schema_version": 2, "snippets": [ { "name": "A", "text": "a" } ] }"#),
        );
        assert!(replace(&path, "Nope", snippet("Nope", "n"), now()).is_err());
        remove(&path, "A").unwrap();
        assert!(read(&path).unwrap().snippets.is_empty());
        assert!(remove(&path, "A").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_copy_gets_a_name_nobody_has() {
        let snippets = vec![snippet("Sig", "x"), snippet("Sig copy", "x")];
        assert_eq!(copy_name(&snippets, "Sig"), "Sig copy 2");
        assert_eq!(copy_name(&snippets, "Other"), "Other copy");
    }
}
