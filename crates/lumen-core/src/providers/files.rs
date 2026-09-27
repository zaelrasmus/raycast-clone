//! File search (F3).
//!
//! # What this is, and what it deliberately is not
//!
//! It is a bounded walk of the folders people keep their own things in --
//! Desktop, Documents, Downloads, Pictures, Video, Music, and anything else
//! they name in the configuration -- read once on a background task and matched
//! from memory.
//!
//! It is *not* a search of the whole disk. Two reasons, and the second is the
//! one that settles it.
//!
//! Windows already has an index, reachable through `ISearchQueryHelper` and
//! OLE DB. Using it would mean a second query path with its own latency,
//! reached through a COM interface that has no safe binding, for results the
//! user has already told us where to find. And I-3 forbids root search waiting
//! on anything, so the answer would have to be cached anyway -- at which point
//! it is this, with more moving parts.
//!
//! The honest description of the trade is in the README: Lumen finds the file
//! you were working on, not every file on the machine.
//!
//! # Why the skip rules look like this
//!
//! A naive walk of a developer's home folder finds several hundred thousand
//! files, almost none of which anyone would search for by name: dependency
//! trees, build output, virtual environments, version-control internals. They
//! are skipped by directory name, which is crude and correct -- `node_modules`
//! is never the answer.
//!
//! Hidden files are recognised by a leading dot. That is the Unix convention
//! and not the Windows one, which uses a file attribute -- and reading a file
//! attribute is an OS call, which I-1 does not allow in this crate. The
//! directory denylist covers what matters on Windows (`AppData` above all),
//! and the rule ports unchanged to Linux where it is exactly right.

use std::path::{Path, PathBuf};

use lumen_platform::LaunchTarget;
use serde::{Deserialize, Serialize};

use crate::item::{Action, Item, Kind};

/// How far the walk is allowed to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct FileLimits {
    /// Stop after this many entries, across all roots.
    ///
    /// This is a *budget* number, not a taste number, and it was chosen by
    /// measuring rather than by guessing. Ranking cost is linear in the number
    /// of candidates, and the worst case is a one-character query, where
    /// nothing is filtered out early. From `cargo bench -p lumen-core`:
    ///
    /// | Entries | One character typed |
    /// | ------- | ------------------- |
    /// | 5 000   | 1.06 ms             |
    /// | 20 000  | 7.42 ms             |
    ///
    /// The per-frame budget is 8 ms for everything, render included, so 20 000
    /// spends the whole budget on one list. 5 000 leaves the frame with room
    /// to spare and still covers the folders people actually keep work in.
    ///
    /// Raising it is supported and the cost is the table above -- which is why
    /// the table is here rather than in a commit message.
    pub max_entries: usize,
    /// How deep below each root to descend. `0` is the root itself.
    ///
    /// Six is deep enough to reach a file inside a project inside a folder of
    /// projects, and shallow enough that one badly nested tree cannot consume
    /// the whole entry budget.
    pub max_depth: usize,
}

impl Default for FileLimits {
    fn default() -> Self {
        Self {
            max_entries: 5_000,
            max_depth: 6,
        }
    }
}

/// Directory names never descended into.
///
/// Matched exactly and case-insensitively against the directory's own name, not
/// against the path, so a folder the user actually named `Build` is skipped
/// too. That is the right trade: the cost of skipping one folder is one file
/// not found, and the cost of descending into a `node_modules` is an index
/// made of nothing else.
const SKIPPED: &[&str] = &[
    // Dependency and build trees.
    "node_modules",
    "target",
    "build",
    "dist",
    "out",
    "obj",
    "bin",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    "env",
    // Version control and tooling state.
    ".git",
    ".svn",
    ".hg",
    ".cache",
    ".gradle",
    ".idea",
    ".vs",
    ".vscode",
    // Windows' own. `AppData` alone is larger than everything the user made.
    "appdata",
    "application data",
    "$recycle.bin",
    "system volume information",
];

/// Walk `roots` and return one [`Item`] per file found.
///
/// Runs on a background task: it touches the disk, and nothing in the launcher
/// waits for it (I-3). Unreadable directories are skipped rather than reported;
/// a permissions error somewhere under Documents is not something the user can
/// act on and not a reason to return nothing.
pub fn scan(roots: &[PathBuf], limits: FileLimits) -> Vec<Item> {
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Resolved once rather than per file: every subtitle is shortened against
    // it, and there are thousands of them.
    let home = directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf());

    for root in roots {
        // Roots can overlap -- someone whose Desktop is inside their OneDrive
        // Documents -- and a file found twice is a duplicate row.
        if !seen.insert(root.clone()) {
            continue;
        }
        walk(root, 0, limits, home.as_deref(), &mut items);
        if items.len() >= limits.max_entries {
            break;
        }
    }

    items
}

