//! The file index (F16, `docs/FILE-SEARCH.md`).
//!
//! # Entries on disk, names in memory
//!
//! An index is every file and folder the rules allow in the scopes, built
//! once so a keystroke never touches the disk. F3 held its 5 000 entries in
//! memory with the matcher's prepared form of each, which is affordable at
//! 5 000 and not at the hundreds of thousands a second drive holds. So it is
//! split, as slice 0 measured:
//!
//! - [`walk`] finds the entries, with the skip rules;
//! - [`Store`] keeps them in SQLite, in the cache folder, so a start loads
//!   them instead of walking again;
//! - [`Names`] holds only their names, folded into one blob, and answers a
//!   query by scanning it.
//!
//! A query asks [`Names`] for the best few entries, then the store for just
//! those rows, in one statement.
//!
//! # Never where anything waits
//!
//! Building walks the disk and querying reads the database, so both run off
//! the UI thread, and root search never waits for either (I-3).

pub mod names;
pub mod store;
pub mod walk;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use parking_lot::Mutex;

pub use names::{Kinds, Match, Names};
pub use store::{Meta, Row, Store, digest};
pub use walk::{CEILING, Entry, WalkRules, Walked, walk};

use lumen_platform::LaunchTarget;

use crate::frecency::Frecency;
use crate::item::{Action, Item, Kind};

/// One row a query found, with the characters of its title that matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHit {
    pub item: Item,
    /// Character offsets into the title, as root search's highlights are.
    pub highlights: Vec<u32>,
}

/// The index, loaded and ready to query.
pub struct FileIndex {
    names: Names,
    /// Behind a lock because SQLite's connection is not `Sync`, and a query
    /// arrives from whichever thread asks.
    store: Mutex<Store>,
    /// Frecency by entry, for the few entries that have any: looked up for
    /// every match, so it is a map, not a call into [`Frecency`] by path.
    boosts: HashMap<u32, u32>,
    meta: Meta,
    /// Resolved once rather than per row: every subtitle is shortened
    /// against it.
    home: Option<PathBuf>,
}

/// A path as it should be *shown*: the user's home replaced by `~`.
///
/// Purely cosmetic and applied only to the subtitle -- the action and the
/// searchable keyword both keep the real path. A file subtitle is the longest
/// text in the list, and on Windows the first thirty characters of it are
/// `C:\Users\<name>\` on every single row: thirty characters that say nothing,
/// pushing out the part that does.
pub(crate) fn for_display(path: &Path, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return path.display().to_string();
    };

    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}

