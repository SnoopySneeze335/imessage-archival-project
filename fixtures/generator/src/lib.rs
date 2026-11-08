//! Generates a fake, deterministic, unencrypted iPhone backup for tests.
//!
//! The layout follows a local backup made by the Apple Devices app or
//! iTunes: `Info.plist`, `Manifest.plist`, `Status.plist`, `Manifest.db`, and
//! every file stored at `<first two hex chars>/<sha1("domain-relativePath")>`.
//!
//! Nothing here is real. See `scenario.rs` for the invented cast.

pub mod addressbook;
pub mod media;
pub mod reference;
pub mod scenario;
pub mod sms;
pub mod typedstream;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Uid, Value};
use rusqlite::{Connection, params};
use serde::Serialize;
use sha1::{Digest, Sha1};

use scenario::{Scenario, utc};

/// Domain and path of the Messages database inside a backup.
pub const SMS_DB: (&str, &str) = ("HomeDomain", "Library/SMS/sms.db");
/// Domain and path of the contacts database inside a backup.
pub const ADDRESS_BOOK_DB: (&str, &str) =
    ("HomeDomain", "Library/AddressBook/AddressBook.sqlitedb");
/// Name of the counts file written next to the backup's own files.
pub const EXPECTED_FILE: &str = "fixture-expected.json";

/// Fictional device identity.
const UDID: &str = "FIXTURE0-0000000000000001";
const DEVICE_NAME: &str = "Fixture iPhone";
const PRODUCT_VERSION: &str = "26.1";
const BUILD_VERSION: &str = "23B0";
const PRODUCT_TYPE: &str = "iPhone14,2";

/// Counts an ingester must reproduce from this backup.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Expected {
    pub generator_version: &'static str,
    pub messages: usize,
    pub messages_in_chats: usize,
    pub messages_without_chat: usize,
    pub recoverable_deleted: usize,
    pub reactions: usize,
    pub replies: usize,
    pub edited: usize,
    pub unsent: usize,
    pub group_events: usize,
    /// Messages whose text is only in `attributedBody` (`text` is NULL).
    pub text_only_in_attributed_body: usize,
    pub chats: usize,
    /// Chats beyond the first that share a `chat_identifier` (one address,
    /// several chat rows); these must merge into one conversation.
    pub duplicated_chats: usize,
    pub handles: usize,
    /// Handle rows beyond the first for the same normalized address.
    pub duplicate_handles: usize,
    pub unique_addresses: usize,
    pub addressbook_people: usize,
    pub addresses_with_name: usize,
    pub attachments: usize,
    pub attachments_present: usize,
    pub attachments_missing_file: usize,
    pub attachments_without_path: usize,
    pub attachment_bytes_present: u64,
    pub messages_per_chat: BTreeMap<String, usize>,
}

