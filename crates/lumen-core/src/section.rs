//! Editing one section of `config.json` from the settings window.
//!
//! Shared by every page that writes the file -- translation (F7), timers
//! (F8) -- because the rules are about the file, not about what is in it:
//!
//! - **Only the named section changes.** The file is edited as a JSON
//!   document, in its own key order, so every other setting -- and anything
//!   this build does not know -- is written back as it was read.
//! - **A file that does not parse is never overwritten.** Someone halfway
//!   through a hand edit must not lose it to a save from the settings window.
//! - **Nothing is left half written.** The new contents go to a sibling file
//!   that is renamed over the old one.
//!
//! The daemon's watcher on `config.json` does the rest, as after a hand edit.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// The section called `name` as written, for editing.
///
/// A missing file or section is the defaults. A file that does not parse is
/// an error: the loader forgives it, the writer must not.
pub fn read<T: DeserializeOwned + Default>(path: &Path, name: &str) -> Result<T> {
    let root = document(path)?;
    match root.get(name) {
        None => Ok(T::default()),
        Some(section) => serde_json::from_value(section.clone()).with_context(|| {
            format!(
                "the {name} section of {} has a mistake in it",
                path.display()
            )
        }),
    }
}

/// Read the file, change its section called `name`, write it back.
pub fn edit(
    path: &Path,
    name: &str,
    change: impl FnOnce(&mut Map<String, Value>) -> Result<()>,
) -> Result<()> {
    let mut root = document(path)?;
    let section = root
        .entry(name)
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(section) = section else {
        bail!("the {name} section of {} is not an object", path.display());
    };
    change(section)?;
    write(path, &Value::Object(root))
}

/// The list at `key` in `section`, created if absent.
pub fn list<'a>(section: &'a mut Map<String, Value>, key: &str) -> Result<&'a mut Vec<Value>> {
    match section
        .entry(key)
        .or_insert_with(|| Value::Array(Vec::new()))
    {
        Value::Array(list) => Ok(list),
        _ => bail!("{key} is not a list"),
    }
}

/// Where the entry called `name` is in `list`. By name rather than position:
/// the file can be edited by hand while the page is open.
pub fn position(list: &[Value], name: &str) -> Result<usize> {
    list.iter()
        .position(|entry| {
            entry
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|written| written.trim() == name.trim())
        })
        .with_context(|| format!("there is no preset called {name:?} any more"))
}

/// The whole file, as a document. Missing is empty.
fn document(path: &Path) -> Result<Map<String, Value>> {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let mistake = || {
        format!(
            "{} has a mistake in it; fix it before saving from Lumen",
            path.display()
        )
    };
    match serde_json::from_str(&source).with_context(mistake)? {
        Value::Object(root) => Ok(root),
        _ => bail!(mistake()),
    }
}

fn write(path: &Path, root: &Value) -> Result<()> {
    let mut contents = serde_json::to_string_pretty(root)?;
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

pub(crate) fn staging_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".saving");
    path.with_file_name(name)
}
