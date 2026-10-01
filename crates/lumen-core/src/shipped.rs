//! Files Lumen ships into the user's folders, kept current unless edited
//! (F15).
//!
//! The themes and styles are written into the config folder so they can be
//! read, copied and changed like any other. Until F15 they were written once
//! and never again -- the user may have edited them, and reverting that on
//! every launch would be the most annoying behaviour possible. But it also
//! meant a preset improved in a release never reached anybody who had it.
//!
//! So Lumen remembers a fingerprint of what it wrote. At startup, a file
//! whose fingerprint is still Lumen's is one nobody edited, and it gets the
//! version this build ships. A file that was edited never matches and is
//! never touched. A file there before fingerprints were kept is left alone
//! too -- nothing says whose it is -- until it equals a shipped version,
//! after which it follows the releases like any other.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};

/// Write `files` -- (name without `.json`, contents) -- into `dir`, as the
/// module says, with the fingerprints in `record`.
pub fn install(dir: &Path, files: &[(&str, &str)], record: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let folder = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut written: BTreeMap<String, String> = std::fs::read_to_string(record)
        .ok()
        .and_then(|source| serde_json::from_str(&source).ok())
        .unwrap_or_default();
    let before = written.clone();

    for (name, source) in files {
        let path = dir.join(format!("{name}.json"));
        let key = format!("{folder}/{name}.json");
        let shipped = fingerprint(source.as_bytes());
        match std::fs::read(&path) {
            Err(_) => {
                std::fs::write(&path, source)
                    .with_context(|| format!("writing {}", path.display()))?;
                written.insert(key, shipped);
            }
            Ok(contents) => {
                let current = fingerprint(&contents);
                if current == shipped {
                    written.insert(key, shipped);
                } else if written.get(&key) == Some(&current) {
                    std::fs::write(&path, source)
                        .with_context(|| format!("updating {}", path.display()))?;
                    log::info!("updated the shipped {key} to this version");
                    written.insert(key, shipped);
                }
            }
        }
    }

    if written != before {
        if let Some(parent) = record.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(record, serde_json::to_string_pretty(&written)?)
            .with_context(|| format!("writing {}", record.display()))?;
    }
    Ok(())
}

/// FNV-1a, 64 bits, as hex: stable across builds and Rust versions, which
/// `std`'s hasher does not promise, and nothing here needs to resist anyone.
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("lumen-shipped-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        (root.join("styles"), root.join("data").join("shipped.json"))
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap()
    }

    #[test]
    fn an_unedited_file_follows_the_release() {
        let (dir, record) = dir("follow");
        install(&dir, &[("ink", "v1")], &record).unwrap();
        install(&dir, &[("ink", "v2")], &record).unwrap();
        assert_eq!(read(&dir, "ink"), "v2");
    }

    #[test]
    fn an_edited_file_is_never_touched() {
        let (dir, record) = dir("edited");
        install(&dir, &[("ink", "v1")], &record).unwrap();
        std::fs::write(dir.join("ink.json"), "mine").unwrap();
        install(&dir, &[("ink", "v2")], &record).unwrap();
        assert_eq!(read(&dir, "ink"), "mine");
    }

    /// From before fingerprints were kept: nothing says whose it is, so it
    /// stays -- until it equals a shipped version, and then it follows.
    #[test]
    fn a_file_of_unknown_origin_is_left_until_it_is_adopted() {
        let (dir, record) = dir("unknown");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ink.json"), "old").unwrap();
        install(&dir, &[("ink", "v1")], &record).unwrap();
        assert_eq!(read(&dir, "ink"), "old");

        std::fs::write(dir.join("ink.json"), "v1").unwrap();
        install(&dir, &[("ink", "v1")], &record).unwrap();
        install(&dir, &[("ink", "v2")], &record).unwrap();
        assert_eq!(read(&dir, "ink"), "v2");
    }

    #[test]
    fn the_fingerprint_is_stable() {
        assert_eq!(fingerprint(b""), "cbf29ce484222325");
        assert_eq!(fingerprint(b"a"), "af63dc4c8601ec8c");
    }
}
