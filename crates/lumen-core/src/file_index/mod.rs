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
//! # One or two letters search what you use
//!
//! In root a query of one or two letters is nearly every name's first byte,
//! and a scan of all of them -- 4.4 ms at 350 000, measured in slice 1a --
//! is spent on rows nobody wants: with `a` typed, a file never opened that
//! happens to start with *a* is noise. So root asks such a query only of the
//! entries with frecency and those whose whole name is that short
//! ([`FileIndex::root_query`]); every other name is reached by its first
//! three letters. `docs/FILE-SEARCH.md`, "One or two letters search what you
//! use".
//!
//! # Never where anything waits
//!
//! Building walks the disk and querying reads the database, so both run off
//! the UI thread, and root search never waits for either (I-3).
//!
//! # One build at a time
//!
//! Every build of an index file holds that file's lock from the walk to the
//! load ([`FileIndex::rebuild`]), so two never write the database at once,
//! and one asked for while another walks waits for it. The one waited for
//! sees it was superseded and writes nothing.
//!
//! # Kept current without walking everything
//!
//! Slice 2. A change the watcher reports ([`watch`]) is applied to the
//! folder it happened in, not to the scope: that one folder is read again
//! by the same rules, what left it is removed with everything under it,
//! what arrived is added with everything under it ([`update`]). At start,
//! nothing was watched while the daemon was not running, so every indexed
//! folder's time is compared with the one recorded and only the folders
//! whose time moved are read again ([`FileIndex::reconcile`]). A full walk
//! is for a store that cannot be used, rules that changed, and *Rebuild
//! File Index*.

pub mod names;
pub mod store;
mod update;
pub mod walk;
pub mod watch;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use anyhow::Result;
use parking_lot::{Mutex, RwLock};

pub use names::{Kinds, MIN_SUBSTRING, Match, Names};
pub use store::{Meta, Row, Store, digest};
pub use walk::{CEILING, Entry, WalkRules, Walked, default_skips, walk};
pub use watch::{FileWatch, watch_targets};

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

/// What applying changes to an index gave.
pub struct Updated {
    /// The index with the changes, when an entry was added or removed;
    /// `None` when none was, and the index installed is still right.
    pub index: Option<FileIndex>,
    /// How many folders' times were compared with the disk: every indexed
    /// one for a reconcile, none for a batch from the watcher.
    pub checked: usize,
    /// How many folders were read again.
    pub reread: usize,
    /// A scope's own folder was read again, so the folders directly in it --
    /// the trees the watcher watches -- may not be the ones it watches.
    pub retarget: bool,
}

/// The index as the settings page shows it (slice 4): sent from the
/// launcher, which holds the index, to the page, which holds none of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexStatus {
    /// Whether an index is installed at all; the rest is empty until one is.
    pub loaded: bool,
    pub entries: usize,
    /// As [`FileIndex::per_scope`], in the order of the scopes walked.
    pub per_scope: Vec<usize>,
    /// As [`FileIndex::truncated`].
    pub truncated: Option<PathBuf>,
    /// As [`FileIndex::walked_at`].
    pub walked_at: Option<u64>,
    /// A load, reconcile or walk is running.
    pub indexing: bool,
}

/// The index, loaded and ready to query.
pub struct FileIndex {
    names: Names,
    /// Behind a lock because SQLite's connection is not `Sync`, and a query
    /// arrives from whichever thread asks.
    store: Mutex<Store>,
    /// Frecency by entry, for the few entries that have any: looked up for
    /// every match, so it is a map, not a call into [`Frecency`] by path.
    /// Behind a lock so a use can update it while the index is shared with
    /// the scans in flight; written only by [`Self::set_boosts`], briefly.
    boosts: RwLock<HashMap<u32, u32>>,
    /// The entries whose whole name is one or two characters, found once at
    /// load: with the boosted ones, all a short query searches.
    short: Vec<u32>,
    meta: Meta,
    /// Resolved once rather than per row: every subtitle is shortened
    /// against it.
    home: Option<PathBuf>,
}

/// The lock of every index file a build has opened, by path.
static BUILDS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = LazyLock::new(Default::default);

