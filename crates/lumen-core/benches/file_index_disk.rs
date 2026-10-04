//! The file index without a ceiling (F16): what "a hot layer in memory and the
//! full index on disk" would cost, measured before it is designed.
//!
//! Today every name is held in memory (`file_index::Names`) under a 500 000
//! ceiling, and the store keeps each entry's full path. This bench builds the
//! same corpora five ways on disk -- today's layout, a compact folder tree, the
//! tree with an FTS5 word index (full and compact detail), the tree with an
//! FTS5 trigram index -- and the in-memory names built from the tree, and
//! measures their size, their build, every query of slice 0 against each, the
//! first query of a fresh process, and the process's memory while querying.
//!
//! The corpora are the user's real index (copied, never opened in place), its
//! home-folder subset, and slice 0's generator at one and two million entries,
//! grown as a folder tree so a folders table means something.
//!
//! Everything is written under `target/disk-bench` and removed at the end.
//! Child processes of this same binary measure the cold-ish first query and
//! the working set, so neither is polluted by the corpora this one holds.

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lumen_core::file_index::{Entry, IndexDetail, Kinds, Names, Store, Walked};
use memchr::memmem::Finder;
use rusqlite::{Connection, OpenFlags, params};

/// Slice 0's queries and two words; three real words are added at run time.
const QUERIES: [&str; 7] = ["a", "in", "inv", "invoice", "2024", "zqxj", "inv 2024"];

/// The capped variant of every query.
const LIMIT: usize = 200;

const WARMUP: usize = 20;
const RUNS: usize = 200;

/// The page cache every query connection runs with: resident memory must not
/// grow with the number of files.
const CACHE: &str = "PRAGMA cache_size = -8192;";

/// The synthetic sizes. Real corpora come from the user's index.
const SYNTHETIC: [usize; 2] = [1_000_000, 2_000_000];

/// A fixed-seed xorshift64, as in slice 0.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const FOLDERS: [&str; 28] = [
    "Documents",
    "Projects",
    "lumen",
    "src",
    "Photos",
    "2024",
    "2025",
    "Invoices",
    "Downloads",
    "notes",
    "archive",
    "Desktop",
    "Music",
    "Videos",
    "Acme Corp",
    "Globex",
    "Initech",
    "Umbrella",
    "Hooli",
    "drafts",
    "old",
    "backup",
    "scans",
    "taxes",
    "school",
    "assets",
    "docs",
    "tests",
];

const WORDS: [&str; 48] = [
    "invoice",
    "Report",
    "budget",
    "Meeting",
    "notes",
    "draft",
    "final",
    "Resume",
    "contract",
    "proposal",
    "summary",
    "Screenshot",
    "IMG",
    "photo",
    "holiday",
    "Beach",
    "family",
    "receipt",
    "statement",
    "Tax",
    "install",
    "index",
    "main",
    "lib",
    "config",
    "README",
    "changelog",
    "Presentation",
    "slides",
    "Quarterly",
    "roadmap",
    "design",
    "mockup",
    "logo",
    "banner",
    "export",
    "backup",
    "letter",
    "Application",
    "form",
    "schedule",
    "timesheet",
    "payroll",
    "recording",
    "interview",
    "lecture",
    "Thesis",
    "chapter",
];

const EXTENSIONS: [&str; 16] = [
    "pdf", "docx", "png", "jpg", "rs", "md", "txt", "xlsx", "mp4", "zip", "pptx", "csv", "json",
    "heic", "mov", "html",
];

/// One file or folder.
struct Item {
    folder: u32,
    name: String,
    is_dir: bool,
    scope: i64,
    modified: Option<i64>,
}

/// A corpus as a folder tree: every folder path once, and the items in it.
#[derive(Default)]
struct Corpus {
    /// Each folder's full path, as first seen: what today's layout stores per
    /// row.
    paths: Vec<String>,
    parents: Vec<Option<u32>>,
    /// The last component of each folder's path.
    folder_names: Vec<String>,
    ids: HashMap<String, u32>,
    items: Vec<Item>,
}

impl Corpus {
    /// The folder at `path`, added with its ancestors if new.
    fn folder(&mut self, path: &str) -> u32 {
        let trimmed = path.trim_end_matches(['\\', '/']);
        let key = if trimmed.is_empty() { path } else { trimmed };
        if let Some(&id) = self.ids.get(key) {
            return id;
        }
        let (parent, name) = match key.rfind(['\\', '/']) {
            Some(at) if at > 0 => (Some(self.folder(&key[..at])), &key[at + 1..]),
            _ => (None, key),
        };
        let id = self.paths.len() as u32;
        self.paths.push(path.to_string());
        self.parents.push(parent);
        self.folder_names.push(name.to_string());
        self.ids.insert(key.to_string(), id);
        id
    }

    fn push(
        &mut self,
        parent: &str,
        name: String,
        is_dir: bool,
        scope: i64,
        modified: Option<i64>,
    ) {
        let folder = self.folder(parent);
        self.items.push(Item {
            folder,
            name,
            is_dir,
            scope,
            modified,
        });
    }
}

/// Slice 0's generator, grown as a tree: one entry in eleven is a folder
/// (the real index has 9.6 %), each under a random earlier folder, and every
/// file in a random folder.
fn synthetic(count: usize) -> Corpus {
    const SEPARATORS: [&str; 3] = ["-", "_", " "];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut corpus = Corpus::default();
    let root = corpus.folder(r"C:\Users\bench");
    let mut folders = vec![root];

    let dirs = count / 11;
    for index in 0..dirs {
        let parent = folders[rng.below(folders.len())];
        let word = rng.pick(&FOLDERS);
        let mut name = match rng.below(4) {
            0 => format!("{word} {}", rng.below(100)),
            _ => word.to_string(),
        };
        let parent_path = corpus.paths[parent as usize].clone();
        if corpus.ids.contains_key(&format!("{parent_path}\\{name}")) {
            name = format!("{name} ({index})");
        }
        let path = format!("{parent_path}\\{name}");
        let modified = Some((rng.next() >> 33) as i64);
        corpus.push(&parent_path, name, true, 0, modified);
        folders.push(corpus.folder(&path));
    }

    for _ in dirs..count {
        let folder = folders[rng.below(folders.len())];
        let separator = rng.pick(&SEPARATORS);
        let words = 1 + rng.below(3);
        let mut name = (0..words)
            .map(|_| rng.pick(&WORDS))
            .collect::<Vec<_>>()
            .join(separator);
        match rng.below(10) {
            0..=1 => name = format!("{name}{separator}{}", 2015 + rng.below(12)),
            2..=3 => name = format!("{name}{separator}{}", rng.below(1000)),
            _ => {}
        }
        name = format!("{name}.{}", rng.pick(&EXTENSIONS));
        corpus.items.push(Item {
            folder,
            name,
            is_dir: false,
            scope: 0,
            modified: None,
        });
    }
    corpus
}

/// One row of the user's index.
struct RealRow {
    parent: String,
    name: String,
    is_dir: bool,
    scope: i64,
    modified: Option<i64>,
}

/// The rows of the user's index, whichever layout it is in: version 2, a
/// full path a row, or version 3 (step c2a), each folder once.
fn load_real(path: &Path) -> Vec<RealRow> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open the copy of the real index");
    let version: i32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("the real index's version");
    if version >= 3 {
        return load_real_v3(&connection);
    }
    let mut statement = connection
        .prepare("SELECT parent, name, is_dir, scope, modified FROM entries ORDER BY id")
        .expect("prepare the real entries");
    statement
        .query_map([], |row| {
            Ok(RealRow {
                parent: row.get(0)?,
                name: row.get(1)?,
                is_dir: row.get(2)?,
                scope: row.get(3)?,
                modified: row.get(4)?,
            })
        })
        .expect("read the real entries")
        .collect::<Result<Vec<_>, _>>()
        .expect("read a real entry")
}

/// Version 3's rows: each folder's path through its parents -- a parent's
/// row comes first -- and a folder entry's time from its own `folders` row.
fn load_real_v3(connection: &Connection) -> Vec<RealRow> {
    let mut paths: HashMap<i64, (String, i64)> = HashMap::new();
    let mut times: HashMap<(i64, String), Option<i64>> = HashMap::new();
    let mut statement = connection
        .prepare("SELECT id, parent_id, name, scope, modified FROM folders ORDER BY id")
        .expect("prepare the real folders");
    let mut rows = statement.query([]).expect("read the real folders");
    while let Some(row) = rows.next().expect("a real folder") {
        let id: i64 = row.get(0).expect("an id");
        let parent: Option<i64> = row.get(1).expect("a parent");
        let name: String = row.get(2).expect("a name");
        let scope: i64 = row.get(3).expect("a scope");
        let path = match parent {
            None => name.clone(),
            Some(parent) => {
                let joined = Path::new(&paths[&parent].0).join(&name);
                times.insert((parent, name), row.get(4).expect("a time"));
                joined.to_string_lossy().into_owned()
            }
        };
        paths.insert(id, (path, scope));
    }
    let mut statement = connection
        .prepare("SELECT folder_id, name, is_dir FROM files ORDER BY id")
        .expect("prepare the real files");
    let mut rows = statement.query([]).expect("read the real files");
    let mut found = Vec::new();
    while let Some(row) = rows.next().expect("a real file") {
        let folder: i64 = row.get(0).expect("a folder");
        let name: String = row.get(1).expect("a name");
        let is_dir: bool = row.get(2).expect("a kind");
        let (parent, scope) = paths[&folder].clone();
        let modified = if is_dir {
            times.get(&(folder, name.clone())).copied().flatten()
        } else {
            None
        };
        found.push(RealRow {
            parent,
            name,
            is_dir,
            scope,
            modified,
        });
    }
    found
}

fn corpus_of<'a>(rows: impl IntoIterator<Item = &'a RealRow>) -> Corpus {
    let mut corpus = Corpus::default();
    for row in rows {
        corpus.push(
            &row.parent,
            row.name.clone(),
            row.is_dir,
            row.scope,
            row.modified,
        );
    }
    corpus
}

/// Lower-case as `Names` does, a character at a time.
fn fold(text: &str, out: &mut String) {
    for character in text.chars() {
        out.extend(character.to_lowercase());
    }
}

fn words(query: &str) -> Vec<String> {
    let mut folded = String::new();
    fold(query, &mut folded);
    folded.split_whitespace().map(str::to_string).collect()
}

fn starts_word(byte: u8) -> bool {
    matches!(byte, b' ' | b'-' | b'_' | b'.' | b'(' | b'[' | b'\n')
}

/// Frequent words of the real names: alphabetic, four letters or more.
fn frequent_words(corpus: &Corpus) -> Vec<(String, usize)> {
    let mut counts = HashMap::<String, usize>::new();
    let mut folded = String::new();
    for item in &corpus.items {
        folded.clear();
        fold(&item.name, &mut folded);
        for token in folded.split(|c: char| !c.is_alphanumeric()) {
            if token.len() >= 4 && token.chars().all(|c| c.is_ascii_alphabetic()) {
                *counts.entry(token.to_string()).or_default() += 1;
            }
        }
    }
    let mut counts = counts.into_iter().collect::<Vec<_>>();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    counts
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    /// Today's layout: the full parent path in every row.
    V0,
    /// A folder tree: each folder path once.
    V1,
    /// V1 and an FTS5 word index, default detail.
    V2,
    /// V2 with `detail=none, columnsize=0`: the compact word index.
    V2c,
    /// V1 and an FTS5 trigram index.
    V3,
}

impl Variant {
    fn label(self) -> &'static str {
        match self {
            Variant::V0 => "V0 full paths (today)",
            Variant::V1 => "V1 folders + files",
            Variant::V2 => "V2 V1 + FTS5 words",
            Variant::V2c => "V2c V1 + FTS5 words, detail=none",
            Variant::V3 => "V3 V1 + FTS5 trigram",
        }
    }

    fn file(self) -> &'static str {
        match self {
            Variant::V0 => "v0.db",
            Variant::V1 => "v1.db",
            Variant::V2 => "v2.db",
            Variant::V2c => "v2c.db",
            Variant::V3 => "v3.db",
        }
    }
}

const WORD_INDEX: &str = "CREATE VIRTUAL TABLE words USING fts5(
     name, content='files', content_rowid='id',
     tokenize=\"unicode61 separators '-_.()[]'\", prefix='1 2 3'";

