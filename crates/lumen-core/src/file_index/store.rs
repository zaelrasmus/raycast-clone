//! The entries of the index, on disk (F16).
//!
//! # A record, and a word index beside it
//!
//! Slice 0 measured SQLite's trigram index at 19 ms to collect every match of
//! a million names, against a budget of 4, so the names are still matched in
//! memory: the database is the record of what was indexed, which a start
//! loads in milliseconds instead of walking the disk again, and from which a
//! query fetches the paths of the few rows it found. Beside the record is a
//! word index of the names, for root to ask the disk directly once the
//! ceiling is gone (`docs/FILE-SEARCH.md`, "Decision: B"); it is kept
//! current with every write and not yet read.
//!
//! # Each folder once
//!
//! A path is stored as its folder's row and its own name, and a folder as
//! its parent's row and its name, so a folder's path is written once however
//! many entries are in it: 36 MB for the real 500 000-entry index where a
//! full path a row took 115. A folder no entry leads to -- a scope's own --
//! is a row with no parent, named by its full path. An entry that is a
//! folder has both: its row in `files`, where its name is matched, and its
//! row in `folders`, under the same parent and with the same name, where its
//! time and its contents are.
//!
//! # The word index
//!
//! `words` holds each name's tokens ([`super::tokens`]): its words, whole
//! and in parts. It is contentless -- the tokens are not stored twice, only
//! indexed -- and keyed by the folded name's length above the row id,
//! `(len << 32) | id`, so the index's own order is shortest name first: the
//! order root ranks a tier in, which a query reads from the start and stops
//! at what it needs. `contentless_delete` lets a row be deleted by its key
//! alone, without the tokens it was indexed with. A rename changes the
//! length and so the key; it is a delete and an insert, never an update.
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
//! Every method here speaks of rows by `files`' row id; the names map their
//! entries to them ([`Names::row`]). A rebuild numbers the rows from 1 in
//! the order the walk found them. A change applied in place (slice 2)
//! deletes rows and adds new ones after the last -- `AUTOINCREMENT`, so a
//! deleted row's id is never given to another entry until the next rebuild
//! numbers everything again. An index still installed while its store
//! changed under it then finds each of its rows as it was, or gone. A
//! folder's row is created after its parent's, so its id is the larger.
//!
//! # Changed in place, and versioned
//!
//! Every write that adds or removes rows counts up `version`, and an index
//! remembers the version it was made from. A batch applied to an index of
//! an older version cannot patch its names -- the store has moved on without
//! it -- and loads them from the store instead.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension as _, Transaction, params};

use super::names::{Names, fold};
use super::tokens::tokens;
use super::walk::{Entry, WalkRules, Walked};

/// Stored in SQLite's own `user_version`. Anything else is dropped: version
/// 1 had no folder times and reused row ids, version 2 a full path a row
/// and no word index.
const SCHEMA_VERSION: i32 = 3;

