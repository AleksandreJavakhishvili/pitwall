//! `sql` (wit/sql.wit, platform-services.md §14.2): the extension's own
//! SQLite database, run by Pitwall with native SQLite (not in the guest).
//! One file per extension, `<data>/extension-data/<id>/db.sqlite`,
//! opened on first use, in WAL mode.
//!
//! What a tile's SQL can't do, enforced here:
//! - reach any other file: no `ATTACH` (and so no `VACUUM INTO`), no
//!   extension loading (off in rusqlite unless asked for);
//! - change how Pitwall keeps the file: only read-only `PRAGMA`s pass
//!   (and `foreign_keys`), so the size limit (`max_page_count`) and the
//!   schema version `migrate` keeps stay Pitwall's; SQLite's defensive
//!   mode is on;
//! - run past its call's deadline: a progress handler stops it;
//! - grow past its quota: `max_page_count`, so writes fail with
//!   `SQLITE_FULL` (`too-large`);
//! - send back more than 10 000 rows or 8 MiB at once.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::limits::Limit;
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, OpenFlags};

pub use crate::bindings::pitwall::tile::sql::{Rows, Value};
use crate::limits::{MAX_SQL_BYTES, MAX_SQL_ROWS};
use crate::ui::HostError;

/// Pragmas a tile may run: ones that only read, and `foreign_keys`.
const PRAGMAS: [&str; 12] = ["table_info", "table_xinfo", "table_list", "index_list", "index_info", "index_xinfo", "foreign_key_list", "foreign_keys", "page_count", "freelist_count", "integrity_check", "quick_check"];

/// The file of extension data folder `dir`'s database.
pub fn db_file(dir: &Path) -> PathBuf {
    dir.join("db.sqlite")
}

/// An extension's database, opened on first use.
pub(crate) struct Db {
    file: PathBuf,
    quota: u64,
    conn: Option<Connection>,
    /// Set while a tile's statement runs: the authorizer applies to it,
    /// not to Pitwall's own.
    guest: Arc<AtomicBool>,
}

fn failed(e: impl std::fmt::Display) -> HostError {
    HostError::Failed(crate::host::cut(&e.to_string(), 500))
}

/// SQLite's error as the tile gets it.
fn sql_error(e: rusqlite::Error) -> HostError {
    use rusqlite::ErrorCode;
    match &e {
        rusqlite::Error::SqliteFailure(f, _) => match f.code {
            ErrorCode::DiskFull | ErrorCode::TooBig => HostError::TooLarge,
            ErrorCode::OperationInterrupted => HostError::Busy,
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => HostError::Busy,
            ErrorCode::SystemIoFailure | ErrorCode::CannotOpen | ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase | ErrorCode::OutOfMemory | ErrorCode::ReadOnly => failed(&e),
            _ => HostError::Invalid(crate::host::cut(&e.to_string(), 500)),
        },
        _ => HostError::Invalid(crate::host::cut(&e.to_string(), 500)),
    }
}

fn to_sql(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Integer(i) => SqlValue::Integer(*i),
        Value::Real(r) => SqlValue::Real(*r),
        Value::Text(t) => SqlValue::Text(t.clone()),
        Value::Blob(b) => SqlValue::Blob(b.clone()),
    }
}

fn from_sql(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Integer(i),
        ValueRef::Real(r) => Value::Real(r),
        ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Blob(b.to_vec()),
    }
}

fn size_of(v: &Value) -> usize {
    match v {
        Value::Text(t) => t.len(),
        Value::Blob(b) => b.len(),
        _ => 8,
    }
}

impl Db {
    pub fn new(dir: &Path, quota: u64) -> Db {
        Db { file: db_file(dir), quota, conn: None, guest: Arc::new(AtomicBool::new(false)) }
    }

    /// The most the database may take, in bytes.
    pub fn quota(&self) -> u64 {
        self.quota
    }

