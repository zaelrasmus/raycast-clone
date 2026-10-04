//! The entries of the index, on disk (F16).
//!
//! # A record, and a word index beside it
//!
//! Slice 0 measured SQLite's trigram index at 19 ms to collect every match of
//! a million names, against a budget of 4, so the names of a hot layer are
//! matched in memory ([`Store::hot`]): the database is the record of what
//! was indexed, which a start loads in milliseconds instead of walking the
//! disk again, and from which a query fetches the paths of the few rows it
//! found. Beside the record is a word index of the names, so root asks the
//! disk for what the hot layer does not hold, with no ceiling on the index
//! (`docs/FILE-SEARCH.md`, "Decision: B"): kept current with every write,
//! and asked by [`Store::ranked`] and [`Store::substring`].
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
//! # Ranked from disk
//!
//! Root wants the best few matches, not every one, so [`Store::ranked`] asks
//! for them a tier at a time, each in the order it ranks -- the shorter name
//! first, then the row -- and stops each as soon as it has what it needs:
//! exact names and name starts from the `(len, folded)` index, one seek a
//! length; word starts from `words`, whose own order is that one. A common
//! word costs the rows it returns, not the tens of thousands it matches.
//!
//! # How much detail
//!
//! [`IndexDetail`] is the user's trade between disk and what the index finds
//! without a scan. It decides only the word tables -- `words`' detail, and
//! whether `grams`, a trigram index of the folded names, exists -- so
//! changing it rebuilds those from `files` and never walks the disk. The
//! level is recorded in `meta`; a store opened at another one rebuilds its
//! word tables and nothing else ([`Store::set_detail`]). So is the layout of
//! `words` itself ([`WORDS_LAYOUT`]): a store built with another rebuilds
//! `words` the same way at open.
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

use serde::{Deserialize, Serialize};

use super::names::{Kinds, MIN_SUBSTRING, Names, fold, is_exact, word_tier, words};
use super::tokens::{fts_query, matches, tokens};
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
    DROP TABLE IF EXISTS grams;
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
    -- `words` and `grams` are created by the level (`IndexDetail`).
    CREATE TABLE meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );"#;

/// The layout of `words` beside its level, recorded in `meta` as
/// `words_layout`. A store with another -- or none: step c2b's, whose
/// prefixes stopped at 3 -- rebuilds `words` from `files` at open, never a
/// walk. Counted up whenever [`words_table`] changes.
const WORDS_LAYOUT: &str = "2";

/// The word index at `detail`.
///
/// Prefixes up to 4 characters: a longer one makes SQLite gather every
/// match before it can order them, which at `detail=none` is the full list
/// of a common prefix (step c2b's `html` and `inv 2024`, ~30 ms at 2M).
///
/// The tokens come split and folded, so the tokenizer only cuts at the
/// spaces between them: no diacritic removed, as the names remove none, and
/// a combining mark kept inside its token, as the split keeps it.
fn words_table(detail: IndexDetail) -> String {
    format!(
        "CREATE VIRTUAL TABLE words USING fts5(
             tokens,
             content = '',
             contentless_delete = 1,
             prefix = '1 2 3 4',
             detail = {},
             tokenize = \"unicode61 remove_diacritics 0 categories 'L* N* Co M*'\"
         );",
        detail.words_detail()
    )
}

/// The trigram index of the folded names, at [`IndexDetail::Complete`].
/// Keyed as `words` is, so its order too is shortest first. Case-sensitive
/// because the names come folded: the tokenizer's own folding is not
/// `Names`', and would find what the check after it then drops.
const GRAMS_TABLE: &str = "CREATE VIRTUAL TABLE grams USING fts5(
        folded,
        content = '',
        contentless_delete = 1,
        tokenize = 'trigram case_sensitive 1'
    );";

/// How much the index keeps beside its record of the names: what it finds
/// without a scan, for what it costs on disk (`files.index-detail`, F16).
/// No level drops an entry, and every level ranks root's matches the same.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IndexDetail {
    /// `balanced` and a trigram index: a match inside a word is found by
    /// the index rather than a scan. The largest.
    Complete,
    /// The word index with its positions.
    #[default]
    Balanced,
    /// The word index without positions (`detail=none`): the same matches
    /// and the same ranking in a smaller file. With prefixes up to four
    /// letters indexed, as fast as `Balanced` at 2M entries.
    Compact,
}

impl IndexDetail {
    pub const ALL: [IndexDetail; 3] = [Self::Complete, Self::Balanced, Self::Compact];

    /// As written in `config.json` and in `meta`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Balanced => "balanced",
            Self::Compact => "compact",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.as_str() == text)
    }

    fn words_detail(self) -> &'static str {
        match self {
            Self::Complete | Self::Balanced => "full",
            Self::Compact => "none",
        }
    }

    fn grams(self) -> bool {
        self == Self::Complete
    }
}

/// What was indexed besides the entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    /// The [`digest`] of the rules the entries were walked with; empty when
    /// no walk has finished.
    pub digest: String,
    /// As [`Walked::per_scope`].
    pub per_scope: Vec<usize>,
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

/// One entry a ranked query found, best first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedRow {
    /// The entry's row id.
    pub id: u32,
    pub path: PathBuf,
    /// As it is on disk, not folded.
    pub name: String,
    pub is_dir: bool,
    /// As [`super::Match::tier`]: 4 exact, 3 the name's start, 2 a word's
    /// start, 1 inside a word.
    pub tier: u8,
}

/// The names a start holds in memory ([`Store::hot`]).
#[derive(Debug)]
pub struct Hot {
    pub names: Names,
    /// Every entry is in `names`: root need never ask the disk.
    pub complete: bool,
    /// How many entries the store holds, in `names` or not.
    pub entries: usize,
}

/// The entries, in SQLite.
pub struct Store {
    connection: Connection,
    /// The level the word tables are built at.
    detail: IndexDetail,
    /// How many rows the ranked queries have read from `files` by id: what
    /// shows a tier stopped where it should.
    fetched: std::cell::Cell<u64>,
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
    // The rules once had a ceiling, written here; an index walked under it
    // may have been cut short, so its digest matches no rules today and it
    // is walked again, once, to its end.
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
    /// nothing when what is there cannot be read. At the level it was built
    /// at; a new one is [`IndexDetail::Balanced`].
    pub fn open(path: &Path) -> Result<Store> {
        Self::open_at(path, None)
    }

    /// [`Self::open`], with the word tables at `detail`: rebuilt from `files`
    /// when they were built at another level (see "How much detail" above).
    pub fn open_with(path: &Path, detail: IndexDetail) -> Result<Store> {
        Self::open_at(path, Some(detail))
    }

