//! Clipboard history, persisted in SQLite (F3).
//!
//! The plan is explicit about the weight of this one:
//!
//! > The clipboard is the first component with genuine persistent-state
//! > complexity. It requires a system event listener, schema migrations and a
//! > retention policy. Treat it as a subproject, not as one more feature.
//!
//! SQLite rather than a JSON file, and the reason is the write pattern: history
//! is appended on every copy, forever. Rewriting a whole file per copy is both
//! wasteful and a corruption risk on a crash, and a capped list still needs
//! deletion, deduplication and ordering -- which is a database whether or not
//! it is spelled like one.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension, params};

/// The schema this build expects.
///
/// Stored in SQLite's own `user_version`, so migrating is a comparison rather
/// than a table of its own.
const SCHEMA_VERSION: i32 = 2;

/// What an entry holds, as it was put on the clipboard (F12).
///
/// Only what the system distinguishes is stored. A link or a colour is text
/// that reads as one, which is a question [`super::kind`] answers each time the
/// entry is read: a rule improved later then applies to the whole history, and
/// nothing has to be migrated to benefit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Payload {
    #[default]
    Text,
    /// A list of files, as Explorer copies them. `text` is the paths, one per
    /// line: a Windows path cannot contain a line break.
    Files,
}

impl Payload {
    fn column(self) -> &'static str {
        match self {
            Payload::Text => "text",
            Payload::Files => "files",
        }
    }

    /// Unknown values read as text rather than failing the whole history: a
    /// row written by a later version is still something that was copied.
    fn from_column(value: &str) -> Self {
        match value {
            "files" => Payload::Files,
            _ => Payload::Text,
        }
    }
}

/// One recorded clipboard entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub payload: Payload,
    /// The text, or for [`Payload::Files`] the paths, one per line.
    pub text: String,
    /// Seconds since the Unix epoch.
    pub copied_at: i64,
    /// The executable that was in the foreground, if it could be identified.
    pub source: Option<String>,
}

impl Entry {
    /// The files, for [`Payload::Files`]; empty for text.
    pub fn files(&self) -> Vec<std::path::PathBuf> {
        match self.payload {
            Payload::Files => self.text.lines().map(Into::into).collect(),
            Payload::Text => Vec::new(),
        }
    }

    /// A single-line preview for the result list.
    ///
    /// Clipboard contents are often long and often multi-line; a row is one
    /// line tall and fixed (I-10), so the preview is built here rather than
    /// left to the renderer to truncate.
    pub fn preview(&self, limit: usize) -> String {
        let mut lines = self.text.lines();
        let first = lines.next().unwrap_or("").trim();
        let has_more_lines = lines.next().is_some();

        let mut preview: String = first.chars().take(limit).collect();
        if first.chars().count() > limit || has_more_lines {
            preview.push('…');
        }
        preview
    }
}

/// How much history is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Retention {
    /// Most recent entries to keep.
    pub max_entries: usize,
    /// Entries older than this are dropped.
    pub max_age_days: u32,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            // Enough to cover "what did I copy this morning" without turning
            // into an archive nobody asked to keep.
            max_entries: 500,
            max_age_days: 30,
        }
    }
}

/// The history database.
pub struct History {
    connection: Connection,
    retention: Retention,
}

