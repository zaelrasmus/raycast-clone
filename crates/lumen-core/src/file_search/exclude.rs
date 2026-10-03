//! *Exclude from Index*: a line in a `.lumenignore` (F16).
//!
//! Written where the user would look for it: the nearest `.lumenignore`
//! already above the item, up to its scope's folder, so one folder's rules
//! stay in one file; else a new one beside the item. The line is the item's
//! path from that file's folder, anchored with a leading `/`, so it leaves
//! out this item and not every file of the same name below.
//!
//! Nothing else is told: the walk reads `.lumenignore` (`walk::walker`), and
//! a changed ignore file reads its folder and everything below it again
//! (`update::folders`), whether the watcher reports it or the view hands it
//! to the index itself.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, bail};

/// The ignore file Lumen reads and writes.
pub const IGNORE_FILE: &str = ".lumenignore";

/// Leave `path` out of the index by a line in a `.lumenignore`, chosen
/// among `scopes` as above. The ignore file written.
pub fn exclude(path: &Path, is_dir: bool, scopes: &[PathBuf]) -> Result<PathBuf> {
    let Some(scope) = scopes
        .iter()
        .filter(|scope| path.starts_with(scope) && path != scope.as_path())
        .max_by_key(|scope| scope.components().count())
    else {
        bail!("{} is in none of the indexed folders", path.display());
    };
    let parent = path.parent().context("an item with no folder")?;

    let mut folder = parent;
    let file = loop {
        let candidate = folder.join(IGNORE_FILE);
        if candidate.is_file() {
            break candidate;
        }
        match folder.parent() {
            Some(up) if folder != scope.as_path() && up.starts_with(scope) => folder = up,
            _ => break parent.join(IGNORE_FILE),
        }
    };
    let home = file.parent().expect("joined to a folder");
    let relative = path.strip_prefix(home).expect("below its ignore file");
    let line = rule(relative, is_dir);

    let existing = match std::fs::read_to_string(&file) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", file.display()));
        }
    };
    if existing.lines().any(|known| known.trim_end() == line) {
        return Ok(file);
    }
    let mut contents = existing;
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(&line);
    contents.push('\n');
    std::fs::write(&file, contents).with_context(|| format!("writing {}", file.display()))?;
    Ok(file)
}

/// The gitignore line for `relative`: anchored, `/`-separated, a folder
/// marked by a trailing `/`, and the characters a pattern would read as
/// wildcards escaped.
fn rule(relative: &Path, is_dir: bool) -> String {
    let mut line = String::new();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            line.push('/');
            let part = part.to_string_lossy();
            let trailing = part.len() - part.trim_end_matches(' ').len();
            let kept = &part[..part.len() - trailing];
            for character in kept.chars() {
                if matches!(character, '\\' | '*' | '?' | '[' | ']') {
                    line.push('\\');
                }
                line.push(character);
            }
            for _ in 0..trailing {
                line.push_str("\\ ");
            }
        }
    }
    if is_dir {
        line.push('/');
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::walk::tests::Tree;
    use crate::file_index::{FileIndex, Kinds, walk};

    fn found(tree: &Tree, rules: &crate::file_index::WalkRules) -> Vec<String> {
        walk(rules)
            .entries
            .iter()
            .map(|entry| {
                entry
                    .path
                    .strip_prefix(&tree.0)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }

    #[test]
    fn without_an_ignore_file_above_one_is_made_beside_the_item() {
        let tree = Tree::new("exclude-beside");
        tree.file("work/drafts/old.txt").file("work/drafts/new.txt");
        let written = exclude(
            &tree.path("work/drafts/old.txt"),
            false,
            std::slice::from_ref(&tree.0),
        )
        .unwrap();

        assert_eq!(written, tree.path("work/drafts").join(IGNORE_FILE));
        assert_eq!(std::fs::read_to_string(&written).unwrap(), "/old.txt\n");
        let left = found(&tree, &tree.rules());
        assert!(!left.contains(&"work/drafts/old.txt".to_string()));
        assert!(left.contains(&"work/drafts/new.txt".to_string()));
    }

    #[test]
    fn the_nearest_ignore_file_above_is_added_to() {
        let tree = Tree::new("exclude-nearest");
        tree.write("work/.lumenignore", "*.tmp")
            .file("work/a/b/report.pdf")
            .file("work/a/report.pdf");
        let written = exclude(
            &tree.path("work/a/b/report.pdf"),
            false,
            std::slice::from_ref(&tree.0),
        )
        .unwrap();

        assert_eq!(written, tree.path("work").join(IGNORE_FILE));
        assert_eq!(
            std::fs::read_to_string(&written).unwrap(),
            "*.tmp\n/a/b/report.pdf\n"
        );
        let left = found(&tree, &tree.rules());
        assert!(!left.contains(&"work/a/b/report.pdf".to_string()));
        assert!(left.contains(&"work/a/report.pdf".to_string()));
    }

    /// Above the scope is not the user's to write in for this.
    #[test]
    fn an_ignore_file_above_the_scope_is_not_used() {
        let tree = Tree::new("exclude-scope");
        tree.write(".lumenignore", "").file("scope/x/item.txt");
        let scope = tree.path("scope");
        let written = exclude(&tree.path("scope/x/item.txt"), false, &[scope]).unwrap();
        assert_eq!(written, tree.path("scope/x").join(IGNORE_FILE));
    }

    #[test]
    fn a_folder_is_excluded_with_everything_in_it_and_only_once() {
        let tree = Tree::new("exclude-folder");
        tree.file("cache [old]/a.bin").file("keep/b.txt");
        let path = tree.path("cache [old]");
        exclude(&path, true, std::slice::from_ref(&tree.0)).unwrap();
        let written = exclude(&path, true, std::slice::from_ref(&tree.0)).unwrap();

        assert_eq!(
            std::fs::read_to_string(written).unwrap(),
            "/cache \\[old\\]/\n"
        );
        let left = found(&tree, &tree.rules());
        assert_eq!(left, ["keep", "keep/b.txt"]);
    }

    #[test]
    fn an_item_outside_every_scope_is_refused() {
        let tree = Tree::new("exclude-outside");
        tree.file("a.txt");
        assert!(exclude(&tree.path("a.txt"), false, &[tree.path("elsewhere")]).is_err());
    }

    /// The path the watcher takes: the ignore file reported changed, and
    /// its folder read again by the index.
    #[test]
    fn the_index_reads_the_change_and_forgets_the_item() {
        let tree = Tree::new("exclude-index");
        tree.file("notes/secret-plan.md").file("notes/plan.md");
        let db = tree.path(".files.db");
        let rules = tree.rules();
        let index = FileIndex::build(&db, &rules).unwrap();
        assert_eq!(
            index
                .query("plan", Kinds::FoldersAndFiles, 10, &|| false)
                .len(),
            2
        );

        let written = exclude(
            &tree.path("notes/secret-plan.md"),
            false,
            std::slice::from_ref(&tree.0),
        )
        .unwrap();
        let updated = index
            .update(&db, &rules, &[written], &|| false)
            .unwrap()
            .unwrap()
            .index
            .expect("an entry left");
        let titles: Vec<String> = updated
            .query("plan", Kinds::FoldersAndFiles, 10, &|| false)
            .into_iter()
            .map(|hit| hit.item.title)
            .collect();
        assert_eq!(titles, ["plan.md"]);
    }
}