    fn open(&mut self) -> Result<&Connection, HostError> {
        if self.conn.is_none() {
            if let Some(dir) = self.file.parent() {
                std::fs::create_dir_all(dir).map_err(failed)?;
            }
            let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            let conn = Connection::open_with_flags(&self.file, flags).map_err(failed)?;
            conn.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true).map_err(failed)?;
            conn.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false).map_err(failed)?;
            conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0).map_err(failed)?;
            conn.set_limit(Limit::SQLITE_LIMIT_LENGTH, MAX_SQL_BYTES as i32).map_err(failed)?;
            conn.pragma_update(None, "journal_mode", "WAL").map_err(failed)?;
            let page: u64 = conn.pragma_query_value(None, "page_size", |r| r.get(0)).map_err(failed)?;
            conn.pragma_update(None, "max_page_count", (self.quota / page.max(512)).max(1)).map_err(failed)?;
            let guest = self.guest.clone();
            conn.authorizer(Some(move |cx: AuthContext<'_>| {
                if !guest.load(Ordering::Relaxed) {
                    return Authorization::Allow;
                }
                match cx.action {
                    AuthAction::Attach { .. } | AuthAction::Detach { .. } => Authorization::Deny,
                    AuthAction::Pragma { pragma_name, pragma_value } => {
                        let name = pragma_name.to_ascii_lowercase();
                        let read = pragma_value.is_none() || name == "foreign_keys" || name.ends_with("_check") || name.starts_with("table_") || name.starts_with("index_") || name == "foreign_key_list";
                        if PRAGMAS.contains(&name.as_str()) && read {
                            Authorization::Allow
                        } else {
                            Authorization::Deny
                        }
                    }
                    _ => Authorization::Allow,
                }
            }));
            self.conn = Some(conn);
        }
        self.conn.as_ref().ok_or_else(|| HostError::Failed("the database isn't open".into()))
    }

    /// Run `f` on the connection as the tile, stopped at `deadline` or
    /// when `stop` is set.
    fn as_guest<T>(&mut self, deadline: Instant, stop: Arc<AtomicBool>, f: impl FnOnce(&Connection) -> Result<T, HostError>) -> Result<T, HostError> {
        let guest = self.guest.clone();
        let conn = self.open()?;
        conn.progress_handler(1000, Some(move || stop.load(Ordering::Relaxed) || Instant::now() >= deadline));
        guest.store(true, Ordering::Relaxed);
        let r = f(conn);
        guest.store(false, Ordering::Relaxed);
        conn.progress_handler(0, None::<fn() -> bool>);
        r
    }

    pub fn execute(&mut self, statement: &str, params: &[Value], deadline: Instant, stop: Arc<AtomicBool>) -> Result<u64, HostError> {
        let params: Vec<SqlValue> = params.iter().map(to_sql).collect();
        self.as_guest(deadline, stop, |c| c.execute(statement, rusqlite::params_from_iter(params.iter())).map(|n| n as u64).map_err(sql_error))
    }

    pub fn query(&mut self, statement: &str, params: &[Value], deadline: Instant, stop: Arc<AtomicBool>) -> Result<Rows, HostError> {
        let params: Vec<SqlValue> = params.iter().map(to_sql).collect();
        self.as_guest(deadline, stop, |c| {
            let mut st = c.prepare(statement).map_err(sql_error)?;
            let columns: Vec<String> = st.column_names().into_iter().map(String::from).collect();
            let n = columns.len();
            let mut q = st.query(rusqlite::params_from_iter(params.iter())).map_err(sql_error)?;
            let (mut rows, mut bytes) = (Vec::new(), 0usize);
            while let Some(row) = q.next().map_err(sql_error)? {
                if rows.len() >= MAX_SQL_ROWS {
                    return Err(HostError::TooLarge);
                }
                let mut vals = Vec::with_capacity(n);
                for i in 0..n {
                    let v = from_sql(row.get_ref(i).map_err(sql_error)?);
                    bytes += size_of(&v);
                    vals.push(v);
                }
                if bytes > MAX_SQL_BYTES {
                    return Err(HostError::TooLarge);
                }
                rows.push(vals);
            }
            Ok(Rows { columns, rows })
        })
    }

    /// Run the steps after the database's `user_version`, each in its own
    /// transaction; the version now.
    pub fn migrate(&mut self, steps: &[String], deadline: Instant, stop: Arc<AtomicBool>) -> Result<u32, HostError> {
        let conn = self.open()?;
        if !conn.is_autocommit() {
            return Err(HostError::Invalid("a transaction is open: commit it before migrating".into()));
        }
        let have: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(failed)?;
        if (steps.len() as u64) < u64::from(have) {
            return Err(HostError::Invalid(format!("the database has {have} migrations and the tile gives {}: a tile can't go back a schema", steps.len())));
        }
        for (i, step) in steps.iter().enumerate().skip(have as usize) {
            let n = i as u32 + 1;
            let ran = self.as_guest(deadline, stop.clone(), |c| c.execute_batch(&format!("BEGIN;\n{step}\n;")).map_err(sql_error));
            let conn = self.open()?;
            match ran {
                Ok(()) => {
                    conn.pragma_update(None, "user_version", n).map_err(failed)?;
                    conn.execute_batch("COMMIT").map_err(failed)?;
                }
                Err(e) => {
                    if !conn.is_autocommit() {
                        let _ = conn.execute_batch("ROLLBACK");
                    }
                    let why = match e {
                        HostError::Invalid(w) | HostError::Failed(w) => w,
                        HostError::TooLarge => "the database is full".into(),
                        HostError::Busy => "it took too long".into(),
                        HostError::Denied => "not allowed".into(),
                    };
                    return Err(HostError::Invalid(format!("migration {n} failed: {why}")));
                }
            }
        }
        Ok(steps.len().max(have as usize) as u32)
    }

    pub fn begin(&mut self) -> Result<(), HostError> {
        let conn = self.open()?;
        if !conn.is_autocommit() {
            return Err(HostError::Invalid("a transaction is already open".into()));
        }
        conn.execute_batch("BEGIN").map_err(sql_error)
    }

    pub fn commit(&mut self) -> Result<(), HostError> {
        let conn = self.open()?;
        if conn.is_autocommit() {
            return Err(HostError::Invalid("no transaction is open".into()));
        }
        conn.execute_batch("COMMIT").map_err(sql_error)
    }

    pub fn rollback(&mut self) -> Result<(), HostError> {
        let conn = self.open()?;
        if conn.is_autocommit() {
            return Err(HostError::Invalid("no transaction is open".into()));
        }
        conn.execute_batch("ROLLBACK").map_err(sql_error)
    }

    /// After a call into the tile: a transaction it left open is rolled
    /// back. Whether one was.
    pub fn end_call(&mut self) -> bool {
        match &self.conn {
            Some(c) if !c.is_autocommit() => {
                let _ = c.execute_batch("ROLLBACK");
                true
            }
            _ => false,
        }
    }

    /// The bytes the database takes now (0 before it is made).
    pub fn usage(&mut self) -> u64 {
        if self.conn.is_none() && !self.file.is_file() {
            return 0;
        }
        let Ok(conn) = self.open() else { return 0 };
        let count: u64 = conn.pragma_query_value(None, "page_count", |r| r.get(0)).unwrap_or(0);
        let size: u64 = conn.pragma_query_value(None, "page_size", |r| r.get(0)).unwrap_or(0);
        count * size
    }
}

