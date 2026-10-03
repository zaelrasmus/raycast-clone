//! Slice 0 of the file index (F16, `docs/FILE-SEARCH.md`): which storage answers
//! a file query under 4 ms at p95 with a million names, and what it costs.
//!
//! Two candidates are measured on the same synthetic corpus. SQLite with an FTS5
//! trigram index keeps the names on disk behind a bounded page cache; the other
//! keeps nothing but the names in memory, as one blob scanned with `memmem`. The
//! design depends only on the query budget and the idle memory budget, so the
//! storage that meets both wins -- and that is decided by these numbers, written
//! before either is built.
//!
//! A plain `main` rather than criterion: the budget is stated as a percentile,
//! and criterion reports means and confidence intervals, not p95.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use memchr::memmem::Finder;
use rusqlite::{Connection, params};

/// Each query from a single character to a long word, plus one that matches
/// nothing -- the case where a scan cannot stop early.
const QUERIES: [&str; 6] = ["a", "in", "inv", "invoice", "2024", "zqxj"];

/// A home folder after the skip rules is estimated at 100 000 to 1 000 000
/// entries; these are the two ends of that estimate.
const SIZES: [usize; 2] = [100_000, 1_000_000];

const WARMUP: usize = 20;
const RUNS: usize = 200;

/// What a capped query returns: a few hundred candidates are all the ranking
/// step should ever see.
const LIMIT: usize = 500;

/// One file or folder of the synthetic home.
struct Entry {
    parent: String,
    name: String,
    /// The name as it is matched: lower-cased, as the real index would store it.
    key: String,
    is_dir: bool,
}

/// A fixed-seed xorshift64, so every run measures exactly the same corpus
/// without pulling a random-number crate into the build.
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

/// A synthetic home folder of `count` entries.
///
/// Names are varied on purpose, as in the search bench: a corpus of repeated
/// strings lets a substring search hit the same few pages and branches every
/// time, and would measure something kinder than a real disk.
fn corpus(count: usize) -> Vec<Entry> {
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
        "pdf", "docx", "png", "jpg", "rs", "md", "txt", "xlsx", "mp4", "zip", "pptx", "csv",
        "json", "heic", "mov", "html",
    ];
    const SEPARATORS: [&str; 3] = ["-", "_", " "];

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    (0..count)
        .map(|_| {
            let depth = 1 + rng.below(6);
            let parent = (0..depth)
                .map(|_| rng.pick(&FOLDERS))
                .collect::<Vec<_>>()
                .join("/");

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

            let is_dir = rng.below(10) == 0;
            if !is_dir {
                name = format!("{name}.{}", rng.pick(&EXTENSIONS));
            }

            Entry {
                parent,
                key: name.to_lowercase(),
                name,
                is_dir,
            }
        })
        .collect()
}

/// Times `run` individually after a warm-up, and returns the sorted samples and
/// the number of matches of the last run.
fn time(mut run: impl FnMut() -> usize) -> (Vec<Duration>, usize) {
    for _ in 0..WARMUP {
        run();
    }
    let mut matches = 0;
    let mut samples = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            matches = run();
            start.elapsed()
        })
        .collect::<Vec<_>>();
    samples.sort();
    (samples, matches)
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn row(candidate: &str, query: &str, samples: &[Duration], matches: usize) {
    let p50 = samples[samples.len() / 2];
    let p95 = samples[samples.len() * 95 / 100];
    let max = samples[samples.len() - 1];
    println!(
        "| {candidate} | `{query}` | {:.3} | {:.3} | {:.3} | {matches} |",
        ms(p50),
        ms(p95),
        ms(max)
    );
}

/// The exclusive upper bound of a prefix range: the prefix with its last
/// character incremented, so `key >= "in" AND key < "io"` is every key that
/// starts with `in` and the index answers it as one range.
fn prefix_end(prefix: &str) -> String {
    let mut chars = prefix.chars().collect::<Vec<_>>();
    let last = chars.pop().expect("a non-empty prefix");
    let next = char::from_u32(last as u32 + 1).expect("a prefix that is not the last char");
    chars.push(next);
    chars.into_iter().collect()
}

