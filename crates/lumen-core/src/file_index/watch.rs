//! Watching the scopes for changes (F16, slice 2).
//!
//! # Never a watch over a skipped tree
//!
//! A recursive watch reports every change beneath it, skipped or not, and a
//! home folder holds `AppData`, where browsers write all day. Filtering
//! those reports would still mean being woken for each of them. So a scope
//! is watched as its parts: its own folder without its subfolders, to see
//! what arrives at the top, and each folder directly in it that the rules
//! allow, with everything beneath ([`watch_targets`]). `AppData` and
//! `node_modules` are never watched, so their churn is never delivered.
//! A folder that arrives at the top later gets its watch when the change
//! that brought it is applied ([`FileWatch::sync`]). The scope's own watch
//! still hears that `AppData`'s time moved when an entry arrives directly
//! in it -- rare, and one listing of the scope's folder to apply.
//!
//! # Push and return
//!
//! The callback runs on `notify`'s thread and hands the paths `notify`
//! already allocated to the sink, which pushes them and returns (I-15).
//! Coalescing and applying happen elsewhere, once a burst has settled.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use notify::{Event, EventKind, RecursiveMode, Watcher};

use super::walk::{self, WalkRules};

/// A folder to watch, and whether everything beneath it is watched too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub path: PathBuf,
    pub recursive: bool,
}

/// What to watch for `rules`: each scope walked on its own without its
/// subfolders, and every folder directly in it the rules allow, with
/// everything beneath (see "Never a watch over a skipped tree" above).
///
/// Reads each scope's folder, so it runs off the UI thread like the walk.
pub fn watch_targets(rules: &WalkRules) -> Vec<Target> {
    let mut targets: Vec<Target> = Vec::new();
    let mut add = |target: Target| {
        if !targets.iter().any(|known| known.path == target.path) {
            targets.push(target);
        }
    };
    for index in walk::effective_scopes(rules) {
        let scope = &rules.scopes[index];
        if !scope.is_dir() {
            continue;
        }
        add(Target {
            path: scope.clone(),
            recursive: false,
        });
        // The same rules as the walk's, one level down: what the walk
        // would descend into is what is watched.
        for entry in walk::list(scope, scope, rules, &[]) {
            if entry.is_dir {
                add(Target {
                    path: entry.path,
                    recursive: true,
                });
            }
        }
    }
    targets
}

/// The watches of the file index. Dropping it stops them.
pub struct FileWatch {
    watcher: notify::RecommendedWatcher,
    watched: Vec<Target>,
}

impl FileWatch {
    /// A watcher that hands `sink` the paths of every change it sees, and
    /// watches nothing until [`Self::sync`] says what.
    ///
    /// An error, or a report that says changes were lost, is handed on as
    /// an empty path: the applier then compares every folder's time, as a
    /// start does, rather than trusting a batch with a hole in it.
    pub fn start(sink: impl Fn(Vec<PathBuf>) + Send + 'static) -> Result<Self> {
        let watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let Ok(event) = event else {
                // Not allocating: an empty `Vec` holds nothing.
                sink(Vec::new());
                return;
            };
            // Reading a file is not a change, and the index reads none.
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            if event.need_rescan() {
                sink(Vec::new());
                return;
            }
            sink(event.paths);
        })
        .context("creating the file index's watcher")?;
        Ok(Self {
            watcher,
            watched: Vec::new(),
        })
    }

    /// Watch `targets` and nothing else: what is no longer a target is
    /// unwatched, what is new is watched. A folder that cannot be watched
    /// is logged and left out, and the rest are still watched.
    pub fn sync(&mut self, targets: Vec<Target>) {
        for old in &self.watched {
            if !targets.contains(old) {
                // Gone already when the folder was deleted, which is fine.
                let _ = self.watcher.unwatch(&old.path);
            }
        }
        let mut watched = Vec::with_capacity(targets.len());
        for target in targets {
            if self.watched.contains(&target) {
                watched.push(target);
                continue;
            }
            match self.watcher.watch(&target.path, mode(&target)) {
                Ok(()) => watched.push(target),
                Err(error) => log::warn!(
                    "not watching {} for the file index: {error}",
                    target.path.display()
                ),
            }
        }
        self.watched = watched;
    }

    /// The folders watched, for the log.
    pub fn watched(&self) -> &[Target] {
        &self.watched
    }
}

fn mode(target: &Target) -> RecursiveMode {
    if target.recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    }
}

#[cfg(test)]
mod tests {
    use super::super::walk::tests::{Tree, rules};
    use super::*;

    /// A home folder's `AppData` and a project's `node_modules` are where
    /// the churn is, and neither is watched; the folder itself is watched
    /// without its subfolders, everything else with them.
    #[test]
    fn skipped_and_hidden_folders_get_no_watch() {
        let tree = Tree::new("watch-targets");
        tree.file("AppData/Local/cache.db")
            .file("node_modules/left-pad/index.js")
            .file(".config/app.toml")
            .file("Documents/notes.md")
            .file("Downloads/setup.exe")
            .file("Videos/clip.mp4")
            .file("readme.txt");
        let mut rules = rules(std::slice::from_ref(&tree.0));
        rules.exclusions = vec![tree.path("Videos")];

        let targets = watch_targets(&rules);

        assert_eq!(
            targets,
            [
                Target {
                    path: tree.0.clone(),
                    recursive: false,
                },
                Target {
                    path: tree.path("Documents"),
                    recursive: true,
                },
                Target {
                    path: tree.path("Downloads"),
                    recursive: true,
                },
            ]
        );
    }

    /// A scope inside another is walked as part of it, and watched as part
    /// of it too.
    #[test]
    fn a_scope_inside_another_is_not_watched_twice() {
        let tree = Tree::new("watch-nested");
        tree.file("Documents/notes.md");

        let targets = watch_targets(&rules(&[tree.0.clone(), tree.path("Documents")]));
        assert_eq!(targets.len(), 2);
    }

    /// The watch pushes what changed, and nothing about a skipped tree.
    #[test]
    fn a_change_in_a_watched_folder_is_pushed() {
        let tree = Tree::new("watch-pushed");
        tree.dir("Documents").dir("AppData");
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut watch = FileWatch::start(move |paths| {
            let _ = sender.send(paths);
        })
        .unwrap();
        watch.sync(watch_targets(&tree.rules()));

        tree.file("AppData/noise.db");
        tree.file("Documents/plan.md");

        let wanted = tree.path("Documents").join("plan.md");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut seen = Vec::new();
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            let Ok(paths) = receiver.recv_timeout(left) else {
                break;
            };
            seen.extend(paths);
            if seen.contains(&wanted) {
                break;
            }
        }
        drop(watch);
        assert!(seen.contains(&wanted), "{seen:?}");
        // The scope's own watch may hear that `AppData`'s time moved; what
        // happens inside it is never heard.
        let appdata = tree.path("AppData");
        assert!(
            !seen
                .iter()
                .any(|path| path.starts_with(&appdata) && *path != appdata),
            "{seen:?}"
        );
    }
}
