//! AddressBook.sqlitedb: the subset of the iOS contacts database that holds
//! names, organizations, phone numbers and emails.

use rusqlite::{Connection, params};

use crate::scenario::{Scenario, apple_s, utc};

/// `ABMultiValue.property` values.
pub const PROPERTY_PHONE: i64 = 3;
pub const PROPERTY_EMAIL: i64 = 4;

const SCHEMA: &str = r"
CREATE TABLE ABPerson (
    ROWID INTEGER PRIMARY KEY AUTOINCREMENT,
    First TEXT, Last TEXT, Middle TEXT,
    FirstPhonetic TEXT, MiddlePhonetic TEXT, LastPhonetic TEXT,
    Organization TEXT, Department TEXT, Note TEXT,
    Kind INTEGER, Birthday TEXT, JobTitle TEXT, Nickname TEXT,
    Prefix TEXT, Suffix TEXT, FirstSort TEXT, LastSort TEXT,
    CreationDate INTEGER, ModificationDate INTEGER,
    CompositeNameFallback TEXT, ExternalIdentifier TEXT, guid TEXT
);
CREATE TABLE ABMultiValue (
    UID INTEGER PRIMARY KEY,
    record_id INTEGER,
    property INTEGER,
    identifier INTEGER,
    label INTEGER,
    value TEXT,
    guid TEXT
);
CREATE TABLE ABMultiValueLabel (value TEXT, UNIQUE (value));
CREATE INDEX ABMultiValueRecordIDIndex ON ABMultiValue (record_id);
";

pub fn write(conn: &Connection, s: &Scenario) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    let tx = conn.unchecked_transaction()?;
    for label in ["_$!<Mobile>!$_", "_$!<Home>!$_", "_$!<Work>!$_"] {
        tx.execute("INSERT INTO ABMultiValueLabel (value) VALUES (?1)", [label])?;
    }
    let created = apple_s(utc(2022, 11, 1, 12, 0));
    let mut uid = 1i64;
    for (i, c) in s.contacts.iter().enumerate() {
        let rowid = i as i64 + 1;
        let kind = i64::from(c.organization.is_some() && c.first.is_none());
        tx.execute(
            "INSERT INTO ABPerson (ROWID, First, Last, Organization, Kind, CreationDate, ModificationDate, guid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
            params![rowid, c.first, c.last, c.organization, kind, created + rowid * 86_400, format!("{rowid:08X}-AB00-4000-8000-000000000000:ABPerson")],
        )?;
        for (property, values, label) in [
            (PROPERTY_PHONE, &c.phones, 1i64),
            (PROPERTY_EMAIL, &c.emails, 2),
        ] {
            for (n, v) in values.iter().enumerate() {
                tx.execute(
                    "INSERT INTO ABMultiValue (UID, record_id, property, identifier, label, value, guid) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![uid, rowid, property, n as i64, label, v, format!("{uid:08X}-AB10-4000-8000-000000000000")],
                )?;
                uid += 1;
            }
        }
    }
    tx.commit()
}