fn walk(
    directory: &Path,
    depth: usize,
    limits: FileLimits,
    home: Option<&Path>,
    items: &mut Vec<Item>,
) {
    if items.len() >= limits.max_entries {
        return;
    }

    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };

    // Collected first so that files are added before descending: a breadth-ish
    // order means the entry budget is spent near the roots, where the things
    // people look for actually live.
    let mut directories = Vec::new();

    for entry in entries.flatten() {
        if items.len() >= limits.max_entries {
            return;
        }

        let name = entry.file_name();
        let name = name.to_string_lossy();
        if is_hidden(&name) {
            continue;
        }

        let Ok(kind) = entry.file_type() else {
            continue;
        };

        // Symlinks are not followed. A link back up its own tree is an infinite
        // walk, and the target is almost always reachable by its real path
        // anyway.
        if kind.is_symlink() {
            continue;
        }

        if kind.is_dir() {
            if depth < limits.max_depth && !is_skipped(&name) {
                directories.push(entry.path());
            }
            continue;
        }

        items.push(item(&entry.path(), &name, home));
    }

    for directory in directories {
        walk(&directory, depth + 1, limits, home, items);
    }
}

/// A path as it should be *shown*: the user's home replaced by `~`.
///
/// Purely cosmetic and applied only to the subtitle -- the action and the
/// searchable keyword both keep the real path. A file subtitle is the longest
/// text in the list, and on Windows the first thirty characters of it are
/// `C:\Users\<name>\` on every single row: thirty characters that say nothing,
/// pushing out the part that does.
fn for_display(path: &Path, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return path.display().to_string();
    };

    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

fn is_skipped(name: &str) -> bool {
    SKIPPED
        .iter()
        .any(|skipped| name.eq_ignore_ascii_case(skipped))
}

fn item(path: &Path, name: &str, home: Option<&Path>) -> Item {
    let parent = path
        .parent()
        .map(|parent| for_display(parent, home))
        .unwrap_or_default();

    Item {
        // The path, which is what makes a file unique and is stable across
        // rescans -- so frecency remembers the file you always open.
        id: format!("file:{}", path.display()),
        title: name.to_string(),
        // The folder, because a name on its own does not tell two `notes.md`
        // apart, and the folder is how people remember which is which.
        subtitle: Some(parent),
        icon: None,
        // The whole path is matched, so `proj inv` finds
        // `Documents/projects/invoice.pdf` without the user typing the folder
        // separators.
        keywords: vec![path.display().to_string()],
        actions: vec![
            // Through the shell, which is what knows the default application
            // for an extension.
            Action::Launch(LaunchTarget::ShellExecute(path.display().to_string())),
            Action::CopyToClipboard(path.display().to_string()),
        ],
        kind: Kind::File,
    }
}