/// Every table of an older version dropped, and version 3's created.
const SCHEMA: &str = r#"
    DROP TABLE IF EXISTS entries;
    DROP TABLE IF EXISTS meta;
    DROP TABLE IF EXISTS words;
    DROP TABLE IF EXISTS files;
    DROP TABLE IF EXISTS folders;
    CREATE TABLE folders (
        id        INTEGER PRIMARY KEY AUTOINCREMENT,
        -- NULL for a folder known by its full path: a scope's own.
        parent_id INTEGER,
        name      TEXT    NOT NULL,
        -- Which scope of the rules, by position, the entries in it are of.
        scope     INTEGER NOT NULL,
        -- The folder's modification time, for the start reconcile.
        modified  INTEGER
    );
    -- A path to its folder, a name at a time.
    CREATE INDEX folders_path ON folders (parent_id, name);
    CREATE TABLE files (
        -- Never reused (see the module documentation).
        id        INTEGER PRIMARY KEY AUTOINCREMENT,
        folder_id INTEGER NOT NULL,
        -- As it is on disk.
        name      TEXT    NOT NULL,
        is_dir    INTEGER NOT NULL,
        -- The name folded as the names in memory fold it, and its length in
        -- bytes: what root ranks by.
        folded    TEXT    NOT NULL,
        len       INTEGER NOT NULL
    );
    -- A path to its entry, for frecency's ids and a folder's contents.
    CREATE INDEX files_path ON files (folder_id, name);
    -- An exact name, or a name's start, one seek a length.
    CREATE INDEX files_folded ON files (len, folded);
    -- The tokens come split and folded, so the tokenizer only cuts at the
    -- spaces between them: no diacritic removed, as the names remove none,
    -- and a combining mark kept inside its token, as the split keeps it.
    CREATE VIRTUAL TABLE words USING fts5(
        tokens,
        content = '',
        contentless_delete = 1,
        prefix = '1 2 3',
        tokenize = "unicode61 remove_diacritics 0 categories 'L* N* Co M*'"
    );
    CREATE TABLE meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );"#;

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
    /// When the last full walk was written, in seconds since the epoch: what
    /// the settings page calls *last updated*. Changes applied in place keep
    /// it, since they are not a pass over everything.
    pub walked_at: Option<u64>,
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
    // Only when there is one, so an index walked before the list existed
    // still matches its rules and is not walked again for nothing.
    if !rules.skips_off.is_empty() {
        for name in &rules.skips_off {
            hash.write(name.as_bytes());
            hash.write(&[VALUE]);
        }
        hash.write(&[LIST]);
    }
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
                .execute_batch(SCHEMA)
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
    ///
    /// The words go in last, in the order of their keys: FTS5 writes the
    /// rows it holds to disk whenever a key comes lower than the one before,
    /// so in the order of the rows -- not of the lengths -- it would write a
    /// segment a few rows long at nearly every insert: 28 s for the real
    /// 500 000-entry index, against 6.3 in order.
    ///
    /// A folder no entry led to is a scope's own: a row with no parent,
    /// named by its full path.
    pub fn replace(&mut self, walked: &Walked, digest: &str) -> Result<()> {
        let version = self.meta()?.version + 1;
        self.connection
            .execute("DELETE FROM meta WHERE key = 'digest'", [])
            .context("clearing the file index's digest")?;

        let transaction = self.connection.transaction()?;
        // The rows are numbered from 1 again: an index that remembers one of
        // the old ones checks its name before trusting it.
        transaction.execute_batch(
            "DELETE FROM files; DELETE FROM folders; DELETE FROM meta;
             INSERT INTO words (words) VALUES ('delete-all');
             DELETE FROM sqlite_sequence WHERE name IN ('files', 'folders');",
        )?;
        let mut keys = Vec::with_capacity(walked.entries.len());
        {
            let mut folders: HashMap<&Path, i64> = HashMap::new();
            let mut folded = String::new();
            // Entries come scope by scope, as many of each as it counted.
            let scopes = walked
                .per_scope
                .iter()
                .enumerate()
                .flat_map(|(scope, &count)| std::iter::repeat_n(scope, count));
            for (index, (entry, scope)) in walked.entries.iter().zip(scopes).enumerate() {
                let parent = entry.path.parent().unwrap_or(Path::new(""));
                let folder = match folders.get(parent) {
                    Some(&folder) => folder,
                    None => {
                        let folder = new_root(&transaction, parent, scope)?;
                        folders.insert(parent, folder);
                        folder
                    }
                };
                let written = write_entry(
                    &transaction,
                    Some(index as i64 + 1),
                    folder,
                    scope,
                    entry,
                    &mut folded,
                )?;
                if let Some(own) = written.folder {
                    folders.insert(&entry.path, own);
                }
                keys.push((written.key, index));
            }
        }
        keys.sort_unstable();
        let mut split = String::new();
        for (key, index) in keys {
            write_words(&transaction, key, &name(&walked.entries[index]), &mut split)?;
        }
        write_meta(
            &transaction,
            &Meta {
                digest: digest.to_string(),
                per_scope: walked.per_scope.clone(),
                truncated: walked.truncated.clone(),
                roots: walked.roots.clone(),
                version,
                walked_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|since| since.as_secs()),
            },
        )?;
        transaction.commit().context("writing the file index")
    }

    /// The names, in entry order, and what was recorded about them.
    pub fn load(&self) -> Result<(Names, Meta)> {
        let mut names = Names::default();
        let mut statement = self
            .connection
            .prepare("SELECT id, name, is_dir FROM files ORDER BY id")?;
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
                "walked_at" => meta.walked_at = value.parse().ok(),
                _ => {}
            }
        }
        Ok(meta)
    }

    /// How many entries there are.
    pub fn count(&self) -> Result<usize> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Every indexed folder and its recorded time: what the start reconcile
    /// compares with the disk. Not the scopes' own, which are no entries.
    ///
    /// One pass in the order of the rows, each folder's path its parent's
    /// and its name, since a parent's row comes first.
    pub fn folders(&self) -> Result<Vec<(PathBuf, Option<i64>)>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, parent_id, name, modified FROM folders ORDER BY id")?;
        let mut rows = statement.query([])?;
        let mut known: HashMap<i64, PathBuf> = HashMap::new();
        let mut folders = Vec::new();
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let parent: Option<i64> = row.get(1)?;
            let name = row.get_ref(2)?.as_str()?;
            let path = match parent {
                None => PathBuf::from(name),
                Some(parent) => match known.get(&parent) {
                    Some(path) => path.join(name),
                    None => folder_path(&self.connection, id, &mut known)?,
                },
            };
            if parent.is_some() {
                folders.push((path.clone(), row.get(3)?));
            }
            known.insert(id, path);
        }
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
            folders: RefCell::default(),
        })
    }

    /// The entries `ids`, by row id, in the order asked for. An id with no
    /// entry is left out.
    pub fn rows(&self, ids: &[u32]) -> Result<Vec<Row>> {
        let mut found = HashMap::with_capacity(ids.len());
        // A query's rows share their folders, so each is looked up once.
        let mut known = HashMap::new();
        // In chunks, under SQLite's limit on the parameters of a statement.
        for chunk in ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let mut statement = self.connection.prepare(&format!(
                "SELECT id, folder_id, name, is_dir FROM files WHERE id IN ({placeholders})"
            ))?;
            let mut rows = statement.query(rusqlite::params_from_iter(
                chunk.iter().map(|&id| i64::from(id)),
            ))?;
            while let Some(row) = rows.next()? {
                let name: String = row.get(2)?;
                let folder = folder_path(&self.connection, row.get(1)?, &mut known)?;
                let row = Row {
                    id: row.get(0)?,
                    path: folder.join(&name),
                    name,
                    is_dir: row.get(3)?,
                };
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
        let Some(folder) = folder_of(&self.connection, parent)? else {
            return Ok(None);
        };
        let id = self
            .connection
            .query_row(
                "SELECT id FROM files WHERE folder_id = ?1 AND name = ?2",
                params![folder, name.to_string_lossy()],
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

/// An entry's name as it is on disk.
fn name(entry: &Entry) -> Cow<'_, str> {
    entry
        .path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default()
}

/// The key of row `id` in the word index: the folded name's length above the
/// row (see "The word index" above).
fn key(len: usize, id: i64) -> i64 {
    ((len as i64) << 32) | id
}

/// A folder known by its full path, of scope `scope`, and its row.
fn new_root(connection: &Connection, path: &Path, scope: usize) -> Result<i64> {
    connection
        .prepare_cached("INSERT INTO folders (parent_id, name, scope) VALUES (NULL, ?1, ?2)")?
        .execute(params![path.to_string_lossy(), scope as i64])?;
    Ok(connection.last_insert_rowid())
}

/// What [`write_entry`] wrote.
struct Written {
    /// The entry's row.
    id: i64,
    /// Its row in `folders`, when it is a folder.
    folder: Option<i64>,
    /// Its key in the word index, whose words are not yet written.
    key: i64,
}

/// Write `entry`, of scope `scope`, into folder `folder` as row `id` -- the
/// next row when `None` -- and into `folders` too when it is one. `folded`
/// is scratch space.
fn write_entry(
    connection: &Connection,
    id: Option<i64>,
    folder: i64,
    scope: usize,
    entry: &Entry,
    folded: &mut String,
) -> Result<Written> {
    let name = name(entry);
    folded.clear();
    fold(&name, folded);
    connection
        .prepare_cached(
            "INSERT INTO files (id, folder_id, name, is_dir, folded, len)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![
            id,
            folder,
            name,
            entry.is_dir,
            &*folded,
            folded.len() as i64
        ])?;
    let id = connection.last_insert_rowid();
    let own = if entry.is_dir {
        connection
            .prepare_cached(
                "INSERT INTO folders (parent_id, name, scope, modified)
                 VALUES (?1, ?2, ?3, ?4)",
            )?
            .execute(params![folder, name, scope as i64, entry.modified])?;
        Some(connection.last_insert_rowid())
    } else {
        None
    };
    Ok(Written {
        id,
        folder: own,
        key: key(folded.len(), id),
    })
}

/// Index `name`'s tokens under `key`. `split` is scratch space.
fn write_words(connection: &Connection, key: i64, name: &str, split: &mut String) -> Result<()> {
    split.clear();
    tokens(name, split);
    connection
        .prepare_cached("INSERT INTO words (rowid, tokens) VALUES (?1, ?2)")?
        .execute(params![key, &*split])?;
    Ok(())
}

/// The row of the indexed folder at `path`: down from the deepest folder
/// known by its full path that holds it, a name at a time.
fn folder_of(connection: &Connection, path: &Path) -> Result<Option<i64>> {
    let mut roots =
        connection.prepare_cached("SELECT id, name FROM folders WHERE parent_id IS NULL")?;
    let mut deepest: Option<(i64, PathBuf)> = None;
    let mut rows = roots.query([])?;
    while let Some(row) = rows.next()? {
        let root = PathBuf::from(row.get_ref(1)?.as_str()?);
        let deeper = deepest
            .as_ref()
            .is_none_or(|(_, other)| root.components().count() > other.components().count());
        if path.starts_with(&root) && deeper {
            deepest = Some((row.get(0)?, root));
        }
    }
    let Some((mut id, root)) = deepest else {
        return Ok(None);
    };
    let mut down =
        connection.prepare_cached("SELECT id FROM folders WHERE parent_id = ?1 AND name = ?2")?;
    for part in path.strip_prefix(&root)?.components() {
        let next = down
            .query_row(params![id, part.as_os_str().to_string_lossy()], |row| {
                row.get(0)
            })
            .optional()?;
        let Some(next) = next else {
            return Ok(None);
        };
        id = next;
    }
    Ok(Some(id))
}

/// The path of folder `id`, through its parents: each read once into
/// `known`, which holds the paths already read.
fn folder_path(
    connection: &Connection,
    id: i64,
    known: &mut HashMap<i64, PathBuf>,
) -> Result<PathBuf> {
    let mut statement =
        connection.prepare_cached("SELECT parent_id, name FROM folders WHERE id = ?1")?;
    let mut below = Vec::new();
    let mut at = id;
    let mut path = loop {
        if let Some(path) = known.get(&at) {
            break path.clone();
        }
        let (parent, name): (Option<i64>, String) =
            statement.query_row(params![at], |row| Ok((row.get(0)?, row.get(1)?)))?;
        match parent {
            Some(parent) => {
                below.push((at, name));
                at = parent;
            }
            None => {
                let path = PathBuf::from(name);
                known.insert(at, path.clone());
                break path;
            }
        }
    };
    for (folder, name) in below.into_iter().rev() {
        path.push(name);
        known.insert(folder, path.clone());
    }
    Ok(path)
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
    if let Some(walked_at) = meta.walked_at {
        insert.execute(params!["walked_at", walked_at.to_string()])?;
    }
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
    /// The folders found by path in this batch, so a subtree that arrived
    /// is not looked up a name at a time for each of its entries. Forgotten
    /// whenever a folder is removed.
    folders: RefCell<HashMap<PathBuf, i64>>,
}

impl Batch<'_> {
    /// The row of the indexed folder at `path`.
    fn folder(&self, path: &Path) -> Result<Option<i64>> {
        if let Some(&id) = self.folders.borrow().get(path) {
            return Ok(Some(id));
        }
        let id = folder_of(&self.transaction, path)?;
        if let Some(id) = id {
            self.folders.borrow_mut().insert(path.to_path_buf(), id);
        }
        Ok(id)
    }

    /// The row in `folders` of `path`, an entry of its parent's.
    fn own_folder(&self, path: &Path) -> Result<Option<i64>> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Ok(None);
        };
        let Some(parent) = self.folder(parent)? else {
            return Ok(None);
        };
        Ok(self
            .transaction
            .prepare_cached("SELECT id FROM folders WHERE parent_id = ?1 AND name = ?2")?
            .query_row(params![parent, name.to_string_lossy()], |row| row.get(0))
            .optional()?)
    }

    /// Whether `path` is an indexed folder: one the rules allowed when it was
    /// last read, so a change in it is applied.
    pub fn is_folder(&self, path: &Path) -> Result<bool> {
        Ok(self.own_folder(path)?.is_some())
    }

    /// The entries directly in `folder`.
    pub fn children(&self, folder: &Path) -> Result<Vec<Child>> {
        let Some(folder) = self.folder(folder)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.transaction.prepare_cached(
            "SELECT files.id, files.name, files.is_dir, folders.scope
             FROM files JOIN folders ON folders.id = files.folder_id
             WHERE files.folder_id = ?1",
        )?;
        let children = statement
            .query_map(params![folder], |row| {
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

    /// Remove row `id` and its words.
    fn forget(&self, id: i64) -> Result<()> {
        let len: Option<i64> = self
            .transaction
            .prepare_cached("DELETE FROM files WHERE id = ?1 RETURNING len")?
            .query_row(params![id], |row| row.get(0))
            .optional()?;
        if let Some(len) = len {
            self.transaction
                .prepare_cached("DELETE FROM words WHERE rowid = ?1")?
                .execute(params![key(len as usize, id)])?;
        }
        Ok(())
    }

    /// Remove `child` of `folder` and, when it is a folder, everything under
    /// it: its rows, their words and the folders of the subtree. The rows
    /// removed, with their scopes.
    pub fn remove(&self, folder: &Path, child: &Child) -> Result<Vec<(u32, usize)>> {
        let mut removed = vec![(child.id, child.scope)];
        self.forget(i64::from(child.id))?;
        if !child.is_dir {
            return Ok(removed);
        }
        let Some(top) = self.own_folder(&folder.join(&child.name))? else {
            return Ok(removed);
        };
        let subtree = self
            .transaction
            .prepare_cached(
                "WITH RECURSIVE below (id) AS (
                     VALUES (?1)
                     UNION ALL
                     SELECT folders.id FROM folders JOIN below ON folders.parent_id = below.id
                 )
                 SELECT folders.id, folders.scope FROM below JOIN folders USING (id)",
            )?
            .query_map(params![top], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? as usize))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (inner, scope) in subtree {
            let rows = self
                .transaction
                .prepare_cached("SELECT id FROM files WHERE folder_id = ?1")?
                .query_map(params![inner], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for row in rows {
                self.forget(row)?;
                removed.push((row as u32, scope));
            }
            self.transaction
                .prepare_cached("DELETE FROM folders WHERE id = ?1")?
                .execute(params![inner])?;
        }
        self.folders.borrow_mut().clear();
        Ok(removed)
    }

    /// Add `entry`, of scope `scope`, and give its new row.
    pub fn insert(&self, scope: usize, entry: &Entry) -> Result<u32> {
        let parent = entry.path.parent().unwrap_or(Path::new(""));
        let folder = match self.folder(parent)? {
            Some(folder) => folder,
            // A scope's own folder, when nothing was in it before.
            None => {
                let folder = new_root(&self.transaction, parent, scope)?;
                self.folders
                    .borrow_mut()
                    .insert(parent.to_path_buf(), folder);
                folder
            }
        };
        let mut scratch = String::new();
        let written = write_entry(&self.transaction, None, folder, scope, entry, &mut scratch)?;
        write_words(&self.transaction, written.key, &name(entry), &mut scratch)?;
        if let Some(own) = written.folder {
            self.folders.borrow_mut().insert(entry.path.clone(), own);
        }
        u32::try_from(written.id).context("the file index ran out of row ids; rebuild it")
    }

    /// Record the time `folder` had when it was read again.
    pub fn set_modified(&self, folder: &Path, modified: Option<i64>) -> Result<()> {
        if let Some(own) = self.own_folder(folder)? {
            self.transaction
                .prepare_cached("UPDATE folders SET modified = ?2 WHERE id = ?1")?
                .execute(params![own, modified])?;
        }
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
    use super::super::update;
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

        let mut meta = meta;
        assert!(meta.walked_at.is_some_and(|at| at > 1_700_000_000));
        meta.walked_at = None;
        assert_eq!(
            meta,
            Meta {
                digest: "digest-1".into(),
                per_scope: vec![3, 0],
                truncated: Some(tree.0.join("second")),
                roots: vec![Some(5), None],
                version: 1,
                walked_at: None,
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
        changed.skips_off.push("node_modules".into());
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

    /// Two scopes, with folders two deep in the first.
    fn deeper(root: &Path) -> Walked {
        let (home, drive) = (root.join("home"), root.join("drive"));
        let entry = |path: PathBuf, modified: Option<i64>| Entry {
            is_dir: modified.is_some(),
            modified,
            path,
        };
        Walked {
            entries: vec![
                entry(home.join("a"), Some(1)),
                entry(home.join("a").join("b"), Some(2)),
                entry(home.join("a").join("b").join("JavaScript Notes.md"), None),
                entry(home.join("top.txt"), None),
                entry(drive.join("GitHub"), Some(3)),
                entry(drive.join("GitHub").join("es2024.js"), None),
            ],
            per_scope: vec![4, 2],
            truncated: None,
            roots: vec![Some(10), Some(20)],
        }
    }

    fn scalar(store: &Store, sql: &str) -> i64 {
        store
            .connection
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }

    /// The rows the word index finds for `query`, in its order, as each
    /// row's id and the length its key carries.
    fn indexed(store: &Store, query: &str) -> Vec<(u32, i64)> {
        let mut statement = store
            .connection
            .prepare("SELECT rowid FROM words WHERE words MATCH ?1")
            .unwrap();
        statement
            .query_map([query], |row| row.get::<_, i64>(0))
            .unwrap()
            .map(|key| {
                let key = key.unwrap();
                ((key & 0xFFFF_FFFF) as u32, key >> 32)
            })
            .collect()
    }

    #[test]
    fn a_version_2_index_is_dropped_and_rebuilt_as_version_3() {
        let tree = Tree::new("store-version-2");
        let path = tree.path("files.db");
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE entries (
                         id INTEGER PRIMARY KEY AUTOINCREMENT, scope INTEGER NOT NULL,
                         parent TEXT NOT NULL, name TEXT NOT NULL,
                         is_dir INTEGER NOT NULL, modified INTEGER);
                     CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     INSERT INTO entries VALUES (1, 0, 'C:\\old', 'kept.txt', 0, NULL);
                     INSERT INTO meta VALUES ('digest', 'old');
                     PRAGMA user_version = 2;",
                )
                .unwrap();
        }

        let mut store = Store::open(&path).unwrap();
        assert_eq!(scalar(&store, "PRAGMA user_version"), 3);
        assert_eq!(
            scalar(
                &store,
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'entries'"
            ),
            0
        );
        let (names, meta) = store.load().unwrap();
        assert_eq!(names.len(), 0);
        assert_eq!(meta, Meta::default());
        store.replace(&walked(&tree.0), "fresh").unwrap();
        assert_eq!(store.load().unwrap().0.len(), 3);
    }

    #[test]
    fn each_folder_is_stored_once_and_paths_are_rebuilt_through_it() {
        let tree = Tree::new("store-folders");
        let mut store = Store::open(&tree.path("files.db")).unwrap();
        store.replace(&deeper(&tree.0), "d").unwrap();
        let (home, drive) = (tree.0.join("home"), tree.0.join("drive"));

        // Two scopes' roots and the three folders in them.
        assert_eq!(scalar(&store, "SELECT COUNT(*) FROM folders"), 5);
        assert_eq!(
            scalar(
                &store,
                "SELECT COUNT(*) FROM folders WHERE parent_id IS NULL"
            ),
            2
        );
        let paths = store
            .rows(&[3, 6, 4])
            .unwrap()
            .into_iter()
            .map(|row| row.path)
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                home.join("a").join("b").join("JavaScript Notes.md"),
                drive.join("GitHub").join("es2024.js"),
                home.join("top.txt"),
            ]
        );

        let mut folders = store.folders().unwrap();
        folders.sort();
        let mut expected = vec![
            (home.join("a"), Some(1)),
            (home.join("a").join("b"), Some(2)),
            (drive.join("GitHub"), Some(3)),
        ];
        expected.sort();
        assert_eq!(folders, expected);

        let deep = home.join("a").join("b").join("JavaScript Notes.md");
        assert_eq!(store.id_of(&deep).unwrap(), Some(3));
        assert_eq!(store.id_of(&drive.join("GitHub")).unwrap(), Some(5));
        assert_eq!(store.id_of(&home.join("b")).unwrap(), None);
        assert_eq!(store.count().unwrap(), 6);
    }

    /// Whole words and their parts, each keyed by the folded name's length
    /// and the row.
    #[test]
    fn every_name_is_indexed_by_its_whole_words_and_their_parts() {
        let tree = Tree::new("store-words");
        let mut store = Store::open(&tree.path("files.db")).unwrap();
        store.replace(&deeper(&tree.0), "d").unwrap();

        let notes = "javascript notes.md".len() as i64;
        for word in ["javascript", "java", "script", "notes", "md"] {
            assert_eq!(indexed(&store, word), [(3, notes)], "{word}");
        }
        for word in ["github", "git", "hub"] {
            assert_eq!(indexed(&store, word), [(5, 6)], "{word}");
        }
        for word in ["es2024", "es", "2024"] {
            assert_eq!(indexed(&store, word), [(6, 9)], "{word}");
        }
        assert!(indexed(&store, "ascript").is_empty());
        // The index's order is the length's, whatever the rows' order.
        assert_eq!(indexed(&store, "j*"), [(6, 9), (3, notes)]);
    }

    /// A store of `tree`'s rules, built by a walk.
    fn kept(tree: &Tree) -> Store {
        let mut store = Store::open(&tree.path(".files.db")).unwrap();
        store
            .replace(&super::super::walk::walk(&tree.rules()), "d")
            .unwrap();
        store
    }

    /// Apply the changes at `changed`, relative to `tree`, as an update does.
    fn change(store: &mut Store, tree: &Tree, changed: &[&str]) {
        let rules = tree.rules();
        let changed = changed
            .iter()
            .map(|path| tree.path(path))
            .collect::<Vec<_>>();
        let mut meta = store.meta().unwrap();
        let mut count = store.count().unwrap();
        let batch = store.batch().unwrap();
        let folders = update::folders(&changed).unwrap();
        update::apply(&batch, &rules, folders, &mut meta, &mut count).unwrap();
        batch.finish(&meta).unwrap();
    }

    /// What the word index finds by a token starting with `word`, as paths
    /// relative to `tree`, `/`-separated.
    fn by_word(store: &Store, tree: &Tree, word: &str) -> Vec<String> {
        let ids = indexed(store, &format!("\"{word}\"*"))
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        store
            .rows(&ids)
            .unwrap()
            .into_iter()
            .map(|row| {
                row.path
                    .strip_prefix(&tree.0)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }

    #[test]
    fn an_update_indexes_what_arrived_by_its_words() {
        let tree = Tree::new("store-update-add");
        tree.file("notes.md");
        let mut store = kept(&tree);

        tree.file("projects/AnnualReport2024.pdf");
        change(&mut store, &tree, &["projects"]);

        for word in ["annualreport", "report", "2024"] {
            assert_eq!(
                by_word(&store, &tree, word),
                ["projects/AnnualReport2024.pdf"],
                "{word}"
            );
        }
        assert_eq!(by_word(&store, &tree, "projects"), ["projects"]);
        assert_eq!(scalar(&store, "SELECT COUNT(*) FROM files"), 3);
    }

    #[test]
    fn an_update_removes_a_deleted_file_from_the_words() {
        let tree = Tree::new("store-update-delete");
        tree.file("notes.md").file("plan.md");
        let mut store = kept(&tree);
        assert_eq!(by_word(&store, &tree, "plan"), ["plan.md"]);

        std::fs::remove_file(tree.path("plan.md")).unwrap();
        change(&mut store, &tree, &["plan.md"]);

        assert!(by_word(&store, &tree, "plan").is_empty());
        assert_eq!(by_word(&store, &tree, "md"), ["notes.md"]);
    }

    #[test]
    fn deleting_a_folder_removes_its_subtree_from_every_table() {
        let tree = Tree::new("store-update-subtree");
        tree.file("keep.txt")
            .file("old/one.txt")
            .file("old/deeper/two.txt")
            .file("older/three.txt");
        let mut store = kept(&tree);

        std::fs::remove_dir_all(tree.path("old")).unwrap();
        change(&mut store, &tree, &["old"]);

        for word in ["one", "two", "deeper"] {
            assert!(by_word(&store, &tree, word).is_empty(), "{word}");
        }
        // A sibling whose name starts with the deleted one's is not under it.
        assert_eq!(by_word(&store, &tree, "old"), ["older"]);
        assert_eq!(by_word(&store, &tree, "three"), ["older/three.txt"]);
        assert_eq!(
            scalar(
                &store,
                "SELECT COUNT(*) FROM folders WHERE name IN ('old', 'deeper')"
            ),
            0
        );
        assert_eq!(scalar(&store, "SELECT COUNT(*) FROM files"), 3);
    }

    /// A rename changes the folded length, and with it the word index's key:
    /// the old key goes with the old words.
    #[test]
    fn a_renamed_file_is_found_by_its_new_words_and_never_by_its_old() {
        let tree = Tree::new("store-update-rename");
        tree.file("draft.md");
        let mut store = kept(&tree);

        std::fs::rename(tree.path("draft.md"), tree.path("FinalReport.md")).unwrap();
        change(&mut store, &tree, &["draft.md", "FinalReport.md"]);

        assert!(by_word(&store, &tree, "draft").is_empty());
        for word in ["finalreport", "final", "report", "md"] {
            assert_eq!(by_word(&store, &tree, word), ["FinalReport.md"], "{word}");
        }
        let id = store.id_of(&tree.path("FinalReport.md")).unwrap().unwrap();
        assert_eq!(indexed(&store, "md"), [(id, "finalreport.md".len() as i64)]);
    }

    /// Not a check: the size and build time of a real index in this layout,
    /// for `docs/FILE-SEARCH.md` ("Step c2a"). Reads a copy of a version 2
    /// index at `LUMEN_V2_INDEX` and builds it again at `LUMEN_V3_INDEX`
    /// through [`Store::replace`], as a rebuild writes it.
    #[test]
    #[ignore = "measures a real index named by LUMEN_V2_INDEX"]
    fn a_real_index_is_measured_in_this_layout() {
        let from = std::env::var("LUMEN_V2_INDEX").expect("LUMEN_V2_INDEX");
        let to = PathBuf::from(std::env::var("LUMEN_V3_INDEX").expect("LUMEN_V3_INDEX"));
        let old =
            Connection::open_with_flags(&from, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut walked = Walked::default();
        let mut statement = old
            .prepare("SELECT scope, parent, name, is_dir, modified FROM entries ORDER BY scope, id")
            .unwrap();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let scope = row.get::<_, i64>(0).unwrap() as usize;
            if walked.per_scope.len() <= scope {
                walked.per_scope.resize(scope + 1, 0);
            }
            walked.per_scope[scope] += 1;
            walked.entries.push(Entry {
                path: Path::new(&row.get::<_, String>(1).unwrap())
                    .join(row.get::<_, String>(2).unwrap()),
                is_dir: row.get(3).unwrap(),
                modified: row.get(4).unwrap(),
            });
        }

        let _ = std::fs::remove_file(&to);
        let mut store = Store::open(&to).unwrap();
        let start = std::time::Instant::now();
        store.replace(&walked, "measured").unwrap();
        let built = start.elapsed();
        let bytes = std::fs::metadata(&to).unwrap().len();
        let start = std::time::Instant::now();
        let (names, _) = store.load().unwrap();
        let loaded = start.elapsed();
        assert_eq!(names.len(), walked.entries.len());
        eprintln!(
            "{} entries, {} folders: {:.1} MB, built in {:.2} s, loaded in {} ms",
            names.len(),
            scalar(&store, "SELECT COUNT(*) FROM folders"),
            bytes as f64 / 1_048_576.0,
            built.as_secs_f64(),
            loaded.as_millis()
        );
    }

    #[test]
    fn a_renamed_folder_takes_its_subtree_paths_with_it() {
        let tree = Tree::new("store-update-rename-folder");
        tree.file("work/inner/report.txt");
        let mut store = kept(&tree);

        std::fs::rename(tree.path("work"), tree.path("archive")).unwrap();
        change(&mut store, &tree, &["work", "archive"]);

        assert_eq!(
            by_word(&store, &tree, "report"),
            ["archive/inner/report.txt"]
        );
        assert!(by_word(&store, &tree, "work").is_empty());
        let mut folders = store
            .folders()
            .unwrap()
            .into_iter()
            .map(|(path, _)| path)
            .collect::<Vec<_>>();
        folders.sort();
        assert_eq!(
            folders,
            [tree.path("archive"), tree.path("archive").join("inner")]
        );
    }
}
