//! Other drives are offered, not taken (F16, slice 5; `docs/FILE-SEARCH.md`,
//! "Other drives are offered, not taken").
//!
//! Which drives are internal is the platform's question
//! ([`lumen_platform::Platform::internal_drives`]); which of them to offer is
//! this module's: the ones no scope is on, that the user has not answered
//! *Not now* for.

use std::path::{Path, PathBuf};

use lumen_platform::Drive;

/// Whether `path` is a whole drive: a root, with no folder above it.
pub fn is_drive_root(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.parent().is_none()
}

/// The drives of `drives` with no scope on them and not declined, in order.
///
/// A scope is on a drive when its root is the drive's; `~` and `~/...` are
/// on `home`'s. Roots are compared in any spelling, since a hand-edited
/// `config.json` has its own: `D:/`, `d:\` and `D:` are one drive.
pub fn uncovered(
    drives: &[Drive],
    scopes: &[PathBuf],
    declined: &[PathBuf],
    home: Option<&Path>,
) -> Vec<Drive> {
    let home_key = home.and_then(drive_key);
    let taken: Vec<String> = scopes
        .iter()
        .filter_map(|scope| {
            let written = scope.to_string_lossy();
            if written == "~" || written.starts_with("~/") || written.starts_with("~\\") {
                home_key.clone()
            } else {
                drive_key(scope)
            }
        })
        .chain(declined.iter().filter_map(|root| drive_key(root)))
        .collect();
    drives
        .iter()
        .filter(|drive| drive_key(&drive.root).is_none_or(|key| !taken.contains(&key)))
        .cloned()
        .collect()
}

/// Whether `a` and `b` are on the same drive, in any spelling.
pub fn same_drive(a: &Path, b: &Path) -> bool {
    drive_key(a).is_some_and(|key| drive_key(b) == Some(key))
}

/// The drive `path` is on, as one spelling: `D:\Dev`, `d:/` and `D:` are
/// all `d:`. `None` for a relative path, which is on no drive.
fn drive_key(path: &Path) -> Option<String> {
    let root = path.ancestors().last().unwrap_or(path);
    if !is_drive_root(root) {
        return None;
    }
    Some(
        root.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(root: &str, label: &str) -> Drive {
        Drive {
            root: root.into(),
            label: label.into(),
        }
    }

    fn roots(drives: &[Drive]) -> Vec<String> {
        drives
            .iter()
            .map(|drive| drive.root.display().to_string())
            .collect()
    }

    fn paths(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    fn machine() -> Vec<Drive> {
        vec![
            drive("C:\\", "Windows"),
            drive("D:\\", "Data"),
            drive("E:\\", ""),
        ]
    }

    const HOME: &str = "C:\\Users\\ana";

    #[test]
    fn a_drive_with_a_scope_on_it_is_covered() {
        let left = uncovered(
            &machine(),
            &paths(&["D:/Dev/lumen"]),
            &[],
            Some(Path::new(HOME)),
        );
        assert_eq!(roots(&left), ["C:\\", "E:\\"]);
    }

    #[test]
    fn home_covers_its_own_drive() {
        let left = uncovered(
            &machine(),
            &paths(&["~", "~/Documents"]),
            &[],
            Some(Path::new(HOME)),
        );
        assert_eq!(roots(&left), ["D:\\", "E:\\"]);
        // Without a home, `~` is on no drive.
        let left = uncovered(&machine(), &paths(&["~"]), &[], None);
        assert_eq!(left.len(), 3);
    }

    #[test]
    fn a_declined_drive_is_not_offered_in_any_spelling() {
        let left = uncovered(
            &machine(),
            &paths(&["~"]),
            &paths(&["e:/", "D:"]),
            Some(Path::new(HOME)),
        );
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn with_no_scope_every_drive_is_offered() {
        let left = uncovered(&machine(), &[], &[], Some(Path::new(HOME)));
        assert_eq!(roots(&left), ["C:\\", "D:\\", "E:\\"]);
        assert_eq!(left[1].label, "Data");
        // A whole drive as a scope covers it, however it is spelled.
        let left = uncovered(&machine(), &paths(&["d:\\", "E:/"]), &[], None);
        assert_eq!(roots(&left), ["C:\\"]);
    }

    #[test]
    fn a_drive_root_is_a_path_with_no_folder_above_it() {
        assert!(is_drive_root(Path::new("D:\\")));
        assert!(is_drive_root(Path::new("D:/")));
        assert!(is_drive_root(Path::new("/")));
        assert!(!is_drive_root(Path::new("D:\\Games")));
        assert!(!is_drive_root(Path::new("C:/Users/ana")));
        assert!(!is_drive_root(Path::new("")));
    }
}