/// A query as an FTS5 string literal, so its characters are matched rather
/// than read as query syntax.
fn fts_literal(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

/// Candidate A: the names on disk, in SQLite with an FTS5 trigram index.
fn build_sqlite(path: &Path, entries: &[Entry]) -> Connection {
    let connection = Connection::open(path).expect("open the bench database");
    // The bound the real index would run with: resident memory must not grow
    // with the number of files.
    connection
        .execute_batch("PRAGMA cache_size = -8192;")
        .expect("set the cache size");
    connection
        .execute_batch(
            "CREATE TABLE entries(
                 id INTEGER PRIMARY KEY,
                 parent TEXT,
                 name TEXT,
                 key TEXT,
                 is_dir INTEGER
             );
             CREATE INDEX entries_key ON entries(key);",
        )
        .expect("create the entries table");

    // Its absence is a finding, not something to work around: the design
    // depends on the trigram tokenizer being in the bundled build.
    if let Err(error) = connection.execute_batch(
        "CREATE VIRTUAL TABLE names USING fts5(
             key, content='entries', content_rowid='id', tokenize='trigram'
         );",
    ) {
        println!("FTS5 with the trigram tokenizer is unavailable: {error}");
        std::process::exit(1);
    }

    let transaction = connection
        .unchecked_transaction()
        .expect("begin the bulk insert");
    {
        let mut insert = transaction
            .prepare(
                "INSERT INTO entries(id, parent, name, key, is_dir) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .expect("prepare the insert");
        for (id, entry) in entries.iter().enumerate() {
            insert
                .execute(params![
                    id as i64,
                    entry.parent,
                    entry.name,
                    entry.key,
                    entry.is_dir
                ])
                .expect("insert an entry");
        }
    }
    // An external-content table is filled from its content table in one pass.
    transaction
        .execute_batch("INSERT INTO names(names) VALUES ('rebuild');")
        .expect("populate the trigram index");
    transaction.commit().expect("commit the bulk insert");
    connection
}

/// Candidate B: the names alone in memory -- every key in one blob, separated
/// by newlines, and where each one starts.
///
/// One allocation instead of a million strings, so its size is the bytes of
/// the names and four bytes an entry, with no per-string header.
struct Blob {
    text: String,
    starts: Vec<u32>,
}

impl Blob {
    fn build(entries: &[Entry]) -> Self {
        let mut text = String::new();
        let mut starts = Vec::with_capacity(entries.len());
        for entry in entries {
            starts.push(text.len() as u32);
            text.push_str(&entry.key);
            text.push('\n');
        }
        Self { text, starts }
    }

    fn bytes(&self) -> usize {
        self.text.capacity() + self.starts.capacity() * 4
    }

    /// Entries whose key starts with `prefix`. A key shorter than the prefix
    /// fails on its newline, so no end offsets are needed.
    fn prefix(&self, prefix: &str, limit: usize, out: &mut Vec<u32>) {
        out.clear();
        let text = self.text.as_bytes();
        let prefix = prefix.as_bytes();
        for (id, &start) in self.starts.iter().enumerate() {
            if text[start as usize..].starts_with(prefix) {
                out.push(id as u32);
                if out.len() == limit {
                    return;
                }
            }
        }
    }

    /// Entries whose key contains the needle. Hits arrive in order and a key
    /// has no newline to straddle, so consecutive hits in the same entry are
    /// the only duplicates.
    fn substring(&self, finder: &Finder, limit: usize, out: &mut Vec<u32>) {
        out.clear();
        for hit in finder.find_iter(self.text.as_bytes()) {
            let id = (self.starts.partition_point(|&start| start as usize <= hit) - 1) as u32;
            if out.last() != Some(&id) {
                out.push(id);
                if out.len() == limit {
                    return;
                }
            }
        }
    }
}