impl History {
    /// Open, creating and migrating as needed.
    pub fn open(path: &Path, retention: Retention) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }

        let connection =
            Connection::open(path).with_context(|| format!("opening {}", path.display()))?;

        // WAL so a write never blocks the read the launcher is doing, and a
        // crash mid-write cannot corrupt what is already there.
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .context("enabling write-ahead logging")?;

        let mut history = Self {
            connection,
            retention,
        };
        history.migrate()?;
        Ok(history)
    }

    /// Open in memory, for tests.
    pub fn in_memory(retention: Retention) -> Result<Self> {
        let mut history = Self {
            connection: Connection::open_in_memory()?,
            retention,
        };
        history.migrate()?;
        Ok(history)
    }

    /// Bring the schema up to [`SCHEMA_VERSION`].
    ///
    /// Written as a sequence of steps from whatever version is on disk, rather
    /// than a create-if-missing: the second migration is the one that is hard,
    /// and a shape that only supports the first is a shape that gets rewritten
    /// under pressure later.
    fn migrate(&mut self) -> Result<()> {
        let version: i32 = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .context("reading the schema version")?;

        if version > SCHEMA_VERSION {
            anyhow::bail!(
                "the clipboard history was written by a newer version of Lumen \
                 (schema {version}, this build understands {SCHEMA_VERSION})"
            );
        }

        if version < 1 {
            self.connection
                .execute_batch(
                    "CREATE TABLE entries (
                         id        INTEGER PRIMARY KEY,
                         text      TEXT    NOT NULL,
                         copied_at INTEGER NOT NULL,
                         source    TEXT,
                         -- Recency is ordered on this, not on the timestamp.
                         -- `copied_at` has second granularity, so two copies
                         -- in the same second tie and the order becomes
                         -- whatever SQLite feels like -- which is exactly the
                         -- case a clipboard history hits constantly.
                         sequence  INTEGER NOT NULL
                     );
                     -- Recording the same text twice replaces the older row
                     -- rather than adding a duplicate, so copying something
                     -- repeatedly does not fill the history with itself.
                     CREATE UNIQUE INDEX entries_text ON entries (text);
                     CREATE INDEX entries_sequence ON entries (sequence DESC);",
                )
                .context("creating the clipboard schema")?;
        }

        if version < 2 {
            // Files join text (F12). The same path copied as text and as a
            // file are two different things to paste, so uniqueness is on
            // the pair.
            self.connection
                .execute_batch(
                    "ALTER TABLE entries ADD COLUMN kind TEXT NOT NULL DEFAULT 'text';
                     DROP INDEX entries_text;
                     CREATE UNIQUE INDEX entries_content ON entries (kind, text);",
                )
                .context("adding files to the clipboard schema")?;
        }

        self.connection
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .context("recording the schema version")?;

        Ok(())
    }

    /// Record an entry, then apply the retention policy.
    ///
    /// The caller is responsible for the privacy checks: this stores what it is
    /// given. Keeping the decision and the storage apart is what makes the
    /// decision testable without a database.
    pub fn record(&self, payload: Payload, text: &str, source: Option<&str>) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO entries (kind, text, copied_at, source, sequence)
                 VALUES (
                     ?4, ?1, ?2, ?3,
                     COALESCE((SELECT MAX(sequence) FROM entries), 0) + 1
                 )
                 ON CONFLICT (kind, text) DO UPDATE SET
                     copied_at = excluded.copied_at,
                     source    = excluded.source,
                     -- Re-copying moves an entry back to the top rather than
                     -- leaving it where it was.
                     sequence  = excluded.sequence",
                params![text, now(), source, payload.column()],
            )
            .context("recording a clipboard entry")?;

        self.prune()
    }

    /// The most recent entries, newest first.
    pub fn recent(&self, limit: usize) -> Result<Vec<Entry>> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, text, copied_at, source
             FROM entries
             ORDER BY sequence DESC
             LIMIT ?1",
        )?;

        let entries = statement
            .query_map(params![limit as i64], read_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(entries)
    }

    pub fn len(&self) -> Result<usize> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Forget everything. Offered as a command, because a history the user
    /// cannot clear is a history they will not want.
    pub fn clear(&self) -> Result<()> {
        self.connection
            .execute("DELETE FROM entries", [])
            .context("clearing the clipboard history")?;
        Ok(())
    }

    /// Forget one entry. Not an error when it is already gone.
    pub fn delete(&self, id: i64) -> Result<()> {
        self.connection
            .execute("DELETE FROM entries WHERE id = ?1", params![id])
            .context("deleting a clipboard entry")?;
        Ok(())
    }

    /// Put an entry back at the top, as if it had just been copied: pasting
    /// something from the history is using it again (F12).
    pub fn touch(&self, id: i64) -> Result<()> {
        self.connection
            .execute(
                "UPDATE entries SET
                     copied_at = ?2,
                     sequence  = COALESCE((SELECT MAX(sequence) FROM entries), 0) + 1
                 WHERE id = ?1",
                params![id, now()],
            )
            .context("moving a clipboard entry to the top")?;
        Ok(())
    }

    /// Drop what the retention policy no longer covers.
    fn prune(&self) -> Result<()> {
        let cutoff = now() - i64::from(self.retention.max_age_days) * 24 * 60 * 60;

        self.connection
            .execute("DELETE FROM entries WHERE copied_at < ?1", params![cutoff])
            .context("pruning old clipboard entries")?;

        // Keeping the newest N: everything outside the most recent window goes,
        // expressed as a subquery so it is one statement and one transaction.
        self.connection
            .execute(
                "DELETE FROM entries
                 WHERE id NOT IN (
                     SELECT id FROM entries ORDER BY sequence DESC LIMIT ?1
                 )",
                params![self.retention.max_entries as i64],
            )
            .context("trimming the clipboard history")?;

        Ok(())
    }

    /// Look one entry up by id, for the copy-back action.
    pub fn get(&self, id: i64) -> Result<Option<Entry>> {
        let entry = self
            .connection
            .query_row(
                "SELECT id, kind, text, copied_at, source FROM entries WHERE id = ?1",
                params![id],
                read_entry,
            )
            .optional()?;

        Ok(entry)
    }
}

