//! The entries of the index, on disk (F16).
//!
//! # A record, not a search engine
//!
//! Slice 0 measured SQLite's trigram index at 19 ms to collect every match of
//! a million names, against a budget of 4, so nothing here is searched by
//! name. The database is the record of what was indexed: a start loads the
//! names from it in milliseconds instead of walking the disk again, and a
//! query fetches the paths of the few rows it found.
//!
//! # A cache, and treated like one
//!
//! It lives in the cache folder and holds nothing that cannot be walked
//! again, so it is never migrated: another schema version is dropped and
//! created afresh, and a file SQLite cannot read is deleted. Writes skip the
//! rollback journal for the same reason -- a crash halfway through a
//! rebuild costs a rebuild, which is what the journal would have saved.
//! That a rebuild did finish is recorded last, as the rules' digest (see
//! [`Store::replace`]).
//!
//! # Row ids
//!
//! Every method here speaks of rows by SQLite's row id; the names map their
//! entries to them ([`Names::row`]). A rebuild numbers the rows from 1 in
//! the order the walk found them. A change applied in place (slice 2)
//! deletes rows and adds new ones after the last -- `AUTOINCREMENT`, so a
//! deleted row's id is never given to another entry until the next rebuild
//! numbers everything again. An index still installed while its store
//! changed under it then finds each of its rows as it was, or gone.
//!
//! # Changed in place, and versioned
//!
//! Every write that adds or removes rows counts up `version`, and an index
//! remembers the version it was made from. A batch applied to an index of
//! an older version cannot patch its names -- the store has moved on without
//! it -- and loads them from the store instead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension as _, Transaction, params};

use super::names::Names;
use super::walk::{Entry, WalkRules, Walked};

/// Stored in SQLite's own `user_version`. Anything else is dropped: version
/// 1 had no folder times and reused row ids.
const SCHEMA_VERSION: i32 = 2;

/// What was indexed besides the entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    /// The [`digest`] of the rules the entries were walked with; empty when
    /// no walk has finished.
    pub digest: String,
    /// As [`Walked::per_scope`].
    pub per_scope: Vec<usize>,
    /// As [`Walked::truncated`].
    pub truncated: Option<PathBuf>,
    /// As [`Walked::roots`], kept current as folders are read again.
    pub roots: Vec<Option<i64>>,
    /// Counted up by every write that adds or removes rows (see "Changed in
    /// place, and versioned" above).
    pub version: u64,
}

/// One entry, as a query result needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The entry's row id.
    pub id: u32,
    pub path: PathBuf,
    /// As it is on disk, not folded.
    pub name: String,
    pub is_dir: bool,
}

/// The entries, in SQLite.
pub struct Store {
    connection: Connection,
}

/// A fingerprint of `rules`, so an index walked with other rules is known to
/// be stale.
///
/// FNV-1a over every field, with each value and each list ended by a byte no
/// path or pattern contains, so `["ab"]` and `["a", "b"]` differ. Not
/// `std`'s `DefaultHasher`: its output is allowed to change between Rust
/// releases, and a new build must not rebuild every index for nothing.
pub fn digest(rules: &WalkRules) -> String {
    const VALUE: u8 = 0;
    const LIST: u8 = 1;

    let mut hash = Fnv::default();
    for list in [&rules.scopes, &rules.exclusions] {
        for path in list {
            hash.write(path.to_string_lossy().as_bytes());
            hash.write(&[VALUE]);
        }
        hash.write(&[LIST]);
    }
    for pattern in &rules.ignore_patterns {
        hash.write(pattern.as_bytes());
        hash.write(&[VALUE]);
    }
    hash.write(&[LIST]);
    hash.write(&[
        u8::from(rules.use_ignore_files),
        u8::from(rules.include_hidden),
    ]);
    hash.write(&(rules.ceiling as u64).to_le_bytes());
    format!("{:016x}", hash.0)
}

struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

