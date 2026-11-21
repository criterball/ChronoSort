//! `cs-db`: the SQLite catalog and the action journal.
//!
//! The journal is the safety net: every file operation is written as
//! `pending` *before* it happens and marked `done` only *after* the copy was
//! verified. After a crash, anything still `pending` is re-checked, never assumed.

use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub use rusqlite;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("database schema v{found} is newer than this ChronoSort (max v{max}); refusing to touch it")]
    TooNew { found: i64, max: i64 },
    #[error("illegal journal transition {from:?} -> {to:?}")]
    BadTransition { from: JournalState, to: JournalState },
}

pub type Result<T> = std::result::Result<T, DbError>;

/// Ordered list of migrations; index + 1 = schema version.
const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE runs (
        id            INTEGER PRIMARY KEY,
        started_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
        source_root   TEXT NOT NULL,
        dest_root     TEXT NOT NULL,
        mode          TEXT NOT NULL CHECK (mode IN ('copy','move')),
        settings_json TEXT NOT NULL DEFAULT '{}',
        status        TEXT NOT NULL DEFAULT 'planning'
    );
    CREATE TABLE items (
        id            INTEGER PRIMARY KEY,
        run_id        INTEGER NOT NULL REFERENCES runs(id),
        src_path      TEXT NOT NULL,
        size          INTEGER NOT NULL,
        mtime_ns      INTEGER,
        ctime_ns      INTEGER,
        hash          TEXT,
        kind          TEXT NOT NULL,
        decision_json TEXT,
        evidence_json TEXT,
        UNIQUE (run_id, src_path)
    );
    CREATE TABLE journal (
        id          INTEGER PRIMARY KEY,
        run_id      INTEGER NOT NULL REFERENCES runs(id),
        item_id     INTEGER REFERENCES items(id),
        op          TEXT NOT NULL CHECK (op IN ('copy','move')),
        src         TEXT NOT NULL,
        dst         TEXT NOT NULL,
        state       TEXT NOT NULL,
        hash        TEXT,
        error       TEXT,
        updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
    );
    CREATE INDEX journal_run_state ON journal(run_id, state);
    "#,
    // v2: folders created by a run (removed again on undo, if empty)
    r#"
    CREATE TABLE created_dirs (
        id      INTEGER PRIMARY KEY,
        run_id  INTEGER NOT NULL REFERENCES runs(id),
        path    TEXT NOT NULL
    );
    "#,
    // v3: duplicate handling adds 'relocate' (moved to a Duplicates folder) and
    // 'trash' (sent to the Recycle Bin). SQLite cannot alter a CHECK, so the
    // table is rebuilt with every existing row kept.
    r#"
    CREATE TABLE journal_v3 (
        id          INTEGER PRIMARY KEY,
        run_id      INTEGER NOT NULL REFERENCES runs(id),
        item_id     INTEGER REFERENCES items(id),
        op          TEXT NOT NULL CHECK (op IN ('copy','move','relocate','trash')),
        src         TEXT NOT NULL,
        dst         TEXT NOT NULL,
        state       TEXT NOT NULL,
        hash        TEXT,
        error       TEXT,
        updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
    );
    INSERT INTO journal_v3 SELECT * FROM journal;
    DROP TABLE journal;
    ALTER TABLE journal_v3 RENAME TO journal;
    CREATE INDEX journal_run_state ON journal(run_id, state);
    "#,
    // v4: people the user has named. Only face fingerprints (128 numbers), a
    // year and a small face crop are kept: no photo paths. Local only.
    r#"
    CREATE TABLE people (
        id          INTEGER PRIMARY KEY,
        name        TEXT NOT NULL,
        created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
    );
    CREATE TABLE face_samples (
        id          INTEGER PRIMARY KEY,
        person_id   INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE,
        embedding   BLOB NOT NULL,
        year        INTEGER,
        crop        BLOB,
        added_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
    );
    CREATE INDEX face_samples_person ON face_samples(person_id);
    "#,
    // v5: optional birth year (a photo cannot be older than the people in it).
    r#"
    ALTER TABLE people ADD COLUMN birth_year INTEGER;
    "#,
];

mod people;
pub use people::{KnownPerson, MAX_SAMPLES, PersonRow, Sample, clean_name};

