//! Read-only access to a local iPhone backup (Apple Devices / iTunes).
//!
//! Nothing here ever writes into the backup folder: databases are opened
//! with `mode=ro&immutable=1`, so SQLite creates no journal, WAL or shared
//! memory files beside them, and attachment files are only checked for
//! existence.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use plist::Value;
use rusqlite::{Connection, OpenFlags};
use sha1::{Digest, Sha1};

/// Domain and path of the Messages database inside a backup.
pub const SMS_DB: (&str, &str) = ("HomeDomain", "Library/SMS/sms.db");

/// Facts about a backup, read from its own plist files.
#[derive(Debug, Clone)]
pub struct BackupInfo {
    pub device_id: String,
    pub device_name: Option<String>,
    pub product_type: Option<String>,
    pub ios_version: Option<String>,
    pub build_version: Option<String>,
    /// Unix seconds.
    pub backup_date: i64,
    pub is_encrypted: bool,
}

#[derive(Debug)]
pub struct Backup {
    pub root: PathBuf,
    pub info: BackupInfo,
}

impl Backup {
    /// Reads a backup folder's metadata. Refuses encrypted backups, which
    /// arrive with the Import UI (Milestone 7).
    pub fn open(root: &Path) -> Result<Self> {
        anyhow::ensure!(
            root.is_dir(),
            "backup folder {} does not exist",
            root.display()
        );
        anyhow::ensure!(
            root.join("Manifest.db").is_file(),
            "{} has no Manifest.db; pass the folder that contains it",
            root.display()
        );
        let manifest = read_plist(&root.join("Manifest.plist"))?;
        let info_plist = read_plist(&root.join("Info.plist")).ok();
        let lockdown = manifest.get("Lockdown").and_then(Value::as_dictionary);

        let text = |key_info: &str, key_lockdown: &str| -> Option<String> {
            info_plist
                .as_ref()
                .and_then(|d| d.get(key_info))
                .or_else(|| lockdown.and_then(|l| l.get(key_lockdown)))
                .and_then(Value::as_string)
                .map(str::to_owned)
        };
        let device_id = text("Target Identifier", "UniqueDeviceID")
            .or_else(|| text("Unique Identifier", "UniqueDeviceID"))
            .context("backup does not name its device (no Target Identifier / UniqueDeviceID)")?;
        let date = info_plist
            .as_ref()
            .and_then(|d| d.get("Last Backup Date"))
            .or_else(|| manifest.get("Date"))
            .and_then(Value::as_date)
            .context("backup has no date (Info.plist Last Backup Date / Manifest.plist Date)")?;
        let backup_date = std::time::SystemTime::from(date)
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let is_encrypted = manifest
            .get("IsEncrypted")
            .and_then(Value::as_boolean)
            .unwrap_or(false);

        let info = BackupInfo {
            device_id,
            device_name: text("Device Name", "DeviceName"),
            product_type: text("Product Type", "ProductType"),
            ios_version: text("Product Version", "ProductVersion"),
            build_version: text("Build Version", "BuildVersion"),
            backup_date,
            is_encrypted,
        };
        if info.is_encrypted {
            bail!(
                "this backup is encrypted; encrypted backups are supported from Milestone 7 (Import UI)"
            );
        }
        Ok(Self {
            root: root.to_path_buf(),
            info,
        })
    }

    /// Where a backed-up file is stored: `<root>/<id[..2]>/<id>`.
    pub fn file_path(&self, domain: &str, relative_path: &str) -> PathBuf {
        let id = file_id(domain, relative_path);
        self.root.join(&id[..2]).join(id)
    }

    /// Opens the Messages database read-only.
    pub fn open_sms_db(&self) -> Result<(Connection, PathBuf)> {
        let path = self.file_path(SMS_DB.0, SMS_DB.1);
        anyhow::ensure!(
            path.is_file(),
            "backup has no Messages database ({})",
            path.display()
        );
        Ok((open_read_only(&path)?, path))
    }

    /// Backup file ID and whether the file exists, for an attachment path as
    /// sms.db stores it (`~/Library/SMS/Attachments/...`).
    pub fn attachment_file(&self, filename: &str) -> (String, bool) {
        let relative = attachment_relative_path(filename);
        let id = file_id("MediaDomain", relative);
        let exists = self.root.join(&id[..2]).join(&id).is_file();
        (id, exists)
    }
}

/// Strips the home-directory prefix from an attachment path.
pub fn attachment_relative_path(filename: &str) -> &str {
    filename
        .strip_prefix("~/")
        .or_else(|| filename.strip_prefix("/var/mobile/"))
        .or_else(|| filename.strip_prefix("/private/var/mobile/"))
        .unwrap_or(filename)
}

/// `fileID` of a file in a backup: SHA-1 of `"<domain>-<relativePath>"`.
pub fn file_id(domain: &str, relative_path: &str) -> String {
    Sha1::digest(format!("{domain}-{relative_path}").as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Opens a SQLite file so that nothing can be written next to it.
pub fn open_read_only(path: &Path) -> Result<Connection> {
    let mut uri = String::from("file:");
    for c in path.to_string_lossy().chars() {
        match c {
            '%' => uri.push_str("%25"),
            '?' => uri.push_str("%3f"),
            '#' => uri.push_str("%23"),
            _ => uri.push(c),
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    let conn = Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("opening {} read-only", path.display()))?;
    Ok(conn)
}

fn read_plist(path: &Path) -> Result<plist::Dictionary> {
    let value = Value::from_file(path).with_context(|| format!("reading {}", path.display()))?;
    value
        .into_dictionary()
        .with_context(|| format!("{} is not a dictionary", path.display()))
}
