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
//! - [`Names`] holds the names of a hot layer of them, folded into one blob,
//!   and answers a query by scanning it.
//!
//! A query asks [`Names`] for the best few entries -- and the store's word
//! index, when the hot layer is not the whole index -- then the store for
//! just those rows (see [`FileIndex`], "A hot layer, then the disk").
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
mod tokens;
mod update;
pub mod walk;
pub mod watch;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use anyhow::Result;
use parking_lot::{Mutex, RwLock};

pub use names::{Kinds, MIN_SUBSTRING, Match, Names};
pub use store::{Hot, IndexDetail, Meta, RankedRow, Row, Store, digest};
pub use walk::{Entry, WalkRules, Walked, default_skips, walk};
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
    /// As [`FileIndex::walked_at`].
    pub walked_at: Option<u64>,
    /// A load, reconcile or walk is running.
    pub indexing: bool,
}

/// The most the hot layer's names may cost in memory, as [`Names::cost`]
/// counts them (`docs/FILE-SEARCH.md`, "Step c3a").
///
/// Idle memory has been measured at 71--75 MB from F8 to F15, against a
/// budget of 100. Eight megabytes of names keeps the launcher near 83 with
/// any index, leaving the rest for SQLite's page cache, the boosts and what
/// later phases add -- where the old ceiling's 500 000 names took 14.4. A
/// home folder, 55 000 entries in about 1.7 MB, fits whole, so for most
/// users the hot layer is the index and root answers exactly as before.
/// Past it, about a quarter of a million entries are in memory -- the used,
/// the folders and the shallowest files -- and the rest are asked of the
/// disk by their words.
pub const HOT_BUDGET: usize = 8 * 1024 * 1024;

/// The index, loaded and ready to query.
///
/// # A hot layer, then the disk
///
/// Only part of the index is in memory: the hot layer, selected at load
/// within [`HOT_BUDGET`] ([`Store::hot`]). Root asks it first, and a match
/// anywhere in a name counts there. From three characters it asks the
/// disk's word index too ([`Store::ranked`]) -- unless the hot layer is the
/// whole index -- and the two are merged into the order the names rank in.
/// What is only on disk is found by the start of one of its words, never
/// inside one: the trade "Decision: B" chose over losing entries. The
/// trade is root's alone. *Search Files* asks the disk for the rest
/// afterwards ([`Self::query_inside`]), so the view finds every entry a
/// match inside a word would.
pub struct FileIndex {
    /// The hot layer.
    hot: Names,
    /// Whether the hot layer is every entry, so the disk has nothing more
    /// to answer.
    hot_is_complete: bool,
    /// What the hot layer was selected within: [`HOT_BUDGET`], but in tests.
    budget: usize,
    /// How many entries the index holds, in memory or not.
    entries: usize,
    /// Behind a lock because SQLite's connection is not `Sync`, and a query
    /// arrives from whichever thread asks.
    store: Mutex<Store>,
    /// What frecency knows of the index. Behind a lock so a use can update
    /// it while the index is shared with the scans in flight; written only
    /// by [`Self::set_boosts`], briefly.
    used: RwLock<Used>,
    /// The hot entries whose whole name is one or two characters, found
    /// once at load: with the used ones, all a short query searches. Every
    /// such name is in the hot layer ([`Store::hot`]).
    short: Vec<u32>,
    meta: Meta,
    /// Resolved once rather than per row: every subtitle is shortened
    /// against it.
    home: Option<PathBuf>,
}

/// Frecency's part of the index (see [`FileIndex::set_boosts`]).
#[derive(Default)]
struct Used {
    /// Frecency by row, for the few entries that have any: looked up for
    /// every match, so it is a map, not a call into [`Frecency`] by path.
    boosts: HashMap<u32, u32>,
    /// The names of the used entries the hot layer was selected without --
    /// used since it was -- so they are found as the hot layer's are: by a
    /// short query, and inside a word.
    outside: Names,
}

/// Where a query's match came from, until it is read as a row.
enum Source {
    /// An entry of the hot layer.
    Hot(u32),
    /// An entry of [`Used::outside`].
    Outside(u32),
    /// A row the disk answered, already read.
    Disk(RankedRow),
}

/// One match and what it is ranked by: as [`Names`] ranks, the better tier,
/// then frecency, then the shorter name, then the row.
struct Candidate {
    tier: u8,
    boost: u32,
    length: usize,
    row: u32,
    source: Source,
}