/// One journal row.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalRow {
    pub id: i64,
    pub run_id: i64,
    pub op: String,
    pub src: String,
    pub dst: String,
    pub state: JournalState,
    pub hash: Option<String>,
}

/// Life cycle of one journal entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalState {
    /// Written before any byte is copied.
    Pending,
    /// Copy verified (hash + times) and renamed into place.
    Done,
    /// Move mode: source was sent to the OS trash after `Done`.
    SourceTrashed,
    Failed,
    /// Reversed by the user (copy removed / source restored).
    Undone,
}

impl JournalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::SourceTrashed => "source_trashed",
            Self::Failed => "failed",
            Self::Undone => "undone",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => Self::Pending,
            "done" => Self::Done,
            "source_trashed" => Self::SourceTrashed,
            "failed" => Self::Failed,
            "undone" => Self::Undone,
            _ => return None,
        })
    }

    /// Allowed state machine. Anything else is a bug and is refused.
    pub fn can_go_to(self, to: Self) -> bool {
        use JournalState::*;
        matches!(
            (self, to),
            (Pending, Done)
                | (Pending, Failed)
                | (Done, SourceTrashed)
                | (Done, Undone)
                | (SourceTrashed, Undone)
                | (Failed, Pending) // retry
        )
    }
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // WAL + NORMAL: the database can never be corrupted by a crash or power
        // cut; at worst the last few commits roll back after a power cut, which
        // `recover` handles by re-checking files on disk. Where it matters
        // (before a source goes to the trash) call `durable_barrier`.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    fn migrate(&mut self) -> Result<()> {
        let max = MIGRATIONS.len() as i64;
        let found = self.schema_version()?;
        if found > max {
            return Err(DbError::TooNew { found, max });
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    /// Run `f` in one transaction (one disk sync instead of one per statement).
    pub fn batch<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let tx = self.conn.unchecked_transaction()?;
        let v = f(self)?;
        tx.commit()?;
        Ok(v)
    }

    /// Force everything committed so far onto the disk (WAL checkpoint + fsync).
    pub fn durable_barrier(&self) -> Result<()> {
        self.conn.query_row("PRAGMA wal_checkpoint(FULL)", [], |_| Ok(()))?;
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn create_run(&self, source_root: &str, dest_root: &str, mode: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO runs (source_root, dest_root, mode) VALUES (?1, ?2, ?3)",
            params![source_root, dest_root, mode],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Record intent *before* touching any file.
    pub fn journal_begin(&self, run_id: i64, op: &str, src: &str, dst: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO journal (run_id, op, src, dst, state) VALUES (?1, ?2, ?3, ?4, 'pending')",
            params![run_id, op, src, dst],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn journal_state(&self, id: i64) -> Result<Option<JournalState>> {
        let s: Option<String> =
            self.conn.query_row("SELECT state FROM journal WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        Ok(s.and_then(|s| JournalState::parse(&s)))
    }

    /// Move an entry to a new state, enforcing the state machine.
    pub fn journal_set(&self, id: i64, to: JournalState, hash: Option<&str>, error: Option<&str>) -> Result<()> {
        let from = self.journal_state(id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        if !from.can_go_to(to) {
            return Err(DbError::BadTransition { from, to });
        }
        self.conn.execute(
            "UPDATE journal SET state = ?2, hash = COALESCE(?3, hash), error = ?4,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE id = ?1",
            params![id, to.as_str(), hash, error],
        )?;
        Ok(())
    }

    pub fn set_run_status(&self, run_id: i64, status: &str) -> Result<()> {
        self.conn.execute("UPDATE runs SET status = ?2 WHERE id = ?1", params![run_id, status])?;
        Ok(())
    }

    /// Most recent run id, if any.
    pub fn last_run(&self) -> Result<Option<i64>> {
        Ok(self.conn.query_row("SELECT MAX(id) FROM runs", [], |r| r.get(0))?)
    }

    pub fn run_mode(&self, run_id: i64) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT mode FROM runs WHERE id = ?1", [run_id], |r| r.get(0)).optional()?)
    }

    pub fn add_created_dir(&self, run_id: i64, path: &str) -> Result<()> {
        self.conn.execute("INSERT INTO created_dirs (run_id, path) VALUES (?1, ?2)", params![run_id, path])?;
        Ok(())
    }

    /// Folders created by a run, deepest first (safe removal order).
    pub fn created_dirs(&self, run_id: i64) -> Result<Vec<String>> {
        let mut st =
            self.conn.prepare("SELECT path FROM created_dirs WHERE run_id = ?1 ORDER BY length(path) DESC, id DESC")?;
        let rows = st.query_map([run_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn rows(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<JournalRow>> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st.query_map(p, |r| {
            let state: String = r.get(5)?;
            Ok(JournalRow {
                id: r.get(0)?,
                run_id: r.get(1)?,
                op: r.get(2)?,
                src: r.get(3)?,
                dst: r.get(4)?,
                state: JournalState::parse(&state).unwrap_or(JournalState::Failed),
                hash: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// All entries of a run, in the order they were written.
    pub fn journal_entries(&self, run_id: i64) -> Result<Vec<JournalRow>> {
        self.rows("SELECT id, run_id, op, src, dst, state, hash FROM journal WHERE run_id = ?1 ORDER BY id", [run_id])
    }

    /// Pending entries of *any* run (left behind by a crash or power cut).
    pub fn journal_pending_all(&self) -> Result<Vec<JournalRow>> {
        self.rows("SELECT id, run_id, op, src, dst, state, hash FROM journal WHERE state = 'pending' ORDER BY id", [])
    }

    /// Entries interrupted by a crash: these must be re-verified on resume.
    pub fn journal_pending(&self, run_id: i64) -> Result<Vec<(i64, String, String)>> {
        let mut st = self
            .conn
            .prepare("SELECT id, src, dst FROM journal WHERE run_id = ?1 AND state = 'pending' ORDER BY id")?;
        let rows = st.query_map([run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_and_reopens() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("cs.db");
        {
            let db = Db::open(&p).unwrap();
            assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
            db.create_run("a", "b", "copy").unwrap();
        }
        let db = Db::open(&p).unwrap(); // idempotent
        let n: i64 = db.conn().query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn refuses_newer_schema() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("cs.db");
        Connection::open(&p).unwrap().pragma_update(None, "user_version", 999).unwrap();
        assert!(matches!(Db::open(&p), Err(DbError::TooNew { found: 999, .. })));
    }

    #[test]
    fn journal_state_machine() {
        let db = Db::open_in_memory().unwrap();
        let run = db.create_run("src", "dst", "copy").unwrap();
        let j = db.journal_begin(run, "copy", "src/a.jpg", "dst/2005/a.jpg").unwrap();
        assert_eq!(db.journal_pending(run).unwrap().len(), 1);
        // Cannot trash a source whose copy was never verified.
        assert!(matches!(
            db.journal_set(j, JournalState::SourceTrashed, None, None),
            Err(DbError::BadTransition { .. })
        ));
        db.journal_set(j, JournalState::Done, Some("blake3:abc"), None).unwrap();
        assert!(db.journal_pending(run).unwrap().is_empty());
        db.journal_set(j, JournalState::SourceTrashed, None, None).unwrap();
        db.journal_set(j, JournalState::Undone, None, None).unwrap();
        assert_eq!(db.journal_state(j).unwrap(), Some(JournalState::Undone));
    }

    #[test]
    fn v2_journal_rows_survive_the_v3_migration() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("cs.db");
        {
            // Build a v2 database by hand, with one journal row.
            let c = Connection::open(&p).unwrap();
            for (i, sql) in MIGRATIONS.iter().take(2).enumerate() {
                c.execute_batch(sql).unwrap();
                c.pragma_update(None, "user_version", (i + 1) as i64).unwrap();
            }
            c.execute("INSERT INTO runs (source_root, dest_root, mode) VALUES ('s','d','copy')", []).unwrap();
            c.execute(
                "INSERT INTO journal (run_id, op, src, dst, state, hash) VALUES (1,'copy','a','b','done','h')",
                [],
            )
            .unwrap();
        }
        let db = Db::open(&p).unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
        let rows = db.journal_entries(1).unwrap();
        assert_eq!((rows[0].src.as_str(), rows[0].hash.as_deref()), ("a", Some("h")));
        db.journal_begin(1, "relocate", "x", "y").unwrap();
        db.journal_begin(1, "trash", "x", "").unwrap();
        assert!(db.journal_begin(1, "delete", "x", "").is_err(), "unknown operations are still refused");
    }

    #[test]
    fn mode_is_checked() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.create_run("a", "b", "delete").is_err());
    }
}
