//! A typed path lists its folder (F16, step 6d).
//!
//! When the whole query is a path -- `C:\Users\`, `d:/src/lu`, `~\Doc` --
//! the answer is not the index's but the disk's: the folder up to the last
//! separator, read now, its entries filtered by what follows. The user
//! named the folder, so Lumen's skip list and their exclusions do not
//! apply; only hidden entries stay hidden unless asked for. Whether an
//! entry is hidden is `ignore`'s question, as in the walk: it reads the
//! Windows hidden attribute itself, so this module makes no OS call (I-1).
//!
//! Pure besides the one directory read, which runs off the UI thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use ignore::WalkBuilder;

use super::Found;

/// A query that is a path, split where completion and listing need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typed {
    /// The folder to list: the query up to its last separator, `~` expanded.
    pub dir: PathBuf,
    /// The text after the last separator, matched inside an entry's name.
    pub filter: String,
    /// The query up to and including its last separator, as typed, so a
    /// completion keeps `~\` and the user's separators.
    pub shown: String,
}

/// The path `query` names, if the whole of it is one: it starts with a
/// drive's root (`C:\`, `d:/`) or the home folder's (`~\`, `~/`).
/// Separators may be mixed.
///
/// Leading spaces are dropped, since they are never part of a path one
/// types; trailing ones are kept, in the filter, since a name may end in
/// one. Not paths: `C:` and `~` alone (a separator says "list it"),
/// network shares, and anything relative, which has no folder to be
/// relative to.
pub fn parse(query: &str, home: &Path) -> Option<Typed> {
    let query = query.trim_start();
    let bytes = query.as_bytes();
    let is_separator = |byte: u8| byte == b'\\' || byte == b'/';
    let rest_at = match bytes {
        [b'~', separator, ..] if is_separator(*separator) => 2,
        [letter, b':', separator, ..]
            if letter.is_ascii_alphabetic() && is_separator(*separator) =>
        {
            3
        }
        _ => return None,
    };
    // The prefix is ASCII, and so are the separators: every index is a
    // character boundary.
    let split = query.rfind(['\\', '/'])? + 1;
    let shown = &query[..split];
    let dir = if bytes[0] == b'~' {
        home.join(&query[rest_at..split])
    } else {
        PathBuf::from(shown)
    };
    Some(Typed {
        dir,
        filter: query[split..].to_string(),
        shown: shown.to_string(),
    })
}