impl FileIndex {
    /// The index as the store last recorded it, without touching the disk
    /// beyond the database: what a start does.
    pub fn load(store: Store) -> Result<FileIndex> {
        let (names, meta) = store.load()?;
        // The names are in memory now; the pages that held them are not
        // needed again.
        store.release_memory();
        Ok(Self {
            names,
            store: Mutex::new(store),
            boosts: HashMap::new(),
            meta,
            home: directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()),
        })
    }

    /// Walk `rules` and make the result the whole index. Seconds to minutes,
    /// depending on the scopes.
    pub fn build(mut store: Store, rules: &WalkRules) -> Result<FileIndex> {
        let walked = walk(rules);
        store.replace(&walked, &digest(rules))?;
        drop(walked);
        Self::load(store)
    }

    #[cfg(test)]
    fn with_home(mut self, home: &Path) -> Self {
        self.home = Some(home.to_path_buf());
        self
    }

    /// The best `limit` files and folders for `query`, best first.
    ///
    /// Ranked by how well the name matched, then frecency, then the shorter
    /// name ([`Names::find`]). A database that cannot be read answers
    /// nothing rather than an error: the launcher has nothing to tell the
    /// user mid-keystroke, and the next rebuild replaces it.
    pub fn query(&self, query: &str, kinds: Kinds, limit: usize) -> Vec<FileHit> {
        let found = self.names.find(query, kinds, limit, |entry| {
            self.boosts.get(&entry).copied().unwrap_or(0)
        });
        if found.is_empty() {
            return Vec::new();
        }
        let ids = found.iter().map(|found| found.entry).collect::<Vec<_>>();
        let rows = match self.store.lock().rows(&ids) {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("could not read the file index: {error:#}");
                return Vec::new();
            }
        };

        let words = names::words(query);
        rows.into_iter()
            .map(|row| FileHit {
                highlights: highlights(&row.name, &words),
                item: self.item(row),
            })
            .collect()
    }

    fn item(&self, row: Row) -> Item {
        let path = row.path.display().to_string();
        let (prefix, kind) = if row.is_dir {
            ("folder", Kind::Folder)
        } else {
            ("file", Kind::File)
        };
        Item {
            // The path, which is what makes an entry unique and is stable
            // across rebuilds, so frecency remembers it. `file:` is F3's
            // prefix, kept so the history of files opened before the index
            // still counts.
            id: format!("{prefix}:{path}"),
            subtitle: Some(
                row.path
                    .parent()
                    .map(|parent| for_display(parent, self.home.as_deref()))
                    .unwrap_or_default(),
            ),
            title: row.name,
            icon: None,
            // Already matched, by the index: nothing is matched against
            // these again.
            keywords: Vec::new(),
            actions: vec![
                // Through the shell, which knows the default application for
                // an extension and opens a folder in Explorer.
                Action::Launch(LaunchTarget::ShellExecute(path.clone())),
                Action::CopyToClipboard(path),
            ],
            kind,
        }
    }

    /// Take frecency's boosts for the entries that have one.
    ///
    /// Only ids of files and folders are looked up, each once, by path; an id
    /// whose path is no longer indexed is left out. Called when the index is
    /// loaded and again whenever frecency changes.
    pub fn set_boosts(&mut self, frecency: &Frecency) {
        let store = self.store.lock();
        let mut boosts = HashMap::new();
        for id in frecency.ids() {
            let (path, is_dir) = if let Some(path) = id.strip_prefix("file:") {
                (path, false)
            } else if let Some(path) = id.strip_prefix("folder:") {
                (path, true)
            } else {
                continue;
            };
            let boost = frecency.boost(id);
            if boost == 0 {
                continue;
            }
            match store.id_of(Path::new(path)) {
                Ok(Some(entry)) if self.names.is_dir(entry) == is_dir => {
                    boosts.insert(entry, boost);
                }
                Ok(_) => {}
                Err(error) => log::warn!("could not read the file index: {error:#}"),
            }
        }
        store.release_memory();
        drop(store);
        self.boosts = boosts;
    }

    /// How many entries each scope of the rules gave.
    pub fn per_scope(&self) -> &[usize] {
        &self.meta.per_scope
    }

    /// The scope the ceiling cut short, if one was: said to the user, never
    /// hidden.
    pub fn truncated(&self) -> Option<&Path> {
        self.meta.truncated.as_deref()
    }

    /// The [`digest`] of the rules the index was walked with, to compare with
    /// the rules of today.
    pub fn digest(&self) -> &str {
        &self.meta.digest
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// What the index holds in memory: the names and the boosts.
    pub fn memory_bytes(&self) -> usize {
        self.names.memory_bytes() + self.boosts.capacity() * 8
    }
}

/// The characters of `title` each word matched, where it matched best.
///
/// The title is folded a character at a time, as the names are, remembering
/// which character each folded byte came from: lower-casing can change how
/// many there are (`İ` is two), so an offset in the folded name is not one
/// in the title.
fn highlights(title: &str, words: &[String]) -> Vec<u32> {
    let mut folded = String::with_capacity(title.len());
    let mut source = Vec::with_capacity(title.len());
    for (index, character) in title.chars().enumerate() {
        names::fold(character.encode_utf8(&mut [0; 4]), &mut folded);
        source.resize(folded.len(), index as u32);
    }

    let mut highlights = Vec::new();
    for word in words {
        if let Some((at, _)) = names::best_occurrence(&folded, word, 0) {
            highlights.extend_from_slice(&source[at..at + word.len()]);
        }
    }
    highlights.sort_unstable();
    highlights.dedup();
    highlights
}

#[cfg(test)]
mod tests {
    use super::walk::tests::{Tree, rules};
    use super::*;
    use crate::item::{Action, Kind};
    use lumen_platform::LaunchTarget;

    /// A tree, and an index of it built into a database inside it -- which
    /// is hidden from the walk by its own name, `.files.db`.
    fn index(tree: &Tree) -> FileIndex {
        let store = Store::open(&tree.path(".files.db")).unwrap();
        FileIndex::build(store, &tree.rules())
            .unwrap()
            .with_home(&tree.0)
    }

    fn titles(hits: &[FileHit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.item.title.as_str()).collect()
    }

    #[test]
    fn a_file_is_an_item_with_its_folder_shortened_as_the_subtitle() {
        let tree = Tree::new("index-file");
        tree.file("projects/Invoice-2024.pdf");
        let index = index(&tree);

        let hits = index.query("inv", Kinds::FoldersAndFiles, 8);
        assert_eq!(hits.len(), 1);
        let path = tree.path("projects").join("Invoice-2024.pdf");
        let real = path.display().to_string();
        let item = &hits[0].item;

        assert_eq!(item.id, format!("file:{real}"));
        assert_eq!(item.title, "Invoice-2024.pdf");
        assert_eq!(
            item.subtitle.as_deref(),
            Some(format!("~{}projects", std::path::MAIN_SEPARATOR).as_str())
        );
        assert_eq!(item.kind, Kind::File);
        assert_eq!(item.icon, None);
        assert_eq!(
            item.actions,
            [
                Action::Launch(LaunchTarget::ShellExecute(real.clone())),
                Action::CopyToClipboard(real),
            ]
        );
        assert_eq!(hits[0].highlights, [0, 1, 2]);
    }

