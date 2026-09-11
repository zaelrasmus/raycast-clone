//! Live theme reload (AD-05).
//!
//! Aesthetics get iterated hundreds of times, and recompiling on every iteration
//! kills the design process. Re-reading a token file on change is what turns
//! F2's tuning work into an activity measured in seconds rather than builds --
//! and, per R-11, it is also the concrete mitigation for losing motivation to
//! the long wait before the aesthetic phase begins.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use notify::{Event, RecursiveMode, Watcher};

use crate::theme::Theme;

/// Editors do not write a file once.
///
/// A save is commonly a truncate, a write and a rename, arriving as three
/// separate events within a few milliseconds. Reloading on each one means
/// parsing a half-written file and flashing the base theme on screen.
const DEBOUNCE: Duration = Duration::from_millis(120);

/// Watches the themes folder and reports the active theme when it changes.
///
/// The returned handle must be kept alive: dropping it stops the watch.
pub struct ThemeWatcher {
    _watcher: notify::RecommendedWatcher,
    changes: mpsc::Receiver<Theme>,
}

impl ThemeWatcher {
    /// Watch `themes_dir` for changes to the theme named `theme_name`.
    ///
    /// The whole directory is watched rather than the single file, because an
    /// atomic save replaces the file's identity and a watch on the old inode
    /// stops firing after the first save -- the classic way live reload appears
    /// to work once and then quietly stops.
    pub fn start(themes_dir: &Path, theme_name: &str) -> Result<Self> {
        let (tx, changes) = mpsc::channel();
        let watched_file = themes_dir.join(format!("{theme_name}.json"));

        let mut last_reload = Instant::now() - DEBOUNCE;
        let target = watched_file.clone();

        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let Ok(event) = event else { return };
            if !event.paths.iter().any(|path| same_file(path, &target)) {
                return;
            }
            if last_reload.elapsed() < DEBOUNCE {
                return;
            }
            last_reload = Instant::now();

            match Theme::load(&target) {
                Ok(theme) => {
                    // A closed channel means the daemon is shutting down. Not
                    // an error, and nothing to log about it.
                    let _ = tx.send(theme);
                }
                // Keep the current theme on a parse error. A syntax error
                // mid-edit is the normal case, and dropping to the base theme
                // on every stray keystroke would make editing unusable.
                Err(error) => log::warn!("theme reload skipped: {error:#}"),
            }
        })
        .context("creating a filesystem watcher")?;

        watcher
            .watch(themes_dir, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching {}", themes_dir.display()))?;

        Ok(Self {
            _watcher: watcher,
            changes,
        })
    }

    /// The most recent theme, if one arrived since the last call.
    ///
    /// Non-blocking and coalescing: if several reloads queued up between frames,
    /// only the newest matters. Called from the UI thread, so it must never
    /// wait.
    pub fn latest(&self) -> Option<Theme> {
        self.changes.try_iter().last()
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