/// Builds `variant` of `corpus` at `path`: the bulk insert in one
/// transaction, indexes after it, then the FTS build.
fn build(path: &Path, corpus: &Corpus, variant: Variant) -> Duration {
    remove(path);
    let start = Instant::now();
    let connection = Connection::open(path).expect("open a bench database");
    connection
        .execute_batch(
            "PRAGMA journal_mode = OFF;
             PRAGMA synchronous = OFF;
             PRAGMA cache_size = -65536;",
        )
        .expect("configure the build");
    let transaction = connection
        .unchecked_transaction()
        .expect("begin the bulk insert");
    if variant == Variant::V0 {
        transaction
            .execute_batch(
                "CREATE TABLE entries (
                     id       INTEGER PRIMARY KEY AUTOINCREMENT,
                     scope    INTEGER NOT NULL,
                     parent   TEXT    NOT NULL,
                     name     TEXT    NOT NULL,
                     is_dir   INTEGER NOT NULL,
                     modified INTEGER
                 );",
            )
            .expect("create today's table");
        {
            let mut insert = transaction
                .prepare(
                    "INSERT INTO entries (id, scope, parent, name, is_dir, modified)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .expect("prepare the insert");
            for (id, item) in (1i64..).zip(&corpus.items) {
                insert
                    .execute(params![
                        id,
                        item.scope,
                        corpus.paths[item.folder as usize],
                        item.name,
                        item.is_dir,
                        item.modified
                    ])
                    .expect("insert an entry");
            }
        }
        transaction
            .execute_batch("CREATE INDEX entries_path ON entries (parent, name);")
            .expect("index the paths");
    } else {
        transaction
            .execute_batch(
                "CREATE TABLE folders (
                     id        INTEGER PRIMARY KEY,
                     parent_id INTEGER,
                     name      TEXT NOT NULL
                 );
                 CREATE TABLE files (
                     id        INTEGER PRIMARY KEY,
                     folder_id INTEGER NOT NULL,
                     scope     INTEGER NOT NULL,
                     name      TEXT    NOT NULL,
                     is_dir    INTEGER NOT NULL,
                     modified  INTEGER
                 );",
            )
            .expect("create the tree");
        {
            let mut insert = transaction
                .prepare("INSERT INTO folders (id, parent_id, name) VALUES (?1, ?2, ?3)")
                .expect("prepare the folders");
            for (id, (parent, name)) in
                (1i64..).zip(corpus.parents.iter().zip(&corpus.folder_names))
            {
                insert
                    .execute(params![id, parent.map(|parent| parent as i64 + 1), name])
                    .expect("insert a folder");
            }
            let mut insert = transaction
                .prepare(
                    "INSERT INTO files (id, folder_id, scope, name, is_dir, modified)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .expect("prepare the files");
            for (id, item) in (1i64..).zip(&corpus.items) {
                insert
                    .execute(params![
                        id,
                        item.folder as i64 + 1,
                        item.scope,
                        item.name,
                        item.is_dir,
                        item.modified
                    ])
                    .expect("insert a file");
            }
        }
        transaction
            .execute_batch(
                "CREATE INDEX folders_path ON folders (parent_id, name);
                 CREATE INDEX files_path ON files (folder_id, name);",
            )
            .expect("index the tree");
        let fts = match variant {
            Variant::V2 => Some(format!("{WORD_INDEX});")),
            Variant::V2c => Some(format!("{WORD_INDEX}, detail=none, columnsize=0);")),
            Variant::V3 => Some(
                "CREATE VIRTUAL TABLE trigrams USING fts5(
                     name, content='files', content_rowid='id', tokenize='trigram');"
                    .to_string(),
            ),
            _ => None,
        };
        if let Some(create) = fts {
            // Its absence is a finding: report it and stop.
            if let Err(error) = transaction.execute_batch(&create) {
                println!("FTS5 table unavailable ({}): {error}", variant.label());
                std::process::exit(1);
            }
            let table = if variant == Variant::V3 {
                "trigrams"
            } else {
                "words"
            };
            transaction
                .execute_batch(&format!("INSERT INTO {table}({table}) VALUES ('rebuild');"))
                .expect("build the FTS index");
        }
    }
    transaction.commit().expect("commit the bulk insert");
    drop(connection);
    start.elapsed()
}

fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-journal"));
}

fn size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// The names of V1, loaded as the hot layer would be at start.
fn load_names(connection: &Connection) -> Names {
    let mut statement = connection
        .prepare("SELECT name, is_dir FROM files ORDER BY id")
        .expect("prepare the names");
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        })
        .expect("read the names")
        .collect::<Result<Vec<_>, _>>()
        .expect("read a name");
    Names::from_entries(rows.iter().map(|(name, is_dir)| (name.as_str(), *is_dir)))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    /// FTS5 word-prefix MATCH on V2.
    Words,
    /// The same on V2c.
    WordsCompact,
    /// FTS5 trigram MATCH on V3; words of three characters or more.
    Trigram,
    /// `LIKE '%q%'` on V1's names, a full scan; words of three or more.
    Like,
    /// V1's names streamed in rowid order and matched by `Names`' rules
    /// with `memmem`: no index beyond the tree.
    Stream,
    /// `Names::find` on the names loaded from V1.
    Blob,
}

const METHODS: [Method; 6] = [
    Method::Words,
    Method::WordsCompact,
    Method::Trigram,
    Method::Like,
    Method::Stream,
    Method::Blob,
];

impl Method {
    fn name(self) -> &'static str {
        match self {
            Method::Words => "v2",
            Method::WordsCompact => "v2c",
            Method::Trigram => "v3",
            Method::Like => "like",
            Method::Stream => "stream",
            Method::Blob => "v4",
        }
    }

    fn parse(text: &str) -> Self {
        *METHODS
            .iter()
            .find(|method| method.name() == text)
            .expect("a known method")
    }

    fn variant(self) -> Variant {
        match self {
            Method::Words => Variant::V2,
            Method::WordsCompact => Variant::V2c,
            Method::Trigram => Variant::V3,
            Method::Like | Method::Stream | Method::Blob => Variant::V1,
        }
    }
}

/// One open variant: a read-only connection with the bounded cache, and the
/// names when the method is the blob.
struct Engine {
    connection: Connection,
    names: Option<Names>,
}

impl Engine {
    fn open(path: &Path, method: Method) -> Self {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("open a variant");
        connection.execute_batch(CACHE).expect("bound the cache");
        let names = (method == Method::Blob).then(|| load_names(&connection));
        Self { connection, names }
    }

    /// The number of matches of `query` by `method`, all of them or the first
    /// `limit`; `None` where the method cannot answer the query.
    fn search(&self, method: Method, query: &str, limit: Option<usize>) -> Option<usize> {
        let words = words(query);
        let sql_limit = limit.map_or(-1, |limit| limit as i64);
        match method {
            Method::Words | Method::WordsCompact => {
                let fts = words
                    .iter()
                    .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                Some(self.count(
                    "SELECT rowid FROM words WHERE words MATCH ?1 LIMIT ?2",
                    params![fts, sql_limit],
                ))
            }
            Method::Trigram => {
                if words.iter().any(|word| word.chars().count() < 3) {
                    return None;
                }
                let fts = words
                    .iter()
                    .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                Some(self.count(
                    "SELECT rowid FROM trigrams WHERE trigrams MATCH ?1 LIMIT ?2",
                    params![fts, sql_limit],
                ))
            }
            Method::Like => {
                if words.iter().any(|word| word.chars().count() < 3) {
                    return None;
                }
                let mut sql = String::from("SELECT id FROM files WHERE ");
                for index in 0..words.len() {
                    if index > 0 {
                        sql.push_str(" AND ");
                    }
                    let _ = write!(sql, "name LIKE ?{}", index + 1);
                }
                let _ = write!(sql, " LIMIT {sql_limit}");
                let patterns = words
                    .iter()
                    .map(|word| format!("%{word}%"))
                    .collect::<Vec<_>>();
                Some(self.count(&sql, rusqlite::params_from_iter(patterns)))
            }
            Method::Stream => Some(self.stream(&words, limit.unwrap_or(usize::MAX))),
            Method::Blob => {
                let names = self.names.as_ref().expect("names loaded");
                let limit = limit.unwrap_or(names.len());
                Some(
                    names
                        .find(query, Kinds::FoldersAndFiles, limit, |_| 0)
                        .len(),
                )
            }
        }
    }

    fn count(&self, sql: &str, params: impl rusqlite::Params) -> usize {
        let mut statement = self
            .connection
            .prepare_cached(sql)
            .expect("prepare a query");
        let mut rows = statement.query(params).expect("run a query");
        let mut count = 0;
        while rows.next().expect("read a row").is_some() {
            count += 1;
        }
        count
    }

    fn stream(&self, words: &[String], limit: usize) -> usize {
        let finders = words
            .iter()
            .map(|word| Finder::new(word.as_bytes()))
            .collect::<Vec<_>>();
        let mut statement = self
            .connection
            .prepare_cached("SELECT name FROM files ORDER BY id")
            .expect("prepare the stream");
        let mut rows = statement.query([]).expect("stream the names");
        let mut folded = String::new();
        let mut count = 0;
        while let Some(row) = rows.next().expect("read a name") {
            let name = row
                .get_ref(0)
                .and_then(|v| Ok(v.as_str()?))
                .expect("a name");
            folded.clear();
            fold(name, &mut folded);
            let bytes = folded.as_bytes();
            let all = words.iter().zip(&finders).all(|(word, finder)| {
                if word.chars().count() >= 3 {
                    finder.find(bytes).is_some()
                } else {
                    finder
                        .find_iter(bytes)
                        .any(|at| at == 0 || starts_word(bytes[at - 1]))
                }
            });
            if all {
                count += 1;
                if count == limit {
                    break;
                }
            }
        }
        count
    }
}

/// Sorted samples, how many runs, and the matches of the last run.
struct Timing {
    p50: Duration,
    p95: Duration,
    max: Duration,
    runs: usize,
    matches: usize,
}

/// Times `run` after a warm-up. A query whose first run is slow gets fewer
/// runs, so a full scan at two million does not take the whole budget.
fn time(mut run: impl FnMut() -> usize) -> Timing {
    let start = Instant::now();
    run();
    let probe = start.elapsed();
    let (warmup, runs) = if probe < Duration::from_millis(5) {
        (WARMUP, RUNS)
    } else if probe < Duration::from_millis(50) {
        (5, 50)
    } else {
        (2, 15)
    };
    for _ in 0..warmup {
        run();
    }
    let mut matches = 0;
    let mut samples = (0..runs)
        .map(|_| {
            let start = Instant::now();
            matches = run();
            start.elapsed()
        })
        .collect::<Vec<_>>();
    samples.sort();
    Timing {
        p50: samples[runs / 2],
        p95: samples[runs * 95 / 100],
        max: samples[runs - 1],
        runs,
        matches,
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}

fn cell(timing: Option<&Timing>) -> String {
    match timing {
        None => "n/a".to_string(),
        Some(t) => {
            let mut text = format!("{:.2} / {:.2} / {:.2}", ms(t.p50), ms(t.p95), ms(t.max));
            if t.runs != RUNS {
                let _ = write!(text, " ({})", t.runs);
            }
            text
        }
    }
}

/// The working set, peak working set and private bytes of process `pid`.
fn working_set(pid: u32) -> Option<(u64, u64, u64)> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "$p = Get-Process -Id {pid}; \"$($p.WorkingSet64) $($p.PeakWorkingSet64) $($p.PrivateMemorySize64)\""
            ),
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut numbers = text.split_whitespace().map(|n| n.parse::<u64>().ok());
    Some((numbers.next()??, numbers.next()??, numbers.next()??))
}