impl Store {
    /// Open the index at `path`, creating it -- and creating it again from
    /// nothing when what is there cannot be read.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        match Self::open_as_is(path) {
            Ok(store) => Ok(store),
            Err(error) => {
                log::warn!(
                    "rebuilding the unreadable file index at {}: {error:#}",
                    path.display()
                );
                std::fs::remove_file(path)
                    .with_context(|| format!("removing {}", path.display()))?;
                Self::open_as_is(path)
            }
        }
    }

    fn open_as_is(path: &Path) -> Result<Store> {
        let connection =
            Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        // SQLite reads nothing until asked, so a file that is not a database
        // fails here, at the first statement, rather than at `open`.
        connection
            .execute_batch(
                // A megabyte of page cache, not the default two: the cache
                // only serves the few rows of a query, and is released when
                // idle anyway.
                "PRAGMA cache_size = -1024;
                 PRAGMA journal_mode = OFF;
                 PRAGMA synchronous = OFF;",
            )
            .context("configuring the file index")?;
        let version: i32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .context("reading the file index's schema version")?;
        if version != SCHEMA_VERSION {
            connection
                .execute_batch(
                    "DROP TABLE IF EXISTS entries;
                     DROP TABLE IF EXISTS meta;
                     CREATE TABLE entries (
                         -- Never reused (see the module documentation).
                         id       INTEGER PRIMARY KEY AUTOINCREMENT,
                         -- Which scope of the rules, by position: removing a
                         -- scope drops its rows without walking the rest.
                         scope    INTEGER NOT NULL,
                         parent   TEXT    NOT NULL,
                         name     TEXT    NOT NULL,
                         is_dir   INTEGER NOT NULL,
                         -- A folder's modification time, for the start
                         -- reconcile; NULL for a file.
                         modified INTEGER
                     );
                     -- A path to its entry, for frecency's ids.
                     CREATE INDEX entries_path ON entries (parent, name);
                     CREATE TABLE meta (
                         key   TEXT PRIMARY KEY,
                         value TEXT NOT NULL
                     );",
                )
                .context("creating the file index")?;
            connection
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .context("recording the file index's schema version")?;
        }
        Ok(Store { connection })
    }

    /// Make `walked` the whole index, walked with rules whose [`digest`] is
    /// `digest`.
    ///
    /// One transaction, but with no journal to roll it back (see the module
    /// documentation), so the digest is removed first and written last: an
    /// index whose rebuild was cut short has no digest, matches no rules,
    /// and is rebuilt again.
    pub fn replace(&mut self, walked: &Walked, digest: &str) -> Result<()> {
        let version = self.meta()?.version + 1;
        self.connection
            .execute("DELETE FROM meta WHERE key = 'digest'", [])
            .context("clearing the file index's digest")?;

        let transaction = self.connection.transaction()?;
        // The rows are numbered from 1 again: an index that remembers one of
        // the old ones checks its name before trusting it.
        transaction.execute_batch(
            "DELETE FROM entries; DELETE FROM meta;
             DELETE FROM sqlite_sequence WHERE name = 'entries';",
        )?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO entries (id, scope, parent, name, is_dir, modified)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            // Entries come scope by scope, as many of each as it counted.
            let scopes = walked
                .per_scope
                .iter()
                .enumerate()
                .flat_map(|(scope, &count)| std::iter::repeat_n(scope, count));
            for (index, (entry, scope)) in walked.entries.iter().zip(scopes).enumerate() {
                let parent = entry
                    .path
                    .parent()
                    .map(|parent| parent.to_string_lossy())
                    .unwrap_or_default();
                let name = entry
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default();
                insert.execute(params![
                    index as i64 + 1,
                    scope as i64,
                    parent,
                    name,
                    entry.is_dir,
                    entry.modified
                ])?;
            }
        }
        write_meta(
            &transaction,
            &Meta {
                digest: digest.to_string(),
                per_scope: walked.per_scope.clone(),
                truncated: walked.truncated.clone(),
                roots: walked.roots.clone(),
                version,
            },
        )?;
        transaction.commit().context("writing the file index")
    }

    /// The names, in entry order, and what was recorded about them.
    pub fn load(&self) -> Result<(Names, Meta)> {
        let mut names = Names::default();
        let mut statement = self
            .connection
            .prepare("SELECT id, name, is_dir FROM entries ORDER BY id")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            names.push(row.get(0)?, row.get_ref(1)?.as_str()?, row.get(2)?);
        }
        names.shrink();
        Ok((names, self.meta()?))
    }

    /// What was recorded besides the entries.
    pub fn meta(&self) -> Result<Meta> {
        let mut meta = Meta::default();
        let mut statement = self.connection.prepare("SELECT key, value FROM meta")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let value: String = row.get(1)?;
            match row.get_ref(0)?.as_str()? {
                "digest" => meta.digest = value,
                "per_scope" => {
                    meta.per_scope = value
                        .split(',')
                        .filter_map(|count| count.parse().ok())
                        .collect();
                }
                "truncated" => meta.truncated = Some(PathBuf::from(value)),
                // One time a scope, empty where it had none.
                "roots" => {
                    meta.roots = value.split(',').map(|time| time.parse().ok()).collect();
                }
                "version" => meta.version = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        Ok(meta)
    }

    /// How many entries there are.
    pub fn count(&self) -> Result<usize> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Every indexed folder and its recorded time: what the start reconcile
    /// compares with the disk.
    pub fn folders(&self) -> Result<Vec<(PathBuf, Option<i64>)>> {
        let mut statement = self
            .connection
            .prepare("SELECT parent, name, modified FROM entries WHERE is_dir = 1")?;
        let folders = statement
            .query_map([], |row| {
                let parent: String = row.get(0)?;
                let name: String = row.get(1)?;
                Ok((Path::new(&parent).join(name), row.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(folders)
    }

    /// Start changing the index in place. Nothing is kept until
    /// [`Batch::finish`]; as in [`Self::replace`], the digest is removed
    /// first and written last, so a batch cut short leaves an index that is
    /// rebuilt.
    pub(crate) fn batch(&mut self) -> Result<Batch<'_>> {
        self.connection
            .execute("DELETE FROM meta WHERE key = 'digest'", [])
            .context("clearing the file index's digest")?;
        Ok(Batch {
            transaction: self.connection.transaction()?,
        })
    }

    /// The entries `ids`, by row id, in the order asked for. An id with no
    /// entry is left out.
    pub fn rows(&self, ids: &[u32]) -> Result<Vec<Row>> {
        let mut found = HashMap::with_capacity(ids.len());
        // In chunks, under SQLite's limit on the parameters of a statement.
        for chunk in ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let mut statement = self.connection.prepare(&format!(
                "SELECT id, parent, name, is_dir FROM entries WHERE id IN ({placeholders})"
            ))?;
            let rows = statement.query_map(
                rusqlite::params_from_iter(chunk.iter().map(|&id| i64::from(id))),
                |row| {
                    let parent: String = row.get(1)?;
                    let name: String = row.get(2)?;
                    Ok(Row {
                        id: row.get(0)?,
                        path: Path::new(&parent).join(&name),
                        name,
                        is_dir: row.get(3)?,
                    })
                },
            )?;
            for row in rows {
                let row = row?;
                found.insert(row.id, row);
            }
        }
        Ok(ids.iter().filter_map(|id| found.remove(id)).collect())
    }

    /// The row of the entry at `path`, if it is indexed.
    pub fn id_of(&self, path: &Path) -> Result<Option<u32>> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Ok(None);
        };
        let id = self
            .connection
            .query_row(
                "SELECT id FROM entries WHERE parent = ?1 AND name = ?2",
                params![parent.to_string_lossy(), name.to_string_lossy()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(id)
    }

    /// Give back the page cache. The names are in memory once loaded, so the
    /// database holds nothing while the launcher is idle.
    pub fn release_memory(&self) {
        if let Err(error) = self.connection.execute_batch("PRAGMA shrink_memory;") {
            log::warn!("could not release the file index's memory: {error}");
        }
    }
}

/// Write `meta` in place of what was recorded, the digest last.
fn write_meta(transaction: &Transaction<'_>, meta: &Meta) -> Result<()> {
    transaction.execute("DELETE FROM meta", [])?;
    let mut insert = transaction.prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")?;
    let counts = meta
        .per_scope
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",");
    insert.execute(params!["per_scope", counts])?;
    if let Some(truncated) = &meta.truncated {
        insert.execute(params!["truncated", truncated.to_string_lossy()])?;
    }
    let roots = meta
        .roots
        .iter()
        .map(|time| time.map(|time| time.to_string()).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(",");
    insert.execute(params!["roots", roots])?;
    insert.execute(params!["version", meta.version.to_string()])?;
    insert.execute(params!["digest", meta.digest])?;
    Ok(())
}

/// One entry directly in a folder, as the store has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Child {
    pub id: u32,
    pub name: String,
    pub is_dir: bool,
    pub scope: usize,
}

/// Changes to the index in place, in one transaction (see
/// [`Store::batch`]). Every read sees the writes before it, so a folder read
/// again after its parent's new subtree was added finds it already there.
pub(crate) struct Batch<'a> {
    transaction: Transaction<'a>,
}