/// The folders searched when the configuration names none.
///
/// The user's own directories, resolved through the platform's conventions
/// rather than assumed to be `~/Documents`: on Windows they can be redirected
/// to OneDrive, and on Linux they are whatever `user-dirs.dirs` says.
pub fn default_roots() -> Vec<PathBuf> {
    let Some(dirs) = directories::UserDirs::new() else {
        return Vec::new();
    };

    [
        dirs.desktop_dir(),
        dirs.document_dir(),
        dirs.download_dir(),
        dirs.picture_dir(),
        dirs.video_dir(),
        dirs.audio_dir(),
    ]
    .into_iter()
    .flatten()
    .map(Path::to_path_buf)
    .filter(|path| path.is_dir())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small tree under a unique temporary directory, cleaned up by the
    /// caller. No test fixture crate: this is twenty lines and one dependency
    /// fewer.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("lumen-files-{name}"));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn file(&self, relative: &str) -> &Self {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"x").unwrap();
            self
        }

        fn scan(&self, limits: FileLimits) -> Vec<Item> {
            scan(std::slice::from_ref(&self.0), limits)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn titles(items: &[Item]) -> Vec<&str> {
        items.iter().map(|item| item.title.as_str()).collect()
    }

    #[test]
    fn files_are_found_with_their_folder_as_the_subtitle() {
        let tree = Tree::new("basic");
        tree.file("notes.md").file("projects/invoice.pdf");

        let items = tree.scan(FileLimits::default());

        assert_eq!(items.len(), 2);
        let invoice = items
            .iter()
            .find(|item| item.title == "invoice.pdf")
            .unwrap();
        assert!(invoice.subtitle.as_deref().unwrap().ends_with("projects"));
        assert_eq!(invoice.kind, Kind::File);
    }

    /// The whole path is searchable, so a folder name finds what is inside it
    /// without the user typing separators.
    #[test]
    fn the_path_is_matched_even_though_only_the_name_is_shown() {
        let tree = Tree::new("keywords");
        tree.file("projects/invoice.pdf");

        let items = tree.scan(FileLimits::default());

        assert_eq!(items[0].title, "invoice.pdf");
        assert!(items[0].keywords[0].contains("projects"));
    }

    /// The rule that keeps the index about the user's own files rather than
    /// about their dependency trees.
    #[test]
    fn dependency_and_build_directories_are_never_descended_into() {
        let tree = Tree::new("skips");
        tree.file("app/main.rs")
            .file("app/node_modules/left-pad/index.js")
            .file("app/target/debug/build.log")
            .file("app/.git/config");

        assert_eq!(titles(&tree.scan(FileLimits::default())), ["main.rs"]);
    }

    #[test]
    fn dotted_files_are_treated_as_hidden() {
        let tree = Tree::new("hidden");
        tree.file(".env").file("readme.txt");

        assert_eq!(titles(&tree.scan(FileLimits::default())), ["readme.txt"]);
    }

    /// Depth is counted from the root, and the root's own files are depth zero.
    #[test]
    fn the_walk_stops_at_the_depth_limit() {
        let tree = Tree::new("depth");
        tree.file("one.txt").file("a/two.txt").file("a/b/three.txt");

        let shallow = tree.scan(FileLimits {
            max_depth: 1,
            ..Default::default()
        });

        let mut found = titles(&shallow);
        found.sort();
        assert_eq!(found, ["one.txt", "two.txt"]);
    }

    #[test]
    fn the_entry_budget_is_never_exceeded() {
        let tree = Tree::new("budget");
        for index in 0..50 {
            tree.file(&format!("file-{index}.txt"));
        }

        let capped = tree.scan(FileLimits {
            max_entries: 10,
            ..Default::default()
        });

        assert_eq!(capped.len(), 10);
    }

    /// Roots can overlap -- a Desktop inside a OneDrive Documents -- and the
    /// same file must not produce two rows.
    #[test]
    fn a_repeated_root_is_walked_once() {
        let tree = Tree::new("overlap");
        tree.file("only.txt");

        let items = scan(&[tree.0.clone(), tree.0.clone()], FileLimits::default());

        assert_eq!(items.len(), 1);
    }

    #[test]
    fn a_root_that_does_not_exist_is_not_an_error() {
        let items = scan(
            &[PathBuf::from("no/such/folder/anywhere")],
            FileLimits::default(),
        );
        assert!(items.is_empty());
    }

    /// Opening is the primary action; the path is there for the action panel.
    #[test]
    fn a_path_under_the_home_directory_is_shown_with_a_tilde() {
        let home = PathBuf::from("C:/Users/someone");
        let shown = for_display(&home.join("Documents/notes"), Some(&home));

        assert!(shown.starts_with('~'), "{shown}");
        assert!(shown.ends_with("notes"));
        assert!(!shown.contains("someone"));
    }

    /// A path somewhere else entirely is left alone rather than mangled.
    #[test]
    fn a_path_outside_the_home_directory_is_shown_in_full() {
        let home = PathBuf::from("C:/Users/someone");
        let elsewhere = PathBuf::from("D:/work/report");

        assert_eq!(
            for_display(&elsewhere, Some(&home)),
            elsewhere.display().to_string()
        );
    }

    /// Only the subtitle is shortened. Opening and copying must use the real
    /// path, or a tilde reaches the shell and nothing opens.
    #[test]
    fn the_action_and_the_keyword_keep_the_real_path() {
        let tree = Tree::new("realpath");
        tree.file("thing.txt");

        let items = tree.scan(FileLimits::default());
        let real = tree.0.join("thing.txt").display().to_string();

        assert_eq!(items[0].keywords[0], real);
        assert_eq!(
            items[0].primary(),
            Action::Launch(LaunchTarget::ShellExecute(real))
        );
    }

    #[test]
    fn a_file_opens_with_the_shell_and_can_have_its_path_copied() {
        let tree = Tree::new("actions");
        tree.file("thing.txt");

        let items = tree.scan(FileLimits::default());

        assert!(matches!(items[0].primary(), Action::Launch(_)));
        assert!(
            items[0]
                .actions
                .iter()
                .any(|action| matches!(action, Action::CopyToClipboard(_)))
        );
    }
}
