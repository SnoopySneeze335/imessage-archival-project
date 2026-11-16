//! user.db: what only the user authors (password, sessions, settings; later
//! people, links and merges). It cannot be rebuilt from backups, so every
//! schema change is a migration and is tested from the previous version.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::dirs::Dirs;

/// Schema migrations, applied in order. `PRAGMA user_version` records how many
/// have run. Never edit a released migration; add a new one.
const MIGRATIONS: &[&str] = &[include_str!("0001_initial.sql")];

/// How long a session lasts without use. Each use (at most once an hour)
/// extends it again.
pub const SESSION_LIFETIME_NS: i64 = 90 * 24 * 3600 * 1_000_000_000;
const SESSION_TOUCH_NS: i64 = 3600 * 1_000_000_000;

/// Opens (creating if needed) user.db and migrates it.
pub fn open(dirs: &Dirs) -> Result<Connection> {
    dirs.check()?;
    std::fs::create_dir_all(&dirs.db)
        .with_context(|| format!("creating database directory {}", dirs.db.display()))?;
    let mut conn = Connection::open(dirs.user_db())
        .with_context(|| format!("opening {}", dirs.user_db().display()))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;
    migrate(&mut conn)?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let current = conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize;
    anyhow::ensure!(
        current <= MIGRATIONS.len(),
        "user.db schema version {current} is newer than this build supports ({})",
        MIGRATIONS.len()
    );
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("user.db migration {}", i + 1))?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

/// Number of schema migrations this build knows.
pub fn schema_version() -> usize {
    MIGRATIONS.len()
}

pub fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i64)
}

/// The stored password hash, if a password has been set.
pub fn password_hash(conn: &Connection) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT hash FROM password WHERE id = 1", [], |r| r.get(0))
        .optional()?)
}

/// Stores a new password hash and ends every session.
pub fn set_password_hash(conn: &Connection, hash: &str) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<()> {
        conn.execute(
            "INSERT INTO password (id, hash, set_at) VALUES (1, ?1, ?2)
             ON CONFLICT (id) DO UPDATE SET hash = excluded.hash, set_at = excluded.set_at",
            params![hash, now_ns()],
        )?;
        conn.execute("DELETE FROM session", [])?;
        Ok(())
    })();
    conn.execute_batch(if result.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    result
}

/// Records a new session for a token's hash.
pub fn create_session(conn: &Connection, token_hash: &str) -> Result<()> {
    let now = now_ns();
    conn.execute("DELETE FROM session WHERE expires_at < ?1", params![now])?;
    conn.execute(
        "INSERT INTO session (token_hash, created_at, last_seen_at, expires_at) VALUES (?1, ?2, ?2, ?3)",
        params![token_hash, now, now + SESSION_LIFETIME_NS],
    )?;
    Ok(())
}

/// Whether a session is valid now; extends it when used.
pub fn check_session(conn: &Connection, token_hash: &str) -> Result<bool> {
    let now = now_ns();
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT last_seen_at, expires_at FROM session WHERE token_hash = ?1",
            [token_hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        Some((_, expires)) if expires < now => {
            conn.execute("DELETE FROM session WHERE token_hash = ?1", [token_hash])?;
            Ok(false)
        }
        Some((seen, _)) => {
            if now - seen > SESSION_TOUCH_NS {
                conn.execute(
                    "UPDATE session SET last_seen_at = ?2, expires_at = ?3 WHERE token_hash = ?1",
                    params![token_hash, now, now + SESSION_LIFETIME_NS],
                )?;
            }
            Ok(true)
        }
        None => Ok(false),
    }
}

pub fn delete_session(conn: &Connection, token_hash: &str) -> Result<()> {
    conn.execute("DELETE FROM session WHERE token_hash = ?1", [token_hash])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_end_when_the_password_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = open(&Dirs::under(tmp.path())).unwrap();
        assert_eq!(password_hash(&conn).unwrap(), None);
        set_password_hash(&conn, "h1").unwrap();
        create_session(&conn, "t").unwrap();
        assert!(check_session(&conn, "t").unwrap());
        assert!(!check_session(&conn, "other").unwrap());
        set_password_hash(&conn, "h2").unwrap();
        assert_eq!(password_hash(&conn).unwrap().as_deref(), Some("h2"));
        assert!(!check_session(&conn, "t").unwrap());
    }

    #[test]
    fn expired_sessions_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = open(&Dirs::under(tmp.path())).unwrap();
        create_session(&conn, "t").unwrap();
        conn.execute("UPDATE session SET expires_at = 1", [])
            .unwrap();
        assert!(!check_session(&conn, "t").unwrap());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM session", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn reopening_runs_no_migration_twice() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = Dirs::under(tmp.path());
        drop(open(&dirs).unwrap());
        let conn = open(&dirs).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v as usize, schema_version());
    }
}