fn database_path(count: usize) -> PathBuf {
    // The target directory rather than the system temp folder: a million-entry
    // database is large, and the system drive is not where it should land.
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("file_index_bench_{count}.db"))
}

fn remove_database(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-journal"));
}

fn measure(count: usize) {
    println!("\n## {count} entries\n");
    let entries = corpus(count);

    let path = database_path(count);
    remove_database(&path);
    let start = Instant::now();
    let connection = build_sqlite(&path, &entries);
    let sqlite_build = start.elapsed();
    let sqlite_size = std::fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let version: String = connection
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .expect("read the SQLite version");

    let start = Instant::now();
    let blob = Blob::build(&entries);
    let blob_build = start.elapsed();

    println!("| | value |");
    println!("| - | - |");
    println!("| SQLite version | {version} |");
    println!("| SQLite build | {:.0} ms |", ms(sqlite_build));
    println!(
        "| SQLite file size | {:.1} MB |",
        sqlite_size as f64 / 1_048_576.0
    );
    println!("| Blob build | {:.0} ms |", ms(blob_build));
    println!(
        "| Blob memory | {:.1} MB |",
        blob.bytes() as f64 / 1_048_576.0
    );

    println!("\n| candidate | query | p50 ms | p95 ms | max ms | matches |");
    println!("| - | - | - | - | - | - |");

    let mut prefix = connection
        .prepare("SELECT id FROM entries WHERE key >= ?1 AND key < ?2 LIMIT 500")
        .expect("prepare the prefix query");
    let mut capped = connection
        .prepare("SELECT rowid FROM names WHERE names MATCH ?1 LIMIT 500")
        .expect("prepare the trigram query");
    let mut uncapped = connection
        .prepare("SELECT rowid FROM names WHERE names MATCH ?1")
        .expect("prepare the uncapped trigram query");

    for query in QUERIES {
        if query.chars().count() < 3 {
            let end = prefix_end(query);
            let (samples, matches) = time(|| {
                prefix
                    .query_map(params![query, end], |row| row.get::<_, i64>(0))
                    .expect("run the prefix query")
                    .collect::<Result<Vec<_>, _>>()
                    .expect("read the prefix rows")
                    .len()
            });
            row("SQLite prefix, 500", query, &samples, matches);
        } else {
            let literal = fts_literal(query);
            for (candidate, statement) in [
                ("SQLite trigram, 500", &mut capped),
                ("SQLite trigram, all", &mut uncapped),
            ] {
                let (samples, matches) = time(|| {
                    statement
                        .query_map(params![literal], |row| row.get::<_, i64>(0))
                        .expect("run the trigram query")
                        .collect::<Result<Vec<_>, _>>()
                        .expect("read the trigram rows")
                        .len()
                });
                row(candidate, query, &samples, matches);
            }
        }
    }

    let mut out = Vec::new();
    for query in QUERIES {
        let finder = Finder::new(query.as_bytes());
        for (candidate, limit) in [("Blob, all", usize::MAX), ("Blob, 500", LIMIT)] {
            let (samples, matches) = time(|| {
                if query.chars().count() < 3 {
                    blob.prefix(query, limit, &mut out);
                } else {
                    blob.substring(&finder, limit, &mut out);
                }
                out.len()
            });
            let kind = if query.chars().count() < 3 {
                "prefix"
            } else {
                "memmem"
            };
            row(&format!("{candidate} ({kind})"), query, &samples, matches);
        }
    }

    drop(prefix);
    drop(capped);
    drop(uncapped);
    drop(connection);
    remove_database(&path);
}

fn main() {
    // `cargo bench` passes `--bench`; nothing here takes arguments.
    println!("# File index, slice 0");
    println!("\n{WARMUP} warm-up runs, then {RUNS} timed runs per row.");
    for count in SIZES {
        measure(count);
    }
}