    fn open_at(path: &Path, detail: Option<IndexDetail>) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        match Self::open_as_is(path, detail) {
            Ok(store) => Ok(store),
            Err(error) => {
                log::warn!(
                    "rebuilding the unreadable file index at {}: {error:#}",
                    path.display()
                );
                std::fs::remove_file(path)
                    .with_context(|| format!("removing {}", path.display()))?;
                Self::open_as_is(path, detail)
            }
        }
    }

    fn open_as_is(path: &Path, detail: Option<IndexDetail>) -> Result<Store> {
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
                .execute_batch(&words_table(IndexDetail::Balanced))
                .context("creating the file index's words")?;
            record_detail(&connection, IndexDetail::Balanced.as_str())?;
            record_words_layout(&connection)?;
            connection
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .context("recording the file index's schema version")?;
        }
        let recorded: Option<String> = connection
            .query_row("SELECT value FROM meta WHERE key = 'detail'", [], |row| {
                row.get(0)
            })
            .optional()
            .context("reading the file index's level")?;
        // Step c2a recorded none, and built `balanced`'s words. Anything not a
        // level is a change of level cut short: neither table can be trusted.
        let recorded = match recorded {
            None => Some(IndexDetail::Balanced),
            Some(text) => IndexDetail::parse(&text),
        };
        let layout: Option<String> = connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'words_layout'",
                [],
                |row| row.get(0),
            )
            .optional()
            .context("reading the file index's word layout")?;
        let mut store = Store {
            connection,
            detail: recorded.unwrap_or_default(),
            fetched: std::cell::Cell::new(0),
        };
        match (recorded, detail) {
            (None, wanted) => store.build_word_tables(None, wanted.unwrap_or_default(), true)?,
            (Some(recorded), wanted) if layout.as_deref() != Some(WORDS_LAYOUT) => {
                store.build_word_tables(Some(recorded), wanted.unwrap_or(recorded), true)?
            }
            (Some(recorded), Some(wanted)) if recorded != wanted => store.set_detail(wanted)?,
            _ => {}
        }
        Ok(store)
    }

    /// The level the word tables are built at.
    pub fn detail(&self) -> IndexDetail {
        self.detail
    }

    /// Build the word tables at `detail`, from `files`: what changing
    /// `files.index-detail` costs. Never a walk -- the entries, their rows,
    /// [`Meta`] and its `walked_at` are as they were -- and only the table
    /// whose detail changes: `balanced` to `complete` adds `grams` and
    /// leaves `words` alone. Seconds for the real index, so a caller runs it
    /// off the UI thread.
    pub fn set_detail(&mut self, detail: IndexDetail) -> Result<()> {
        if detail == self.detail {
            return Ok(());
        }
        self.build_word_tables(Some(self.detail), detail, false)
    }

    /// Make the word tables `to`'s, from `from`'s -- or from nothing known,
    /// when `from` is `None`, rebuilding both. `words` is rebuilt as well
    /// when `relayout`: built with another [`WORDS_LAYOUT`].
    ///
    /// As a rebuild records its digest, the level is marked as changing
    /// first and written last: there is no journal (see "A cache, and
    /// treated like one" above), so a change cut short leaves a mark that is
    /// no level, and the next open builds both tables again.
    fn build_word_tables(
        &mut self,
        from: Option<IndexDetail>,
        to: IndexDetail,
        relayout: bool,
    ) -> Result<()> {
        let started = std::time::Instant::now();
        let words = relayout || from.is_none_or(|from| from.words_detail() != to.words_detail());
        let grams = to.grams() && from.is_none_or(|from| !from.grams());
        record_detail(&self.connection, "changing")?;
        let transaction = self.connection.transaction()?;
        if words {
            transaction.execute_batch("DROP TABLE IF EXISTS words;")?;
            transaction.execute_batch(&words_table(to))?;
        }
        if !to.grams() || grams {
            transaction.execute_batch("DROP TABLE IF EXISTS grams;")?;
        }
        if grams {
            transaction.execute_batch(GRAMS_TABLE)?;
        }
        if words || grams {
            // In the order of the keys, as `replace` writes them, and for the
            // same reason.
            let mut rows = transaction
                .prepare("SELECT id, len, name FROM files")?
                .query_map([], |row| {
                    Ok((
                        key(row.get::<_, i64>(1)? as usize, row.get(0)?),
                        row.get(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<(i64, String)>>>()?;
            rows.sort_unstable_by_key(|(key, _)| *key);
            let mut scratch = String::new();
            for (key, name) in &rows {
                if words {
                    write_words(&transaction, *key, name, &mut scratch)?;
                }
                if grams {
                    write_grams(&transaction, *key, name, &mut scratch)?;
                }
            }
        }
        if words {
            record_words_layout(&transaction)?;
        }
        record_detail(&transaction, to.as_str())?;
        transaction
            .commit()
            .context("writing the file index's words")?;
        self.detail = to;
        log::info!(
            "built the file index's words at {} in {} ms",
            to.as_str(),
            started.elapsed().as_millis()
        );
        Ok(())
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
        // the old ones checks its name before trusting it. The level is not
        // the walk's, and stays.
        transaction.execute_batch(
            "DELETE FROM files; DELETE FROM folders;
             DELETE FROM meta WHERE key NOT IN ('detail', 'words_layout');
             INSERT INTO words (words) VALUES ('delete-all');
             DELETE FROM sqlite_sequence WHERE name IN ('files', 'folders');",
        )?;
        if self.detail.grams() {
            transaction.execute_batch("INSERT INTO grams (grams) VALUES ('delete-all');")?;
        }
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
        for &(key, index) in &keys {
            write_words(&transaction, key, &name(&walked.entries[index]), &mut split)?;
        }
        if self.detail.grams() {
            for &(key, index) in &keys {
                write_grams(&transaction, key, &name(&walked.entries[index]), &mut split)?;
            }
        }
        write_meta(
            &transaction,
            &Meta {
                digest: digest.to_string(),
                per_scope: walked.per_scope.clone(),
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

    /// The hot layer: the names of the entries most worth matching in
    /// memory, within `budget` bytes as [`Names::cost`] counts them, in
    /// entry order (`docs/FILE-SEARCH.md`, "Decision: B").
    ///
    /// Taken in one order until the next does not fit: the rows `boosted` --
    /// what was used -- and the names shorter than [`MIN_SUBSTRING`], which
    /// a short query in root searches and nothing else reaches, both
    /// whatever the budget; then folders, the shallowest first; then files,
    /// the shallowest first; within each, by row. A folder is where a user
    /// goes from, and a shallow entry what they name; a file five folders
    /// down is the one they search for by its word.
    ///
    /// One pass over `files`, which reads every folded name as it is stored
    /// -- not folded again -- and keeps them until the order is known; then
    /// the ones taken are copied out and the rest given back. On the real
    /// 500 000 that is 175 ms, where reading the order first and the names
    /// taken in a second pass was 280: every row is read either way, and
    /// stepping through them is the cost. A folder's depth comes from
    /// `folders`, whose rows come after their parents'.
    pub fn hot(&self, boosted: &[u32], budget: usize) -> Result<Hot> {
        let _read = self.read()?;
        let mut depths: HashMap<i64, u32> = HashMap::new();
        {
            let mut statement = self
                .connection
                .prepare("SELECT id, parent_id FROM folders ORDER BY id")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let parent: Option<i64> = row.get(1)?;
                let depth = parent.map_or(0, |parent| depths.get(&parent).map_or(0, |d| d + 1));
                depths.insert(row.get(0)?, depth);
            }
        }

        let mut boosted = boosted.to_vec();
        boosted.sort_unstable();
        let mut every = Names::default();
        // Each entry's place in the order, as one number: the class in the
        // top two bits, the depth below them, the entry in the low 32.
        let mut order: Vec<u64> = Vec::new();
        {
            let mut statement = self
                .connection
                .prepare("SELECT id, folder_id, is_dir, folded FROM files ORDER BY id")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let id: u32 = row.get(0)?;
                let is_dir: bool = row.get(2)?;
                let folded = row.get_ref(3)?.as_str()?;
                // Three characters are at most twelve bytes.
                let class: u64 = if boosted.binary_search(&id).is_ok() {
                    0
                } else if folded.len() < 12 && folded.chars().count() < MIN_SUBSTRING {
                    1
                } else if is_dir {
                    2
                } else {
                    3
                };
                let depth = depths
                    .get(&row.get::<_, i64>(1)?)
                    .map_or(0, |depth| depth + 1)
                    .min((1 << 30) - 1);
                let entry = every.len() as u64;
                every.push_folded(id, folded, is_dir);
                order.push(class << 62 | u64::from(depth) << 32 | entry);
            }
        }
        drop(depths);
        order.sort_unstable();

        let entries = order.len();
        let mut spent = 0;
        let mut taken = entries;
        for (at, &key) in order.iter().enumerate() {
            let cost = Names::cost(every.name_len(key as u32));
            if key >> 62 > 1 && spent + cost > budget {
                taken = at;
                break;
            }
            spent += cost;
        }
        if taken == entries {
            every.shrink();
            return Ok(Hot {
                names: every,
                complete: true,
                entries,
            });
        }
        let mut chosen = order[..taken]
            .iter()
            .map(|&key| key as u32)
            .collect::<Vec<_>>();
        drop(order);
        chosen.sort_unstable();
        Ok(Hot {
            names: every.only(&chosen),
            complete: false,
            entries,
        })
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
            grams: self.detail.grams(),
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

    /// The best `limit` entries for `query`, best first, as root ranks them:
    /// exact names, then names that start with the longest word, then names
    /// with a word that starts with it; within a tier the shorter name, then
    /// the row (see "Ranked from disk" above). Frecency is not here: the
    /// caller merges it.
    ///
    /// Every word must start one of a name's tokens and be in its folded
    /// name ([`super::tokens`]), so a match inside a word is not found:
    /// that is [`Self::substring`]'s. `kinds` and `filter` -- asked with the
    /// folded name and whether it is a folder, as [`Names::find_where`]'s
    /// `accept` is -- are applied as a tier is read, which goes on until it
    /// has `limit` they took or has none left: the best `limit` of a type,
    /// not what was left of the best `limit` of all.
    ///
    /// A query of any length is answered; whether a short one is asked is
    /// root's choice.
    pub fn ranked(
        &self,
        query: &str,
        kinds: Kinds,
        limit: usize,
        filter: &dyn Fn(&str, bool) -> bool,
    ) -> Result<Vec<RankedRow>> {
        let Some(query) = Query::new(query) else {
            return Ok(Vec::new());
        };
        if limit == 0 {
            return Ok(Vec::new());
        }
        let _read = self.read()?;
        let accepts = |row: &Fetched, split: &mut String| {
            if kinds == Kinds::Folders && !row.is_dir {
                return false;
            }
            if !filter(&row.folded, row.is_dir) {
                return false;
            }
            split.clear();
            tokens(&row.name, split);
            matches(&query.words, &row.folded, split)
        };
        let mut found: Vec<(Fetched, u8)> = Vec::new();
        let mut split = String::new();
        let longest = query.longest();
        let max_len = self.max_len()?;

        // Each tier keeps only the rows whose tier it is: one of a better
        // tier was taken -- or turned down -- by that tier, which was read
        // to its end, or this one would not be read at all.
        let mut take = |id: u32, tier: u8, found: &mut Vec<(Fetched, u8)>| -> Result<bool> {
            if let Some(row) = self.fetch(id)?
                && query.tier(&row.folded) == tier
                && accepts(&row, &mut split)
            {
                found.push((row, tier));
            }
            Ok(found.len() == limit)
        };

        // Exact: the whole query, or it and an extension.
        let whole = query.whole.as_str();
        let (stem, past) = (format!("{whole}."), format!("{whole}/"));
        let mut full = self.by_length(
            whole.len()..=whole.len(),
            whole,
            &format!("{whole}\0"),
            &|_| true,
            &mut |id| take(id, 4, &mut found),
        )?;
        if !full {
            full = self.by_length(
                stem.len()..=max_len,
                &stem,
                &past,
                &|folded| is_exact(folded, whole),
                &mut |id| take(id, 4, &mut found),
            )?;
        }

        // The name's start. The other words are checked on the folded name
        // the index holds before a row is read: `inv 2024` at two million
        // names read every one that starts with `2024` to turn down those
        // without `inv`, 18 ms of it.
        if !full {
            full = self.by_length(
                longest.len()..=max_len,
                longest,
                &format!("{longest}{}", char::MAX),
                &|folded| {
                    !is_exact(folded, whole)
                        && query
                            .words
                            .iter()
                            .all(|word| folded.contains(word.as_str()))
                },
                &mut |id| take(id, 3, &mut found),
            )?;
        }

        // A word's start, from the word index in its own order. A query of
        // no letter or digit has no word to ask it about.
        if !full && let Some(fts) = fts_query(&query.words) {
            let mut statement = self
                .connection
                .prepare_cached("SELECT rowid FROM words WHERE words MATCH ?1 ORDER BY rowid")?;
            let mut rows = statement.query(params![fts])?;
            while let Some(row) = rows.next()? {
                let id = (row.get::<_, i64>(0)? & 0xFFFF_FFFF) as u32;
                if take(id, 2, &mut found)? {
                    break;
                }
            }
        }
        self.complete(found)
    }

    /// The entries that have every word of `query` inside a word of their
    /// name -- what `Names` finds and [`Self::ranked`] cannot -- shortest
    /// first, then by row, at most `limit`: *Search Files*' mid-word
    /// matches, all tier 1. As in `Names`, a word must be
    /// [`MIN_SUBSTRING`] characters to match inside a word, so a query with
    /// none that long has none.
    ///
    /// At [`IndexDetail::Complete`] the trigram index finds them; at the
    /// other levels they are a scan of every folded name, a length at a
    /// time from the `(len, folded)` index, which reads no row it does not
    /// match. Abandoned -- answering nothing -- once `cancelled` says so,
    /// asked before the first row and then every [`SCAN_CHECK`] rows.
    pub fn substring(
        &self,
        query: &str,
        kinds: Kinds,
        limit: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<RankedRow>> {
        let Some(query) = Query::new(query) else {
            return Ok(Vec::new());
        };
        let longest = query.longest();
        if limit == 0 || longest.chars().count() < MIN_SUBSTRING {
            return Ok(Vec::new());
        }
        let _read = self.read()?;
        // Inside a word of the name, and not certainly a word's start: what
        // the folded name alone decides, so a name that only has the query
        // as words -- the tens of thousands of `.html` for `html` -- is
        // never read from `files` to be turned down.
        let inside = |folded: &str| {
            query
                .words
                .iter()
                .all(|word| word_tier(folded, word).is_some())
                && !surely_words(&query.words, folded)
        };
        let mut split = String::new();
        let mut take = |row: Fetched, found: &mut Vec<(Fetched, u8)>| {
            if kinds == Kinds::Folders && !row.is_dir {
                return;
            }
            split.clear();
            tokens(&row.name, &mut split);
            if !matches(&query.words, &row.folded, &split) {
                found.push((row, 1));
            }
        };
        let mut found = Vec::new();
        let mut read = 0usize;
        let mut stop = || {
            read += 1;
            read % SCAN_CHECK == 1 && cancelled()
        };

        if self.detail.grams() {
            // Every word of three characters or more as a phrase of its
            // trigrams; a shorter one is checked on the name.
            let fts = query
                .words
                .iter()
                .filter(|word| word.chars().count() >= MIN_SUBSTRING)
                .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" AND ");
            // The folded name joined in, so a row is fetched whole only once
            // it is a candidate.
            let mut statement = self.connection.prepare_cached(
                "SELECT files.id, files.folded FROM grams
                 JOIN files ON files.id = grams.rowid & 4294967295
                 WHERE grams MATCH ?1 ORDER BY grams.rowid",
            )?;
            let mut rows = statement.query(params![fts])?;
            while let Some(row) = rows.next()? {
                if stop() {
                    return Ok(Vec::new());
                }
                if !inside(row.get_ref(1)?.as_str()?) {
                    continue;
                }
                if let Some(row) = self.fetch(row.get(0)?)? {
                    take(row, &mut found);
                    if found.len() == limit {
                        break;
                    }
                }
            }
            return self.complete(found);
        }

        let mut statement = self
            .connection
            .prepare_cached("SELECT id, folded FROM files WHERE len = ?1")?;
        let mut ids = Vec::new();
        'lengths: for len in longest.len()..=self.max_len()? {
            ids.clear();
            let mut rows = statement.query(params![len as i64])?;
            while let Some(row) = rows.next()? {
                if stop() {
                    return Ok(Vec::new());
                }
                if inside(row.get_ref(1)?.as_str()?) {
                    ids.push(row.get::<_, u32>(0)?);
                }
            }
            ids.sort_unstable();
            for &id in &ids {
                if let Some(row) = self.fetch(id)? {
                    take(row, &mut found);
                    if found.len() == limit {
                        break 'lengths;
                    }
                }
            }
        }
        self.complete(found)
    }

    /// One read transaction for the whole of a query.
    ///
    /// Outside one, every statement is its own: on Windows a lock of the
    /// file and a read of its header for each seek, which made a seek cost
    /// 25 us instead of 4 -- a query's few hundred of them, its whole
    /// budget. Rolled back when dropped, having written nothing.
    fn read(&self) -> Result<Transaction<'_>> {
        self.connection
            .unchecked_transaction()
            .context("starting a read of the file index")
    }

    /// The longest folded name, in bytes: where a scan by length ends.
    fn max_len(&self) -> Result<usize> {
        let len: Option<i64> = self
            .connection
            .prepare_cached("SELECT MAX(len) FROM files")?
            .query_row([], |row| row.get(0))?;
        Ok(len.unwrap_or(0) as usize)
    }

    /// Visit, a length at a time from the shortest of `lengths`, the rows
    /// whose folded name is in `[lower, upper)` and that `keep` keeps, by
    /// row within a length, until `visit` says it has enough. Whether it
    /// did.
    ///
    /// One seek a length on `(len, folded)`, reading only the index: within
    /// one length the rows come in the order of their names, so a length's
    /// are sorted by row before any is visited -- all of one length, which
    /// for thousands of the same name is thousands of index entries and
    /// still only `limit` rows read.
    fn by_length(
        &self,
        lengths: std::ops::RangeInclusive<usize>,
        lower: &str,
        upper: &str,
        keep: &dyn Fn(&str) -> bool,
        visit: &mut dyn FnMut(u32) -> Result<bool>,
    ) -> Result<bool> {
        let mut statement = self.connection.prepare_cached(
            "SELECT id, folded FROM files WHERE len = ?1 AND folded >= ?2 AND folded < ?3",
        )?;
        let mut ids = Vec::new();
        for len in lengths {
            ids.clear();
            let mut rows = statement.query(params![len as i64, lower, upper])?;
            while let Some(row) = rows.next()? {
                if keep(row.get_ref(1)?.as_str()?) {
                    ids.push(row.get::<_, u32>(0)?);
                }
            }
            ids.sort_unstable();
            for &id in &ids {
                if visit(id)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Row `id` as a ranked query weighs it.
    fn fetch(&self, id: u32) -> Result<Option<Fetched>> {
        self.fetched.set(self.fetched.get() + 1);
        Ok(self
            .connection
            .prepare_cached("SELECT folder_id, name, is_dir, folded FROM files WHERE id = ?1")?
            .query_row(params![id], |row| {
                Ok(Fetched {
                    id,
                    folder: row.get(0)?,
                    name: row.get(1)?,
                    is_dir: row.get(2)?,
                    folded: row.get(3)?,
                })
            })
            .optional()?)
    }

    /// The rows found, with their paths: each folder looked up once.
    fn complete(&self, found: Vec<(Fetched, u8)>) -> Result<Vec<RankedRow>> {
        let mut known = HashMap::new();
        found
            .into_iter()
            .map(|(row, tier)| {
                let folder = folder_path(&self.connection, row.folder, &mut known)?;
                Ok(RankedRow {
                    id: row.id,
                    path: folder.join(&row.name),
                    name: row.name,
                    is_dir: row.is_dir,
                    tier,
                })
            })
            .collect()
    }

    /// Give back the page cache. The names are in memory once loaded, so the
    /// database holds nothing while the launcher is idle.
    pub fn release_memory(&self) {
        if let Err(error) = self.connection.execute_batch("PRAGMA shrink_memory;") {
            log::warn!("could not release the file index's memory: {error}");
        }
    }
}

/// How many rows a mid-word query reads between asking whether it was
/// cancelled: as `Names`' scan asks, often enough that a superseded one
/// stops within a fraction of a millisecond.
const SCAN_CHECK: usize = 4096;

/// A query's words as the ranked queries use them.
struct Query {
    words: Vec<String>,
    /// The first of the longest words, which decides the tier, as in
    /// `Names`.
    longest: usize,
    /// The words joined by a space: what an exact name is.
    whole: String,
}

impl Query {
    fn new(query: &str) -> Option<Self> {
        let words = words(query);
        let longest = (0..words.len())
            .rev()
            .max_by_key(|&index| words[index].len())?;
        let whole = words.join(" ");
        Some(Self {
            words,
            longest,
            whole,
        })
    }

    fn longest(&self) -> &str {
        &self.words[self.longest]
    }

    /// The tier of a name every word matches: 4 exact, 3 starting with the
    /// longest word, 2 a word start.
    fn tier(&self, folded: &str) -> u8 {
        if is_exact(folded, &self.whole) {
            4
        } else if folded.starts_with(self.longest()) {
            3
        } else {
            2
        }
    }
}

/// Whether every run of every one of `words` is in `folded` where a run of
/// the name starts -- at its start, or after an ASCII character that is
/// neither letter nor digit -- so the name surely matches them as words
/// ([`matches`]) and has no place in [`Store::substring`]. Only ASCII counts
/// as a separator: the one case the folded name decides alone. A run that
/// starts at a capital, or after a separator beyond ASCII, says `false`, and
/// the row is read and checked whole.
fn surely_words(words: &[String], folded: &str) -> bool {
    words.iter().all(|word| {
        super::tokens::runs(word).all(|run| {
            folded.match_indices(run).any(|(at, _)| {
                folded[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|before| before.is_ascii() && !before.is_ascii_alphanumeric())
            })
        })
    })
}

/// A row a ranked query read.
struct Fetched {
    id: u32,
    folder: i64,
    name: String,
    is_dir: bool,
    folded: String,
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

/// Index `name`, folded, by its trigrams under `key`. `folded` is scratch
/// space.
fn write_grams(connection: &Connection, key: i64, name: &str, folded: &mut String) -> Result<()> {
    folded.clear();
    fold(name, folded);
    connection
        .prepare_cached("INSERT INTO grams (rowid, folded) VALUES (?1, ?2)")?
        .execute(params![key, &*folded])?;
    Ok(())
}

/// Record that `words` is built with [`WORDS_LAYOUT`].
fn record_words_layout(connection: &Connection) -> Result<()> {
    connection
        .execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('words_layout', ?1)",
            params![WORDS_LAYOUT],
        )
        .context("recording the file index's word layout")?;
    Ok(())
}

/// Record the word tables' level, or `changing` while they are rebuilt.
fn record_detail(connection: &Connection, detail: &str) -> Result<()> {
    connection
        .execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('detail', ?1)",
            params![detail],
        )
        .context("recording the file index's level")?;
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

/// Write `meta` in place of what was recorded, the digest last. The level
/// and the word layout are not the walk's, and stay.
fn write_meta(transaction: &Transaction<'_>, meta: &Meta) -> Result<()> {
    transaction.execute(
        "DELETE FROM meta WHERE key NOT IN ('detail', 'words_layout')",
        [],
    )?;
    let mut insert = transaction.prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")?;
    let counts = meta
        .per_scope
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",");
    insert.execute(params!["per_scope", counts])?;
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
    /// Whether the store has `grams`, kept as `words` is.
    grams: bool,
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
            let key = key(len as usize, id);
            self.transaction
                .prepare_cached("DELETE FROM words WHERE rowid = ?1")?
                .execute(params![key])?;
            if self.grams {
                self.transaction
                    .prepare_cached("DELETE FROM grams WHERE rowid = ?1")?
                    .execute(params![key])?;
            }
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
        if self.grams {
            write_grams(&self.transaction, written.key, &name(entry), &mut scratch)?;
        }
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
            roots: vec![None],
        };
        store.replace(&fewer, "two").unwrap();

        let (names, meta) = store.load().unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(meta.digest, "two");
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
            roots: vec![Some(10), Some(20)],
        }
    }

    /// The hot layer is a prefix of one order -- the used, the names too
    /// short for a query to reach otherwise, then folders and then files,
    /// each the shallowest first -- cut where the budget ends; the used and
    /// the short are taken whatever the budget. Only when every entry fits
    /// is it complete.
    #[test]
    fn the_hot_layer_is_the_used_then_folders_then_files_by_depth_within_its_budget() {
        let tree = Tree::new("store-hot");
        let mut store = Store::open(&tree.path("files.db")).unwrap();
        // Rows 1 to 6: home/a, home/a/b, home/a/b/JavaScript Notes.md,
        // home/top.txt, drive/GitHub, drive/GitHub/es2024.js.
        let walked = deeper(&tree.0);
        store.replace(&walked, "d").unwrap();
        let cost = |row: u32| {
            let mut folded = String::new();
            fold(&name(&walked.entries[row as usize - 1]), &mut folded);
            Names::cost(folded.len())
        };
        let rows = |hot: &Hot| {
            (0..hot.names.len() as u32)
                .map(|entry| hot.names.row(entry))
                .collect::<Vec<_>>()
        };

        // Used, short, the folder, then the files by depth.
        let order: [u32; 6] = [3, 1, 2, 5, 4, 6];
        let all = order.iter().map(|&row| cost(row)).sum::<usize>();
        let hot = store.hot(&[3], all).unwrap();
        assert!(hot.complete);
        assert_eq!(rows(&hot), [1, 2, 3, 4, 5, 6]);
        assert_eq!(hot.names.bytes(), all);

        for taken in 3..order.len() {
            let budget = order[..taken].iter().map(|&row| cost(row)).sum::<usize>();
            let hot = store.hot(&[3], budget + cost(order[taken]) - 1).unwrap();
            assert!(!hot.complete, "{taken}");
            let mut expected = order[..taken].to_vec();
            expected.sort_unstable();
            assert_eq!(rows(&hot), expected, "{taken}");
            assert!(hot.names.bytes() <= budget, "{taken}");
        }

        let hot = store.hot(&[3], 0).unwrap();
        assert!(!hot.complete);
        assert_eq!(rows(&hot), [1, 2, 3]);
        let hot = store.hot(&[], 0).unwrap();
        assert_eq!(rows(&hot), [1, 2]);
        assert!(hot.names.holds(0, "a", true));
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
        let batch = store.batch().unwrap();
        let folders = update::folders(&changed).unwrap();
        update::apply(&batch, &rules, folders, &mut meta).unwrap();
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

    // Step c2b: ranked from disk.

    /// Names every tier has something to say about; `corpus` adds a few
    /// thousand more made of common words, so a limit cuts every tier.
    const NAMED: [(&str, bool); 34] = [
        ("Invoice.pdf", false),
        ("invoice", true),
        ("invoice.tar.gz", false),
        ("INV", false),
        ("inv-2024.txt", false),
        ("my-invoice_v2.final.pdf", false),
        ("AnnualReport2024.pdf", false),
        ("JavaScript Notes.md", false),
        ("GitHub", true),
        ("github-desktop", true),
        ("Cargo.toml", false),
        (".cargo", true),
        ("cargo", true),
        ("data", true),
        ("Data Sets", true),
        ("metadata.json", false),
        ("reinvent.txt", false),
        ("html", true),
        ("index.html", false),
        ("HTMLParser.js", false),
        ("report.html", false),
        ("myreport.doc", false),
        ("c++ notes", false),
        ("Meeting Notes.md", false),
        ("meeting notes", true),
        ("invoice-b.pdf", false),
        ("invoice-a.pdf", false),
        ("Invoice.PDF", false),
        ("2024", true),
        ("es2024.js", false),
        ("a", false),
        ("in", true),
        ("İstanbul.jpg", false),
        ("Ünïcödé Invoice.txt", false),
    ];

    const COMMON: [&str; 16] = [
        "invoice", "Report", "data", "html", "cargo", "notes", "Java", "Script", "git", "Hub",
        "2024", "final", "in", "a", "meta", "preview",
    ];

    /// `NAMED`, then `count` names of `COMMON`'s words joined by a separator
    /// or run together, every seventh a folder: the same every run.
    fn corpus(count: usize) -> Vec<(String, bool)> {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move |below: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % below as u64) as usize
        };
        let mut names = NAMED
            .iter()
            .map(|&(name, is_dir)| (name.to_string(), is_dir))
            .collect::<Vec<_>>();
        for index in 0..count {
            let separator = ["-", "_", " ", ".", ""][next(5)];
            let words = (0..1 + next(3))
                .map(|_| COMMON[next(COMMON.len())])
                .collect::<Vec<_>>()
                .join(separator);
            let name = match next(4) {
                0 => format!("{words}.pdf"),
                1 => format!("{words}.html"),
                2 => format!("{words} {index}.txt"),
                _ => words,
            };
            names.push((name, index % 7 == 0));
        }
        names
    }

    /// `names` as a walk of one scope finds them, each in a folder of its
    /// own under one of the first folders, so paths go through parents.
    /// None of it exists on disk, so a walk would find nothing.
    fn walked_names(root: &Path, names: &[(String, bool)]) -> Walked {
        let mut entries = Vec::new();
        let mut folders = vec![root.to_path_buf()];
        for (index, (name, is_dir)) in names.iter().enumerate() {
            let own = folders[index % folders.len()].join(format!("f{index}"));
            entries.push(Entry {
                path: own.clone(),
                is_dir: true,
                modified: Some(1),
            });
            let path = own.join(name);
            if *is_dir && folders.len() < 64 {
                folders.push(path.clone());
            }
            entries.push(Entry {
                path,
                is_dir: *is_dir,
                modified: is_dir.then_some(1),
            });
        }
        Walked {
            per_scope: vec![entries.len()],
            entries,
            roots: vec![Some(1)],
        }
    }

    /// A store at `detail`, at `file` in `tree`, of `walked`.
    fn store_of(tree: &Tree, file: &str, detail: IndexDetail, walked: &Walked) -> Store {
        let mut store = Store::open_with(&tree.path(file), detail).unwrap();
        store.replace(walked, "d").unwrap();
        store
    }

    /// Every row of `store` as its id, its name and whether it is a folder.
    fn every_row(store: &Store) -> Vec<(u32, String, bool)> {
        let mut statement = store
            .connection
            .prepare("SELECT id, name, is_dir FROM files ORDER BY id")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    /// Every match of `query` among `rows`, ranked in memory as `Names`
    /// ranks -- the tier from the longest word, then the shorter name, then
    /// the row -- with the word index's matching: what `ranked` must equal.
    fn reference(
        rows: &[(u32, String, bool)],
        query: &str,
        kinds: Kinds,
        limit: usize,
        filter: &dyn Fn(&str, bool) -> bool,
    ) -> Vec<(u32, u8)> {
        let words = words(query);
        if words.is_empty() {
            return Vec::new();
        }
        let longest = (0..words.len())
            .rev()
            .max_by_key(|&index| words[index].len())
            .unwrap();
        let whole = words.join(" ");
        let mut found = rows
            .iter()
            .filter_map(|(id, name, is_dir)| {
                let mut folded = String::new();
                fold(name, &mut folded);
                let mut split = String::new();
                tokens(name, &mut split);
                let kept = (kinds == Kinds::FoldersAndFiles || *is_dir)
                    && filter(&folded, *is_dir)
                    && matches(&words, &folded, &split);
                let tier = if is_exact(&folded, &whole) {
                    4
                } else if folded.starts_with(words[longest].as_str()) {
                    3
                } else {
                    2
                };
                kept.then_some((std::cmp::Reverse(tier), folded.len(), *id))
            })
            .collect::<Vec<_>>();
        found.sort_unstable();
        found.truncate(limit);
        found
            .into_iter()
            .map(|(tier, _, id)| (id, tier.0))
            .collect()
    }

    fn ids_and_tiers(found: &[RankedRow]) -> Vec<(u32, u8)> {
        found.iter().map(|row| (row.id, row.tier)).collect()
    }

    const RANKED_QUERIES: [&str; 24] = [
        "inv",
        "invoice",
        "report",
        "2024",
        "html",
        "data",
        "cargo",
        "javascript",
        "java",
        "script",
        "github",
        "git",
        "hub",
        "inv 2024",
        "meeting notes",
        "notes meeting",
        "c++",
        "a",
        "in",
        "zqxj",
        ".cargo",
        "my-inv",
        "ünïcödé",
        "invoice.pdf",
    ];

    fn any(_: &str, _: bool) -> bool {
        true
    }

    fn pdf(folded: &str, _: bool) -> bool {
        folded.ends_with(".pdf")
    }

    /// Exact names, names without their extension, name starts, word starts,
    /// two words, folders only, a type filter: at every level the same as
    /// ranking every match in memory, at every limit.
    #[test]
    fn ranked_from_disk_equals_ranking_every_match_in_memory() {
        let tree = Tree::new("store-ranked");
        let walked = walked_names(&tree.0, &corpus(3000));
        for detail in IndexDetail::ALL {
            let store = store_of(&tree, &format!("{}.db", detail.as_str()), detail, &walked);
            let rows = every_row(&store);
            for query in RANKED_QUERIES {
                for kinds in [Kinds::FoldersAndFiles, Kinds::Folders] {
                    for filter in [&any as &dyn Fn(&str, bool) -> bool, &pdf] {
                        for limit in [1, 8, 64, 100_000] {
                            let found = store.ranked(query, kinds, limit, filter).unwrap();
                            let expected = reference(&rows, query, kinds, limit, filter);
                            assert_eq!(
                                ids_and_tiers(&found),
                                expected,
                                "{detail:?} {query:?} {kinds:?} {limit}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A row found is whole: its path through its folders, its name as on
    /// disk, its kind.
    #[test]
    fn a_ranked_row_has_its_path_and_its_name_as_on_disk() {
        let tree = Tree::new("store-ranked-rows");
        let mut store = Store::open(&tree.path("files.db")).unwrap();
        store.replace(&deeper(&tree.0), "d").unwrap();

        let found = store
            .ranked("javascript", Kinds::FoldersAndFiles, 8, &any)
            .unwrap();
        assert_eq!(
            found,
            [RankedRow {
                id: 3,
                path: tree
                    .0
                    .join("home")
                    .join("a")
                    .join("b")
                    .join("JavaScript Notes.md"),
                name: "JavaScript Notes.md".into(),
                is_dir: false,
                tier: 3,
            }]
        );
        let found = store.ranked("hub", Kinds::Folders, 8, &any).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, tree.0.join("drive").join("GitHub"));
        assert!(found[0].is_dir);
    }

    #[test]
    fn ties_are_broken_by_length_then_row() {
        let tree = Tree::new("store-ranked-ties");
        let names = [
            "invoice-b.pdf",
            "Invoice.PDF",
            "invoice-a.pdf",
            "invoice.pdf",
            "invoice",
        ]
        .map(|name| (name.to_string(), false));
        let store = store_of(
            &tree,
            "files.db",
            IndexDetail::Balanced,
            &walked_names(&tree.0, &names),
        );
        let found = store
            .ranked("invoice", Kinds::FoldersAndFiles, 8, &any)
            .unwrap()
            .into_iter()
            .map(|row| (row.name, row.tier))
            .collect::<Vec<_>>();
        let expected = [
            ("invoice", 4),
            ("Invoice.PDF", 4),
            ("invoice.pdf", 4),
            ("invoice-b.pdf", 3),
            ("invoice-a.pdf", 3),
        ]
        .map(|(name, tier)| (name.to_string(), tier));
        assert_eq!(found, expected);
    }

    /// Thousands of the same name: each tier reads the rows it returns and
    /// no more.
    #[test]
    fn every_tier_stops_at_the_limit_when_thousands_share_a_name() {
        let tree = Tree::new("store-ranked-limit");
        let mut names = Vec::new();
        for _ in 0..3000 {
            names.push(("index.js".to_string(), false));
            names.push(("old-index.js".to_string(), false));
        }
        let walked = walked_names(&tree.0, &names);
        for detail in IndexDetail::ALL {
            let store = store_of(&tree, &format!("{}.db", detail.as_str()), detail, &walked);
            // Exact, name start, word start past the name starts.
            for (query, tier) in [("index", 4), ("inde", 3), ("old", 3), ("js", 2)] {
                for limit in [8, 64] {
                    let before = store.fetched.get();
                    let found = store
                        .ranked(query, Kinds::FoldersAndFiles, limit, &any)
                        .unwrap();
                    assert_eq!(found.len(), limit, "{detail:?} {query}");
                    assert!(found.iter().all(|row| row.tier == tier), "{query}");
                    assert!(found.windows(2).all(|pair| pair[0].id < pair[1].id));
                    assert_eq!(
                        store.fetched.get() - before,
                        limit as u64,
                        "{detail:?} {query} {limit}"
                    );
                }
            }
        }
    }

    /// Thousands of names that start with the longest word but lack another:
    /// turned down on the folded name in the index, never read.
    #[test]
    fn a_name_start_without_the_other_words_is_never_read() {
        let tree = Tree::new("store-ranked-other-words");
        let mut names = (0..3000)
            .map(|index| (format!("2024 report {index}.pdf"), false))
            .collect::<Vec<_>>();
        names.extend((0..20).map(|index| (format!("2024 invoice {index}.pdf"), false)));
        let walked = walked_names(&tree.0, &names);
        for detail in IndexDetail::ALL {
            let store = store_of(&tree, &format!("{}.db", detail.as_str()), detail, &walked);
            let before = store.fetched.get();
            let found = store
                .ranked("inv 2024", Kinds::FoldersAndFiles, 8, &any)
                .unwrap();
            assert_eq!(found.len(), 8, "{detail:?}");
            assert!(found.iter().all(|row| row.tier == 3), "{detail:?}");
            assert_eq!(store.fetched.get() - before, 8, "{detail:?}");
        }
    }

    /// Typed whole across a capital, or as either part.
    #[test]
    fn javascript_and_github_are_found_whole_and_by_their_parts() {
        let tree = Tree::new("store-ranked-parts");
        let mut store = Store::open(&tree.path("files.db")).unwrap();
        store.replace(&deeper(&tree.0), "d").unwrap();
        let found = |query: &str| {
            store
                .ranked(query, Kinds::FoldersAndFiles, 8, &any)
                .unwrap()
                .into_iter()
                .map(|row| row.name)
                .collect::<Vec<_>>()
        };
        for query in ["javascript", "java", "script", "javascript notes", "scr no"] {
            assert_eq!(found(query), ["JavaScript Notes.md"], "{query}");
        }
        for query in ["github", "git", "hub"] {
            assert_eq!(found(query), ["GitHub"], "{query}");
        }
        assert!(found("ascript").is_empty());
    }

    /// Every match inside a word that `ranked` cannot find, as `Names` finds
    /// it, shortest first: what `substring` must equal.
    fn mid_word(rows: &[(u32, String, bool)], query: &str, kinds: Kinds, limit: usize) -> Vec<u32> {
        let words = words(query);
        if words
            .iter()
            .all(|word| word.chars().count() < MIN_SUBSTRING)
        {
            return Vec::new();
        }
        let mut found = rows
            .iter()
            .filter_map(|(id, name, is_dir)| {
                let mut folded = String::new();
                fold(name, &mut folded);
                let mut split = String::new();
                tokens(name, &mut split);
                ((kinds == Kinds::FoldersAndFiles || *is_dir)
                    && words.iter().all(|word| word_tier(&folded, word).is_some())
                    && !matches(&words, &folded, &split))
                .then_some((folded.len(), *id))
            })
            .collect::<Vec<_>>();
        found.sort_unstable();
        found.truncate(limit);
        found.into_iter().map(|(_, id)| id).collect()
    }

    #[test]
    fn substring_finds_what_is_inside_a_word_at_every_level() {
        let tree = Tree::new("store-substring");
        let walked = walked_names(&tree.0, &corpus(3000));
        for detail in IndexDetail::ALL {
            let store = store_of(&tree, &format!("{}.db", detail.as_str()), detail, &walked);
            let rows = every_row(&store);
            for query in [
                "inv", "data", "port", "tml", "ript", "inv 2024", "zqxj", "a", "eta in",
            ] {
                for kinds in [Kinds::FoldersAndFiles, Kinds::Folders] {
                    for limit in [1, 8, 200, 100_000] {
                        let found = store.substring(query, kinds, limit, &|| false).unwrap();
                        assert!(found.iter().all(|row| row.tier == 1));
                        assert_eq!(
                            found.iter().map(|row| row.id).collect::<Vec<_>>(),
                            mid_word(&rows, query, kinds, limit),
                            "{detail:?} {query:?} {kinds:?} {limit}"
                        );
                    }
                }
            }
            let names = |query: &str| {
                store
                    .substring(query, Kinds::FoldersAndFiles, 100_000, &|| false)
                    .unwrap()
                    .into_iter()
                    .map(|row| row.name)
                    .collect::<Vec<_>>()
            };
            assert!(names("data").contains(&"metadata.json".to_string()));
            assert!(!names("data").contains(&"data".to_string()));
            assert!(names("inv").contains(&"reinvent.txt".to_string()));
            assert!(names("report").contains(&"myreport.doc".to_string()));
        }
    }

    /// Thousands of names with the query as a word and a few with it inside
    /// one: only those few are read from `files`. Reading each of the others
    /// to turn it down was 2.6 s a query for `html` in the real index -- the
    /// c2b bench's half-hour "stall".
    #[test]
    fn substring_reads_no_row_that_has_the_query_only_as_a_word() {
        let tree = Tree::new("store-substring-words");
        let mut names = (0..3000)
            .map(|index| (format!("page {index}.html"), false))
            .collect::<Vec<_>>();
        names.extend(["myhtml.txt", "xhtml", "PageHtml.md"].map(|name| (name.to_string(), false)));
        let walked = walked_names(&tree.0, &names);
        for detail in IndexDetail::ALL {
            let store = store_of(&tree, &format!("{}.db", detail.as_str()), detail, &walked);
            let before = store.fetched.get();
            let found = store
                .substring("html", Kinds::FoldersAndFiles, 200, &|| false)
                .unwrap()
                .into_iter()
                .map(|row| row.name)
                .collect::<Vec<_>>();
            assert_eq!(found, ["xhtml", "myhtml.txt"], "{detail:?}");
            // The two, and `PageHtml.md`, whose `html` starts a part at a
            // capital: only its name as on disk says so.
            assert_eq!(store.fetched.get() - before, 3, "{detail:?}");
        }
    }

    #[test]
    fn a_cancelled_substring_answers_nothing() {
        let tree = Tree::new("store-substring-cancel");
        let walked = walked_names(&tree.0, &corpus(3000));
        for detail in IndexDetail::ALL {
            let store = store_of(&tree, &format!("{}.db", detail.as_str()), detail, &walked);
            let found = store
                .substring("eta", Kinds::FoldersAndFiles, 200, &|| false)
                .unwrap();
            assert!(!found.is_empty(), "{detail:?}");
            let asked = std::cell::Cell::new(0);
            let cancelled = || {
                asked.set(asked.get() + 1);
                true
            };
            let found = store
                .substring("eta", Kinds::FoldersAndFiles, 200, &cancelled)
                .unwrap();
            assert!(found.is_empty(), "{detail:?}");
            assert_eq!(asked.get(), 1, "{detail:?}");

            // Cancelled halfway through a scan, past its first check.
            if !detail.grams() {
                asked.set(0);
                let later = || {
                    asked.set(asked.get() + 1);
                    asked.get() > 1
                };
                let found = store
                    .substring("eta", Kinds::FoldersAndFiles, 100_000, &later)
                    .unwrap();
                assert!(found.is_empty(), "{detail:?}");
                assert_eq!(asked.get(), 2, "{detail:?}");
            }
        }
    }

    /// The word tables rebuilt from `files`, never a walk: the corpus is of
    /// paths that do not exist, which a walk would find none of, and the
    /// walk's time is the one written by the walk.
    #[test]
    fn changing_the_level_rebuilds_only_the_word_tables() {
        let tree = Tree::new("store-level");
        let path = tree.path("files.db");
        let walked = walked_names(&tree.0, &corpus(500));
        let mut store = store_of(&tree, "files.db", IndexDetail::Balanced, &walked);
        let meta = store.meta().unwrap();
        let rows = every_row(&store);
        let answers = |store: &Store| {
            RANKED_QUERIES
                .iter()
                .map(|query| {
                    let found = store
                        .ranked(query, Kinds::FoldersAndFiles, 64, &any)
                        .unwrap();
                    ids_and_tiers(&found)
                })
                .collect::<Vec<_>>()
        };
        let balanced = answers(&store);
        let grams = |store: &Store| {
            scalar(
                store,
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'grams'",
            )
        };

        for detail in [
            IndexDetail::Compact,
            IndexDetail::Complete,
            IndexDetail::Balanced,
            IndexDetail::Complete,
        ] {
            store.set_detail(detail).unwrap();
            assert_eq!(store.detail(), detail);
            assert_eq!(store.meta().unwrap(), meta, "{detail:?}");
            assert_eq!(every_row(&store), rows, "{detail:?}");
            assert_eq!(answers(&store), balanced, "{detail:?}");
            assert_eq!(grams(&store), i64::from(detail == IndexDetail::Complete));
        }
        drop(store);

        // Recorded: opened plainly, it stays; opened at another, it changes.
        let store = Store::open(&path).unwrap();
        assert_eq!(store.detail(), IndexDetail::Complete);
        drop(store);
        let store = Store::open_with(&path, IndexDetail::Compact).unwrap();
        assert_eq!(store.detail(), IndexDetail::Compact);
        assert_eq!(grams(&store), 0);
        assert_eq!(store.meta().unwrap(), meta);
        assert_eq!(answers(&store), balanced);
        drop(store);
        assert_eq!(Store::open(&path).unwrap().detail(), IndexDetail::Compact);
    }

    /// A change of level cut short leaves its mark, and the next open builds
    /// the word tables again rather than trust half of them.
    #[test]
    fn a_change_of_level_cut_short_is_built_again_at_open() {
        let tree = Tree::new("store-level-cut");
        let path = tree.path("files.db");
        let walked = walked_names(&tree.0, &corpus(200));
        let store = store_of(&tree, "files.db", IndexDetail::Balanced, &walked);
        let expected = store
            .ranked("invoice", Kinds::FoldersAndFiles, 64, &any)
            .unwrap();
        assert!(expected.len() > 8);
        store
            .connection
            .execute_batch(
                "INSERT INTO words (words) VALUES ('delete-all');
                 INSERT OR REPLACE INTO meta (key, value) VALUES ('detail', 'changing');",
            )
            .unwrap();
        drop(store);

        let store = Store::open(&path).unwrap();
        assert_eq!(store.detail(), IndexDetail::Balanced);
        let found = store
            .ranked("invoice", Kinds::FoldersAndFiles, 64, &any)
            .unwrap();
        assert_eq!(found, expected);
    }

    /// A store whose `words` has step c2b's layout -- prefixes up to 3, no
    /// `words_layout` -- is known at open and its `words` rebuilt from
    /// `files`: the entries, their rows and the walk's meta stay as they were.
    #[test]
    fn a_store_with_the_old_word_layout_rebuilds_its_words_at_open() {
        let tree = Tree::new("store-layout");
        let path = tree.path("files.db");
        let walked = walked_names(&tree.0, &corpus(300));
        let store = store_of(&tree, "files.db", IndexDetail::Compact, &walked);
        let meta = store.meta().unwrap();
        let rows = every_row(&store);
        let expected = store
            .ranked("invoice", Kinds::FoldersAndFiles, 64, &any)
            .unwrap();
        assert!(expected.len() > 8);
        // Step c2b's table, and empty: only a rebuild can find the words.
        store
            .connection
            .execute_batch(
                "DROP TABLE words;
                 CREATE VIRTUAL TABLE words USING fts5(
                     tokens, content = '', contentless_delete = 1,
                     prefix = '1 2 3', detail = none);
                 DELETE FROM meta WHERE key = 'words_layout';",
            )
            .unwrap();
        drop(store);

        let words_sql = |store: &Store| {
            store
                .connection
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name = 'words'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.detail(), IndexDetail::Compact);
        assert!(words_sql(&store).contains("prefix = '1 2 3 4'"));
        assert!(words_sql(&store).contains("detail = none"));
        assert_eq!(store.meta().unwrap(), meta);
        assert_eq!(every_row(&store), rows);
        let found = store
            .ranked("invoice", Kinds::FoldersAndFiles, 64, &any)
            .unwrap();
        assert_eq!(found, expected);
        drop(store);

        // Recorded: the next open rebuilds nothing, and a walk keeps it.
        let mut store = Store::open(&path).unwrap();
        store.replace(&walked, "rules").unwrap();
        assert_eq!(
            scalar(
                &store,
                "SELECT COUNT(*) FROM meta WHERE key = 'words_layout' AND value = '2'"
            ),
            1
        );
    }

    /// At `complete`, what arrives is found inside its words by the trigram
    /// index, and what leaves is not.
    #[test]
    fn an_update_keeps_the_trigram_index_current() {
        let tree = Tree::new("store-update-grams");
        tree.file("notes.md").file("plan.md");
        let mut store = kept(&tree);
        store.set_detail(IndexDetail::Complete).unwrap();
        let inside = |store: &Store, query: &str| {
            store
                .substring(query, Kinds::FoldersAndFiles, 64, &|| false)
                .unwrap()
                .into_iter()
                .map(|row| row.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(inside(&store, "lan"), ["plan.md"]);

        tree.file("projects/myreport.pdf");
        std::fs::remove_file(tree.path("plan.md")).unwrap();
        change(&mut store, &tree, &["projects", "plan.md"]);

        assert_eq!(inside(&store, "port"), ["myreport.pdf"]);
        assert!(inside(&store, "lan").is_empty());
        assert_eq!(
            scalar(&store, "SELECT COUNT(*) FROM grams WHERE grams MATCH 'lan'"),
            0
        );
    }
}