/// At most `cap` entries of `typed.dir` whose names contain `typed.filter`,
/// in any case: those that start with it first, then folders, then by
/// name. Hidden ones only with `include_hidden`. A folder that is missing,
/// is a file or cannot be read lists nothing.
///
/// The order needs every match, so a huge folder is read whole before the
/// cap applies: one directory read and a sort of its matches, which is
/// milliseconds for tens of thousands of entries. Stops, with nothing, as
/// soon as `cancelled` says the query was superseded.
pub fn list(typed: &Typed, include_hidden: bool, cap: usize, cancelled: &AtomicBool) -> Vec<Found> {
    let filter = typed.filter.to_lowercase();
    let mut builder = WalkBuilder::new(&typed.dir);
    builder
        // No ignore files, no parents' rules: the user named this folder.
        .standard_filters(false)
        .hidden(!include_hidden)
        .follow_links(false)
        .max_depth(Some(1));

    // (starts with the filter, is a folder, lowercase name), to sort by.
    let mut matches: Vec<(bool, bool, String, Found)> = Vec::new();
    for entry in builder.build() {
        if cancelled.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let Ok(entry) = entry else { continue };
        if entry.depth() == 0 {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let Some(at) = name.find(&filter) else {
            continue;
        };
        // A link to a folder is listed as the folder it opens.
        let is_dir = match entry.file_type() {
            Some(kind) if kind.is_symlink() => entry.path().is_dir(),
            Some(kind) => kind.is_dir(),
            None => false,
        };
        let found = Found {
            path: entry.into_path(),
            is_dir,
        };
        matches.push((at == 0, is_dir, name, found));
    }

    matches.sort_by(|a, b| {
        (b.0, b.1)
            .cmp(&(a.0, a.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.path.cmp(&b.3.path))
    });
    matches.truncate(cap);
    matches.into_iter().map(|(.., found)| found).collect()
}

/// The query that completes `typed` to `found`: the prefix as typed, the
/// entry's name, and for a folder the separator the user last typed, so
/// the next keystroke lists inside it.
pub fn complete(typed: &Typed, found: &Found) -> String {
    let mut query = format!("{}{}", typed.shown, found.name());
    if found.is_dir {
        query.push(if typed.shown.ends_with('/') {
            '/'
        } else {
            '\\'
        });
    }
    query
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::walk::tests::Tree;

    fn typed(dir: &Path, filter: &str) -> Typed {
        Typed {
            dir: dir.to_path_buf(),
            filter: filter.into(),
            shown: format!("{}\\", dir.display()),
        }
    }

    /// The names `list` answers, folders marked by a trailing `/`.
    fn names(found: &[Found]) -> Vec<String> {
        found
            .iter()
            .map(|found| {
                let slash = if found.is_dir { "/" } else { "" };
                format!("{}{slash}", found.name())
            })
            .collect()
    }

    fn listed(typed: &Typed, include_hidden: bool, cap: usize) -> Vec<String> {
        names(&list(typed, include_hidden, cap, &AtomicBool::new(false)))
    }

    #[test]
    fn a_drive_root_in_either_separator_and_case_is_a_path() {
        let home = Path::new("H:\\home");
        let parsed = parse("C:\\Users\\do", home).unwrap();
        assert_eq!(parsed.dir, PathBuf::from("C:\\Users\\"));
        assert_eq!(parsed.filter, "do");
        assert_eq!(parsed.shown, "C:\\Users\\");

        let parsed = parse("d:/src/lu", home).unwrap();
        assert_eq!(parsed.dir, PathBuf::from("d:/src/"));
        assert_eq!(parsed.filter, "lu");
        assert_eq!(parsed.shown, "d:/src/");

        let parsed = parse("C:\\", home).unwrap();
        assert_eq!(parsed.dir, PathBuf::from("C:\\"));
        assert_eq!(parsed.filter, "");
        assert_eq!(parsed.shown, "C:\\");
    }

    #[test]
    fn separators_may_be_mixed() {
        let parsed = parse("C:/Users\\me/Doc", Path::new("H:\\home")).unwrap();
        assert_eq!(parsed.dir, PathBuf::from("C:/Users\\me/"));
        assert_eq!(parsed.filter, "Doc");
        assert_eq!(parsed.shown, "C:/Users\\me/");
    }

    #[test]
    fn a_tilde_is_the_home_folder_and_stays_typed() {
        let home = Path::new("H:\\home");
        let parsed = parse("~\\Documents\\re", home).unwrap();
        assert_eq!(parsed.dir, home.join("Documents\\"));
        assert_eq!(parsed.filter, "re");
        assert_eq!(parsed.shown, "~\\Documents\\");

        let parsed = parse("~/Do", home).unwrap();
        assert_eq!(parsed.dir, home);
        assert_eq!(parsed.filter, "Do");
        assert_eq!(parsed.shown, "~/");
    }

    #[test]
    fn leading_spaces_are_dropped_and_trailing_ones_filter() {
        let parsed = parse("  C:\\Users\\a ", Path::new("H:\\home")).unwrap();
        assert_eq!(parsed.shown, "C:\\Users\\");
        assert_eq!(parsed.filter, "a ");
    }

    #[test]
    fn anything_else_is_not_a_path() {
        let home = Path::new("H:\\home");
        for query in [
            "",
            "C:",
            "C",
            "~",
            "~foo",
            "\\\\server\\share\\",
            ".\\src\\",
            "..\\src\\",
            "./src/",
            "notes",
            "foo C:\\",
            "1:\\",
            "CD:\\",
            "\\Users\\",
            "/usr/",
        ] {
            assert_eq!(parse(query, home), None, "{query:?}");
        }
    }

    #[test]
    fn names_starting_with_the_filter_come_first_then_folders() {
        let tree = Tree::new("browse-order");
        tree.file("my-report.txt")
            .file("Report.pdf")
            .dir("reports")
            .dir("old-reports")
            .file("notes.txt");

        assert_eq!(
            listed(&typed(&tree.0, "REP"), false, 10),
            ["reports/", "Report.pdf", "old-reports/", "my-report.txt"]
        );
    }

    #[test]
    fn an_empty_filter_lists_everything_folders_first() {
        let tree = Tree::new("browse-empty");
        tree.file("b.txt").file("A.txt").dir("z").dir("C");

        assert_eq!(
            listed(&typed(&tree.0, ""), false, 10),
            ["C/", "z/", "A.txt", "b.txt"]
        );
    }

    #[test]
    fn the_cap_applies_after_the_order() {
        let tree = Tree::new("browse-cap");
        tree.file("a.txt").file("b.txt").dir("y").dir("z");

        assert_eq!(listed(&typed(&tree.0, ""), false, 3), ["y/", "z/", "a.txt"]);
        assert!(listed(&typed(&tree.0, ""), false, 0).is_empty());
    }

    #[test]
    fn a_missing_folder_lists_nothing() {
        let tree = Tree::new("browse-missing");
        assert!(listed(&typed(&tree.path("gone"), ""), true, 10).is_empty());
        tree.file("a-file");
        assert!(listed(&typed(&tree.path("a-file"), ""), true, 10).is_empty());
    }

    #[test]
    fn dotted_entries_are_hidden_unless_asked_for() {
        let tree = Tree::new("browse-dotted");
        tree.file(".env").dir(".config").file("readme.txt");

        assert_eq!(listed(&typed(&tree.0, ""), false, 10), ["readme.txt"]);
        assert_eq!(
            listed(&typed(&tree.0, ""), true, 10),
            [".config/", ".env", "readme.txt"]
        );
    }

    /// The user typed the hidden folder, so its own entries are listed.
    #[test]
    fn a_hidden_folder_typed_by_name_is_listed() {
        let tree = Tree::new("browse-into-dotted");
        tree.file(".config/app.toml");

        assert_eq!(
            listed(&typed(&tree.path(".config"), ""), false, 10),
            ["app.toml"]
        );
    }

    /// The Windows hidden attribute, set with `attrib` where there is one.
    /// Elsewhere there is no such attribute, and nothing to test.
    #[test]
    fn the_hidden_attribute_hides_unless_asked_for() {
        let tree = Tree::new("browse-attribute");
        tree.file("secret.txt").file("open.txt");
        let set = std::process::Command::new("attrib")
            .arg("+h")
            .arg(tree.path("secret.txt"))
            .status();
        if !set.is_ok_and(|status| status.success()) {
            return;
        }

        assert_eq!(listed(&typed(&tree.0, ""), false, 10), ["open.txt"]);
        assert_eq!(
            listed(&typed(&tree.0, ""), true, 10),
            ["open.txt", "secret.txt"]
        );
        let _ = std::process::Command::new("attrib")
            .arg("-h")
            .arg(tree.path("secret.txt"))
            .status();
    }

    #[test]
    fn a_cancelled_listing_stops() {
        let tree = Tree::new("browse-cancelled");
        tree.file("a.txt").file("b.txt");

        let typed = typed(&tree.0, "");
        assert!(list(&typed, true, 10, &AtomicBool::new(true)).is_empty());
        assert_eq!(listed(&typed, true, 10).len(), 2);
    }

    #[test]
    fn completing_keeps_the_typed_prefix_and_ends_a_folder_with_its_separator() {
        let home = Path::new("H:\\home");
        let file = Found {
            path: PathBuf::from("H:\\home\\Documents\\report.pdf"),
            is_dir: false,
        };
        let folder = Found {
            path: PathBuf::from("H:\\home\\Documents\\reports"),
            is_dir: true,
        };

        let back = parse("~\\Documents\\rep", home).unwrap();
        assert_eq!(complete(&back, &file), "~\\Documents\\report.pdf");
        assert_eq!(complete(&back, &folder), "~\\Documents\\reports\\");

        let forward = parse("C:/Users/me/Documents/rep", home).unwrap();
        assert_eq!(
            complete(&forward, &file),
            "C:/Users/me/Documents/report.pdf"
        );
        assert_eq!(
            complete(&forward, &folder),
            "C:/Users/me/Documents/reports/"
        );
    }
}
