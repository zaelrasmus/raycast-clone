//! *Recently Used*: what *Search Files* shows before anything is typed
//! (F16).
//!
//! Two lists, merged. Lumen's own -- the files and folders opened from it,
//! which frecency remembers -- comes first, best first: it is what the user
//! reached for here. Then the system's recent items, newest first, the list
//! Explorer shows, so the view is useful before Lumen has any history. The
//! system's list carries no times Lumen could rank against its own, so the
//! two are concatenated, not interleaved.

use std::path::{Path, PathBuf};

use super::Found;
use crate::frecency::Frecency;

/// The most rows *Recently Used* shows.
pub const RECENT_LIMIT: usize = 50;

/// The files and folders frecency remembers, best first; ties by path, so
/// the order is the same every time.
pub fn from_frecency(frecency: &Frecency) -> Vec<PathBuf> {
    let mut used: Vec<(u32, &str)> = frecency
        .ids()
        .filter_map(|id| {
            let path = id
                .strip_prefix("file:")
                .or_else(|| id.strip_prefix("folder:"))?;
            Some((frecency.boost(id), path))
        })
        .collect();
    used.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    used.into_iter()
        .map(|(_, path)| PathBuf::from(path))
        .collect()
}

/// `lumen` then `system`, each path once, only those `kind` finds on disk
/// -- `Some(is_dir)` -- and at most `limit` of them.
pub fn merge(
    lumen: Vec<PathBuf>,
    system: Vec<PathBuf>,
    kind: impl Fn(&Path) -> Option<bool>,
    limit: usize,
) -> Vec<Found> {
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();
    for path in lumen.into_iter().chain(system) {
        if merged.len() == limit {
            break;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        if let Some(is_dir) = kind(&path) {
            merged.push(Found { path, is_dir });
        }
    }
    merged
}

/// *Recently Used*, read from the disk: a check that each path is still
/// there. Off the UI thread.
pub fn recently_used(frecency: &Frecency, system: Vec<PathBuf>) -> Vec<Found> {
    merge(
        from_frecency(frecency),
        system,
        |path| {
            std::fs::metadata(path)
                .ok()
                .map(|metadata| metadata.is_dir())
        },
        RECENT_LIMIT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn names(found: &[Found]) -> Vec<String> {
        found
            .iter()
            .map(|found| found.path.display().to_string())
            .collect()
    }

    /// Everything exists but `gone`; `dir` is a folder.
    fn kind(path: &Path) -> Option<bool> {
        match path.to_str() {
            Some("gone") | None => None,
            Some(name) => Some(name.ends_with("dir")),
        }
    }

    #[test]
    fn lumens_own_come_first_then_the_systems_in_their_order() {
        let merged = merge(paths(&["a", "b"]), paths(&["c", "d"]), kind, 10);
        assert_eq!(names(&merged), ["a", "b", "c", "d"]);
    }

    #[test]
    fn a_path_in_both_lists_is_shown_once_where_lumen_has_it() {
        let merged = merge(paths(&["b", "a"]), paths(&["a", "c", "b"]), kind, 10);
        assert_eq!(names(&merged), ["b", "a", "c"]);
    }

    #[test]
    fn a_path_no_longer_on_disk_is_left_out() {
        let merged = merge(paths(&["gone", "a"]), paths(&["gone", "c"]), kind, 10);
        assert_eq!(names(&merged), ["a", "c"]);
    }

    #[test]
    fn at_most_the_limit_is_shown() {
        let merged = merge(paths(&["a", "b"]), paths(&["c", "d"]), kind, 3);
        assert_eq!(names(&merged), ["a", "b", "c"]);
    }

    #[test]
    fn a_folder_is_known_as_one() {
        let merged = merge(paths(&["some-dir"]), Vec::new(), kind, 10);
        assert!(merged[0].is_dir);
    }

    /// Applications and commands share the history; only files and folders
    /// are recent files.
    #[test]
    fn only_files_and_folders_come_from_frecency_most_used_first() {
        let mut frecency = Frecency::new();
        frecency.record("app:Firefox");
        frecency.record("file:C:\\notes.md");
        frecency.record("folder:C:\\projects");
        frecency.record("folder:C:\\projects");
        assert_eq!(
            from_frecency(&frecency),
            paths(&["C:\\projects", "C:\\notes.md"])
        );
    }
}