/// A copy of `source` in `dir` written with unbuffered I/O (`robocopy /J`),
/// so none of its pages are in the OS file cache: the cold-ish condition
/// that needs no administrator to purge the cache.
fn unbuffered_copy(source: &Path, dir: &Path) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let file = source.file_name()?;
    let target = dir.join(file);
    remove(&target);
    let status = Command::new("robocopy")
        .arg(source.parent()?)
        .arg(dir)
        .arg(file)
        .args(["/J", "/NJH", "/NJS", "/NFL", "/NDL", "/NP", "/R:0", "/W:0"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    (status.code().is_some_and(|code| code < 8) && target.exists()).then_some(target)
}

/// How long a sequential buffered read of the whole file takes.
fn read_ms(path: &Path) -> f64 {
    let mut file = File::open(path).expect("open for reading");
    let mut buffer = vec![0u8; 1 << 20];
    let start = Instant::now();
    while file.read(&mut buffer).expect("read") > 0 {}
    ms(start.elapsed())
}

/// Runs this binary as a child and returns what it printed.
fn spawn(args: &[&str]) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let output = Command::new(exe).arg("--child").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The child side: `cold <method> <db> <query>` or `mem <method> <db>
/// <queries joined by |>`.
fn child(args: &[String]) {
    match args[0].as_str() {
        "cold" => {
            let method = Method::parse(&args[1]);
            let start = Instant::now();
            let engine = Engine::open(Path::new(&args[2]), method);
            let open = start.elapsed();
            let start = Instant::now();
            let matches = engine.search(method, &args[3], None).unwrap_or(0);
            let first = start.elapsed();
            let start = Instant::now();
            engine.search(method, &args[3], None);
            let second = start.elapsed();
            println!(
                "{:.2} {:.2} {:.2} {matches}",
                ms(open),
                ms(first),
                ms(second)
            );
        }
        "mem" => {
            let mut names_bytes = 0;
            let mut engine = None;
            if args[1] != "none" {
                let method = Method::parse(&args[1]);
                let opened = Engine::open(Path::new(&args[2]), method);
                for _ in 0..2 {
                    for query in args[3].split('|') {
                        opened.search(method, query, None);
                        opened.search(method, query, Some(LIMIT));
                    }
                }
                names_bytes = opened.names.as_ref().map_or(0, Names::memory_bytes);
                engine = Some(opened);
            }
            // SAFETY: plain reads of SQLite's global memory counters.
            let (used, high) = unsafe {
                (
                    rusqlite::ffi::sqlite3_memory_used(),
                    rusqlite::ffi::sqlite3_memory_highwater(0),
                )
            };
            let (ws, peak, private) = working_set(std::process::id()).unwrap_or_default();
            println!("{ws} {peak} {private} {used} {high} {names_bytes}");
            drop(engine);
        }
        "ranked" => {
            let strategy = Strategy::parse(&args[1]);
            let start = Instant::now();
            let ranker = Ranker::open(Path::new(&args[2]));
            let open = start.elapsed();
            let query = Query::new(&args[3]);
            let top = args[4].parse::<usize>().expect("a number of rows");
            let start = Instant::now();
            let found = ranker.run(strategy, &query, top).len();
            let first = start.elapsed();
            let start = Instant::now();
            ranker.run(strategy, &query, top);
            let second = start.elapsed();
            println!("{:.2} {:.2} {:.2} {found}", ms(open), ms(first), ms(second));
        }
        "store" => {
            let start = Instant::now();
            let store = Store::open(Path::new(&args[1])).expect("open the store");
            let open = start.elapsed();
            let top = args[3].parse::<usize>().expect("a number of rows");
            let ranked = || {
                store
                    .ranked(&args[2], Kinds::FoldersAndFiles, top, &|_, _| true)
                    .expect("a ranked query")
                    .len()
            };
            let start = Instant::now();
            let found = ranked();
            let first = start.elapsed();
            let start = Instant::now();
            ranked();
            let second = start.elapsed();
            println!("{:.2} {:.2} {:.2} {found}", ms(open), ms(first), ms(second));
        }
        other => panic!("unknown child mode {other}"),
    }
}

// The ranked best N: can the disk answer root's "the best N matches,
// ranked" -- exact, name start, word start, then the shorter name, then the
// entry -- rather than its first N in storage order? Words are split as the
// design says, at separators and at a change to a capital or a digit.

/// The queries of the ranked section: slice 0's, the design's examples and
/// two frequent real words.
const RANKED_QUERIES: [&str; 11] = [
    "inv", "invoice", "report", "2024", "html", "license", "preview", "zqxj", "inv 2024", "cargo",
    "data",
];

/// Words whose matches the split may add -- or lose, where a word typed out
/// whole runs across a capital -- counted on the real corpora.
const SPLIT_PROBES: [&str; 9] = [
    "report",
    "2024",
    "preview",
    "license",
    "html",
    "inv",
    "powershell",
    "javascript",
    "github",
];

/// How many ranked rows root asks the disk for.
const TOPS: [usize; 2] = [8, 64];

/// The split word index's tokenizer. The words are split and lower-cased
/// before it sees them, so it only cuts at the spaces between them; no
/// diacritics are removed, as `Names` removes none.
const SPLIT_TOKENIZER: &str = "tokenize=\"unicode61 remove_diacritics 0\", prefix='1 2 3'";

/// The words `name` is indexed as, lower-cased and joined by spaces: cut at
/// every character that is not a letter or a digit -- `unicode61` cuts there
/// anyway, a superset of space `-` `_` `.` `(` `[` -- and where a capital
/// follows a non-capital or a digit a non-digit. `AnnualReport2024` is
/// `annual report 2024`; `HTMLParser` stays one word.
fn split_words(name: &str, out: &mut String) {
    let begin = out.len();
    let mut previous: Option<char> = None;
    for character in name.chars() {
        if !character.is_alphanumeric() {
            previous = None;
            continue;
        }
        let starts = previous.is_none_or(|previous| {
            (character.is_uppercase() && !previous.is_uppercase())
                || (character.is_numeric() && !previous.is_numeric())
        });
        if starts && out.len() > begin {
            out.push(' ');
        }
        out.extend(character.to_lowercase());
        previous = Some(character);
    }
}

/// `Names`' exact tier: the whole name, or the name without its extension.
fn is_exact(name: &str, query: &str) -> bool {
    let Some(rest) = name.strip_prefix(query) else {
        return false;
    };
    rest.is_empty() || (!query.is_empty() && rest.starts_with('.') && !rest[1..].contains('.'))
}

/// What a ranked row is ordered by, ascending: the tier upside down (0 is
/// exact), the folded name's length in bytes, the entry.
type Key = (u8, u32, u32);

/// A query as the ranked strategies use it.
struct Query {
    words: Vec<String>,
    /// The first of the longest words, which decides the tier as in `Names`.
    longest: usize,
    whole: String,
}

impl Query {
    fn new(text: &str) -> Self {
        let words = words(text);
        let longest = (0..words.len())
            .rev()
            .max_by_key(|&index| words[index].len())
            .expect("at least one word");
        let whole = words.join(" ");
        Self {
            words,
            longest,
            whole,
        }
    }

    fn key_word(&self) -> &str {
        &self.words[self.longest]
    }

    /// Every word as a word prefix; `initial` makes the longest one start
    /// the name's first word (`^`).
    fn fts(&self, initial: bool) -> String {
        self.words
            .iter()
            .enumerate()
            .map(|(index, word)| {
                let caret = if initial && index == self.longest {
                    "^"
                } else {
                    ""
                };
                format!("{caret}\"{}\"*", word.replace('"', "\"\""))
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }

    /// Whether every word starts one of the name's split words.
    fn matches(&self, split: &str) -> bool {
        self.words.iter().all(|word| {
            split
                .split(' ')
                .any(|token| token.starts_with(word.as_str()))
        })
    }

    /// The tier of a name that matches: 4 exact, 3 starting with the
    /// longest word, 2 a word start.
    fn tier(&self, folded: &str) -> u8 {
        if is_exact(folded, &self.whole) {
            4
        } else if folded.starts_with(self.key_word()) {
            3
        } else {
            2
        }
    }

    fn key(&self, folded: &str, id: u32) -> Key {
        (4 - self.tier(folded), folded.len() as u32, id)
    }
}

/// Every name of a corpus folded and split, for the reference ranking and
/// the builds.
struct Prepared {
    folded: Vec<String>,
    split: Vec<String>,
}

fn prepare(corpus: &Corpus) -> Prepared {
    let mut prepared = Prepared {
        folded: Vec::with_capacity(corpus.items.len()),
        split: Vec::with_capacity(corpus.items.len()),
    };
    for item in &corpus.items {
        let mut folded = String::new();
        fold(&item.name, &mut folded);
        let mut split = String::new();
        split_words(&item.name, &mut split);
        prepared.folded.push(folded);
        prepared.split.push(split);
    }
    prepared
}

/// Every match of `query`, ranked in memory: what each strategy's best N must
/// equal. With the count of each tier, 4 to 2.
fn reference(prepared: &Prepared, query: &Query) -> (Vec<u32>, [usize; 3]) {
    let mut keys = prepared
        .split
        .iter()
        .zip(&prepared.folded)
        .zip(1u32..)
        .filter(|((split, _), _)| query.matches(split))
        .map(|((_, folded), id)| query.key(folded, id))
        .collect::<Vec<_>>();
    keys.sort_unstable();
    let mut tiers = [0; 3];
    for key in &keys {
        tiers[key.0 as usize] += 1;
    }
    (keys.into_iter().map(|key| key.2).collect(), tiers)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Strategy {
    /// Every match from the word index, its name fetched, ranked in Rust in
    /// a bounded heap.
    All,
    /// Exact names, then name starts, from an index on `lower(name)`; then
    /// word starts from the word index ordered by length in SQL; stopping
    /// once N are found.
    Tiered,
    /// Exact names from the same index, then the name starts and the word
    /// starts from a word index whose row id is the name's length and its
    /// entry: its own order is the ranking, so it stops after N.
    Keyed,
}

impl Strategy {
    fn name(self) -> &'static str {
        match self {
            Strategy::All => "all",
            Strategy::Tiered => "tiered",
            Strategy::Keyed => "keyed",
        }
    }

    fn parse(text: &str) -> Self {
        [Strategy::All, Strategy::Tiered, Strategy::Keyed]
            .into_iter()
            .find(|strategy| strategy.name() == text)
            .expect("a known strategy")
    }
}

/// One ranked row: the entry and its name, as root would show it.
struct Row {
    id: u32,
    #[allow(dead_code)]
    name: String,
}

/// A read-only connection to a split database.
struct Ranker {
    connection: Connection,
}

impl Ranker {
    fn open(path: &Path) -> Self {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("open a split database");
        connection.execute_batch(CACHE).expect("bound the cache");
        Self { connection }
    }

    fn run(&self, strategy: Strategy, query: &Query, top: usize) -> Vec<Row> {
        match strategy {
            Strategy::All => self.all(query, top),
            Strategy::Tiered => self.tiered(query, top),
            Strategy::Keyed => self.keyed(query, top),
        }
    }

    fn name(&self, id: u32) -> String {
        self.connection
            .prepare_cached("SELECT name FROM files WHERE id = ?1")
            .expect("prepare a name")
            .query_row([id], |row| row.get(0))
            .expect("a name")
    }

    fn all(&self, query: &Query, top: usize) -> Vec<Row> {
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT f.id, f.name FROM words CROSS JOIN files f ON f.id = words.rowid
                 WHERE words MATCH ?1",
            )
            .expect("prepare every match");
        let mut rows = statement.query([query.fts(false)]).expect("match");
        let mut heap = BinaryHeap::<(Key, String)>::with_capacity(top + 1);
        let mut folded = String::new();
        while let Some(row) = rows.next().expect("read a match") {
            let id = row.get::<_, u32>(0).expect("an id");
            let name = row
                .get_ref(1)
                .and_then(|v| Ok(v.as_str()?))
                .expect("a name");
            folded.clear();
            fold(name, &mut folded);
            let key = query.key(&folded, id);
            if heap.len() < top {
                heap.push((key, name.to_string()));
            } else if heap.peek().is_some_and(|(worst, _)| key < *worst) {
                heap.pop();
                heap.push((key, name.to_string()));
            }
        }
        heap.into_sorted_vec()
            .into_iter()
            .map(|((_, _, id), name)| Row { id, name })
            .collect()
    }

    /// Every exact match, ranked: the whole name, or the name and an
    /// extension, found on the `lower(name)` index.
    fn exact(&self, query: &Query) -> Vec<(Key, String)> {
        let mut found = Vec::new();
        let mut folded = String::new();
        let mut split = String::new();
        let stem = format!("{}.", query.whole);
        let past = format!("{}/", query.whole);
        for (sql, bounds) in [
            (
                "SELECT id, name FROM files WHERE lower(name) = ?1",
                vec![query.whole.as_str()],
            ),
            (
                "SELECT id, name FROM files WHERE lower(name) >= ?1 AND lower(name) < ?2",
                vec![stem.as_str(), past.as_str()],
            ),
        ] {
            let mut statement = self
                .connection
                .prepare_cached(sql)
                .expect("prepare the exact");
            let mut rows = statement
                .query(rusqlite::params_from_iter(bounds))
                .expect("the exact");
            while let Some(row) = rows.next().expect("read an exact") {
                let id = row.get::<_, u32>(0).expect("an id");
                let name = row.get::<_, String>(1).expect("a name");
                folded.clear();
                fold(&name, &mut folded);
                split.clear();
                split_words(&name, &mut split);
                if is_exact(&folded, &query.whole) && query.matches(&split) {
                    found.push((query.key(&folded, id), name));
                }
            }
        }
        found.sort_unstable();
        found
    }

    fn tiered(&self, query: &Query, top: usize) -> Vec<Row> {
        let mut out = self.exact(query);
        if out.len() >= top {
            out.truncate(top);
            return rows_of(out);
        }
        let mut known = out.iter().map(|(key, _)| key.2).collect::<HashSet<_>>();

        // Name starts, the shortest first, past the exact ones.
        let word = query.key_word();
        let upper = format!("{word}\u{10FFFF}");
        let need = top - out.len();
        let mut limit = (top + known.len()) as i64;
        let mut folded = String::new();
        let mut split = String::new();
        loop {
            let mut statement = self
                .connection
                .prepare_cached(
                    "SELECT id, name FROM files WHERE lower(name) >= ?1 AND lower(name) < ?2
                     ORDER BY length(CAST(name AS BLOB)), id LIMIT ?3",
                )
                .expect("prepare the name starts");
            let mut rows = statement
                .query(params![word, upper, limit])
                .expect("the name starts");
            let mut read = 0;
            let mut starts = Vec::new();
            while let Some(row) = rows.next().expect("read a name start") {
                read += 1;
                let id = row.get::<_, u32>(0).expect("an id");
                if known.contains(&id) {
                    continue;
                }
                let name = row.get::<_, String>(1).expect("a name");
                folded.clear();
                fold(&name, &mut folded);
                split.clear();
                split_words(&name, &mut split);
                if folded.starts_with(word) && query.matches(&split) {
                    starts.push((query.key(&folded, id), name));
                }
            }
            if starts.len() < need && read == limit && limit >= 0 {
                // Filtered below what was asked: every name start, then.
                limit = -1;
                continue;
            }
            known.extend(starts.iter().map(|(key, _)| key.2));
            out.extend(starts.into_iter().take(need));
            break;
        }
        if out.len() >= top {
            return rows_of(out);
        }

        // Word starts, the shortest first, past every row above.
        let need = top - out.len();
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT f.id, f.name FROM words CROSS JOIN files f ON f.id = words.rowid
                 WHERE words MATCH ?1 ORDER BY length(CAST(f.name AS BLOB)), f.id LIMIT ?2",
            )
            .expect("prepare the word starts");
        let mut rows = statement
            .query(params![query.fts(false), (need + known.len()) as i64])
            .expect("the word starts");
        let mut added = 0;
        while let Some(row) = rows.next().expect("read a word start") {
            let id = row.get::<_, u32>(0).expect("an id");
            if known.contains(&id) {
                continue;
            }
            let name = row.get::<_, String>(1).expect("a name");
            out.push(((2, 0, id), name));
            added += 1;
            if added == need {
                break;
            }
        }
        rows_of(out)
    }

    fn keyed(&self, query: &Query, top: usize) -> Vec<Row> {
        let mut out = self.exact(query);
        if out.len() >= top {
            out.truncate(top);
            return rows_of(out);
        }
        let mut known = out.iter().map(|(key, _)| key.2).collect::<HashSet<_>>();
        let word = query.key_word();
        let mut folded = String::new();
        // `^`: the name's first word starts with the longest word -- the
        // name starts, and names such as `.cargo` that the check drops.
        for initial in [true, false] {
            let mut statement = self
                .connection
                .prepare_cached("SELECT rowid FROM ranked WHERE ranked MATCH ?1")
                .expect("prepare the keyed words");
            let mut rows = statement
                .query([query.fts(initial)])
                .expect("the keyed words");
            while let Some(row) = rows.next().expect("read a keyed word") {
                let id = (row.get::<_, i64>(0).expect("a key") & 0xFFFF_FFFF) as u32;
                if known.contains(&id) {
                    continue;
                }
                let name = self.name(id);
                if initial {
                    folded.clear();
                    fold(&name, &mut folded);
                    if !folded.starts_with(word) {
                        continue;
                    }
                }
                known.insert(id);
                out.push(((0, 0, id), name));
                if out.len() == top {
                    return rows_of(out);
                }
            }
        }
        rows_of(out)
    }
}

fn rows_of(found: Vec<(Key, String)>) -> Vec<Row> {
    found
        .into_iter()
        .map(|((_, _, id), name)| Row { id, name })
        .collect()
}

/// The ranked section's tables, filled corpus by corpus.
#[derive(Default)]
struct RankedReport {
    sizes: String,
    splits: String,
    latency: String,
    fresh: String,
    scan: String,
    notes: String,
}

/// One build step of a split database: its label, its time, the file after.
struct Step {
    label: &'static str,
    took: Duration,
    bytes: u64,
}

/// V1 with a column of split words and an FTS5 index of it -- `detail=none`
/// when `compact` -- and, unless compact, the `lower(name)` index and the
/// length-keyed contentless word index. Each step its own transaction.
fn build_split(path: &Path, corpus: &Corpus, prepared: &Prepared, compact: bool) -> Vec<Step> {
    remove(path);
    let connection = Connection::open(path).expect("open a split database");
    connection
        .execute_batch(
            "PRAGMA journal_mode = OFF;
             PRAGMA synchronous = OFF;
             PRAGMA cache_size = -65536;",
        )
        .expect("configure the build");
    let mut steps = Vec::new();
    let mut step = |label: &'static str, work: &dyn Fn(&rusqlite::Transaction)| {
        let start = Instant::now();
        let transaction = connection.unchecked_transaction().expect("begin a step");
        work(&transaction);
        transaction.commit().expect("commit a step");
        steps.push(Step {
            label,
            took: start.elapsed(),
            bytes: size(path),
        });
    };

    step(
        "V1 + split words column",
        &|transaction: &rusqlite::Transaction| {
            transaction
                .execute_batch(
                    "CREATE TABLE folders (
                     id        INTEGER PRIMARY KEY,
                     parent_id INTEGER,
                     name      TEXT NOT NULL
                 );
                 CREATE TABLE files (
                     id        INTEGER PRIMARY KEY,
                     folder_id INTEGER NOT NULL,
                     scope     INTEGER NOT NULL,
                     name      TEXT    NOT NULL,
                     is_dir    INTEGER NOT NULL,
                     modified  INTEGER,
                     tokens    TEXT    NOT NULL
                 );",
                )
                .expect("create the tree");
            let mut insert = transaction
                .prepare("INSERT INTO folders (id, parent_id, name) VALUES (?1, ?2, ?3)")
                .expect("prepare the folders");
            for (id, (parent, name)) in
                (1i64..).zip(corpus.parents.iter().zip(&corpus.folder_names))
            {
                insert
                    .execute(params![id, parent.map(|parent| parent as i64 + 1), name])
                    .expect("insert a folder");
            }
            let mut insert = transaction
                .prepare(
                    "INSERT INTO files (id, folder_id, scope, name, is_dir, modified, tokens)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .expect("prepare the files");
            for ((id, item), split) in (1i64..).zip(&corpus.items).zip(&prepared.split) {
                insert
                    .execute(params![
                        id,
                        item.folder as i64 + 1,
                        item.scope,
                        item.name,
                        item.is_dir,
                        item.modified,
                        split
                    ])
                    .expect("insert a file");
            }
            transaction
                .execute_batch(
                    "CREATE INDEX folders_path ON folders (parent_id, name);
                 CREATE INDEX files_path ON files (folder_id, name);",
                )
                .expect("index the tree");
        },
    );
    let detail = if compact {
        ", detail=none, columnsize=0"
    } else {
        ""
    };
    step(
        "FTS5 split words",
        &|transaction: &rusqlite::Transaction| {
            transaction
                .execute_batch(&format!(
                    "CREATE VIRTUAL TABLE words USING fts5(
                     tokens, content='files', content_rowid='id', {SPLIT_TOKENIZER}{detail});
                 INSERT INTO words(words) VALUES ('rebuild');"
                ))
                .expect("build the split word index");
        },
    );
    if !compact {
        step(
            "index on lower(name)",
            &|transaction: &rusqlite::Transaction| {
                transaction
                    .execute_batch("CREATE INDEX files_lower ON files (lower(name));")
                    .expect("index the lower-cased names");
            },
        );
        step(
            "length-keyed contentless FTS5",
            &|transaction: &rusqlite::Transaction| {
                transaction
                .execute_batch(&format!(
                    "CREATE VIRTUAL TABLE ranked USING fts5(tokens, content='', {SPLIT_TOKENIZER});"
                ))
                .expect("create the keyed word index");
                let mut insert = transaction
                    .prepare("INSERT INTO ranked (rowid, tokens) VALUES (?1, ?2)")
                    .expect("prepare the keyed words");
                for ((id, folded), split) in (1i64..).zip(&prepared.folded).zip(&prepared.split) {
                    insert
                        .execute(params![((folded.len() as i64) << 32) | id, split])
                        .expect("insert keyed words");
                }
            },
        );
    }
    steps
}

