//! The real iOS 26.1 schemas, applied exactly.
//!
//! `docs/reference/*.sql` are `.schema` dumps of the Messages and Contacts
//! databases from a real backup (no data). The generator builds its databases
//! from these files so the fixture has the same tables, columns, indexes and
//! triggers as the real thing, and [`verify`] proves it after every build.
//!
//! Three kinds of statement cannot simply be executed:
//!
//! - `sqlite_sequence` and `sqlite_stat1` are SQLite's own tables and cannot
//!   be created by name. SQLite creates the first with any AUTOINCREMENT
//!   table and the second on `ANALYZE`, with exactly the text in the dumps.
//!   This SQLite build also creates `sqlite_stat4` on `ANALYZE`; iOS's does
//!   not, so it is dropped.
//! - The AddressBook's FTS4 tables use Apple's `ab_cf_tokenizer`, which only
//!   exists on Apple platforms. Their schema rows are written directly, as on
//!   a real backup read off-device: the table is declared but unusable, and
//!   its shadow tables are ordinary tables.
//! - The sqlite3 shell prints FTS shadow tables as `CREATE TABLE IF NOT
//!   EXISTS 'x'(...)`, but SQLite stores them as `CREATE TABLE 'x'(...)`. The
//!   stored form is what [`Statement::sql`] holds and what [`verify`] checks.
//! - Many triggers call Apple-only SQL functions (`verify_chat`,
//!   `ab_update_value_from_trigger`, ...). Indexes and triggers are created
//!   after the data is inserted, so no trigger ever fires.

use rusqlite::{Connection, params};

pub const SMS_SCHEMA: &str = include_str!("../../../docs/reference/sms-schema.sql");
pub const ADDRESS_BOOK_SCHEMA: &str =
    include_str!("../../../docs/reference/addressbook-schema.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Table,
    VirtualTable,
    /// `sqlite_sequence`, `sqlite_stat1`: created by SQLite itself.
    Internal,
    Index,
    Trigger,
}

#[derive(Debug, Clone)]
pub struct Statement {
    pub kind: Kind,
    pub name: String,
    /// Exactly the text SQLite stores in `sqlite_master.sql`.
    pub sql: String,
}

/// Splits a `.schema` dump into statements. Every statement starts on a line
/// beginning with `CREATE `; other lines continue the previous statement.
pub fn statements(schema: &str) -> Vec<Statement> {
    let mut texts: Vec<String> = Vec::new();
    for line in schema.lines() {
        if line.starts_with("CREATE ") || texts.is_empty() {
            texts.push(line.to_owned());
        } else {
            let last = texts.last_mut().expect("non-empty");
            last.push('\n');
            last.push_str(line);
        }
    }
    texts
        .into_iter()
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            let mut sql = t
                .trim_end()
                .strip_suffix(';')
                .unwrap_or(t.trim_end())
                .to_owned();
            if let Some(rest) = sql.strip_prefix("CREATE TABLE IF NOT EXISTS ") {
                sql = format!("CREATE TABLE {rest}");
            }
            let (kind, rest) = if let Some(r) = sql.strip_prefix("CREATE VIRTUAL TABLE ") {
                (Kind::VirtualTable, r)
            } else if let Some(r) = sql.strip_prefix("CREATE TABLE ") {
                let kind = if r.starts_with("sqlite_") {
                    Kind::Internal
                } else {
                    Kind::Table
                };
                (kind, r)
            } else if let Some(r) = sql
                .strip_prefix("CREATE INDEX ")
                .or_else(|| sql.strip_prefix("CREATE UNIQUE INDEX "))
            {
                (Kind::Index, r)
            } else if let Some(r) = sql.strip_prefix("CREATE TRIGGER ") {
                (Kind::Trigger, r)
            } else {
                panic!(
                    "unrecognized schema statement: {}",
                    &sql[..sql.len().min(60)]
                );
            };
            let name = rest
                .split(|c: char| c.is_whitespace() || c == '(')
                .next()
                .unwrap_or_default()
                .trim_matches(|c| c == '\'' || c == '"')
                .to_owned();
            Statement { kind, name, sql }
        })
        .collect()
}