impl Batch<'_> {
    /// Whether `path` is an indexed folder: one the rules allowed when it was
    /// last read, so a change in it is applied.
    pub fn is_folder(&self, path: &Path) -> Result<bool> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Ok(false);
        };
        Ok(self
            .transaction
            .query_row(
                "SELECT 1 FROM entries WHERE parent = ?1 AND name = ?2 AND is_dir = 1",
                params![parent.to_string_lossy(), name.to_string_lossy()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The entries directly in `folder`.
    pub fn children(&self, folder: &Path) -> Result<Vec<Child>> {
        let mut statement = self
            .transaction
            .prepare_cached("SELECT id, name, is_dir, scope FROM entries WHERE parent = ?1")?;
        let children = statement
            .query_map(params![folder.to_string_lossy()], |row| {
                Ok(Child {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    is_dir: row.get(2)?,
                    scope: row.get::<_, i64>(3)? as usize,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(children)
    }

    /// Remove `child` of `folder` and, when it is a folder, everything under
    /// it. The rows removed, with their scopes.
    pub fn remove(&self, folder: &Path, child: &Child) -> Result<Vec<(u32, usize)>> {
        let mut removed = vec![(child.id, child.scope)];
        self.transaction
            .execute("DELETE FROM entries WHERE id = ?1", params![child.id])?;
        if child.is_dir {
            // A folder's subtree is every row whose parent is the folder, or
            // starts with it and a separator: a range of the index on
            // `parent`, not a scan. The separator's successor ends it.
            let path = folder.join(&child.name).to_string_lossy().into_owned();
            let separator = std::path::MAIN_SEPARATOR;
            let after = char::from_u32(separator as u32 + 1).expect("a character follows it");
            let range = params![path, format!("{path}{separator}"), format!("{path}{after}")];
            let within = "parent = ?1 OR (parent >= ?2 AND parent < ?3)";
            let mut statement = self
                .transaction
                .prepare(&format!("SELECT id, scope FROM entries WHERE {within}"))?;
            let below = statement
                .query_map(range, |row| {
                    Ok((row.get(0)?, row.get::<_, i64>(1)? as usize))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            removed.extend(below);
            self.transaction
                .execute(&format!("DELETE FROM entries WHERE {within}"), range)?;
        }
        Ok(removed)
    }

    /// Add `entry`, of scope `scope`, and give its new row.
    pub fn insert(&self, scope: usize, entry: &Entry) -> Result<u32> {
        let parent = entry
            .path
            .parent()
            .map(|parent| parent.to_string_lossy())
            .unwrap_or_default();
        let name = entry
            .path
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        let mut insert = self.transaction.prepare_cached(
            "INSERT INTO entries (scope, parent, name, is_dir, modified)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        insert.execute(params![
            scope as i64,
            parent,
            name,
            entry.is_dir,
            entry.modified
        ])?;
        u32::try_from(self.transaction.last_insert_rowid())
            .context("the file index ran out of row ids; rebuild it")
    }

    /// Record the time `folder` had when it was read again.
    pub fn set_modified(&self, folder: &Path, modified: Option<i64>) -> Result<()> {
        let (Some(parent), Some(name)) = (folder.parent(), folder.file_name()) else {
            return Ok(());
        };
        self.transaction.execute(
            "UPDATE entries SET modified = ?3 WHERE parent = ?1 AND name = ?2 AND is_dir = 1",
            params![parent.to_string_lossy(), name.to_string_lossy(), modified],
        )?;
        Ok(())
    }

    /// Write `meta` and make every change of the batch the index.
    pub fn finish(self, meta: &Meta) -> Result<()> {
        write_meta(&self.transaction, meta)?;
        self.transaction
            .commit()
            .context("writing changes to the file index")
    }
}

#[cfg(test)]
mod tests {
    use super::super::names::Kinds;
    use super::super::walk::Entry;
    use super::super::walk::tests::{Tree, rules};
    use super::*;

    fn walked(root: &Path) -> Walked {
        Walked {
            entries: vec![
                Entry {
                    path: root.join("notes.md"),
                    is_dir: false,
                    modified: None,
                },
                Entry {
                    path: root.join("projects"),
                    is_dir: true,
                    modified: Some(7),
                },
                Entry {
                    path: root.join("projects").join("Invoice.pdf"),
                    is_dir: false,
                    modified: None,
                },
            ],
            per_scope: vec![3, 0],
            truncated: Some(root.join("second")),
            roots: vec![Some(5), None],
        }
    }

    #[test]
    fn what_is_replaced_loads_back_in_order() {
        let tree = Tree::new("store-round-trip");
        let mut store = Store::open(&tree.path("files.db")).unwrap();
        store.replace(&walked(&tree.0), "digest-1").unwrap();

        let (names, meta) = store.load().unwrap();
        assert_eq!(names.len(), 3);
        let found = names.find("invoice", Kinds::FoldersAndFiles, 8, |_| 0);
        assert_eq!(found.iter().map(|m| m.entry).collect::<Vec<_>>(), [2]);
        let folders = names.find("proj", Kinds::Folders, 8, |_| 0);
        assert_eq!(folders.iter().map(|m| m.entry).collect::<Vec<_>>(), [1]);

        assert_eq!(
            meta,
            Meta {
                digest: "digest-1".into(),
                per_scope: vec![3, 0],
                truncated: Some(tree.0.join("second")),
                roots: vec![Some(5), None],
                version: 1,
            }
        );
    }

    /// A second replace is the whole index, not an addition to it.
    #[test]
    fn a_replace_drops_what_was_there() {
        let tree = Tree::new("store-replace");
        let mut store = Store::open(&tree.path("files.db")).unwrap();
        store.replace(&walked(&tree.0), "one").unwrap();

        let fewer = Walked {
            entries: vec![Entry {
                path: tree.0.join("only.txt"),
                is_dir: false,
                modified: None,
            }],
            per_scope: vec![1],
            truncated: None,
            roots: vec![None],
        };
        store.replace(&fewer, "two").unwrap();

        let (names, meta) = store.load().unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(meta.digest, "two");
        assert_eq!(meta.truncated, None);
        assert_eq!(meta.per_scope, [1]);
    }

    #[test]
    fn rows_and_paths_map_to_each_other() {
        let tree = Tree::new("store-rows");
        let mut store = Store::open(&tree.path("files.db")).unwrap();
        store.replace(&walked(&tree.0), "d").unwrap();

        let rows = store.rows(&[3, 1]).unwrap();
        assert_eq!(
            rows,
            [
                Row {
                    id: 3,
                    path: tree.0.join("projects").join("Invoice.pdf"),
                    name: "Invoice.pdf".into(),
                    is_dir: false,
                },
                Row {
                    id: 1,
                    path: tree.0.join("notes.md"),
                    name: "notes.md".into(),
                    is_dir: false,
                },
            ]
        );

        assert_eq!(
            store
                .id_of(&tree.0.join("projects").join("Invoice.pdf"))
                .unwrap(),
            Some(3)
        );
        assert_eq!(store.id_of(&tree.0.join("projects")).unwrap(), Some(2));
        assert_eq!(store.id_of(&tree.0.join("missing.txt")).unwrap(), None);
        assert!(store.rows(&[]).unwrap().is_empty());
    }

    #[test]
    fn the_index_survives_reopening() {
        let tree = Tree::new("store-reopen");
        let path = tree.path("files.db");
        Store::open(&path)
            .unwrap()
            .replace(&walked(&tree.0), "kept")
            .unwrap();

        let (names, meta) = Store::open(&path).unwrap().load().unwrap();
        assert_eq!(names.len(), 3);
        assert_eq!(meta.digest, "kept");
    }

    /// A cache from another schema is rebuilt, not migrated.
    #[test]
    fn another_schema_version_is_dropped_and_recreated() {
        let tree = Tree::new("store-version");
        let path = tree.path("files.db");
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE entries(whatever TEXT);
                     INSERT INTO entries VALUES ('old');
                     PRAGMA user_version = 99;",
                )
                .unwrap();
        }

        let mut store = Store::open(&path).unwrap();
        let (names, meta) = store.load().unwrap();
        assert_eq!(names.len(), 0);
        assert_eq!(meta, Meta::default());
        store.replace(&walked(&tree.0), "fresh").unwrap();
        assert_eq!(store.load().unwrap().0.len(), 3);
    }

    #[test]
    fn a_corrupt_file_is_deleted_and_recreated() {
        let tree = Tree::new("store-corrupt");
        let path = tree.path("files.db");
        std::fs::write(&path, vec![0x5a; 8192]).unwrap();

        let mut store = Store::open(&path).unwrap();
        assert_eq!(store.load().unwrap().0.len(), 0);
        store.replace(&walked(&tree.0), "fresh").unwrap();
        assert_eq!(store.load().unwrap().0.len(), 3);
    }

    #[test]
    fn the_folder_is_created_if_missing() {
        let tree = Tree::new("store-folder");
        let path = tree.path("cache/deeper/files.db");
        Store::open(&path).unwrap();
        assert!(path.exists());
    }

    /// Stable across runs, so a restart does not rebuild; different when any
    /// rule changes, so a changed rule does.
    #[test]
    fn the_digest_follows_every_rule() {
        let base = rules(&[PathBuf::from("C:/Users/someone")]);
        assert_eq!(digest(&base), digest(&base.clone()));
        assert!(!digest(&base).is_empty());

        let mut variants = Vec::new();
        let mut changed = base.clone();
        changed.scopes.push(PathBuf::from("D:/"));
        variants.push(changed);
        let mut changed = base.clone();
        changed
            .exclusions
            .push(PathBuf::from("C:/Users/someone/Videos"));
        variants.push(changed);
        let mut changed = base.clone();
        changed.ignore_patterns.push("*.log".into());
        variants.push(changed);
        let mut changed = base.clone();
        changed.use_ignore_files = false;
        variants.push(changed);
        let mut changed = base.clone();
        changed.include_hidden = true;
        variants.push(changed);
        let mut changed = base.clone();
        changed.ceiling = 10;
        variants.push(changed);

        for variant in variants {
            assert_ne!(digest(&base), digest(&variant), "{variant:?}");
        }
    }

    /// Two lists are not one list: `["a b"]` and `["a", "b"]` differ.
    #[test]
    fn the_digest_keeps_list_boundaries() {
        let mut one = rules(&[]);
        one.ignore_patterns = vec!["ab".into()];
        let mut two = rules(&[]);
        two.ignore_patterns = vec!["a".into(), "b".into()];
        assert_ne!(digest(&one), digest(&two));
    }
}