/// Every rowid an FTS5 `words` table matches `fts` with.
fn rowids(connection: &Connection, fts: &str) -> HashSet<i64> {
    let mut statement = connection
        .prepare_cached("SELECT rowid FROM words WHERE words MATCH ?1")
        .expect("prepare the rowids");
    statement
        .query_map([fts], |row| row.get(0))
        .expect("match")
        .collect::<Result<_, _>>()
        .expect("a rowid")
}

/// Where `found` first differs from `expected`: `ok`, or `@i` and how many
/// of the N are missing.
fn verdict(found: &[Row], expected: &[u32]) -> String {
    let ids = found.iter().map(|row| row.id).collect::<Vec<_>>();
    if ids == expected {
        return "ok".to_string();
    }
    let at = ids
        .iter()
        .zip(expected)
        .position(|(a, b)| a != b)
        .unwrap_or(ids.len().min(expected.len()));
    let wanted = expected.iter().collect::<HashSet<_>>();
    let missing = expected.len() - ids.iter().filter(|id| wanted.contains(id)).count();
    format!("@{at} -{missing}")
}

fn measure_ranked(label: &str, corpus: &Corpus, dir: &Path, scan: bool, report: &mut RankedReport) {
    let entries = corpus.items.len();
    eprintln!("[{label}] ranked: {entries} entries");
    let dir = dir.join(format!("ranked-{}", label.replace(' ', "-")));
    std::fs::create_dir_all(&dir).expect("create the corpus folder");
    let prepared = prepare(corpus);

    // Sizes and builds: V1, the previous V2 and V2c, then the split ones.
    let mut row = |layer: &str, bytes: u64, took: Duration| {
        let _ = writeln!(
            report.sizes,
            "| {label} | {layer} | {:.1} | {:.0} | {:.1} |",
            mb(bytes),
            bytes as f64 / entries as f64,
            took.as_secs_f64()
        );
    };
    let mut previous = HashMap::new();
    for variant in [Variant::V1, Variant::V2, Variant::V2c] {
        let path = dir.join(variant.file());
        let took = build(&path, corpus, variant);
        eprintln!("[{label}] built {}", variant.label());
        row(
            &format!("{} (previous)", variant.label()),
            size(&path),
            took,
        );
        previous.insert(variant.file(), size(&path));
    }
    let split = dir.join("v2s.db");
    let steps = build_split(&split, corpus, &prepared, false);
    eprintln!("[{label}] built V2-split");
    let compact = dir.join("v2cs.db");
    let compact_steps = build_split(&compact, corpus, &prepared, true);
    eprintln!("[{label}] built V2c-split");
    let v1 = previous[Variant::V1.file()];
    let mut before = 0;
    let mut delta = HashMap::new();
    for step in &steps {
        row(
            &format!("V2-split step: {} (+ file growth)", step.label),
            step.bytes - before,
            step.took,
        );
        delta.insert(step.label, step.bytes - before);
        before = step.bytes;
    }
    let fts_step = |steps: &[Step]| (steps[0].took + steps[1].took, steps[1].bytes);
    let (took, bytes) = fts_step(&steps[..2]);
    row("**V2-split** (V1 + words column + FTS5 split)", bytes, took);
    let (took, bytes) = fts_step(&compact_steps);
    row(
        "**V2c-split** (V1 + words column + FTS5 split, detail=none)",
        bytes,
        took,
    );
    let _ = writeln!(
        report.sizes,
        "| {label} | whole V2-split file, all four steps | {:.1} | {:.0} | {:.1} |",
        mb(steps[3].bytes),
        steps[3].bytes as f64 / entries as f64,
        steps
            .iter()
            .map(|step| step.took)
            .sum::<Duration>()
            .as_secs_f64()
    );
    let keyed_only = v1 + delta["index on lower(name)"] + delta["length-keyed contentless FTS5"];
    let _ = writeln!(
        report.sizes,
        "| {label} | strategy c alone: V1 + lower(name) index + keyed FTS5 (computed) | {:.1} | {:.0} | - |",
        mb(keyed_only),
        keyed_only as f64 / entries as f64
    );

    // How many more names a word start finds once words are split.
    if !label.starts_with("synth") {
        let old = Connection::open_with_flags(
            dir.join(Variant::V2.file()),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("open V2");
        let new = Connection::open_with_flags(&split, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open V2-split");
        for probe in SPLIT_PROBES {
            let fts = Query::new(probe).fts(false);
            let before = rowids(&old, &fts);
            let after = rowids(&new, &fts);
            let gained = after.difference(&before).count();
            let lost = before.difference(&after).count();
            let union = before.union(&after).count();
            let examples = after
                .difference(&before)
                .take(3)
                .chain(before.difference(&after).take(2))
                .map(|&id| format!("`{}`", corpus.items[id as usize - 1].name))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                report.splits,
                "| {label} | `{probe}` | {} | {} | +{gained} | -{lost} | {union} | {examples} |",
                before.len(),
                after.len()
            );
        }
    }
    for variant in [Variant::V2, Variant::V2c] {
        remove(&dir.join(variant.file()));
    }

    // Ranked best N, warm.
    let full = Ranker::open(&split);
    let small = Ranker::open(&compact);
    let cases = [
        (&full, Strategy::All),
        (&small, Strategy::All),
        (&full, Strategy::Tiered),
        (&full, Strategy::Keyed),
    ];
    for text in RANKED_QUERIES {
        let query = Query::new(text);
        let (expected, tiers) = reference(&prepared, &query);
        eprintln!("[{label}] ranked `{text}`");
        for top in TOPS {
            let wanted = &expected[..top.min(expected.len())];
            let mut cells = Vec::new();
            let mut verdicts = Vec::new();
            for (index, (ranker, strategy)) in cases.iter().enumerate() {
                let found = ranker.run(*strategy, &query, top);
                let verdict = verdict(&found, wanted);
                if verdict != "ok" {
                    let at = found.iter().zip(wanted).find(|(row, id)| row.id != **id);
                    if let Some((row, id)) = at {
                        let _ = writeln!(
                            report.notes,
                            "- {label} `{text}` N={top} {}: got `{}` (#{}) where the reference has `{}` (#{id}).",
                            ["a", "a'", "b", "c"][index],
                            corpus.items[row.id as usize - 1].name,
                            row.id,
                            corpus.items[*id as usize - 1].name
                        );
                    }
                }
                verdicts.push(verdict);
                cells.push(cell(Some(&time(|| {
                    ranker.run(*strategy, &query, top).len()
                }))));
            }
            let _ = writeln!(
                report.latency,
                "| {label} | `{text}` | {} ({}/{}/{}) | {top} | {} | {} |",
                expected.len(),
                tiers[0],
                tiers[1],
                tiers[2],
                cells.join(" | "),
                verdicts.join(" / ")
            );
        }
    }
    drop(full);
    drop(small);

    // The first query of a fresh process, the file as the build and the
    // warm runs left it in the OS cache.
    for (path, strategy, name) in [
        (&split, Strategy::All, "a"),
        (&compact, Strategy::All, "a'"),
        (&split, Strategy::Tiered, "b"),
        (&split, Strategy::Keyed, "c"),
    ] {
        for text in ["inv", "html", "data"] {
            for top in TOPS {
                let printed = spawn(&[
                    "ranked",
                    strategy.name(),
                    &path.to_string_lossy(),
                    text,
                    &top.to_string(),
                ]);
                let cell = printed.map_or("failed".to_string(), |text| {
                    let parts = text.split_whitespace().collect::<Vec<_>>();
                    format!("{} + {} / {} | {}", parts[0], parts[1], parts[2], parts[3])
                });
                let _ = writeln!(
                    report.fresh,
                    "| {label} | {name} | `{text}` | {top} | {cell} |"
                );
            }
        }
    }

    // Search Files' mid-word plain scan on V1.
    let v1_path = dir.join(Variant::V1.file());
    if scan {
        let engine = Engine::open(&v1_path, Method::Like);
        for text in RANKED_QUERIES {
            eprintln!("[{label}] scan `{text}`");
            let all = time(|| engine.search(Method::Like, text, None).unwrap_or(0));
            let capped = time(|| engine.search(Method::Like, text, Some(LIMIT)).unwrap_or(0));
            let _ = writeln!(
                report.scan,
                "| {label} | `{text}` | {} | {} | {} |",
                all.matches,
                cell(Some(&all)),
                cell(Some(&capped))
            );
        }
    }

    for path in [&v1_path, &split, &compact] {
        remove(path);
    }
    let _ = std::fs::remove_dir(&dir);
}

fn print_ranked(report: &RankedReport) {
    println!("\n## R1. Size and build of the split word index\n");
    println!(
        "Built as the previous section builds: one transaction a step, `journal_mode=OFF`, 64 MB build cache. Words split before FTS5 sees them (at every non-letter-or-digit, and where a capital follows a non-capital or a digit a non-digit), lower-cased, joined by spaces; FTS5 `unicode61 remove_diacritics 0`, `prefix='1 2 3'`. A step's MB is how much the file grew.\n"
    );
    println!("| corpus | layer | MB | B/entry | build s |");
    println!("| - | - | - | - | - |");
    print!("{}", report.sizes);

    println!("\n## R2. What splitting at capitals and digits finds\n");
    println!(
        "Word-prefix matches (`\"q\"*`) on the previous V2 (separators `-_.()[]` and unicode61's own, diacritics removed) and on V2-split.\n"
    );
    println!(
        "| corpus | query | V2 | V2-split | gained | lost | union | examples (gained, then lost) |"
    );
    println!("| - | - | - | - | - | - | - | - |");
    print!("{}", report.splits);

    println!("\n## R3. Ranked best N, warm\n");
    println!(
        "Cells are p50 / p95 / max ms, 200 runs after 20 warm-ups (fewer, shown in brackets, when the first run took over 5 ms). Reference: every match ranked in memory by the Names rules on the split words -- exact, name start, word start, then the shorter folded name, then the entry; frecency omitted. Matches: all (exact / name start / word start). a: every match fetched and ranked in Rust, on V2-split; a': the same on V2c-split; b: tiered SQL; c: exact from the index, then a length-keyed word index read in its own order. Correct: `ok`, or `@i -m` (first differing position, how many of the reference's N are missing).\n"
    );
    println!("| corpus | query | matches (4/3/2) | N | a | a' | b | c | correct a / a' / b / c |");
    println!("| - | - | - | - | - | - | - | - | - |");
    print!("{}", report.latency);

    println!("\n## R4. First query in a fresh process\n");
    println!(
        "`open + first / second` ms in a new process; the file as the build and the warm runs left it in the OS cache.\n"
    );
    println!("| corpus | strategy | query | N | open + first / second | rows |");
    println!("| - | - | - | - | - | - |");
    print!("{}", report.fresh);

    println!("\n## R5. Search Files: mid-word plain scan on V1\n");
    println!("`LIKE '%q%'` on every word, warm. Cells as above.\n");
    println!("| corpus | query | matches | all | LIMIT {LIMIT} |");
    println!("| - | - | - | - | - |");
    print!("{}", report.scan);

    println!("\n## R6. Where a strategy differed from the reference\n");
    if report.notes.is_empty() {
        println!("- Every strategy returned the reference's N, in its order.");
    } else {
        print!("{}", report.notes);
    }
}

/// Every table, filled corpus by corpus and printed at the end.
#[derive(Default)]
struct Report {
    sizes: String,
    all: String,
    capped: String,
    cold: String,
    cache_check: String,
    memory: String,
    notes: String,
}

fn measure(label: &str, corpus: &Corpus, queries: &[String], dir: &Path, report: &mut Report) {
    let entries = corpus.items.len();
    eprintln!(
        "[{label}] {entries} entries, {} folders",
        corpus.paths.len()
    );
    let dir = dir.join(label.replace(' ', "-"));
    std::fs::create_dir_all(&dir).expect("create the corpus folder");

    for variant in [
        Variant::V0,
        Variant::V1,
        Variant::V2,
        Variant::V2c,
        Variant::V3,
    ] {
        let path = dir.join(variant.file());
        let built = build(&path, corpus, variant);
        let bytes = size(&path);
        eprintln!(
            "[{label}] built {} in {:.1} s",
            variant.label(),
            built.as_secs_f64()
        );
        let _ = writeln!(
            report.sizes,
            "| {label} | {} | {entries} | {} | {:.1} | {:.0} | {:.1} |",
            variant.label(),
            corpus.paths.len(),
            mb(bytes),
            bytes as f64 / entries as f64,
            built.as_secs_f64()
        );
        if variant == Variant::V0 {
            remove(&path);
        }
    }

    let v1 = dir.join(Variant::V1.file());
    let start = Instant::now();
    let blob = Engine::open(&v1, Method::Blob);
    let loaded = start.elapsed();
    let names_bytes = blob.names.as_ref().map_or(0, Names::memory_bytes) as u64;
    let _ = writeln!(
        report.sizes,
        "| {label} | V4 names in memory, from V1 | {entries} | - | {:.1} (memory) | {:.0} | {:.1} (load from V1) |",
        mb(names_bytes),
        names_bytes as f64 / entries as f64,
        loaded.as_secs_f64()
    );

    // Warm: one connection per variant, every query, all and capped.
    let engines = METHODS
        .iter()
        .map(|&method| {
            if method == Method::Blob {
                None
            } else {
                Some(Engine::open(&dir.join(method.variant().file()), method))
            }
        })
        .collect::<Vec<_>>();
    for query in queries {
        eprintln!("[{label}] warm `{query}`");
        let mut all = Vec::new();
        let mut capped = Vec::new();
        for (index, &method) in METHODS.iter().enumerate() {
            let engine = engines[index].as_ref().unwrap_or(&blob);
            for (limit, out) in [(None, &mut all), (Some(LIMIT), &mut capped)] {
                let timing = engine
                    .search(method, query, limit)
                    .map(|_| time(|| engine.search(method, query, limit).unwrap_or(0)));
                out.push(timing);
            }
        }
        let matches = |timings: &[Option<Timing>], method: Method| {
            let index = METHODS.iter().position(|&m| m == method).expect("a method");
            timings[index]
                .as_ref()
                .map_or("n/a".to_string(), |t| t.matches.to_string())
        };
        let substring = all[METHODS.iter().position(|&m| m == Method::Like).unwrap()]
            .as_ref()
            .map(|t| t.matches);
        let trigram = all[METHODS.iter().position(|&m| m == Method::Trigram).unwrap()]
            .as_ref()
            .map(|t| t.matches);
        if substring != trigram {
            let _ = writeln!(
                report.notes,
                "- {label} `{query}`: LIKE found {substring:?}, trigram {trigram:?}."
            );
        }
        if matches(&all, Method::Stream) != matches(&all, Method::Blob) {
            let _ = writeln!(
                report.notes,
                "- {label} `{query}`: stream found {}, Names {}.",
                matches(&all, Method::Stream),
                matches(&all, Method::Blob)
            );
        }
        if matches(&all, Method::Words) != matches(&all, Method::WordsCompact) {
            let _ = writeln!(
                report.notes,
                "- {label} `{query}`: V2 found {}, V2c {}.",
                matches(&all, Method::Words),
                matches(&all, Method::WordsCompact)
            );
        }
        for (out, timings) in [(&mut report.all, &all), (&mut report.capped, &capped)] {
            let _ = write!(
                out,
                "| {label} | `{query}` | {} / {} / {} |",
                matches(&all, Method::Words),
                substring.map_or("n/a".to_string(), |n| n.to_string()),
                matches(&all, Method::Blob)
            );
            for timing in timings {
                let _ = write!(out, " {} |", cell(timing.as_ref()));
            }
            out.push('\n');
        }
    }
    drop(engines);
    drop(blob);

    // Cold-ish: a fresh process. First with the OS cache as the build and
    // the warm runs left it, then on a copy written without the cache.
    let cold_dir = dir.join("cold");
    let cold_cases = [
        (Method::Words, "inv"),
        (Method::WordsCompact, "inv"),
        (Method::Trigram, "invoice"),
        (Method::Like, "invoice"),
        (Method::Stream, "invoice"),
        (Method::Blob, "invoice"),
    ];
    if let Some(copy) = unbuffered_copy(&dir.join(Variant::V3.file()), &cold_dir) {
        let first = read_ms(&copy);
        let second = read_ms(&copy);
        let _ = writeln!(
            report.cache_check,
            "| {label} | {:.0} MB | {first:.0} | {second:.0} |",
            mb(size(&copy))
        );
        remove(&copy);
    } else {
        let _ = writeln!(report.cache_check, "| {label} | robocopy unavailable | | |");
    }
    for (method, query) in cold_cases {
        eprintln!("[{label}] cold {} `{query}`", method.name());
        let source = dir.join(method.variant().file());
        let warm = spawn(&["cold", method.name(), &source.to_string_lossy(), query]);
        let copy = unbuffered_copy(&source, &cold_dir);
        let uncached = copy
            .as_ref()
            .and_then(|copy| spawn(&["cold", method.name(), &copy.to_string_lossy(), query]));
        if let Some(copy) = copy {
            remove(&copy);
        }
        let split = |text: Option<String>| -> (String, String) {
            match text {
                Some(text) => {
                    let parts = text.split_whitespace().collect::<Vec<_>>();
                    (
                        format!("{} + {} / {}", parts[0], parts[1], parts[2]),
                        parts[3].to_string(),
                    )
                }
                None => ("failed".to_string(), "-".to_string()),
            }
        };
        let (warm, matches) = split(warm);
        let (uncached, _) = split(uncached);
        let _ = writeln!(
            report.cold,
            "| {label} | {} | `{query}` | {warm} | {uncached} | {matches} |",
            method.name()
        );
    }

    // Memory: a fresh process per method, after every query twice.
    let joined = queries.join("|");
    let base = spawn(&["mem", "none", "-", "-"]);
    for (method, text) in std::iter::once((None, base)).chain(
        [
            Method::Words,
            Method::WordsCompact,
            Method::Trigram,
            Method::Stream,
            Method::Blob,
        ]
        .into_iter()
        .map(|method| {
            eprintln!("[{label}] memory {}", method.name());
            let db = dir.join(method.variant().file());
            let text = spawn(&["mem", method.name(), &db.to_string_lossy(), &joined]);
            (Some(method), text)
        })
        .collect::<Vec<_>>(),
    ) {
        let name = method.map_or("no database (baseline)", |method| match method {
            Method::Words => "v2 words, all queries",
            Method::WordsCompact => "v2c words, all queries",
            Method::Trigram => "v3 trigram, queries >= 3 chars",
            Method::Stream => "v1 stream scan, all queries",
            _ => "v4 names loaded from V1, all queries",
        });
        let numbers = text
            .unwrap_or_default()
            .split_whitespace()
            .map(|n| n.parse::<i64>().unwrap_or(0))
            .collect::<Vec<_>>();
        if numbers.len() == 6 {
            let _ = writeln!(
                report.memory,
                "| {label} | {name} | {:.1} | {:.1} | {:.1} | {:.1} / {:.1} | {:.1} |",
                mb(numbers[0] as u64),
                mb(numbers[1] as u64),
                mb(numbers[2] as u64),
                mb(numbers[3] as u64),
                mb(numbers[4] as u64),
                mb(numbers[5] as u64)
            );
        } else {
            let _ = writeln!(report.memory, "| {label} | {name} | failed | | | | |");
        }
    }

    for variant in [Variant::V1, Variant::V2, Variant::V2c, Variant::V3] {
        remove(&dir.join(variant.file()));
    }
    let _ = std::fs::remove_dir(&cold_dir);
    let _ = std::fs::remove_dir(&dir);
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("--child") {
        child(&args[1..]);
        return;
    }
    let started = Instant::now();
    // `DISK_BENCH=ranked` runs only the ranked best N; the rest takes a
    // quarter of an hour.
    let only_ranked = std::env::var("DISK_BENCH").as_deref() == Ok("ranked");
    // `DISK_BENCH=store` measures the production store alone (step c2b).
    let only_store = std::env::var("DISK_BENCH").as_deref() == Ok("store");
    // `DISK_BENCH=root` measures root's query, the hot layer then the disk
    // (step c3a).
    let only_root = std::env::var("DISK_BENCH").as_deref() == Ok("root");
    // `DISK_BENCH=files` measures *Search Files*' two phases (step c3b).
    let only_files = std::env::var("DISK_BENCH").as_deref() == Ok("files");

    // `target/tmp` is where cargo points benches; its parent is `target`.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("the target directory")
        .join("disk-bench");
    std::fs::create_dir_all(&dir).expect("create the bench folder");

    let real_db = dir.join("real.db");
    if !real_db.exists() {
        let local = std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA");
        let source = Path::new(&local).join(r"Lumen\cache\files.db");
        std::fs::copy(&source, &real_db).expect("copy the real index");
    }

    if only_store {
        store_section(&dir, &real_db, started);
        return;
    }
    if only_root {
        root_section(&dir, &real_db, started);
        return;
    }
    if only_files {
        files_section(&dir, &real_db, started);
        return;
    }

    println!("# File index on disk: hot layer and full index");
    let check = Connection::open_in_memory().expect("an in-memory database");
    let version: String = check
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .expect("the version");
    let fts5: bool = check
        .query_row(
            "SELECT sqlite_compileoption_used('ENABLE_FTS5')",
            [],
            |row| row.get(0),
        )
        .expect("the FTS5 option");
    let trigram = check
        .execute_batch("CREATE VIRTUAL TABLE t USING fts5(x, tokenize='trigram');")
        .is_ok();
    println!("\nSQLite {version}; ENABLE_FTS5 = {fts5}; trigram tokenizer = {trigram}.");
    drop(check);

    let rows = load_real(&real_db);
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| r"C:\Users\kiza2".to_string());
    let in_home = |row: &&RealRow| {
        row.parent == home
            || row
                .parent
                .strip_prefix(&home)
                .is_some_and(|rest| rest.starts_with('\\'))
    };
    let under_dot_folder = |row: &RealRow| {
        row.parent[home.len()..]
            .split('\\')
            .any(|part| part.starts_with('.'))
            || (row.is_dir && row.name.starts_with('.'))
    };
    let home_rows = rows.iter().filter(in_home).collect::<Vec<_>>();
    let without_dot_folders = home_rows
        .iter()
        .filter(|row| !under_dot_folder(row))
        .count();
    let without_dot_entries = home_rows
        .iter()
        .filter(|row| !under_dot_folder(row) && !row.name.starts_with('.'))
        .count();
    let real = corpus_of(&rows);
    let home_corpus = corpus_of(home_rows.iter().copied());
    println!("\n## Real corpus counts\n");
    println!("| set | entries | folders (distinct paths incl. ancestors) |");
    println!("| - | - | - |");
    println!(
        "| real index, all | {} | {} |",
        real.items.len(),
        real.paths.len()
    );
    println!(
        "| home (`{home}`) | {} | {} |",
        home_corpus.items.len(),
        home_corpus.paths.len()
    );
    println!("| home without dot-folders | {without_dot_folders} | |");
    println!("| home without dot-folders and dotfiles | {without_dot_entries} | |");
    println!(
        "| real index, folders (`is_dir`) | {} | |",
        rows.iter().filter(|row| row.is_dir).count()
    );
    drop(home_rows);
    drop(rows);

    let frequent = frequent_words(&real);
    println!("\nMost frequent real name words (alphabetic, >= 4 letters):");
    for (rank, (word, count)) in frequent.iter().take(10).enumerate() {
        println!("{}. `{word}` {count}", rank + 1);
    }
    let picked = [0, 9, 99]
        .iter()
        .filter_map(|&rank| frequent.get(rank))
        .map(|(word, count)| (word.clone(), *count))
        .collect::<Vec<_>>();
    println!(
        "\nReal words queried (ranks 1, 10, 100): {}",
        picked
            .iter()
            .map(|(word, count)| format!("`{word}` ({count})"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let queries = QUERIES
        .iter()
        .map(|query| query.to_string())
        .chain(picked.into_iter().map(|(word, _)| word))
        .collect::<Vec<_>>();

    let mut report = Report::default();
    let mut ranked = RankedReport::default();
    if !only_ranked {
        measure("real", &real, &queries, &dir, &mut report);
    }
    measure_ranked("real", &real, &dir, true, &mut ranked);
    drop(real);
    if !only_ranked {
        measure("home", &home_corpus, &queries, &dir, &mut report);
    }
    measure_ranked("home", &home_corpus, &dir, false, &mut ranked);
    drop(home_corpus);
    for count in SYNTHETIC {
        let corpus = synthetic(count);
        let label = format!("synth {}M", count / 1_000_000);
        if !only_ranked {
            measure(&label, &corpus, &queries, &dir, &mut report);
        }
        let scan = count == SYNTHETIC[SYNTHETIC.len() - 1];
        measure_ranked(&label, &corpus, &dir, scan, &mut ranked);
    }

    print_ranked(&ranked);
    if only_ranked {
        println!(
            "\nRanked section only. Total run {:.0} s.",
            started.elapsed().as_secs_f64()
        );
        remove(&real_db);
        return;
    }

    let columns = METHODS
        .iter()
        .map(|method| match method {
            Method::Words => "V2 words",
            Method::WordsCompact => "V2c words",
            Method::Trigram => "V3 trigram",
            Method::Like => "V1 LIKE scan",
            Method::Stream => "V1 stream + memmem",
            Method::Blob => "V4 Names::find",
        })
        .collect::<Vec<_>>();
    let header = format!(
        "| corpus | query | matches words / substring / Names | {} |\n| - | - | - |{}",
        columns.join(" | "),
        " - |".repeat(columns.len())
    );

    println!("\n## 1. Size and build\n");
    println!(
        "Bulk insert in one transaction, `journal_mode=OFF`, `synchronous=OFF`, 64 MB build cache; indexes after the insert, then the FTS `rebuild`. Sizes are the file on disk.\n"
    );
    println!("| corpus | variant | entries | folders | MB | B/entry | build s |");
    println!("| - | - | - | - | - | - | - |");
    print!("{}", report.sizes);

    println!("\n## 2a. Warm queries, every match counted\n");
    println!(
        "Cells are p50 / p95 / max ms; 20 warm-ups and 200 runs, or (n) runs when the first run took over 5 ms (5 warm-ups, 50 runs) or 50 ms (2 warm-ups, 15 runs). SQLite at `cache_size=-8192`, OS cache warm. SQLite methods count rowids only (no names fetched); V4 ranks every match. `n/a`: a trigram or LIKE cannot answer a word under 3 characters.\n"
    );
    println!("{header}");
    print!("{}", report.all);

    println!("\n## 2b. Warm queries, LIMIT {LIMIT}\n");
    println!(
        "SQLite methods stop at the first {LIMIT} in storage order; V4 still scans every name and keeps the best {LIMIT} ranked.\n"
    );
    println!("{header}");
    print!("{}", report.capped);

    println!("\n## 3. Cold-ish first query (fresh process)\n");
    println!(
        "`open + first / second` ms in a new process: open is the connection and pragmas (for v4, loading every name from V1). \"OS warm\" is the file as the build left it in the OS cache; \"uncached copy\" is a copy written by `robocopy /J` (unbuffered I/O) just before. Purging the cache itself needs administrator rights and was not done.\n"
    );
    println!("| corpus | method | query | OS warm | uncached copy | matches |");
    println!("| - | - | - | - | - | - |");
    print!("{}", report.cold);
    println!(
        "\nWhether the copy was really uncached: a full sequential read of the V3 copy right after it was written, then again.\n"
    );
    println!("| corpus | file | first read ms | second read ms |");
    println!("| - | - | - | - |");
    print!("{}", report.cache_check);

    println!("\n## 4. Memory of a querying process\n");
    println!(
        "A fresh process per row, every query run twice (all and LIMIT {LIMIT}), then measured. Working set, peak and private bytes from `Get-Process`; SQLite's own allocator from `sqlite3_memory_used` / `sqlite3_memory_highwater`.\n"
    );
    println!(
        "| corpus | process | working set MB | peak WS MB | private MB | SQLite used / high MB | Names bytes MB |"
    );
    println!("| - | - | - | - | - | - | - |");
    print!("{}", report.memory);

    println!("\n## Notes\n");
    if report.notes.is_empty() {
        println!("- Every cross-check agreed (LIKE = trigram, stream = Names, V2 = V2c).");
    } else {
        print!("{}", report.notes);
    }
    println!(
        "\nSynthetic corpora: slice 0's names, one entry in eleven a folder under a random earlier folder (folder names from slice 0's folder list). Total run {:.0} s.",
        started.elapsed().as_secs_f64()
    );

    remove(&real_db);
}

// The production store (step c2b): `Store::replace` writes each corpus at
// each `files.index-detail` level, as a rebuild does, and `Store::ranked`
// and `Store::substring` are timed on it with the store's own page cache.

/// Root's queries: slice 0's, the design's examples, frequent real words
/// and the two typed whole across a capital.
const STORE_QUERIES: [&str; 13] = [
    "inv",
    "invoice",
    "report",
    "2024",
    "html",
    "license",
    "preview",
    "zqxj",
    "inv 2024",
    "cargo",
    "data",
    "javascript",
    "github",
];

/// *Search Files*' mid-word queries.
const SUBSTRING_QUERIES: [&str; 4] = ["inv", "report", "html", "zqxj"];

/// The queries whose first run is timed in a fresh process.
const FRESH_QUERIES: [&str; 2] = ["invoice", "inv 2024"];

/// Root's budget for a frame, which a ranked query must stay under at p95.
const BUDGET_MS: f64 = 8.0;

/// Root's queries in step c3a: the store's, `a` and `in` -- which root
/// answers from the hot layer alone -- and without `data`.
const ROOT_QUERIES: [&str; 14] = [
    "a",
    "in",
    "inv",
    "invoice",
    "report",
    "2024",
    "html",
    "license",
    "preview",
    "zqxj",
    "inv 2024",
    "cargo",
    "javascript",
    "github",
];

/// How many entries the synthetic frecency of the root section has used.
const USED: usize = 200;

/// Step c3a: the production store of the real index, loaded as a start
/// loads it -- the hot layer within `HOT_BUDGET` -- with a synthetic
/// frecency of [`USED`] entries, and root's query timed on it: the hot layer,
/// then the disk, merged.
fn root_section(dir: &Path, real_db: &Path, started: Instant) {
    use lumen_core::file_index::{FileIndex, HOT_BUDGET};
    use lumen_core::frecency::Frecency;

    println!("# Root's query: the hot layer, then the disk (step c3a)");
    let rows = load_real(real_db);
    let real = corpus_of(&rows);
    drop(rows);
    let walked = walked_of(&real);
    drop(real);
    let entries = walked.entries.len();
    progress(started, &format!("real: {entries} entries"));

    let path = dir.join("store-root.db");
    remove(&path);
    let mut store = Store::open_with(&path, IndexDetail::Balanced).expect("open the store");
    store.replace(&walked, "bench").expect("write the store");
    drop(store);
    progress(started, "built");

    // Every name, as a start loaded them before the hot layer.
    let mut every = Vec::new();
    for _ in 0..5 {
        let store = Store::open(&path).expect("open the store");
        let start = Instant::now();
        let (names, _) = store.load().expect("load every name");
        every.push(start.elapsed());
        drop(names);
    }
    every.sort();
    // The hot layer, as a start loads it now.
    let mut hot = Vec::new();
    for _ in 0..5 {
        let store = Store::open(&path).expect("open the store");
        let start = Instant::now();
        let index = FileIndex::load(store).expect("load the index");
        hot.push(start.elapsed());
        drop(index);
    }
    hot.sort();
    let every_bytes = Store::open(&path)
        .and_then(|store| store.load())
        .map(|(names, _)| names.memory_bytes())
        .expect("load every name");

    let index = FileIndex::load(Store::open(&path).expect("open the store")).expect("load");
    let mut rng = Rng(0x5eed_c3a0);
    let mut frecency = Frecency::new();
    for _ in 0..USED {
        let entry = &walked.entries[rng.below(entries)];
        let prefix = if entry.is_dir { "folder" } else { "file" };
        frecency.record(&format!("{prefix}:{}", entry.path.display()));
    }
    drop(walked);
    let start = Instant::now();
    index.set_boosts(&frecency);
    let boosts_ms = ms(start.elapsed());
    progress(started, "loaded");

    println!("\n## The hot layer\n");
    println!("| | |\n| - | - |");
    println!("| entries | {} |", index.len());
    println!(
        "| hot entries | {} ({:.1} %) |",
        index.hot_len(),
        100.0 * index.hot_len() as f64 / index.len() as f64
    );
    println!("| complete | {} |", index.hot_is_complete());
    println!("| `HOT_BUDGET` | {:.1} MB |", mb(HOT_BUDGET as u64));
    println!(
        "| in memory (hot layer, used outside it, boosts, short) | {:.1} MB |",
        mb(index.memory_bytes() as u64)
    );
    println!(
        "| every name in memory, as before | {:.1} MB |",
        mb(every_bytes as u64)
    );
    println!(
        "| load, hot layer: min / median / max | {:.0} / {:.0} / {:.0} ms |",
        ms(hot[0]),
        ms(hot[2]),
        ms(hot[4])
    );
    println!(
        "| load, every name: min / median / max | {:.0} / {:.0} / {:.0} ms |",
        ms(every[0]),
        ms(every[2]),
        ms(every[4])
    );
    println!("| `set_boosts`, {USED} used | {boosts_ms:.1} ms |");

    println!("\n## `FileIndex::root_query`, N = 8, warm\n");
    println!(
        "p50 / p95 / max ms; folders and files; {USED} entries used. Under three characters root asks the hot layer alone.\n"
    );
    println!("| query | ms | rows | p95 < {BUDGET_MS} |");
    println!("| - | - | - | - |");
    let mut worst = (0.0f64, "");
    for query in ROOT_QUERIES {
        let timing = time(|| {
            index
                .root_query(query, Kinds::FoldersAndFiles, 8, &|| false)
                .len()
        });
        let p95 = ms(timing.p95);
        if p95 > worst.0 {
            worst = (p95, query);
        }
        println!(
            "| `{query}` | {} | {} | {} |",
            cell(Some(&timing)),
            timing.matches,
            if p95 < BUDGET_MS { "yes" } else { "NO" }
        );
    }
    println!("\nWorst p95: {:.2} ms, `{}`.", worst.0, worst.1);
    println!("\nTotal run {:.0} s.", started.elapsed().as_secs_f64());
    drop(index);
    remove(&path);
    remove(real_db);
}

/// *Search Files*' queries in step c3b.
const FILES_QUERIES: [&str; 5] = ["inv", "report", "html", "zqxj", "license"];

/// Step c3b: *Search Files* on the production store of the real index, at
/// `balanced` and `complete`, loaded as a start loads it with [`USED`]
/// entries used. Each query is timed to its first phase -- the hot layer
/// and the disk's word index, what the view draws at once -- and to its
/// second, the matches inside a word outside the hot layer, asked for what
/// room the first left under the limit.
fn files_section(dir: &Path, real_db: &Path, started: Instant) {
    use lumen_core::file_index::FileIndex;
    use lumen_core::file_search::{self, LIMIT as VIEW_LIMIT};
    use lumen_core::frecency::Frecency;

    println!("# Search Files on disk: two phases (step c3b)");
    let rows = load_real(real_db);
    let real = corpus_of(&rows);
    drop(rows);
    let walked = walked_of(&real);
    drop(real);
    let entries = walked.entries.len();
    progress(started, &format!("real: {entries} entries"));
    let mut rng = Rng(0x5eed_c3b0);
    let mut frecency = Frecency::new();
    for _ in 0..USED {
        let entry = &walked.entries[rng.below(entries)];
        let prefix = if entry.is_dir { "folder" } else { "file" };
        frecency.record(&format!("{prefix}:{}", entry.path.display()));
    }

    println!(
        "\np50 / p95 / max ms, warm, the store's own page cache; folders and files, no filter, limit {VIEW_LIMIT}. *First* is `file_search::search`, drawn at once. *Second* is `file_search::search_inside` for the room left, run after the first; `--` when the first filled the limit and no second runs. *Both* is the two in a row: when the list is complete. *Second, room {VIEW_LIMIT}* is the second phase asked for a whole limit, what it costs when the first phase finds little. Rows are what each gave.\n"
    );
    println!(
        "| level | hot | query | first | rows | second | rows | both | second, room {VIEW_LIMIT} | rows |"
    );
    println!("| - | - | - | - | - | - | - | - | - | - |");
    for level in [IndexDetail::Balanced, IndexDetail::Complete] {
        let path = dir.join("store-files.db");
        remove(&path);
        let mut store = Store::open_with(&path, level).expect("open the store");
        store.replace(&walked, "bench").expect("write the store");
        drop(store);
        let index = FileIndex::load(Store::open(&path).expect("open the store")).expect("load");
        index.set_boosts(&frecency);
        progress(started, &format!("{level:?}: built and loaded"));
        let hot = format!(
            "{:.1} %{}",
            100.0 * index.hot_len() as f64 / index.len() as f64,
            if index.hot_is_complete() {
                ", complete"
            } else {
                ""
            }
        );
        for query in FILES_QUERIES {
            let first = time(|| file_search::search(&index, query, None, &|| false).len());
            let room = VIEW_LIMIT - first.matches;
            let inside = file_search::searches_inside(&index, query);
            let second = (inside && room > 0).then(|| {
                time(|| file_search::search_inside(&index, query, None, room, &|| false).len())
            });
            let whole = inside.then(|| {
                time(|| {
                    file_search::search_inside(&index, query, None, VIEW_LIMIT, &|| false).len()
                })
            });
            let both = time(|| {
                let found = file_search::search(&index, query, None, &|| false).len();
                found
                    + file_search::search_inside(&index, query, None, VIEW_LIMIT - found, &|| false)
                        .len()
            });
            println!(
                "| {level:?} | {hot} | `{query}` | {} | {} | {} | {} | {} | {} | {} |",
                cell(Some(&first)),
                first.matches,
                second
                    .as_ref()
                    .map_or("--".to_string(), |second| cell(Some(second))),
                second.as_ref().map_or(0, |second| second.matches),
                cell(Some(&both)),
                whole
                    .as_ref()
                    .map_or("--".to_string(), |whole| cell(Some(whole))),
                whole.as_ref().map_or(0, |whole| whole.matches),
            );
        }
        drop(index);
        remove(&path);
    }
    drop(walked);
    println!("\nTotal run {:.0} s.", started.elapsed().as_secs_f64());
    remove(real_db);
}

/// `corpus` as a walk returns it: every entry's full path, scope by scope,
/// each folder before what is in it, as both generators and the real index
/// already order them.
fn walked_of(corpus: &Corpus) -> Walked {
    let mut order = (0..corpus.items.len()).collect::<Vec<_>>();
    order.sort_by_key(|&index| corpus.items[index].scope);
    let scopes = corpus
        .items
        .iter()
        .map(|item| item.scope)
        .max()
        .map_or(0, |last| last as usize + 1);
    let mut walked = Walked {
        per_scope: vec![0; scopes],
        roots: vec![None; scopes],
        ..Walked::default()
    };
    for index in order {
        let item = &corpus.items[index];
        walked.per_scope[item.scope as usize] += 1;
        walked.entries.push(Entry {
            path: Path::new(&corpus.paths[item.folder as usize]).join(&item.name),
            is_dir: item.is_dir,
            modified: item.modified,
        });
    }
    walked
}

/// The store section's tables, filled corpus by corpus.
#[derive(Default)]
struct StoreReport {
    sizes: String,
    switches: String,
    ranked: String,
    fresh: String,
    substring: String,
    scan: String,
    notes: String,
    /// Each level's worst p95 at the largest corpus, and the queries over.
    acceptance: String,
}

fn store_section(dir: &Path, real_db: &Path, started: Instant) {
    println!("# The file index's store, ranked from disk (step c2b)");
    let rows = load_real(real_db);
    let real = corpus_of(&rows);
    drop(rows);
    let mut report = StoreReport::default();
    let sizes = store_sizes();
    let levels = store_levels();
    let largest = sizes.iter().flatten().max().copied();
    for size in sizes {
        let (label, walked) = match size {
            None => ("real".to_string(), walked_of(&real)),
            Some(count) if count % 1_000_000 == 0 => (
                format!("synth {}M", count / 1_000_000),
                walked_of(&synthetic(count)),
            ),
            Some(count) => (
                format!("synth {}k", count / 1_000),
                walked_of(&synthetic(count)),
            ),
        };
        progress(
            started,
            &format!("{label}: {} entries", walked.entries.len()),
        );
        let last = size.is_some() && size == largest;
        measure_store(&label, &walked, dir, last, &levels, started, &mut report);
    }
    drop(real);

    println!("\n## Size and build, per level\n");
    println!(
        "`Store::open_with(level)` then `Store::replace`, the write path a rebuild takes; the file as written, then vacuumed.\n"
    );
    println!("| corpus | level | entries | MB | vacuumed MB | build s |");
    println!("| - | - | - | - | - | - |");
    print!("{}", report.sizes);

    println!("\n## Changing the level\n");
    println!("`Store::set_detail` on the `balanced` store: only the word tables, no walk.\n");
    println!("| corpus | change | s | MB after |");
    println!("| - | - | - | - |");
    print!("{}", report.switches);

    println!("\n## `Store::ranked`, warm\n");
    println!(
        "p50 / p95 / max ms, 20 warm-ups and 200 runs (fewer for a slow first run, in brackets). Folders and files, no filter, the store's 1 MB page cache, OS cache warm. `rows` is what came back; `ok` is p95 under {BUDGET_MS} ms for both N.\n"
    );
    println!("| corpus | level | query | N=8 | rows | N=64 | rows | p95 < {BUDGET_MS} |");
    println!("| - | - | - | - | - | - | - | - |");
    print!("{}", report.ranked);

    println!("\n## First query in a fresh process\n");
    println!("`open / first / second` ms; the file as the build left it in the OS cache.\n");
    println!("| corpus | level | query | N=8 | N=64 |");
    println!("| - | - | - | - | - |");
    print!("{}", report.fresh);

    println!("\n## `Store::substring`, warm\n");
    println!(
        "Mid-word matches only. `complete` asks the trigram index; `balanced` and `compact` scan every folded name a length at a time.\n"
    );
    println!("| corpus | level | query | all | matches | limit 200 | rows |");
    println!("| - | - | - | - | - | - | - |");
    print!("{}", report.substring);

    println!("\n## Keeping `folded`, or folding while scanning\n");
    println!(
        "A full pass over every name looking for `report`: the stored `folded` column, or the name folded in Rust as it is read -- what dropping the column would cost a scan.\n"
    );
    println!("| corpus | `folded` stored, MB | scan `folded` | scan `name` + fold |");
    println!("| - | - | - | - |");
    print!("{}", report.scan);

    println!("\n## Acceptance: p95 < {BUDGET_MS} ms at the largest corpus\n");
    println!("| level | N | worst query | worst p95 ms | misses |");
    println!("| - | - | - | - | - |");
    print!("{}", report.acceptance);

    println!("\n## Notes\n");
    if report.notes.is_empty() {
        println!("- Every level answered every ranked query identically.");
    } else {
        print!("{}", report.notes);
    }
    println!("\nTotal run {:.0} s.", started.elapsed().as_secs_f64());
    remove(real_db);
}

/// The corpora of the store section: `DISK_BENCH_SIZES`, a comma-separated
/// list of `real` and entry counts (`real,100000`), else the real index and
/// every [`SYNTHETIC`] size. `None` is the real index.
fn store_sizes() -> Vec<Option<usize>> {
    match std::env::var("DISK_BENCH_SIZES") {
        Ok(list) => list
            .split(',')
            .map(|size| match size.trim() {
                "real" => None,
                count => Some(count.parse().expect("a size is `real` or a count")),
            })
            .collect(),
        Err(_) => std::iter::once(None)
            .chain(SYNTHETIC.into_iter().map(Some))
            .collect(),
    }
}

/// The levels of the store section: `DISK_BENCH_LEVELS`, comma-separated,
/// else every level.
fn store_levels() -> Vec<IndexDetail> {
    match std::env::var("DISK_BENCH_LEVELS") {
        Ok(list) => list
            .split(',')
            .map(|level| IndexDetail::parse(level.trim()).expect("a level"))
            .collect(),
        Err(_) => IndexDetail::ALL.to_vec(),
    }
}

/// A step of the run, on stderr with the seconds since it started: what a
/// stall is found by, since the report is printed only at the end.
fn progress(started: Instant, step: &str) {
    eprintln!("[{:7.1} s] {step}", started.elapsed().as_secs_f64());
}

fn measure_store(
    label: &str,
    walked: &Walked,
    dir: &Path,
    last: bool,
    levels: &[IndexDetail],
    started: Instant,
    report: &mut StoreReport,
) {
    let any = |_: &str, _: bool| true;
    let mut answers: Vec<Vec<Vec<u32>>> = Vec::new();
    for &detail in levels {
        let level = detail.as_str();
        let path = dir.join(format!("store-{level}.db"));
        remove(&path);
        progress(started, &format!("{label} {level}: building"));
        let mut store = Store::open_with(&path, detail).expect("open the store");
        let start = Instant::now();
        store.replace(walked, "bench").expect("write the store");
        let built = start.elapsed();
        drop(store);
        let bytes = size(&path);
        let vacuumed = {
            let copy = dir.join("store-vacuumed.db");
            remove(&copy);
            let connection = Connection::open(&path).expect("open for vacuum");
            connection
                .execute("VACUUM INTO ?1", [copy.to_string_lossy()])
                .expect("vacuum");
            drop(connection);
            let bytes = size(&copy);
            remove(&copy);
            bytes
        };
        let _ = writeln!(
            report.sizes,
            "| {label} | {level} | {} | {:.1} | {:.1} | {:.1} |",
            walked.entries.len(),
            mb(bytes),
            mb(vacuumed),
            built.as_secs_f64()
        );
        progress(
            started,
            &format!("{label} {level}: built in {:.1} s", built.as_secs_f64()),
        );

        let store = Store::open(&path).expect("open the store");
        let mut found = Vec::new();
        let mut worst = [(0.0f64, ""); 2];
        let mut misses: [Vec<&str>; 2] = [Vec::new(), Vec::new()];
        for query in STORE_QUERIES {
            progress(started, &format!("{label} {level}: ranked `{query}`"));
            let mut cells = Vec::new();
            let mut fast = true;
            for (at, top) in [8, 64].into_iter().enumerate() {
                let ids = store
                    .ranked(query, Kinds::FoldersAndFiles, top, &any)
                    .expect("a ranked query")
                    .iter()
                    .map(|row| row.id)
                    .collect::<Vec<_>>();
                let timing = time(|| {
                    store
                        .ranked(query, Kinds::FoldersAndFiles, top, &any)
                        .expect("a ranked query")
                        .len()
                });
                let p95 = ms(timing.p95);
                if p95 >= BUDGET_MS {
                    fast = false;
                    misses[at].push(query);
                }
                if p95 > worst[at].0 {
                    worst[at] = (p95, query);
                }
                cells.push(format!("{} | {}", cell(Some(&timing)), timing.matches));
                found.push(ids);
            }
            let _ = writeln!(
                report.ranked,
                "| {label} | {level} | `{query}` | {} | {} |",
                cells.join(" | "),
                if fast { "ok" } else { "**no**" }
            );
        }
        if last {
            for (at, top) in [8, 64].into_iter().enumerate() {
                let over = if misses[at].is_empty() {
                    "none".to_string()
                } else {
                    misses[at].join(", ")
                };
                let _ = writeln!(
                    report.acceptance,
                    "| {level} | {top} | `{}` | {:.2} | {over} |",
                    worst[at].1, worst[at].0
                );
            }
        }
        answers.push(found);

        for query in FRESH_QUERIES {
            progress(started, &format!("{label} {level}: fresh `{query}`"));
            let cells = ["8", "64"]
                .map(|top| {
                    spawn(&["store", &path.to_string_lossy(), query, top])
                        .map(|line| {
                            let parts = line.split_whitespace().collect::<Vec<_>>();
                            format!("{} / {} / {}", parts[0], parts[1], parts[2])
                        })
                        .unwrap_or_else(|| "failed".into())
                })
                .join(" | ");
            let _ = writeln!(report.fresh, "| {label} | {level} | `{query}` | {cells} |");
        }

        for query in SUBSTRING_QUERIES {
            progress(started, &format!("{label} {level}: substring `{query}`"));
            let mut cells = Vec::new();
            for limit in [usize::MAX, 200] {
                let timing = time(|| {
                    store
                        .substring(query, Kinds::FoldersAndFiles, limit, &|| false)
                        .expect("a substring query")
                        .len()
                });
                cells.push(format!("{} | {}", cell(Some(&timing)), timing.matches));
            }
            let _ = writeln!(
                report.substring,
                "| {label} | {level} | `{query}` | {} |",
                cells.join(" | ")
            );
        }
        drop(store);

        if detail == IndexDetail::Balanced {
            progress(started, &format!("{label}: scan of `folded`"));
            measure_scan(label, &path, report);
            measure_switches(label, &path, started, report);
        }
        remove(&path);
    }
    for (level, other) in answers.iter().enumerate().skip(1) {
        if *other != answers[0] {
            let _ = writeln!(
                report.notes,
                "- {label}: `{}` ranked differently from `{}`.",
                levels[level].as_str(),
                levels[0].as_str()
            );
        }
    }
}

/// The scan of every folded name, against the scan of every name folded as
/// it is read.
fn measure_scan(label: &str, path: &Path, report: &mut StoreReport) {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open the store");
    connection
        .execute_batch("PRAGMA cache_size = -1024;")
        .expect("bound the cache");
    let folded_bytes: i64 = connection
        .query_row(
            "SELECT SUM(length(CAST(folded AS BLOB))) FROM files",
            [],
            |row| row.get(0),
        )
        .expect("the folded bytes");
    let finder = Finder::new(b"report");
    let stored = time(|| {
        let mut statement = connection
            .prepare_cached("SELECT id, folded FROM files")
            .expect("prepare the scan");
        let mut rows = statement.query([]).expect("scan");
        let mut found = 0;
        while let Some(row) = rows.next().expect("a row") {
            let folded = row.get_ref(1).expect("folded").as_bytes().expect("text");
            if finder.find(folded).is_some() {
                found += 1;
            }
        }
        found
    });
    let folding = time(|| {
        let mut statement = connection
            .prepare_cached("SELECT id, name FROM files")
            .expect("prepare the scan");
        let mut rows = statement.query([]).expect("scan");
        let mut found = 0;
        let mut folded = String::new();
        while let Some(row) = rows.next().expect("a row") {
            folded.clear();
            fold(
                row.get_ref(1).expect("name").as_str().expect("text"),
                &mut folded,
            );
            if finder.find(folded.as_bytes()).is_some() {
                found += 1;
            }
        }
        found
    });
    let _ = writeln!(
        report.scan,
        "| {label} | {:.1} | {} | {} |",
        mb(folded_bytes as u64),
        cell(Some(&stored)),
        cell(Some(&folding))
    );
    if stored.matches != folding.matches {
        let _ = writeln!(
            report.notes,
            "- {label}: the folded scan found {} `report`, the folding one {}.",
            stored.matches, folding.matches
        );
    }
}

/// The level changed on the `balanced` store and back, timed.
fn measure_switches(label: &str, path: &Path, started: Instant, report: &mut StoreReport) {
    let mut store = Store::open(path).expect("open the store");
    for (change, detail) in [
        ("balanced to complete", IndexDetail::Complete),
        ("complete to balanced", IndexDetail::Balanced),
        ("balanced to compact", IndexDetail::Compact),
        ("compact to balanced", IndexDetail::Balanced),
    ] {
        progress(started, &format!("{label}: set_detail {change}"));
        let start = Instant::now();
        store.set_detail(detail).expect("change the level");
        let took = start.elapsed();
        let _ = writeln!(
            report.switches,
            "| {label} | {change} | {:.1} | {:.1} |",
            took.as_secs_f64(),
            mb(size(path))
        );
    }
}