    #[test]
    fn a_folder_is_an_item_of_its_own_kind() {
        let tree = Tree::new("index-folder");
        tree.file("projects/readme.md");
        let index = index(&tree);

        let hits = index.query("proj", Kinds::Folders, 8);
        assert_eq!(titles(&hits), ["projects"]);
        let item = &hits[0].item;
        assert_eq!(
            item.id,
            format!("folder:{}", tree.path("projects").display())
        );
        assert_eq!(item.kind, Kind::Folder);
        assert_eq!(item.subtitle.as_deref(), Some("~"));
        assert_eq!(item.kind.label(), "Folder");
    }

    #[test]
    fn every_word_is_highlighted_where_it_matched() {
        let tree = Tree::new("index-highlights");
        tree.file("Invoice 2024.pdf");
        let index = index(&tree);

        let hits = index.query("2024 inv", Kinds::FoldersAndFiles, 8);
        assert_eq!(hits[0].highlights, [0, 1, 2, 8, 9, 10, 11]);
    }

    /// Highlights count characters of the title as shown, even where
    /// lower-casing changes how many there are: `İ` is two.
    #[test]
    fn highlights_are_characters_of_the_real_title() {
        let tree = Tree::new("index-unicode");
        tree.file("İstanbul.txt");
        let index = index(&tree);

        let hits = index.query("stan", Kinds::FoldersAndFiles, 8);
        assert_eq!(hits[0].highlights, [1, 2, 3, 4]);
    }

    #[test]
    fn frecency_reorders_matches_of_the_same_tier() {
        let tree = Tree::new("index-boost");
        tree.file("report-a.txt").file("report-b.txt");
        let mut index = index(&tree);

        assert_eq!(
            titles(&index.query("report", Kinds::FoldersAndFiles, 8)),
            ["report-a.txt", "report-b.txt"]
        );

        let mut frecency = Frecency::new();
        frecency.record(&format!("file:{}", tree.path("report-b.txt").display()));
        frecency.record("app:unrelated");
        frecency.record(&format!("file:{}", tree.path("gone.txt").display()));
        index.set_boosts(&frecency);

        assert_eq!(
            titles(&index.query("report", Kinds::FoldersAndFiles, 8)),
            ["report-b.txt", "report-a.txt"]
        );
    }

    #[test]
    fn a_built_index_loads_back_from_its_store() {
        let tree = Tree::new("index-load");
        tree.file("notes.md").file("projects/plan.md");
        let built = index(&tree);
        assert_eq!(built.len(), 3);
        assert_eq!(built.per_scope(), [3]);
        assert_eq!(built.truncated(), None);
        assert_eq!(built.digest(), digest(&tree.rules()));
        assert!(built.memory_bytes() > 0);
        drop(built);

        let loaded = FileIndex::load(Store::open(&tree.path(".files.db")).unwrap())
            .unwrap()
            .with_home(&tree.0);
        assert_eq!(loaded.len(), 3);
        assert_eq!(
            titles(&loaded.query("plan", Kinds::FoldersAndFiles, 8)),
            ["plan.md"]
        );
    }

    #[test]
    fn the_scope_that_did_not_fit_is_reported() {
        let first = Tree::new("index-full-first");
        let second = Tree::new("index-full-second");
        first.file("a.txt").file("b.txt");
        second.file("c.txt").file("d.txt");

        let mut rules = rules(&[first.0.clone(), second.0.clone()]);
        rules.ceiling = 3;
        let store = Store::open(&first.path(".files.db")).unwrap();
        let index = FileIndex::build(store, &rules).unwrap();

        assert_eq!(index.len(), 3);
        assert_eq!(index.per_scope(), [2, 1]);
        assert_eq!(index.truncated(), Some(second.0.as_path()));
    }

    #[test]
    fn a_path_under_the_home_directory_is_shown_with_a_tilde() {
        let home = PathBuf::from("C:/Users/someone");
        let shown = for_display(&home.join("Documents/notes"), Some(&home));

        assert!(shown.starts_with('~'), "{shown}");
        assert!(shown.ends_with("notes"));
        assert!(!shown.contains("someone"));
        assert_eq!(for_display(&home, Some(&home)), "~");
    }
}
