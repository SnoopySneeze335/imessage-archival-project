//! archive.db: the rebuildable store of everything imported from backups.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

/// File name of the archive inside the data directory.
pub const ARCHIVE_DB: &str = "archive.db";

/// Schema migrations, applied in order. `PRAGMA user_version` records how many
/// have run. Never edit a released migration; add a new one.
const MIGRATIONS: &[&str] = &[
    include_str!("0001_initial.sql"),
    include_str!("0002_versions.sql"),
];

/// Path of archive.db inside `data_dir`.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(ARCHIVE_DB)
}

/// Opens (creating if needed) archive.db for writing and migrates it.
pub fn open(data_dir: &Path) -> Result<Connection> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("creating data directory {}", data_dir.display()))?;
    let mut conn = Connection::open(path(data_dir))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Opens an existing archive.db without the ability to write.
pub fn open_read_only(data_dir: &Path) -> Result<Connection> {
    let p = path(data_dir);
    anyhow::ensure!(
        p.is_file(),
        "no archive at {} (run `imessage-ap ingest` first)",
        p.display()
    );
    let conn = Connection::open_with_flags(
        &p,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let version = conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize;
    anyhow::ensure!(
        version == MIGRATIONS.len(),
        "archive schema version {version} does not match this build ({})",
        MIGRATIONS.len()
    );
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let current = conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize;
    anyhow::ensure!(
        current <= MIGRATIONS.len(),
        "archive schema version {current} is newer than this build supports ({})",
        MIGRATIONS.len()
    );
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("archive migration {}", i + 1))?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

/// The migration scripts, oldest first. Tests use this to build an archive at
/// an older schema version.
#[doc(hidden)]
pub fn migrations() -> &'static [&'static str] {
    MIGRATIONS
}

/// Number of schema migrations this build knows.
pub fn schema_version() -> usize {
    MIGRATIONS.len()
}
