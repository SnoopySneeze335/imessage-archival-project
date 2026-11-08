//! AddressBook.sqlitedb, built from the real iOS 26.1 schema
//! (`docs/reference/addressbook-schema.sql`). Only the tables that hold
//! names, organizations, phone numbers and emails get rows.

use rusqlite::{Connection, params};

use crate::scenario::{Scenario, apple_s, utc};

/// `ABMultiValue.property` values.
pub const PROPERTY_PHONE: i64 = 3;
pub const PROPERTY_EMAIL: i64 = 4;

pub fn write(conn: &Connection, s: &Scenario) -> rusqlite::Result<()> {
    crate::reference::create_tables(conn, crate::reference::ADDRESS_BOOK_SCHEMA)?;
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
    tx.commit()?;
    crate::reference::finish(conn, crate::reference::ADDRESS_BOOK_SCHEMA)
}