/// The lock every build of the index at `path` holds while it walks, writes
/// and loads (see "One build at a time" above).
fn build_lock(path: &Path) -> Arc<Mutex<()>> {
    BUILDS.lock().entry(path.to_path_buf()).or_default().clone()
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
        Ok(Self::from_parts(names, meta, store))
    }

    fn from_parts(names: Names, meta: Meta, store: Store) -> FileIndex {
        // The names are in memory now; the pages that held them are not
        // needed again.
        store.release_memory();
        Self {
            short: names.short_entries(),
            names,
            store: Mutex::new(store),
            boosts: RwLock::new(HashMap::new()),
            meta,
            home: directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()),
        }
    }

    /// Walk `rules` and make the result the whole index at `path`. Seconds to
    /// minutes, depending on the scopes.
    pub fn build(path: &Path, rules: &WalkRules) -> Result<FileIndex> {
        Self::rebuild(path, rules, None, &|| false)
            .map(|built| built.expect("a build nothing supersedes finishes"))
    }

    /// Walk `rules` and make the result the whole index at `path`, written
    /// through `current` when one is installed (see [`Self::rebuilt`]).
    ///
    /// Holds the file's build lock throughout (see "One build at a time"
    /// above). `None` when `superseded` says so -- asked before the walk,
    /// after it and after the load -- and once it has, nothing more is
    /// written: a newer build is waiting for the lock, and walks with newer
    /// rules.
    pub fn rebuild(
        path: &Path,
        rules: &WalkRules,
        current: Option<&FileIndex>,
        superseded: &dyn Fn() -> bool,
    ) -> Result<Option<FileIndex>> {
        let lock = build_lock(path);
        let _building = lock.lock();
        if superseded() {
            return Ok(None);
        }
        let started = std::time::Instant::now();
        let walked = walk(rules);
        log::info!(
            "walked the file index's scopes in {} ms",
            started.elapsed().as_millis()
        );
        if superseded() {
            return Ok(None);
        }
        let digest = digest(rules);
        let index = match current {
            Some(current) => current.rebuilt(walked, &digest, path)?,
            None => {
                let mut store = Store::open(path)?;
                store.replace(&walked, &digest)?;
                // The walk's entries are on disk now: given back before the
                // names are loaded, not after, so the two are never held at
                // once.
                drop(walked);
                Self::load(store)?
            }
        };
        Ok((!superseded()).then_some(index))
    }

    /// Apply the paths the watcher reported changed, in the store at `path`,
    /// as this index's rules `rules` allow (see "Kept current without
    /// walking everything" above).
    ///
    /// Under the build lock, so it waits for a build and a build for it;
    /// `None` when `superseded` says so -- a rebuild was asked for, and
    /// covers these changes. An empty path among `changed` says the watcher
    /// lost track of what changed, and every folder is compared instead, as
    /// at start.
    pub fn update(
        &self,
        path: &Path,
        rules: &WalkRules,
        changed: &[PathBuf],
        superseded: &dyn Fn() -> bool,
    ) -> Result<Option<Updated>> {
        let lock = build_lock(path);
        let _building = lock.lock();
        if superseded() {
            return Ok(None);
        }
        let recorded = self.store.lock().meta()?;
        // Rules other than the index's are a rebuild, already asked for.
        if recorded.digest != digest(rules) {
            return Ok(None);
        }
        let (checked, folders) = match update::folders(changed) {
            Some(folders) => (0, folders),
            None => {
                // The folders' times are read with the store unlocked:
                // seconds for a large index, and queries need not wait.
                let stored = self.store.lock().folders()?;
                update::moved(rules, &recorded, stored)
            }
        };
        if superseded() {
            return Ok(None);
        }

        // Through this index's own connection, as a rebuild writes: a
        // query in flight waits for the batch rather than meeting half of
        // it (see `Self::rebuilt`).
        let mut store = self.store.lock();
        // Patched only when the names are the store's as it is; when
        // another write came between, they are loaded from it.
        let current = recorded.version == self.meta.version;
        let mut count = if current {
            self.names.len()
        } else {
            store.count()?
        };
        let mut meta = recorded;
        let batch = store.batch()?;
        let delta = update::apply(&batch, rules, folders, &mut meta, &mut count)?;
        if delta.changed() {
            meta.version += 1;
        }
        batch.finish(&meta)?;
        let mut updated = Updated {
            index: None,
            checked,
            reread: delta.reread,
            retarget: delta.retarget,
        };
        if !delta.changed() {
            store.release_memory();
            return Ok(Some(updated));
        }

        let names = if current {
            self.names.updated(&delta.removed, &delta.added)
        } else {
            store.load()?.0
        };
        drop(delta);
        store.release_memory();
        drop(store);
        updated.index = Some(Self::from_parts(names, meta, Store::open(path)?));
        Ok((!superseded()).then_some(updated))
    }

    /// Bring the index at `path` up to date with the disk after a time
    /// nothing watched it: every indexed folder whose time moved since it
    /// was recorded is read again, and nothing else is.
    pub fn reconcile(
        &self,
        path: &Path,
        rules: &WalkRules,
        superseded: &dyn Fn() -> bool,
    ) -> Result<Option<Updated>> {
        self.update(path, rules, &[PathBuf::new()], superseded)
    }

    /// Make `walked` the whole index in this index's own store, then load it
    /// from the database at `path` -- the same file -- as a new index.
    ///
    /// Through this index's lock rather than a second connection, so a
    /// query of this index in flight waits for the write instead of meeting
    /// a database half replaced.
    ///
    /// This index stays installed until the new one takes its place, and the
    /// rows its names point at have moved under it: entry 7 may now be
    /// another file. So a query never trusts a row by its id alone -- every
    /// row is checked against the name and kind this index matched, and one
    /// that is not that entry is left out ([`Self::query`]). Until the swap
    /// an old index can answer less than it should, never a wrong file.
    /// Called only under the build lock, by [`Self::rebuild`].
    fn rebuilt(&self, walked: Walked, digest: &str, path: &Path) -> Result<FileIndex> {
        self.store.lock().replace(&walked, digest)?;
        drop(walked);
        Self::load(Store::open(path)?)
    }

    #[cfg(test)]
    fn with_home(mut self, home: &Path) -> Self {
        self.home = Some(home.to_path_buf());
        self
    }

    /// What root search shows for `query`: [`Self::query_known`] for one or
    /// two characters, [`Self::query`] for more. The one place that rule is
    /// made (see "One or two letters search what you use" above).
    pub fn root_query(
        &self,
        query: &str,
        kinds: Kinds,
        limit: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Vec<FileHit> {
        if query.trim().chars().count() < MIN_SUBSTRING {
            self.query_known(query, kinds, limit)
        } else {
            self.query(query, kinds, limit, cancelled)
        }
    }

    /// The best `limit` files and folders for `query`, best first, from the
    /// whole index.
    ///
    /// Ranked by how well the name matched, then frecency, then the shorter
    /// name ([`Names::find`]). Answers nothing once `cancelled` says so: the
    /// query was superseded. A database that cannot be read answers nothing
    /// rather than an error: the launcher has nothing to tell the user
    /// mid-keystroke, and the next rebuild replaces it. A row that is no
    /// longer the entry matched is left out (see [`Self::rebuilt`]).
    pub fn query(
        &self,
        query: &str,
        kinds: Kinds,
        limit: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Vec<FileHit> {
        self.query_where(query, kinds, limit, &|_, _| true, cancelled)
    }

    /// [`Self::query`] over the entries `accept` takes, asked with each
    /// matched name -- lower-cased -- and whether it is a folder: what
    /// *Search Files*' type filter searches (F16). Applied inside the scan,
    /// so a filtered query finds its own best `limit`.
    pub fn query_where(
        &self,
        query: &str,
        kinds: Kinds,
        limit: usize,
        accept: &dyn Fn(&str, bool) -> bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Vec<FileHit> {
        let boosts = self.boosts.read();
        let found = self.names.find_where(
            query,
            kinds,
            limit,
            accept,
            |entry| boosts.get(&entry).copied().unwrap_or(0),
            cancelled,
        );
        drop(boosts);
        if cancelled() {
            return Vec::new();
        }
        self.hits(query, &found)
    }

    /// [`Self::query`] over the entries used from Lumen and those whose
    /// whole name is one or two characters: what a short query searches in
    /// root. A few hundred names, looked at one by one, so it is not
    /// cancelled.
    pub fn query_known(&self, query: &str, kinds: Kinds, limit: usize) -> Vec<FileHit> {
        let boosts = self.boosts.read();
        let mut known = boosts
            .keys()
            .copied()
            .chain(self.short.iter().copied())
            .collect::<Vec<_>>();
        known.sort_unstable();
        known.dedup();
        let found = self.names.find_among(query, kinds, limit, &known, |entry| {
            boosts.get(&entry).copied().unwrap_or(0)
        });
        drop(boosts);
        self.hits(query, &found)
    }

    /// The rows of `found`, in its order, as hits for `query` -- each only if
    /// it is still the entry that was matched (see [`Self::rebuilt`]).
    fn hits(&self, query: &str, found: &[Match]) -> Vec<FileHit> {
        if found.is_empty() {
            return Vec::new();
        }
        let ids = found
            .iter()
            .map(|found| self.names.row(found.entry))
            .collect::<Vec<_>>();
        let rows = match self.store.lock().rows(&ids) {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("could not read the file index: {error:#}");
                return Vec::new();
            }
        };

        let words = names::words(query);
        rows.into_iter()
            .filter(|row| self.holds(row.id, &row.name, row.is_dir))
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
    pub fn set_boosts(&self, frecency: &Frecency) {
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
            let path = Path::new(path);
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default();
            // Checked as a query's rows are, so a row that moved under this
            // index boosts nothing rather than another entry.
            match store.id_of(path) {
                Ok(Some(row)) if self.holds(row, &name, is_dir) => {
                    if let Some(entry) = self.names.entry_of(row) {
                        boosts.insert(entry, boost);
                    }
                }
                Ok(_) => {}
                Err(error) => log::warn!("could not read the file index: {error:#}"),
            }
        }
        store.release_memory();
        drop(store);
        *self.boosts.write() = boosts;
    }

    /// Whether the store's row `row` is still the entry these names matched:
    /// one of their rows, with the same name and kind. The one check that
    /// keeps a row which moved under the index from answering for it.
    fn holds(&self, row: u32, name: &str, is_dir: bool) -> bool {
        self.names
            .entry_of(row)
            .is_some_and(|entry| self.names.holds(entry, name, is_dir))
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

    /// When the last full walk was written, in seconds since the epoch.
    pub fn walked_at(&self) -> Option<u64> {
        self.meta.walked_at
    }

    /// What the settings page says about the index; `indexing` is the
    /// host's, which knows what it has started.
    pub fn status(&self, indexing: bool) -> IndexStatus {
        IndexStatus {
            loaded: true,
            entries: self.len(),
            per_scope: self.per_scope().to_vec(),
            truncated: self.truncated().map(Path::to_path_buf),
            walked_at: self.walked_at(),
            indexing,
        }
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// What the index holds in memory: the names, the boosts and the short
    /// names.
    pub fn memory_bytes(&self) -> usize {
        self.names.memory_bytes() + self.boosts.read().capacity() * 8 + self.short.capacity() * 4
    }
}

/// The characters of `title` each word matched, where it matched best.
///
/// The title is folded a character at a time, as the names are, remembering
/// which character each folded byte came from: lower-casing can change how
/// many there are (`İ` is two), so an offset in the folded name is not one
/// in the title.
pub(crate) fn highlights(title: &str, words: &[String]) -> Vec<u32> {
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
        FileIndex::build(&tree.path(".files.db"), &tree.rules())
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

        let hits = index.query("inv", Kinds::FoldersAndFiles, 8, &|| false);
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

        let hits = index.query("proj", Kinds::Folders, 8, &|| false);
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

        let hits = index.query("2024 inv", Kinds::FoldersAndFiles, 8, &|| false);
        assert_eq!(hits[0].highlights, [0, 1, 2, 8, 9, 10, 11]);
    }

    /// Highlights count characters of the title as shown, even where
    /// lower-casing changes how many there are: `İ` is two.
    #[test]
    fn highlights_are_characters_of_the_real_title() {
        let tree = Tree::new("index-unicode");
        tree.file("İstanbul.txt");
        let index = index(&tree);

        let hits = index.query("stan", Kinds::FoldersAndFiles, 8, &|| false);
        assert_eq!(hits[0].highlights, [1, 2, 3, 4]);
    }

    #[test]
    fn frecency_reorders_matches_of_the_same_tier() {
        let tree = Tree::new("index-boost");
        tree.file("report-a.txt").file("report-b.txt");
        let index = index(&tree);

        assert_eq!(
            titles(&index.query("report", Kinds::FoldersAndFiles, 8, &|| false)),
            ["report-a.txt", "report-b.txt"]
        );

        let mut frecency = Frecency::new();
        frecency.record(&format!("file:{}", tree.path("report-b.txt").display()));
        frecency.record("app:unrelated");
        frecency.record(&format!("file:{}", tree.path("gone.txt").display()));
        index.set_boosts(&frecency);

        assert_eq!(
            titles(&index.query("report", Kinds::FoldersAndFiles, 8, &|| false)),
            ["report-b.txt", "report-a.txt"]
        );
    }

    /// One or two letters search what was opened from Lumen, and names that
    /// short; three or more search everything.
    #[test]
    fn root_searches_one_or_two_letters_among_what_is_used() {
        let tree = Tree::new("index-short");
        tree.file("apple.txt").file("ab").file("arch.md");
        let index = index(&tree);
        let root = |query: &str| {
            titles(&index.root_query(query, Kinds::FoldersAndFiles, 8, &|| false))
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        };

        assert_eq!(root("a"), ["ab"]);
        assert_eq!(root(" ab "), ["ab"]);
        assert_eq!(root("app"), ["apple.txt"]);

        let mut frecency = Frecency::new();
        frecency.record(&format!("file:{}", tree.path("arch.md").display()));
        index.set_boosts(&frecency);

        assert_eq!(root("a"), ["arch.md", "ab"]);
        assert_eq!(root("ar"), ["arch.md"]);
        // The whole index still answers a short query when asked directly,
        // as the view will.
        assert_eq!(
            titles(&index.query("a", Kinds::FoldersAndFiles, 8, &|| false)),
            ["arch.md", "ab", "apple.txt"]
        );
    }

    #[test]
    fn a_cancelled_query_answers_nothing() {
        let tree = Tree::new("index-cancel");
        tree.file("report.txt");
        let index = index(&tree);

        assert!(
            index
                .root_query("report", Kinds::FoldersAndFiles, 8, &|| true)
                .is_empty()
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
            titles(&loaded.query("plan", Kinds::FoldersAndFiles, 8, &|| false)),
            ["plan.md"]
        );
    }

    /// A rebuild writes through the index in place and gives back the new
    /// one, which finds what was added since.
    #[test]
    fn an_index_is_rebuilt_through_its_own_store() {
        let tree = Tree::new("index-rebuilt");
        tree.file("notes.md");
        let old = index(&tree);
        tree.file("plan.md");

        let rules = tree.rules();
        let new = old
            .rebuilt(walk(&rules), &digest(&rules), &tree.path(".files.db"))
            .unwrap()
            .with_home(&tree.0);

        assert_eq!(new.len(), 2);
        assert_eq!(
            titles(&new.query("plan", Kinds::FoldersAndFiles, 8, &|| false)),
            ["plan.md"]
        );
    }

    /// A rebuild rewrites the database under the index still installed, so
    /// until the new one takes over, an entry's row can be another file. The
    /// old index then answers less, never the wrong file.
    #[test]
    fn an_index_never_answers_a_row_that_moved_under_it() {
        let tree = Tree::new("index-moved");
        let path = tree.path(".files.db");
        let walked = |entries: &[(&str, bool)]| Walked {
            entries: entries
                .iter()
                .map(|&(name, is_dir)| Entry {
                    path: tree.0.join(name),
                    is_dir,
                    modified: None,
                })
                .collect(),
            per_scope: vec![entries.len()],
            truncated: None,
            roots: vec![None],
        };
        let mut store = Store::open(&path).unwrap();
        store
            .replace(
                &walked(&[
                    ("alpha.txt", false),
                    ("beta.txt", false),
                    ("reports", true),
                    ("same.md", false),
                    ("ab", false),
                ]),
                "old",
            )
            .unwrap();
        let old = FileIndex::load(store).unwrap();

        let new = old
            .rebuilt(
                walked(&[
                    ("beta.txt", false),
                    ("alpha.txt", false),
                    ("reports", false),
                    ("same.md", false),
                    ("ac", false),
                ]),
                "new",
                &path,
            )
            .unwrap();

        let all = Kinds::FoldersAndFiles;
        assert!(old.query("alpha", all, 8, &|| false).is_empty());
        assert!(old.query("beta", all, 8, &|| false).is_empty());
        // The same name, but now a file: not the folder that was matched.
        assert!(
            old.query("reports", Kinds::Folders, 8, &|| false)
                .is_empty()
        );
        assert!(old.query_known("ab", all, 8).is_empty());
        // A row that is still its entry is still found.
        let same = old.query("same", all, 8, &|| false);
        assert_eq!(titles(&same), ["same.md"]);
        assert_eq!(
            same[0].item.id,
            format!("file:{}", tree.path("same.md").display())
        );

        let alpha = new.query("alpha", all, 8, &|| false);
        assert_eq!(titles(&alpha), ["alpha.txt"]);
        assert_eq!(
            alpha[0].item.id,
            format!("file:{}", tree.path("alpha.txt").display())
        );
    }

    /// A build asked for while another holds the file waits for it, rather
    /// than writing the database at the same time.
    #[test]
    fn a_build_waits_for_the_one_before_it() {
        let tree = Tree::new("index-serial");
        tree.file("notes.md");
        let path = tree.path(".files.db");
        let rules = tree.rules();

        let lock = build_lock(&path);
        let held = lock.lock();
        let builder = std::thread::spawn({
            let path = path.clone();
            move || {
                FileIndex::rebuild(&path, &rules, None, &|| false)
                    .unwrap()
                    .map(|index| index.len())
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!builder.is_finished());

        drop(held);
        assert_eq!(builder.join().unwrap(), Some(1));
    }

    /// Superseded while it walked, a build writes nothing: the newer one
    /// waiting for the lock writes its own walk.
    #[test]
    fn a_superseded_build_writes_nothing() {
        let tree = Tree::new("index-superseded");
        tree.file("notes.md");
        let path = tree.path(".files.db");
        let old = index(&tree);
        tree.file("plan.md");
        let rules = tree.rules();

        assert!(
            FileIndex::rebuild(&path, &rules, Some(&old), &|| true)
                .unwrap()
                .is_none()
        );
        let asked = std::cell::Cell::new(0);
        let after_the_walk = || {
            asked.set(asked.get() + 1);
            asked.get() > 1
        };
        assert!(
            FileIndex::rebuild(&path, &rules, Some(&old), &after_the_walk)
                .unwrap()
                .is_none()
        );
        assert_eq!(asked.get(), 2);

        let stored = FileIndex::load(Store::open(&path).unwrap()).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(
            titles(&old.query("notes", Kinds::FoldersAndFiles, 8, &|| false)),
            ["notes.md"]
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
        let index = FileIndex::build(&first.path(".files.db"), &rules).unwrap();

        assert_eq!(index.len(), 3);
        assert_eq!(index.per_scope(), [2, 1]);
        assert_eq!(index.truncated(), Some(second.0.as_path()));
    }

    /// A scope with the database beside it rather than in it, so writing
    /// the database moves no time the reconcile compares.
    struct Scope {
        tree: Tree,
        rules: WalkRules,
    }

    impl Scope {
        fn new(name: &str) -> Self {
            let tree = Tree::new(name);
            tree.dir("home");
            let rules = rules(&[tree.path("home")]);
            Self { tree, rules }
        }

        fn file(&self, relative: &str) -> &Self {
            self.tree.file(&format!("home/{relative}"));
            self
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.tree.path("home").join(relative)
        }

        fn database(&self) -> PathBuf {
            self.tree.path("files.db")
        }

        fn build(&self) -> FileIndex {
            FileIndex::build(&self.database(), &self.rules).unwrap()
        }

        /// `index` with the changes at `changed` applied: the new index,
        /// or `index` itself when nothing changed.
        fn update(&self, index: FileIndex, changed: &[PathBuf]) -> FileIndex {
            index
                .update(&self.database(), &self.rules, changed, &|| false)
                .unwrap()
                .expect("nothing supersedes it")
                .index
                .unwrap_or(index)
        }
    }

    /// What `query` finds, as paths relative to the scope, `/`-separated.
    fn found(index: &FileIndex, scope: &Scope, query: &str) -> Vec<String> {
        index
            .query(query, Kinds::FoldersAndFiles, 64, &|| false)
            .iter()
            .map(|hit| {
                let (_, path) = hit.item.id.split_once(':').unwrap();
                Path::new(path)
                    .strip_prefix(scope.path(""))
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }

    #[test]
    fn a_file_created_is_added() {
        let scope = Scope::new("update-add");
        scope.file("projects/notes.md");
        let index = scope.build();

        scope.file("projects/plan.md");
        let index = scope.update(index, &[scope.path("projects/plan.md")]);

        assert_eq!(found(&index, &scope, "plan"), ["projects/plan.md"]);
        assert_eq!(index.len(), 3);
        assert_eq!(index.per_scope(), [3]);
    }

    #[test]
    fn a_file_deleted_is_removed() {
        let scope = Scope::new("update-delete");
        scope.file("notes.md").file("plan.md");
        let index = scope.build();

        std::fs::remove_file(scope.path("plan.md")).unwrap();
        let index = scope.update(index, &[scope.path("plan.md")]);

        assert!(found(&index, &scope, "plan").is_empty());
        assert_eq!(found(&index, &scope, "notes"), ["notes.md"]);
        assert_eq!(index.per_scope(), [1]);
    }

    /// The watcher may report only the folder; everything under it goes.
    #[test]
    fn a_folder_deleted_takes_its_subtree_with_it() {
        let scope = Scope::new("update-delete-folder");
        scope
            .file("keep.txt")
            .file("old/one.txt")
            .file("old/deeper/two.txt")
            .file("older/three.txt");
        let index = scope.build();

        std::fs::remove_dir_all(scope.path("old")).unwrap();
        let index = scope.update(index, &[scope.path("old")]);

        assert!(found(&index, &scope, "one").is_empty());
        assert!(found(&index, &scope, "two").is_empty());
        assert!(found(&index, &scope, "deeper").is_empty());
        // A sibling whose name starts with the deleted one's is not under it.
        assert_eq!(found(&index, &scope, "three"), ["older/three.txt"]);
        assert_eq!(index.len(), 3);
    }

    /// A rename is a path that left and one that arrived; a folder's
    /// contents follow it.
    #[test]
    fn a_rename_is_followed() {
        let scope = Scope::new("update-rename");
        scope.file("draft.md").file("work/inner/report.txt");
        let index = scope.build();

        std::fs::rename(scope.path("draft.md"), scope.path("final.md")).unwrap();
        std::fs::rename(scope.path("work"), scope.path("archive")).unwrap();
        let index = scope.update(
            index,
            &[
                scope.path("draft.md"),
                scope.path("final.md"),
                scope.path("work"),
                scope.path("archive"),
            ],
        );

        assert!(found(&index, &scope, "draft").is_empty());
        assert_eq!(found(&index, &scope, "final"), ["final.md"]);
        assert_eq!(
            found(&index, &scope, "report"),
            ["archive/inner/report.txt"]
        );
        assert!(found(&index, &scope, "work").is_empty());
        assert_eq!(index.len(), 4);
    }

    /// A folder moved in with files already in it arrives whole.
    #[test]
    fn a_folder_that_arrives_is_walked() {
        let scope = Scope::new("update-arrive");
        scope.file("notes.md");
        let index = scope.build();
        let outside = scope.tree.path("outside");
        scope.tree.file("outside/a/b/deep.txt");

        std::fs::rename(&outside, scope.path("moved")).unwrap();
        let index = scope.update(index, &[scope.path("moved")]);

        assert_eq!(found(&index, &scope, "deep"), ["moved/a/b/deep.txt"]);
        assert_eq!(index.len(), 5);
    }

    /// What the rules leave out stays out when it changes: inside a skipped
    /// folder, an excluded one, a hidden one, or matched by a pattern or an
    /// ignore file.
    #[test]
    fn a_change_the_rules_leave_out_changes_nothing() {
        let mut scope = Scope::new("update-skipped");
        scope
            .file("app/node_modules/pkg/index.js")
            .file("Videos/clip.mp4")
            .file(".config/app.toml")
            .file("docs/.lumenignore")
            .file("docs/readme.md");
        std::fs::write(scope.path("docs/.lumenignore"), "*.tmp\n").unwrap();
        scope.rules.exclusions = vec![scope.path("Videos")];
        scope.rules.ignore_patterns = vec!["*.log".into()];
        let index = scope.build();
        let before = index.len();

        let changed = [
            "app/node_modules/pkg/new.js",
            "app/node_modules/fresh/index.js",
            "Videos/new.mp4",
            ".config/new.toml",
            ".secret",
            "debug.log",
            "docs/scratch.tmp",
        ];
        for path in changed {
            scope.file(path);
        }
        let paths = changed.map(|path| scope.path(path));
        let updated = index
            .update(&scope.database(), &scope.rules, &paths, &|| false)
            .unwrap()
            .unwrap();

        assert!(updated.index.is_none());
        assert_eq!(index.len(), before);
    }

    /// A scope under a folder the outer scope skips owns the changes inside
    /// it: they are applied, and to it, while `AppData` around it stays out.
    #[test]
    fn a_change_in_a_scope_under_a_skipped_folder_is_applied_to_it() {
        let mut scope = Scope::new("update-nested-skipped");
        scope
            .file("top.txt")
            .file("AppData/Local/cache.db")
            .file("AppData/config/settings.json");
        scope.rules.scopes.push(scope.path("AppData/config"));
        let index = scope.build();
        assert_eq!(
            found(&index, &scope, "settings"),
            ["AppData/config/settings.json"]
        );
        assert_eq!(index.per_scope(), [1, 1]);

        scope.file("AppData/config/themes/dark.json");
        scope.file("AppData/Local/noise.db");
        let index = scope.update(
            index,
            &[
                scope.path("AppData/config/themes"),
                scope.path("AppData/Local/noise.db"),
            ],
        );

        assert_eq!(
            found(&index, &scope, "dark"),
            ["AppData/config/themes/dark.json"]
        );
        assert!(found(&index, &scope, "noise").is_empty());
        assert_eq!(index.per_scope(), [1, 3]);
    }

    /// An index made before a batch never answers with a row the batch
    /// removed or gave to another entry -- the row ids of what is gone are
    /// never reused -- and the new index answers with the right paths.
    #[test]
    fn no_index_answers_a_wrong_file_after_an_update() {
        let scope = Scope::new("update-wrong");
        scope.file("alpha.txt").file("beta.txt");
        let old = scope.build();

        std::fs::remove_file(scope.path("alpha.txt")).unwrap();
        scope.file("alpha-2.txt").file("gamma.txt");
        let new = old
            .update(
                &scope.database(),
                &scope.rules,
                &[
                    scope.path("alpha.txt"),
                    scope.path("alpha-2.txt"),
                    scope.path("gamma.txt"),
                ],
                &|| false,
            )
            .unwrap()
            .unwrap()
            .index
            .unwrap();

        assert!(found(&old, &scope, "alpha").is_empty());
        assert!(found(&old, &scope, "gamma").is_empty());
        assert_eq!(found(&old, &scope, "beta"), ["beta.txt"]);
        assert_eq!(found(&new, &scope, "alpha"), ["alpha-2.txt"]);
        assert_eq!(found(&new, &scope, "gamma"), ["gamma.txt"]);
        assert_eq!(found(&new, &scope, "beta"), ["beta.txt"]);

        // A second batch, applied to the index the first made.
        std::fs::remove_file(scope.path("beta.txt")).unwrap();
        let newer = scope.update(new, &[scope.path("beta.txt")]);
        assert!(found(&newer, &scope, "beta").is_empty());
        assert_eq!(found(&newer, &scope, "gamma"), ["gamma.txt"]);

        // And loaded back from the store, the same.
        let loaded = FileIndex::load(Store::open(&scope.database()).unwrap()).unwrap();
        assert_eq!(loaded.len(), newer.len());
        assert_eq!(found(&loaded, &scope, "alpha"), ["alpha-2.txt"]);
    }

    /// An index the store moved on from -- another write came between -- is
    /// not patched; its update loads the names from the store.
    #[test]
    fn an_update_to_an_index_the_store_moved_on_from_loads_it() {
        let scope = Scope::new("update-stale");
        scope.file("notes.md");
        let stale = scope.build();
        scope.file("plan.md");
        drop(scope.build());

        scope.file("todo.md");
        let index = scope.update(stale, &[scope.path("todo.md")]);

        assert_eq!(found(&index, &scope, "plan"), ["plan.md"]);
        assert_eq!(found(&index, &scope, "todo"), ["todo.md"]);
        assert_eq!(index.len(), 3);
    }

    #[test]
    fn the_ceiling_holds_under_updates() {
        let mut scope = Scope::new("update-ceiling");
        scope.file("a.txt").file("b.txt");
        scope.rules.ceiling = 3;
        let index = scope.build();
        assert_eq!(index.truncated(), None);

        scope.file("c.txt").file("d.txt").file("e.txt");
        let index = scope.update(
            index,
            &[
                scope.path("c.txt"),
                scope.path("d.txt"),
                scope.path("e.txt"),
            ],
        );

        assert_eq!(index.len(), 3);
        assert_eq!(index.truncated(), Some(scope.path("").as_path()));
    }

    /// At start nothing was watched while the daemon was not running: only
    /// the folders whose time moved are read again, and what arrived in
    /// them is found.
    #[test]
    fn the_reconcile_reads_again_only_the_folders_that_changed() {
        let scope = Scope::new("reconcile");
        scope
            .file("a/one.txt")
            .file("b/two.txt")
            .file("c/three.txt")
            .file("c/d/four.txt");
        drop(scope.build());

        scope.file("b/added.txt");
        std::fs::remove_file(scope.path("c/d/four.txt")).unwrap();
        let stored = FileIndex::load(Store::open(&scope.database()).unwrap()).unwrap();
        let reconciled = stored
            .reconcile(&scope.database(), &scope.rules, &|| false)
            .unwrap()
            .unwrap();

        // The scope and its four folders.
        assert_eq!(reconciled.checked, 5);
        assert_eq!(reconciled.reread, 2);
        let index = reconciled.index.unwrap();
        assert_eq!(found(&index, &scope, "added"), ["b/added.txt"]);
        assert!(found(&index, &scope, "four").is_empty());

        // Read again, the folders' times are recorded: a second reconcile
        // reads nothing.
        let again = index
            .reconcile(&scope.database(), &scope.rules, &|| false)
            .unwrap()
            .unwrap();
        assert_eq!(again.reread, 0);
        assert!(again.index.is_none());
    }

    /// A batch from a build that has since been superseded writes nothing:
    /// the rebuild asked for covers it.
    #[test]
    fn a_superseded_update_writes_nothing() {
        let scope = Scope::new("update-superseded");
        scope.file("notes.md");
        let index = scope.build();
        scope.file("plan.md");

        assert!(
            index
                .update(
                    &scope.database(),
                    &scope.rules,
                    &[scope.path("plan.md")],
                    &|| true
                )
                .unwrap()
                .is_none()
        );
        let stored = FileIndex::load(Store::open(&scope.database()).unwrap()).unwrap();
        assert_eq!(stored.len(), 1);
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