/// `fileID` of a file in a backup: SHA-1 of `"<domain>-<relativePath>"`.
pub fn file_id(domain: &str, relative_path: &str) -> String {
    let digest = Sha1::digest(format!("{domain}-{relative_path}").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Lowercases emails and turns US phone numbers into E.164. Short codes are
/// left alone. Only for computing fixture expectations; the app has its own.
pub fn normalize_address(raw: &str) -> String {
    if raw.contains('@') {
        return raw.to_lowercase();
    }
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    match digits.len() {
        10 => format!("+1{digits}"),
        11 if digits.starts_with('1') => format!("+{digits}"),
        _ => digits,
    }
}

/// Writes the backup into `out`, which must not exist or be empty.
pub fn generate(out: &Path) -> Result<Expected> {
    if out.exists() && fs::read_dir(out)?.next().is_some() {
        bail!("{} is not empty", out.display());
    }
    fs::create_dir_all(out)?;
    let s = scenario::build();
    let backup_date = SystemTime::UNIX_EPOCH + Duration::from_secs(utc(2025, 5, 1, 9, 30) as u64);

    let mut files: Vec<(String, String, Vec<u8>)> = Vec::new();
    files.push((
        SMS_DB.0.into(),
        SMS_DB.1.into(),
        sqlite_bytes(out, reference::SMS_SCHEMA, "sms.db", |c| sms::write(c, &s))?,
    ));
    files.push((
        ADDRESS_BOOK_DB.0.into(),
        ADDRESS_BOOK_DB.1.into(),
        sqlite_bytes(out, reference::ADDRESS_BOOK_SCHEMA, "AddressBook", |c| {
            addressbook::write(c, &s)
        })?,
    ));
    for a in &s.attachments {
        if let (Some(path), Some(content)) = (&a.filename, &a.content) {
            let rel = path.trim_start_matches("~/").to_owned();
            files.push(("MediaDomain".into(), rel, content.clone()));
        }
    }

    let manifest = Connection::open(out.join("Manifest.db"))?;
    manifest.execute_batch(
        "PRAGMA journal_mode = DELETE;
         CREATE TABLE Files (fileID TEXT PRIMARY KEY, domain TEXT, relativePath TEXT, flags INTEGER, file BLOB);
         CREATE INDEX FilesDomainIdx ON Files(domain);
         CREATE INDEX FilesRelativePathIdx ON Files(relativePath);
         CREATE TABLE Properties (key TEXT PRIMARY KEY, value BLOB);",
    )?;
    let dirs: BTreeSet<(String, String)> = files
        .iter()
        .flat_map(|(domain, rel, _)| {
            let parts: Vec<&str> = rel.split('/').collect();
            (1..parts.len()).map(move |n| (domain.clone(), parts[..n].join("/")))
        })
        .collect();
    let tx = manifest.unchecked_transaction()?;
    for (domain, rel) in &dirs {
        tx.execute(
            "INSERT INTO Files VALUES (?1, ?2, ?3, 2, ?4)",
            params![file_id(domain, rel), domain, rel, mbfile(rel, 0, 0o40755)?],
        )?;
    }
    for (domain, rel, bytes) in &files {
        let id = file_id(domain, rel);
        let dir = out.join(&id[..2]);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(&id), bytes)?;
        tx.execute(
            "INSERT INTO Files VALUES (?1, ?2, ?3, 1, ?4)",
            params![id, domain, rel, mbfile(rel, bytes.len() as u64, 0o100644)?],
        )?;
    }
    tx.commit()?;
    drop(manifest);

    write_plists(out, backup_date)?;
    let expected = expected(&s);
    fs::write(
        out.join(EXPECTED_FILE),
        serde_json::to_string_pretty(&expected)? + "\n",
    )?;
    Ok(expected)
}

/// Builds a SQLite database in a scratch file, checks its schema against the
/// reference dump, and returns its bytes.
fn sqlite_bytes(
    out: &Path,
    schema: &str,
    label: &str,
    fill: impl FnOnce(&Connection) -> rusqlite::Result<()>,
) -> Result<Vec<u8>> {
    let tmp = out.join(".fixture-tmp.sqlite");
    let _ = fs::remove_file(&tmp);
    {
        let conn = Connection::open(&tmp)?;
        conn.execute_batch("PRAGMA journal_mode = DELETE;")?;
        fill(&conn).with_context(|| format!("writing fixture {label}"))?;
    }
    reference::verify(&Connection::open(&tmp)?, schema, label)?;
    let bytes = fs::read(&tmp)?;
    fs::remove_file(&tmp)?;
    Ok(bytes)
}

/// The NSKeyedArchiver `MBFile` record stored in `Manifest.db`'s `file` column.
fn mbfile(relative_path: &str, size: u64, mode: u64) -> Result<Vec<u8>> {
    let t = utc(2025, 4, 30, 12, 0) as u64;
    let mut file = Dictionary::new();
    file.insert("$class".into(), Value::Uid(Uid::new(3)));
    file.insert("Birth".into(), t.into());
    file.insert("Flags".into(), 0u64.into());
    file.insert("GroupID".into(), 501u64.into());
    file.insert("InodeNumber".into(), 0u64.into());
    file.insert("LastModified".into(), t.into());
    file.insert("LastStatusChange".into(), t.into());
    file.insert("Mode".into(), mode.into());
    file.insert("ProtectionClass".into(), 3u64.into());
    file.insert("RelativePath".into(), Value::Uid(Uid::new(2)));
    file.insert("Size".into(), size.into());
    file.insert("UserID".into(), 501u64.into());
    let mut class = Dictionary::new();
    class.insert(
        "$classes".into(),
        Value::Array(vec!["MBFile".into(), "NSObject".into()]),
    );
    class.insert("$classname".into(), "MBFile".into());
    let mut top = Dictionary::new();
    top.insert("root".into(), Value::Uid(Uid::new(1)));
    let mut root = Dictionary::new();
    root.insert("$archiver".into(), "NSKeyedArchiver".into());
    root.insert(
        "$objects".into(),
        Value::Array(vec![
            "$null".into(),
            file.into(),
            relative_path.into(),
            class.into(),
        ]),
    );
    root.insert("$top".into(), top.into());
    root.insert("$version".into(), 100_000u64.into());
    let mut buf = Vec::new();
    Value::Dictionary(root).to_writer_binary(&mut buf)?;
    Ok(buf)
}

fn write_plists(out: &Path, date: SystemTime) -> Result<()> {
    let date = Value::Date(date.into());
    let mut lockdown = Dictionary::new();
    lockdown.insert("BuildVersion".into(), BUILD_VERSION.into());
    lockdown.insert("DeviceName".into(), DEVICE_NAME.into());
    lockdown.insert("ProductType".into(), PRODUCT_TYPE.into());
    lockdown.insert("ProductVersion".into(), PRODUCT_VERSION.into());
    lockdown.insert("SerialNumber".into(), "FIXTURESN01".into());
    lockdown.insert("UniqueDeviceID".into(), UDID.into());

    let mut manifest = Dictionary::new();
    manifest.insert("Applications".into(), Dictionary::new().into());
    manifest.insert("Date".into(), date.clone());
    manifest.insert("IsEncrypted".into(), false.into());
    manifest.insert("Lockdown".into(), lockdown.into());
    manifest.insert("SystemDomainsVersion".into(), "24.0".into());
    manifest.insert("Version".into(), "10.0".into());
    manifest.insert("WasPasscodeSet".into(), false.into());
    Value::Dictionary(manifest).to_file_binary(out.join("Manifest.plist"))?;

    let mut status = Dictionary::new();
    status.insert("BackupState".into(), "new".into());
    status.insert("Date".into(), date.clone());
    status.insert("IsFullBackup".into(), false.into());
    status.insert("SnapshotState".into(), "finished".into());
    status.insert("UUID".into(), "F1C70000-0000-4000-8000-0000000000B1".into());
    status.insert("Version".into(), "3.3".into());
    Value::Dictionary(status).to_file_binary(out.join("Status.plist"))?;

    let mut info = Dictionary::new();
    info.insert("Build Version".into(), BUILD_VERSION.into());
    info.insert("Device Name".into(), DEVICE_NAME.into());
    info.insert("Display Name".into(), DEVICE_NAME.into());
    info.insert("GUID".into(), "F1C70000000000000000000000000001".into());
    info.insert("Last Backup Date".into(), date);
    info.insert("Product Name".into(), "iPhone 13 Pro".into());
    info.insert("Product Type".into(), PRODUCT_TYPE.into());
    info.insert("Product Version".into(), PRODUCT_VERSION.into());
    info.insert("Serial Number".into(), "FIXTURESN01".into());
    info.insert("Target Identifier".into(), UDID.into());
    info.insert("Target Type".into(), "Device".into());
    info.insert("Unique Identifier".into(), UDID.into());
    Value::Dictionary(info).to_file_xml(out.join("Info.plist"))?;
    Ok(())
}

fn expected(s: &Scenario) -> Expected {
    let unique: BTreeSet<String> = s.handles.iter().map(|h| normalize_address(h.id)).collect();
    let named: BTreeSet<String> = s
        .contacts
        .iter()
        .flat_map(|c| c.phones.iter().chain(&c.emails))
        .map(|v| normalize_address(v))
        .collect();
    let mut identifiers: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &s.chats {
        *identifiers.entry(c.chat_identifier.as_str()).or_default() += 1;
    }
    let present: Vec<_> = s
        .attachments
        .iter()
        .filter(|a| a.filename.is_some() && a.content.is_some())
        .collect();
    Expected {
        generator_version: env!("CARGO_PKG_VERSION"),
        messages: s.messages.len(),
        messages_in_chats: s.messages.iter().filter(|m| m.chat.is_some()).count(),
        messages_without_chat: s
            .messages
            .iter()
            .filter(|m| m.chat.is_none() && m.recoverable_in.is_none())
            .count(),
        recoverable_deleted: s
            .messages
            .iter()
            .filter(|m| m.recoverable_in.is_some())
            .count(),
        reactions: s
            .messages
            .iter()
            .filter(|m| m.associated_message_type != 0)
            .count(),
        replies: s
            .messages
            .iter()
            .filter(|m| m.thread_originator_guid.is_some())
            .count(),
        edited: s
            .messages
            .iter()
            .filter(|m| m.date_edited != 0 && m.date_retracted == 0)
            .count(),
        unsent: s.messages.iter().filter(|m| m.date_retracted != 0).count(),
        group_events: s.messages.iter().filter(|m| m.item_type != 0).count(),
        text_only_in_attributed_body: s.messages.iter().filter(|m| m.text_only_in_body).count(),
        chats: s.chats.len(),
        duplicated_chats: identifiers
            .values()
            .filter(|&&n| n > 1)
            .map(|n| n - 1)
            .sum(),
        handles: s.handles.len(),
        duplicate_handles: s.handles.len() - unique.len(),
        unique_addresses: unique.len(),
        addressbook_people: s.contacts.len(),
        addresses_with_name: unique.intersection(&named).count(),
        attachments: s.attachments.len(),
        attachments_present: present.len(),
        attachments_missing_file: s
            .attachments
            .iter()
            .filter(|a| a.filename.is_some() && a.content.is_none())
            .count(),
        attachments_without_path: s
            .attachments
            .iter()
            .filter(|a| a.filename.is_none())
            .count(),
        attachment_bytes_present: present
            .iter()
            .map(|a| a.content.as_ref().map_or(0, |c| c.len() as u64))
            .sum(),
        messages_per_chat: scenario::per_chat_counts(s),
    }
}
