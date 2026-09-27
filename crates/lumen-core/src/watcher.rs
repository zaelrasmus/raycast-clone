//! Live reload (AD-05).
//!
//! Aesthetics get iterated hundreds of times, and recompiling on every iteration
//! kills the design process. Re-reading a token file on change is what turns
//! F2's tuning work into an activity measured in seconds rather than builds --
//! and, per R-11, it is also the concrete mitigation for losing motivation to
//! the long wait before the aesthetic phase begins.
//!
//! The same holds for the configuration and the shortcut sheets: editing a
//! file and having to restart the daemon to see it is exactly the friction
//! that stops people iterating.
//!
//! # Pushed, never polled
//!
//! A watcher calls the sink it was given, on `notify`'s own thread, when a file
//! changes -- and at no other time. Nothing asks "has anything changed?" on a
//! timer. The daemon is hidden almost all of the time, and a poll is a wakeup
//! every few hundred milliseconds for the life of the process to answer a
//! question whose answer is nearly always no: exactly what the idle CPU budget
//! exists to rule out.
//!
//! Like every other sink in Lumen, the callback must push and return (I-15).
//! Parsing happens on the UI thread once the burst of events an editor's
//! truncate-write-rename produces has settled. A file caught half written
//! fails to parse, the caller keeps what it had, and the write that finishes
//! the save produces another event and another, successful, reload.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use notify::{Event, RecursiveMode, Watcher};

/// Watches one file.
///
/// The returned handle must be kept alive: dropping it stops the watch.
pub struct FileWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl FileWatcher {
    /// Call `on_change` whenever `file` changes.
    ///
    /// The *directory* is watched rather than the file, because an atomic
    /// save replaces the file's identity and a watch on the old one stops
    /// firing after the first save -- the classic way live reload appears to
    /// work once and then quietly stops.
    pub fn start(file: &Path, on_change: impl Fn() + Send + 'static) -> Result<Self> {
        let dir = file
            .parent()
            .with_context(|| format!("{} has no parent folder", file.display()))?;
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

        let target = file.to_path_buf();

        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let Ok(event) = event else { return };
            // Reading the file, which the reload itself does, must not
            // trigger another reload.
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            if event.paths.iter().any(|path| same_file(path, &target)) {
                on_change();
            }
        })
        .context("creating a filesystem watcher")?;

        watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching {}", dir.display()))?;

        Ok(Self { _watcher: watcher })
    }
}

/// Compare two paths without touching the filesystem.
///
/// `canonicalize` would be more correct but requires the file to exist, and
/// during an atomic save it briefly does not.
fn same_file(a: &Path, b: &Path) -> bool {
    fn key(path: &Path) -> Option<PathBuf> {
        Some(PathBuf::from(path.file_name()?))
    }
    key(a) == key(b)
}

/// Watches a directory and reports that something in it changed.
///
/// For a folder of files that are read together, like the shortcut sheets.
/// Renaming a sheet, deleting one and editing one all have to produce the
/// same response -- reload the lot -- so which file changed is not reported.
pub struct DirectoryWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl DirectoryWatcher {
    /// Call `on_change` whenever anything in `dir` changes.
    pub fn start(dir: &Path, on_change: impl Fn() + Send + 'static) -> Result<Self> {
        // Created if absent: watching a directory that does not exist fails, and
        // the sheets folder is legitimately missing on a first run.
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let Ok(event) = event else { return };
            // Access events fire when a file is merely read, including by this
            // process. Reloading on those would be a loop.
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            on_change();
        })
        .context("creating a filesystem watcher")?;

        watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching {}", dir.display()))?;

        Ok(Self { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn matches_a_save_regardless_of_the_directory_spelling() {
        assert!(same_file(
            Path::new("C:/Users/x/AppData/Lumen/themes/nocturne.json"),
            Path::new("themes/nocturne.json")
        ));
    }

    #[test]
    fn ignores_other_theme_files_in_the_same_folder() {
        assert!(!same_file(
            Path::new("themes/daylight.json"),
            Path::new("themes/nocturne.json")
        ));
    }

    /// The whole point of pushing: a write is reported without anyone asking.
    #[test]
    fn a_write_is_pushed_to_the_sink() {
        let dir = std::env::temp_dir().join(format!("lumen-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.json");

        let (tx, rx) = mpsc::channel();
        let _watcher = FileWatcher::start(&file, move || {
            let _ = tx.send(());
        })
        .unwrap();

        std::fs::write(&file, "{}").unwrap();
        let reported = rx.recv_timeout(Duration::from_secs(5));

        let _ = std::fs::remove_dir_all(&dir);
        assert!(reported.is_ok(), "no change was reported for {file:?}");
    }
}