impl Candidate {
    fn order(&self) -> (std::cmp::Reverse<u8>, std::cmp::Reverse<u32>, usize, u32) {
        (
            std::cmp::Reverse(self.tier),
            std::cmp::Reverse(self.boost),
            self.length,
            self.row,
        )
    }
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
        Self::load_within(store, &[], HOT_BUDGET)
    }

    /// [`Self::load`], with a hot layer of the rows `boosted` and what else
    /// fits `budget`.
    pub(crate) fn load_within(store: Store, boosted: &[u32], budget: usize) -> Result<FileIndex> {
        let hot = store.hot(boosted, budget)?;
        let meta = store.meta()?;
        Ok(Self::from_parts(hot, budget, meta, store))
    }

    fn from_parts(hot: Hot, budget: usize, meta: Meta, store: Store) -> FileIndex {
        // The names are in memory now; the pages that held them are not
        // needed again.
        store.release_memory();
        Self {
            short: hot.names.short_entries(),
            hot: hot.names,
            hot_is_complete: hot.complete,
            budget,
            entries: hot.entries,
            store: Mutex::new(store),
            used: RwLock::new(Used::default()),
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
        let mut meta = recorded;
        let batch = store.batch()?;
        let delta = update::apply(&batch, rules, folders, &mut meta)?;
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

        // What arrived joins the hot layer -- it is the newest there is --
        // while the layer fits its budget; past it, the layer is selected
        // again, the used first.
        let patched = current.then(|| self.hot.updated(&delta.removed, &delta.added));
        let hot = match patched {
            Some(names) if names.bytes() <= self.budget => Hot {
                names,
                complete: self.hot_is_complete,
                entries: store.count()?,
            },
            _ => {
                let boosted = self.used.read().boosts.keys().copied().collect::<Vec<_>>();
                store.hot(&boosted, self.budget)?
            }
        };
        drop(delta);
        store.release_memory();
        drop(store);
        updated.index = Some(Self::from_parts(hot, self.budget, meta, Store::open(path)?));
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

    /// Build the word tables of the store at `path` at `detail`
    /// (`files.index-detail`, [`Store::set_detail`]): from the entries
    /// already stored, never a walk, and the names in memory are not
    /// touched. Under the build lock and through this index's own
    /// connection, as a rebuild writes, so a query waits for it rather than
    /// meeting half of it. Seconds for a large index: off the UI thread.
    pub fn set_detail(&self, path: &Path, detail: IndexDetail) -> Result<()> {
        let lock = build_lock(path);
        let _building = lock.lock();
        let mut store = self.store.lock();
        store.set_detail(detail)?;
        store.release_memory();
        Ok(())
    }

    /// The index the store at `path` holds, its word tables at `detail`
    /// (`files.index-detail`): what a start loads. A store built at another
    /// level is switched first, from its entries and never a walk, so a
    /// build that follows keeps the level.
    ///
    /// All of it under the build lock: opening can write too -- a store
    /// whose words an older build laid out is laid out again
    /// ([`Store::open`]) -- and a write never meets another.
    pub fn open(path: &Path, detail: IndexDetail) -> Result<FileIndex> {
        let lock = build_lock(path);
        let _building = lock.lock();
        let mut store = Store::open(path)?;
        if store.detail() != detail {
            store.set_detail(detail)?;
            store.release_memory();
        }
        Self::load(store)
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
        // The rows are numbered again, so the used ones are not known by
        // row until frecency is asked again.
        Self::load_within(Store::open(path)?, &[], self.budget)
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

    /// The best `limit` files and folders for `query`, best first: the hot
    /// layer's, and -- unless it is the whole index -- the disk's.
    ///
    /// Ranked by how well the name matched, then frecency, then the shorter
    /// name, as [`Names::find`] ranks; the disk's rows take their frecency
    /// here, by row. Answers nothing once `cancelled` says so: the query was
    /// superseded. A database that cannot be read answers nothing rather
    /// than an error: the launcher has nothing to tell the user
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
    /// *Search Files*' type filter searches (F16). Applied inside the scan
    /// and while the disk is read, so a filtered query finds its own best
    /// `limit`.
    ///
    /// The disk is asked for its best `limit` without frecency. That is
    /// enough: every used entry is in memory -- the hot layer or
    /// [`Used::outside`] -- where frecency ranks it, and frecency only ever
    /// lifts an entry within its tier.
    pub fn query_where(
        &self,
        query: &str,
        kinds: Kinds,
        limit: usize,
        accept: &dyn Fn(&str, bool) -> bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Vec<FileHit> {
        let used = self.used.read();
        let boost = |row: u32| used.boosts.get(&row).copied().unwrap_or(0);
        let hot = self.hot.find_where(
            query,
            kinds,
            limit,
            accept,
            |entry| boost(self.hot.row(entry)),
            cancelled,
        );
        let outside = used.outside.find_where(
            query,
            kinds,
            limit,
            accept,
            |entry| boost(used.outside.row(entry)),
            cancelled,
        );
        if cancelled() {
            return Vec::new();
        }
        let disk = if self.hot_is_complete {
            Vec::new()
        } else {
            match self.store.lock().ranked(query, kinds, limit, accept) {
                Ok(rows) => rows,
                Err(error) => {
                    log::warn!("could not read the file index: {error:#}");
                    Vec::new()
                }
            }
        };
        if cancelled() {
            return Vec::new();
        }
        let found = self.merge(&used, &hot, &outside, disk, limit);
        self.hits(query, &used, found)
    }

    /// Whether [`Self::query_inside`] can find anything [`Self::query`]
    /// did not: only when the hot layer is not the whole index -- a
    /// complete one already finds inside every word -- and `query` has a
    /// word of [`MIN_SUBSTRING`] characters, the least that matches inside
    /// one.
    pub fn searches_inside(&self, query: &str) -> bool {
        !self.hot_is_complete
            && names::words(query)
                .iter()
                .any(|word| word.chars().count() >= MIN_SUBSTRING)
    }

    /// What [`Self::query_where`] leaves out, and only that: the entries
    /// outside the hot layer and the used, matched inside a word
    /// ([`Store::substring_where`]), shortest first. *Search Files*' second
    /// phase (`docs/FILE-SEARCH.md`, "Step c3b"), which root does without.
    ///
    /// Nothing here was in the first phase's answer: that one found every
    /// entry in memory inside a word, and the disk's by a word's start,
    /// which a match inside a word never is. A scan of the disk's names at
    /// every level but `complete` -- tens of milliseconds to hundreds -- so
    /// it answers nothing once `cancelled` says so, asked every few
    /// thousand rows.
    pub fn query_inside(
        &self,
        query: &str,
        kinds: Kinds,
        limit: usize,
        accept: &dyn Fn(&str, bool) -> bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Vec<FileHit> {
        if limit == 0 || !self.searches_inside(query) {
            return Vec::new();
        }
        // A copy of the few used rows outside the layer, so `used` is not
        // held through a scan: a frecency change would wait for it, and
        // root's queries behind that.
        let outside = {
            let used = self.used.read();
            (0..used.outside.len() as u32)
                .map(|entry| used.outside.row(entry))
                .collect::<std::collections::HashSet<_>>()
        };
        let skip = |row: u32| self.hot.entry_of(row).is_some() || outside.contains(&row);
        let rows = match self
            .store
            .lock()
            .substring_where(query, kinds, limit, &skip, accept, cancelled)
        {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("could not read the file index: {error:#}");
                Vec::new()
            }
        };
        if cancelled() {
            return Vec::new();
        }
        let words = names::words(query);
        rows.into_iter()
            .map(|row| FileHit {
                highlights: highlights(&row.name, &words),
                item: self.item(Row {
                    id: row.id,
                    path: row.path,
                    name: row.name,
                    is_dir: row.is_dir,
                }),
            })
            .collect()
    }

    /// [`Self::query`] over the entries used from Lumen and those whose
    /// whole name is one or two characters: what a short query searches in
    /// root. A few hundred names, looked at one by one, so it is not
    /// cancelled; never the disk.
    pub fn query_known(&self, query: &str, kinds: Kinds, limit: usize) -> Vec<FileHit> {
        let used = self.used.read();
        let boost = |row: u32| used.boosts.get(&row).copied().unwrap_or(0);
        let mut known = used
            .boosts
            .keys()
            .filter_map(|&row| self.hot.entry_of(row))
            .chain(self.short.iter().copied())
            .collect::<Vec<_>>();
        known.sort_unstable();
        known.dedup();
        let hot = self.hot.find_among(query, kinds, limit, &known, |entry| {
            boost(self.hot.row(entry))
        });
        let every = (0..used.outside.len() as u32).collect::<Vec<_>>();
        let outside = used
            .outside
            .find_among(query, kinds, limit, &every, |entry| {
                boost(used.outside.row(entry))
            });
        let found = self.merge(&used, &hot, &outside, Vec::new(), limit);
        self.hits(query, &used, found)
    }

    /// The best `limit` of what the hot layer, the used outside it and the
    /// disk found, in the order [`Names`] ranks: each row once, at the best
    /// tier any of them gave it.
    fn merge(
        &self,
        used: &Used,
        hot: &[Match],
        outside: &[Match],
        disk: Vec<RankedRow>,
        limit: usize,
    ) -> Vec<Candidate> {
        let boost = |row: u32| used.boosts.get(&row).copied().unwrap_or(0);
        let memory = |names: &Names, found: &Match, source: Source| Candidate {
            tier: found.tier,
            boost: found.boost,
            length: names.name_len(found.entry),
            row: names.row(found.entry),
            source,
        };
        let mut candidates: Vec<Candidate> = hot
            .iter()
            .map(|found| memory(&self.hot, found, Source::Hot(found.entry)))
            .chain(
                outside
                    .iter()
                    .map(|found| memory(&used.outside, found, Source::Outside(found.entry))),
            )
            .collect();
        for row in disk {
            if let Some(known) = candidates.iter_mut().find(|known| known.row == row.id) {
                known.tier = known.tier.max(row.tier);
                continue;
            }
            let mut folded = String::with_capacity(row.name.len());
            names::fold(&row.name, &mut folded);
            candidates.push(Candidate {
                tier: row.tier,
                boost: boost(row.id),
                length: folded.len(),
                row: row.id,
                source: Source::Disk(row),
            });
        }
        candidates.sort_unstable_by_key(Candidate::order);
        candidates.truncate(limit);
        candidates
    }

    /// The rows of `found`, in its order, as hits for `query` -- each from
    /// memory only if it is still the entry that was matched (see
    /// [`Self::rebuilt`]); the disk's were read as they are.
    fn hits(&self, query: &str, used: &Used, found: Vec<Candidate>) -> Vec<FileHit> {
        if found.is_empty() {
            return Vec::new();
        }
        let ids = found
            .iter()
            .filter(|found| !matches!(found.source, Source::Disk(_)))
            .map(|found| found.row)
            .collect::<Vec<_>>();
        let mut rows = match self.store.lock().rows(&ids) {
            Ok(rows) => rows
                .into_iter()
                .map(|row| (row.id, row))
                .collect::<HashMap<_, _>>(),
            Err(error) => {
                log::warn!("could not read the file index: {error:#}");
                return Vec::new();
            }
        };

        let words = names::words(query);
        found
            .into_iter()
            .filter_map(|found| {
                let row = match found.source {
                    Source::Disk(row) => Row {
                        id: row.id,
                        path: row.path,
                        name: row.name,
                        is_dir: row.is_dir,
                    },
                    Source::Hot(entry) => rows
                        .remove(&found.row)
                        .filter(|row| self.hot.holds(entry, &row.name, row.is_dir))?,
                    Source::Outside(entry) => rows
                        .remove(&found.row)
                        .filter(|row| used.outside.holds(entry, &row.name, row.is_dir))?,
                };
                Some(FileHit {
                    highlights: highlights(&row.name, &words),
                    item: self.item(row),
                })
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
    /// loaded and again whenever frecency changes. A used entry the hot
    /// layer was selected without -- used since -- has its name kept beside
    /// it ([`Used::outside`]), so it is found as if it were in it; a handful
    /// of names, read here by row.
    pub fn set_boosts(&self, frecency: &Frecency) {
        let store = self.store.lock();
        let mut boosts = HashMap::new();
        let mut outside = Vec::new();
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
            match store.id_of(path) {
                // Checked as a query's rows are, so a row that moved under
                // this index boosts nothing rather than another entry.
                Ok(Some(row)) => match self.hot.entry_of(row) {
                    Some(entry) => {
                        if self.hot.holds(entry, &name, is_dir) {
                            boosts.insert(row, boost);
                        }
                    }
                    // A complete hot layer without the row is one the store
                    // moved on from: the row is not this index's entry.
                    None if !self.hot_is_complete => {
                        boosts.insert(row, boost);
                        outside.push((row, is_dir));
                    }
                    None => {}
                },
                Ok(None) => {}
                Err(error) => log::warn!("could not read the file index: {error:#}"),
            }
        }

        outside.sort_unstable();
        let ids = outside.iter().map(|&(row, _)| row).collect::<Vec<_>>();
        let mut names = Names::default();
        match store.rows(&ids) {
            Ok(rows) => {
                for row in rows {
                    let wanted = outside
                        .binary_search_by_key(&row.id, |&(id, _)| id)
                        .map(|at| outside[at].1);
                    if wanted == Ok(row.is_dir) {
                        names.push(row.id, &row.name, row.is_dir);
                    } else {
                        boosts.remove(&row.id);
                    }
                }
            }
            Err(error) => log::warn!("could not read the file index: {error:#}"),
        }
        names.shrink();
        store.release_memory();
        // Unlocked first: a query holds `used` while it waits for the store.
        drop(store);
        *self.used.write() = Used {
            boosts,
            outside: names,
        };
    }

    /// How many entries each scope of the rules gave.
    pub fn per_scope(&self) -> &[usize] {
        &self.meta.per_scope
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
            walked_at: self.walked_at(),
            indexing,
        }
    }

    /// How many entries the index holds, in memory or not.
    pub fn len(&self) -> usize {
        self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries == 0
    }

    /// How many entries the hot layer holds.
    pub fn hot_len(&self) -> usize {
        self.hot.len()
    }

    /// Whether the hot layer is the whole index, so root never asks the
    /// disk.
    pub fn hot_is_complete(&self) -> bool {
        self.hot_is_complete
    }

    /// What the index holds in memory: the hot layer, the used entries
    /// outside it, the boosts and the short names.
    pub fn memory_bytes(&self) -> usize {
        let used = self.used.read();
        self.hot.memory_bytes()
            + used.outside.memory_bytes()
            + used.boosts.capacity() * 8
            + self.short.capacity() * 4
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

    /// A file deleted since the walk is still in the index after the level
    /// changed: the word tables were built from the store, and nothing was
    /// walked.
    #[test]
    fn changing_the_index_detail_walks_nothing() {
        let tree = Tree::new("index-detail");
        tree.file("notes.md").file("projects/myreport.md");
        let built = index(&tree);
        let path = tree.path(".files.db");
        let before = Store::open(&path).unwrap().meta().unwrap();
        std::fs::remove_file(tree.path("notes.md")).unwrap();

        built.set_detail(&path, IndexDetail::Complete).unwrap();

        let store = built.store.lock();
        assert_eq!(store.detail(), IndexDetail::Complete);
        assert_eq!(store.meta().unwrap(), before);
        assert_eq!(store.count().unwrap(), 3);
        let inside = store
            .substring("port", Kinds::FoldersAndFiles, 8, &|| false)
            .unwrap();
        assert_eq!(inside[0].name, "myreport.md");
        let notes = store
            .ranked("notes", Kinds::FoldersAndFiles, 8, &|_, _| true)
            .unwrap();
        assert_eq!(notes[0].name, "notes.md");
    }

    /// The configured level reaches a store built at another when it is
    /// opened, without a walk; the build after it keeps that level.
    #[test]
    fn opening_at_a_level_switches_the_store_and_walks_nothing() {
        let tree = Tree::new("index-open-detail");
        tree.file("notes.md").file("projects/myreport.md");
        drop(index(&tree));
        let path = tree.path(".files.db");
        let before = Store::open(&path).unwrap().meta().unwrap();
        std::fs::remove_file(tree.path("notes.md")).unwrap();

        let opened = FileIndex::open(&path, IndexDetail::Compact).unwrap();
        assert_eq!(opened.len(), 3);
        assert_eq!(opened.store.lock().detail(), IndexDetail::Compact);
        assert_eq!(opened.store.lock().meta().unwrap(), before);
        drop(opened);
        assert_eq!(Store::open(&path).unwrap().detail(), IndexDetail::Compact);

        let rebuilt = FileIndex::build(&path, &tree.rules()).unwrap();
        assert_eq!(rebuilt.len(), 2);
        assert_eq!(rebuilt.store.lock().detail(), IndexDetail::Compact);
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

    /// A store whose words an older build laid out is rebuilt at open, and
    /// that is a write: it waits for a build of the same file, as every
    /// write does.
    #[test]
    fn opening_a_store_that_must_be_laid_out_again_waits_for_a_build() {
        let tree = Tree::new("index-open-lock");
        tree.file("notes.md");
        drop(index(&tree));
        let path = tree.path(".files.db");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("DELETE FROM meta WHERE key = 'words_layout'", [])
            .unwrap();

        let lock = build_lock(&path);
        let held = lock.lock();
        let opener = std::thread::spawn({
            let path = path.clone();
            move || FileIndex::open(&path, IndexDetail::Balanced).unwrap().len()
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!opener.is_finished());

        drop(held);
        assert_eq!(opener.join().unwrap(), 1);
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

    /// No scope is cut short: each counts all it gave, and the settings
    /// page is told every count.
    #[test]
    fn every_scope_is_counted_whole() {
        let first = Tree::new("index-counts-first");
        let second = Tree::new("index-counts-second");
        first.file("a.txt").file("b.txt");
        second.file("c.txt").file("d.txt").file("e.txt");

        let rules = rules(&[first.0.clone(), second.0.clone()]);
        let index = FileIndex::build(&first.path(".files.db"), &rules).unwrap();

        assert_eq!(index.len(), 5);
        assert_eq!(index.per_scope(), [2, 3]);
        let status = index.status(false);
        assert_eq!((status.entries, status.per_scope), (5, vec![2, 3]));
    }

    /// The index at `tree`, built as a start loads it, with a hot layer of
    /// `budget` bytes.
    fn index_within(tree: &Tree, budget: usize) -> FileIndex {
        let path = tree.path(".files.db");
        drop(FileIndex::build(&path, &tree.rules()).unwrap());
        FileIndex::load_within(Store::open(&path).unwrap(), &[], budget)
            .unwrap()
            .with_home(&tree.0)
    }

    fn root(index: &FileIndex, query: &str, limit: usize) -> Vec<String> {
        titles(&index.root_query(query, Kinds::FoldersAndFiles, limit, &|| false))
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    fn use_file(index: &FileIndex, tree: &Tree, relative: &str) {
        let mut frecency = Frecency::new();
        frecency.record(&format!("file:{}", tree.path(relative).display()));
        index.set_boosts(&frecency);
    }

    /// A home-sized index fits the hot layer whole, and root answers what
    /// the names of every entry answer -- a match inside a word included --
    /// without asking the disk.
    #[test]
    fn a_home_sized_index_is_all_hot_and_root_answers_as_every_name_does() {
        let tree = Tree::new("index-hot-whole");
        tree.file("notes.md")
            .file("projects/Invoice-2024.pdf")
            .file("projects/old/myreport.txt")
            .file("projects/old/report final.doc")
            .file("archive/2024/invoices/march invoice.pdf")
            .file("archive/2024/reports.md");
        let index = index(&tree);
        assert!(index.hot_is_complete());
        assert_eq!(index.hot_len(), index.len());

        let store = Store::open(&tree.path(".files.db")).unwrap();
        let (every, _) = store.load().unwrap();
        for query in [
            "inv", "invoice", "port", "report", "2024", "inv 2024", "voic", "old",
        ] {
            let found = every.find(query, Kinds::FoldersAndFiles, 8, |_| 0);
            let ids = found
                .iter()
                .map(|found| every.row(found.entry))
                .collect::<Vec<_>>();
            let expected = store
                .rows(&ids)
                .unwrap()
                .into_iter()
                .map(|row| row.name)
                .collect::<Vec<_>>();
            assert_eq!(root(&index, query, 8), expected, "{query}");
        }
        assert!(root(&index, "port", 8).contains(&"myreport.txt".to_string()));
    }

    /// Past its budget the hot layer holds the shallow and the used. An
    /// entry outside it is found from the disk by a word's start, and not
    /// by a match inside a word -- the trade "Decision: B" accepted --
    /// until it is used, when it is found as the hot layer's are.
    #[test]
    fn past_its_budget_root_finds_the_rest_by_word_start_and_the_used_inside_a_word() {
        let tree = Tree::new("index-hot-partial");
        tree.file("top.txt")
            .file("deep/er/still/my-invoice.pdf")
            .file("deep/er/still/annualreport.txt")
            .file("deep/er/still/usedreport.txt");
        let index = index_within(&tree, 0);
        assert!(!index.hot_is_complete());
        assert_eq!(index.len(), 7);
        assert_eq!(index.hot_len(), 1, "only `er`, whose name is that short");

        assert_eq!(root(&index, "invoice", 8), ["my-invoice.pdf"]);
        assert_eq!(root(&index, "top", 8), ["top.txt"]);
        assert!(root(&index, "voice", 8).is_empty());
        assert!(root(&index, "report", 8).is_empty());
        assert!(root(&index, "us", 8).is_empty());

        use_file(&index, &tree, "deep/er/still/usedreport.txt");
        assert_eq!(root(&index, "report", 8), ["usedreport.txt"]);
        assert_eq!(root(&index, "port", 8), ["usedreport.txt"]);
        assert_eq!(root(&index, "us", 8), ["usedreport.txt"]);
        let hit = &index.root_query("port", Kinds::FoldersAndFiles, 8, &|| false)[0];
        assert_eq!(
            hit.item.id,
            format!(
                "file:{}",
                tree.path("deep")
                    .join("er")
                    .join("still")
                    .join("usedreport.txt")
                    .display()
            )
        );
        assert_eq!(hit.highlights, [6, 7, 8, 9]);
    }

    fn inside(index: &FileIndex, query: &str, limit: usize) -> Vec<String> {
        titles(
            &index.query_inside(query, Kinds::FoldersAndFiles, limit, &|_, _| true, &|| {
                false
            }),
        )
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    /// *Search Files*' second phase: what root's trade leaves out -- a
    /// match inside a word of an entry outside the hot layer -- is found
    /// from the disk, and nothing the first phase could find is repeated.
    #[test]
    fn outside_the_hot_layer_the_second_phase_finds_a_match_inside_a_word() {
        let tree = Tree::new("index-inside");
        tree.file("myreport.txt")
            .file("deep/er/still/annualreport.txt")
            .file("deep/er/still/report.md")
            .file("deep/er/still/usedreport.txt");
        // `er`, which a short query needs, the two other folders, and the
        // shallowest file.
        let budget = Names::cost("er".len())
            + Names::cost("deep".len())
            + Names::cost("still".len())
            + Names::cost("myreport.txt".len());
        let index = index_within(&tree, budget);
        assert!(!index.hot_is_complete());
        assert!(index.searches_inside("port"));
        use_file(&index, &tree, "deep/er/still/usedreport.txt");

        // The first phase: the hot layer inside a word, the disk by a
        // word's start, the used inside a word.
        let first = titles(&index.query("port", Kinds::FoldersAndFiles, 8, &|| false))
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(first, ["usedreport.txt", "myreport.txt"]);
        // The second: only what is outside both, inside a word.
        assert_eq!(inside(&index, "port", 8), ["report.md", "annualreport.txt"]);
        assert!(inside(&index, "report", 8).contains(&"annualreport.txt".to_string()));
        assert!(!inside(&index, "report", 8).contains(&"report.md".to_string()));
        let hit =
            &index.query_inside("port", Kinds::FoldersAndFiles, 8, &|_, _| true, &|| false)[1];
        assert_eq!(hit.highlights, [8, 9, 10, 11]);
        assert_eq!(hit.item.kind, Kind::File);
    }

    /// The second phase keeps to the type filter and the kinds, and a
    /// cancelled one answers nothing.
    #[test]
    fn the_second_phase_keeps_to_the_filter_and_stops_when_cancelled() {
        let tree = Tree::new("index-inside-filter");
        tree.file("a/b/myplan.md")
            .file("a/b/myplan.rs")
            .file("a/b/myplans/x.txt");
        let index = index_within(&tree, 0);
        let code = |name: &str, _: bool| name.ends_with(".rs");
        let found = index.query_inside("plan", Kinds::FoldersAndFiles, 8, &code, &|| false);
        assert_eq!(titles(&found), ["myplan.rs"]);
        let folders = index.query_inside("plan", Kinds::Folders, 8, &|_, _| true, &|| false);
        assert_eq!(titles(&folders), ["myplans"]);
        assert_eq!(inside(&index, "plan", 2).len(), 2);
        assert!(
            index
                .query_inside("plan", Kinds::FoldersAndFiles, 8, &|_, _| true, &|| true)
                .is_empty()
        );
    }

    /// A complete hot layer already finds inside every word, and a query
    /// with no word of three characters has nothing inside one: no second
    /// phase.
    #[test]
    fn there_is_no_second_phase_with_a_complete_hot_layer_or_short_words() {
        let tree = Tree::new("index-inside-none");
        tree.file("a/b/myreport.txt");
        let whole = index(&tree);
        assert!(whole.hot_is_complete());
        assert!(!whole.searches_inside("port"));
        assert!(inside(&whole, "port", 8).is_empty());

        let partial = index_within(&tree, 0);
        assert!(partial.searches_inside(" port "));
        assert!(!partial.searches_inside("po"));
        assert!(!partial.searches_inside("po rt"));
        assert!(partial.searches_inside("my port"));
        assert!(inside(&partial, "po", 8).is_empty());
    }

    /// What the hot layer and the disk each found is ranked as the names of
    /// every entry rank it: tier, frecency, the shorter name, the row.
    #[test]
    fn the_hot_layer_and_the_disk_merge_in_the_order_names_rank() {
        let tree = Tree::new("index-hot-merge");
        tree.file("report")
            .file("report.txt")
            .file("Report-2024.pdf")
            .file("x/old-report.md")
            .file("x/y/my report final.doc")
            .file("x/y/reports/q3 report.xlsx")
            .file("x/y/z/report 2024 draft.md")
            .file("x/y/z/annual-report.md")
            .file("x/y/z/report.md");
        let whole = index(&tree);
        assert!(whole.hot_is_complete());
        let mut budget = 0;
        let mut partial = Vec::new();
        for _ in 0..6 {
            let index = index_within(&tree, budget);
            assert!(!index.hot_is_complete());
            partial.push(index);
            budget += 40;
        }

        for index in std::iter::once(&whole).chain(&partial) {
            use_file(index, &tree, "x/y/z/annual-report.md");
        }
        for query in ["report", "rep", "report 2024", "final", "2024"] {
            for limit in [3, 8] {
                let expected = root(&whole, query, limit);
                for (at, index) in partial.iter().enumerate() {
                    assert_eq!(root(index, query, limit), expected, "{query} {limit} #{at}");
                }
            }
        }
        // Exact names, name starts, then word starts with the used first:
        // frecency orders a tier, and never lifts a match out of it.
        assert_eq!(
            root(&whole, "report", 8),
            [
                "report",
                "report.md",
                "report.txt",
                "reports",
                "Report-2024.pdf",
                "report 2024 draft.md",
                "annual-report.md",
                "old-report.md",
            ]
        );
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

    /// An update adds everything it finds, counted to its scope: nothing
    /// caps it.
    #[test]
    fn an_update_adds_all_it_finds_and_counts_it() {
        let scope = Scope::new("update-all");
        scope.file("a.txt").file("b.txt");
        let index = scope.build();

        scope.file("c.txt").file("d/e.txt").file("d/f.txt");
        let index = scope.update(index, &[scope.path("c.txt"), scope.path("d")]);

        assert_eq!(index.len(), 6);
        assert_eq!(index.per_scope(), [6]);
        assert_eq!(found(&index, &scope, "txt").len(), 5);
    }

    /// What arrives joins the hot layer while it fits; past the budget the
    /// layer is selected again, and what did not fit is found from disk.
    #[test]
    fn an_update_past_the_budget_selects_the_hot_layer_again() {
        let scope = Scope::new("update-hot");
        scope.file("notes.md");
        drop(scope.build());
        let budget = Names::cost("notes.md".len()) + Names::cost("plan.md".len());
        let index =
            FileIndex::load_within(Store::open(&scope.database()).unwrap(), &[], budget).unwrap();
        assert!(index.hot_is_complete());

        scope.file("plan.md");
        let index = scope.update(index, &[scope.path("plan.md")]);
        assert!(index.hot_is_complete());
        assert_eq!(index.hot_len(), 2);

        scope.file("deeper/todo.md");
        let index = scope.update(index, &[scope.path("deeper")]);
        assert!(!index.hot_is_complete());
        assert!(index.hot.bytes() <= budget);
        assert_eq!(index.len(), 4);
        assert_eq!(found(&index, &scope, "todo"), ["deeper/todo.md"]);
        assert_eq!(found(&index, &scope, "notes"), ["notes.md"]);
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