/// Creates every ordinary table. Run before inserting data.
pub fn create_tables(conn: &Connection, schema: &str) -> rusqlite::Result<()> {
    for s in statements(schema).iter() {
        if s.kind == Kind::Table {
            conn.execute_batch(&s.sql)?;
        }
    }
    Ok(())
}

/// Creates indexes and triggers, runs `ANALYZE` if the dump has
/// `sqlite_stat1`, then declares any virtual
/// tables. Run after inserting data; the connection must be closed afterwards
/// because the schema was edited underneath it.
pub fn finish(conn: &Connection, schema: &str) -> rusqlite::Result<()> {
    let stmts = statements(schema);
    for s in stmts
        .iter()
        .filter(|s| matches!(s.kind, Kind::Index | Kind::Trigger))
    {
        conn.execute_batch(&s.sql)?;
    }
    if stmts
        .iter()
        .any(|s| s.kind == Kind::Internal && s.name == "sqlite_stat1")
    {
        conn.execute_batch("ANALYZE; DROP TABLE IF EXISTS sqlite_stat4;")?;
    }
    let virtuals: Vec<_> = stmts
        .iter()
        .filter(|s| s.kind == Kind::VirtualTable)
        .collect();
    if !virtuals.is_empty() {
        conn.execute_batch("PRAGMA writable_schema = ON;")?;
        for s in virtuals {
            conn.execute(
                "INSERT INTO sqlite_master (type, name, tbl_name, rootpage, sql) VALUES ('table', ?1, ?1, 0, ?2)",
                params![s.name, s.sql],
            )?;
        }
        conn.execute_batch("PRAGMA writable_schema = OFF;")?;
    }
    Ok(())
}

/// The `sql` of every schema object in `conn`, sorted.
pub fn schema_of(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL")?;
    let mut out: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    out.sort();
    Ok(out)
}

/// Fails unless `conn`'s schema is exactly `schema`'s statements.
pub fn verify(conn: &Connection, schema: &str, label: &str) -> anyhow::Result<()> {
    let actual = schema_of(conn)?;
    let mut expected: Vec<String> = statements(schema).into_iter().map(|s| s.sql).collect();
    expected.sort();
    if actual != expected {
        let missing: Vec<_> = expected.iter().filter(|e| !actual.contains(e)).collect();
        let extra: Vec<_> = actual.iter().filter(|a| !expected.contains(a)).collect();
        anyhow::bail!(
            "{label} schema differs from the reference: {} missing, {} unexpected (first missing: {:?}, first unexpected: {:?})",
            missing.len(),
            extra.len(),
            missing.first().map(|s| &s[..s.len().min(80)]),
            extra.first().map(|s| &s[..s.len().min(80)]),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_accepts_the_reference_and_rejects_any_difference() {
        for schema in [SMS_SCHEMA, ADDRESS_BOOK_SCHEMA] {
            let conn = Connection::open_in_memory().unwrap();
            create_tables(&conn, schema).unwrap();
            finish(&conn, schema).unwrap();
            drop(conn);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sms.db");
        {
            let conn = Connection::open(&path).unwrap();
            create_tables(&conn, SMS_SCHEMA).unwrap();
            finish(&conn, SMS_SCHEMA).unwrap();
        }
        let conn = Connection::open(&path).unwrap();
        verify(&conn, SMS_SCHEMA, "sms.db").unwrap();
        conn.execute_batch("CREATE INDEX extra_idx ON message(text);")
            .unwrap();
        assert!(verify(&conn, SMS_SCHEMA, "sms.db").is_err());
    }

    #[test]
    fn dumps_parse_into_known_statement_kinds() {
        let sms = statements(SMS_SCHEMA);
        assert!(
            sms.iter()
                .any(|s| s.kind == Kind::Table && s.name == "message")
        );
        assert!(sms.iter().any(
            |s| s.kind == Kind::Trigger && s.name == "chat_service_on_insert_chat_message_join"
        ));
        let ab = statements(ADDRESS_BOOK_SCHEMA);
        assert_eq!(
            ab.iter().filter(|s| s.kind == Kind::VirtualTable).count(),
            2
        );
        assert!(
            ab.iter()
                .any(|s| s.kind == Kind::Table && s.name == "ABPersonFullTextSearch_content")
        );
    }
}