/// The bytes extension data folder `dir`'s database takes on disk (its
/// file and its write-ahead log), for the card.
pub fn usage_on_disk(dir: &Path) -> u64 {
    let f = db_file(dir);
    let wal = f.with_extension("sqlite-wal");
    [f, wal].iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn db(dir: &Path, quota: u64) -> Db {
        Db::new(dir, quota)
    }

    fn later() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    fn no() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn statements_migrations_and_transactions() {
        let t = tempfile::tempdir().unwrap();
        let mut d = db(t.path(), 100 << 20);
        let steps = vec!["CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0);".to_string()];
        assert_eq!(d.migrate(&steps, later(), no()), Ok(1));
        assert_eq!(d.migrate(&steps, later(), no()), Ok(1), "run once");
        let mut more = steps.clone();
        more.push("ALTER TABLE notes ADD COLUMN body TEXT; CREATE INDEX notes_done ON notes(done);".into());
        assert_eq!(d.migrate(&more, later(), no()), Ok(2));
        assert!(matches!(d.migrate(&steps, later(), no()), Err(HostError::Invalid(w)) if w.contains("can't go back")));
        let mut bad = more.clone();
        bad.push("CREATE TABLE x (a); NOT SQL;".into());
        assert!(matches!(d.migrate(&bad, later(), no()), Err(HostError::Invalid(w)) if w.contains("migration 3 failed")));
        assert!(d.query("SELECT name FROM sqlite_master WHERE name = 'x'", &[], later(), no()).unwrap().rows.is_empty(), "a failed step is rolled back");

        assert_eq!(d.execute("INSERT INTO notes (title) VALUES (?), (?)", &[Value::Text("a".into()), Value::Text("b".into())], later(), no()), Ok(2));
        let rows = d.query("SELECT id, title, done FROM notes WHERE done = ?", &[Value::Integer(0)], later(), no()).unwrap();
        assert_eq!(rows.columns, ["id", "title", "done"]);
        assert_eq!(rows.rows[1], vec![Value::Integer(2), Value::Text("b".into()), Value::Integer(0)]);
        assert!(matches!(d.execute("INSERT INTO notes (title) VALUES (NULL)", &[], later(), no()), Err(HostError::Invalid(w)) if w.contains("NOT NULL")));

        d.begin().unwrap();
        d.execute("DELETE FROM notes", &[], later(), no()).unwrap();
        assert!(d.begin().is_err());
        assert!(d.end_call(), "left open: rolled back");
        assert_eq!(d.query("SELECT count(*) FROM notes", &[], later(), no()).unwrap().rows[0][0], Value::Integer(2));
        d.begin().unwrap();
        d.execute("DELETE FROM notes WHERE id = 1", &[], later(), no()).unwrap();
        d.commit().unwrap();
        assert!(!d.end_call());
        assert!(d.usage() > 0 && usage_on_disk(t.path()) > 0);
    }

    #[test]
    fn what_a_tile_cant_do() {
        let t = tempfile::tempdir().unwrap();
        let mut d = db(t.path(), 1 << 20);
        let other = t.path().join("other.sqlite");
        let attach = format!("ATTACH DATABASE '{}' AS other", other.display());
        assert!(d.execute(&attach, &[], later(), no()).is_err());
        assert!(d.execute(&format!("VACUUM INTO '{}'", other.display()), &[], later(), no()).is_err());
        assert!(!other.exists(), "no other file");
        assert!(d.execute("PRAGMA max_page_count = 100000000", &[], later(), no()).is_err(), "its quota is Pitwall's");
        assert!(d.execute("PRAGMA user_version = 9", &[], later(), no()).is_err(), "its schema version is migrate's");
        assert!(d.execute("PRAGMA journal_mode = DELETE", &[], later(), no()).is_err());
        assert!(d.query("PRAGMA table_list", &[], later(), no()).is_ok());
        assert!(d.execute("PRAGMA foreign_keys = ON", &[], later(), no()).is_ok());
        assert!(d.query("SELECT load_extension('x')", &[], later(), no()).is_err());
        // Over its quota: too large.
        d.execute("CREATE TABLE b (x BLOB)", &[], later(), no()).unwrap();
        let mut full = None;
        for _ in 0..40 {
            if let Err(e) = d.execute("INSERT INTO b VALUES (?)", &[Value::Blob(vec![7; 64 << 10])], later(), no()) {
                full = Some(e);
                break;
            }
        }
        assert_eq!(full, Some(HostError::TooLarge));
        // Past the deadline: stopped.
        let slow = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c";
        assert_eq!(d.query(slow, &[], Instant::now() + Duration::from_millis(50), no()).unwrap_err(), HostError::Busy);
        // Too many rows at once.
        let many = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c LIMIT 10001) SELECT x FROM c";
        assert_eq!(d.query(many, &[], later(), no()).unwrap_err(), HostError::TooLarge);
        assert!(d.execute("SELECT 1; SELECT 2", &[], later(), no()).is_err(), "one statement a call");
    }
}