fn read_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let kind: String = row.get(1)?;
    Ok(Entry {
        id: row.get(0)?,
        payload: Payload::from_column(&kind),
        text: row.get(2)?,
        copied_at: row.get(3)?,
        source: row.get(4)?,
    })
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> History {
        History::in_memory(Retention::default()).unwrap()
    }

    #[test]
    fn records_and_returns_entries_newest_first() {
        let history = history();
        history
            .record(Payload::Text, "first", Some("code.exe"))
            .unwrap();
        history.record(Payload::Text, "second", None).unwrap();

        let recent = history.recent(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].text, "second");
        assert_eq!(recent[1].text, "first");
        assert_eq!(recent[1].source.as_deref(), Some("code.exe"));
    }

    /// Copying the same thing repeatedly must not fill the history with it.
    #[test]
    fn recording_the_same_text_twice_moves_it_rather_than_duplicating_it() {
        let history = history();
        history.record(Payload::Text, "repeated", None).unwrap();
        history.record(Payload::Text, "other", None).unwrap();
        history.record(Payload::Text, "repeated", None).unwrap();

        assert_eq!(history.len().unwrap(), 2);
        // ...and it comes back to the top.
        assert_eq!(history.recent(10).unwrap()[0].text, "repeated");
    }

    #[test]
    fn the_entry_cap_is_enforced() {
        let history = History::in_memory(Retention {
            max_entries: 3,
            max_age_days: 30,
        })
        .unwrap();

        for index in 0..10 {
            history
                .record(Payload::Text, &format!("entry {index}"), None)
                .unwrap();
        }

        assert_eq!(history.len().unwrap(), 3);
        assert_eq!(history.recent(10).unwrap()[0].text, "entry 9");
    }

    #[test]
    fn entries_can_be_looked_up_and_cleared() {
        let history = history();
        history.record(Payload::Text, "findable", None).unwrap();

        let id = history.recent(1).unwrap()[0].id;
        assert_eq!(history.get(id).unwrap().unwrap().text, "findable");

        history.clear().unwrap();
        assert!(history.is_empty().unwrap());
        assert_eq!(history.get(id).unwrap(), None);
    }

    #[test]
    fn a_missing_entry_is_not_an_error() {
        assert_eq!(history().get(9999).unwrap(), None);
    }

    #[test]
    fn a_preview_is_one_line_and_bounded() {
        let entry = Entry {
            id: 1,
            payload: Payload::Text,
            text: "first line\nsecond line".into(),
            copied_at: 0,
            source: None,
        };
        assert_eq!(entry.preview(40), "first line…");

        let long = Entry {
            id: 2,
            payload: Payload::Text,
            text: "x".repeat(100),
            copied_at: 0,
            source: None,
        };
        let preview = long.preview(10);
        assert_eq!(preview.chars().count(), 11, "ten characters plus the mark");
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn a_short_single_line_preview_is_not_marked() {
        let entry = Entry {
            id: 1,
            payload: Payload::Text,
            text: "short".into(),
            copied_at: 0,
            source: None,
        };
        assert_eq!(entry.preview(40), "short");
    }

    /// The database survives being closed and reopened, which is the entire
    /// reason it is a database.
    #[test]
    fn history_persists_across_reopening() {
        let dir = std::env::temp_dir().join(format!("lumen-clip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("clipboard.sqlite3");

        {
            let history = History::open(&path, Retention::default()).unwrap();
            history.record(Payload::Text, "persisted", None).unwrap();
        }

        let reopened = History::open(&path, Retention::default()).unwrap();
        assert_eq!(reopened.recent(10).unwrap()[0].text, "persisted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A database from a future version must be refused rather than silently
    /// misread -- that is what the version is for.
    #[test]
    fn a_newer_schema_is_refused() {
        let dir = std::env::temp_dir().join(format!("lumen-clip-future-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clipboard.sqlite3");

        {
            let connection = Connection::open(&path).unwrap();
            connection
                .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
                .unwrap();
        }

        let error = match History::open(&path, Retention::default()) {
            Err(error) => error,
            Ok(_) => panic!("a newer schema must be refused"),
        };
        assert!(error.to_string().contains("newer version"), "{error}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same path copied as text and as a file are two entries: one pastes
    /// the words, the other the file.
    #[test]
    fn files_and_text_with_the_same_content_are_kept_apart() {
        let history = history();
        history.record(Payload::Text, r"C:\a.txt", None).unwrap();
        history.record(Payload::Files, r"C:\a.txt", None).unwrap();

        let recent = history.recent(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].payload, Payload::Files);
        assert_eq!(
            recent[0].files(),
            vec![std::path::PathBuf::from(r"C:\a.txt")]
        );
        assert!(recent[1].files().is_empty());
    }

    #[test]
    fn an_entry_can_be_deleted_and_moved_back_to_the_top() {
        let history = history();
        history.record(Payload::Text, "old", None).unwrap();
        history.record(Payload::Text, "gone", None).unwrap();
        history.record(Payload::Text, "new", None).unwrap();

        let recent = history.recent(10).unwrap();
        history.delete(recent[1].id).unwrap();
        history.touch(recent[2].id).unwrap();

        let texts: Vec<String> = history
            .recent(10)
            .unwrap()
            .into_iter()
            .map(|e| e.text)
            .collect();
        assert_eq!(texts, ["old", "new"]);
        // Already gone is not an error.
        history.delete(recent[1].id).unwrap();
    }

    /// A history written by F3 opens, keeps its rows as text, and dedupes
    /// under the new index.
    #[test]
    fn a_version_one_history_is_migrated() {
        let dir = std::env::temp_dir().join(format!("lumen-clip-v1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clipboard.sqlite3");

        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE entries (
                         id INTEGER PRIMARY KEY, text TEXT NOT NULL,
                         copied_at INTEGER NOT NULL, source TEXT,
                         sequence INTEGER NOT NULL
                     );
                     CREATE UNIQUE INDEX entries_text ON entries (text);
                     CREATE INDEX entries_sequence ON entries (sequence DESC);
                     INSERT INTO entries VALUES (1, 'kept', 0, 'code.exe', 1);
                     PRAGMA user_version = 1;",
                )
                .unwrap();
        }

        let history = History::open(&path, Retention::default()).unwrap();
        let recent = history.recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].payload, Payload::Text);
        assert_eq!(recent[0].source.as_deref(), Some("code.exe"));

        history.record(Payload::Text, "kept", None).unwrap();
        assert_eq!(history.len().unwrap(), 1);

        drop(history);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
