//! Where each store lives (decision 0009).
//!
//! Every store is its own setting and defaults to a folder under the data
//! directory, so a plain `--data /data` keeps working as before:
//!
//! | Store | Default | Setting |
//! | --- | --- | --- |
//! | archive.db, user.db | `<data>` | `--db-dir`, `IMESSAGE_AP_DB_DIR` |
//! | original files | `<data>/blobs` | `--blobs-dir`, `IMESSAGE_AP_BLOBS_DIR` |
//! | thumbnails, previews, conversions | `<data>/derived` | `--derived-dir`, `IMESSAGE_AP_DERIVED_DIR` |
//! | exports (Milestone 9) | `<data>/exports` | `--exports-dir`, `IMESSAGE_AP_EXPORTS_DIR` |
//!
//! `derivative.path` in archive.db stays relative to the data directory
//! (`derived/thumb/ab/cd/<sha256>.webp`), whatever `--derived-dir` says, so
//! moving `derived/` needs no change to the archive. [`Dirs::derived_file`]
//! resolves those paths.

use std::path::{Path, PathBuf};

use anyhow::Result;

/// Folder names under the data directory, used for the defaults and for the
/// paths recorded in archive.db.
pub const BLOBS: &str = "blobs";
pub const DERIVED: &str = "derived";
pub const EXPORTS: &str = "exports";

/// File names of the two databases inside the database directory.
pub const ARCHIVE_DB: &str = "archive.db";
pub const USER_DB: &str = "user.db";

/// Every store's location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// The data directory: the base for every default below.
    pub data: PathBuf,
    /// archive.db and user.db (and the lock that keeps two runs apart).
    pub db: PathBuf,
    /// Original attachment files: `<blobs>/ab/cd/<sha256>`.
    pub blobs: PathBuf,
    /// Thumbnails, previews and conversions: `<derived>/<kind>/ab/cd/…`.
    pub derived: PathBuf,
    /// JSON exports (Milestone 9).
    pub exports: PathBuf,
}

impl Dirs {
    /// Every store in its default place under `data`.
    pub fn under(data: &Path) -> Self {
        Self::new(data, None, None, None, None)
    }

    /// `data`, with any store moved elsewhere.
    pub fn new(
        data: &Path,
        db: Option<&Path>,
        blobs: Option<&Path>,
        derived: Option<&Path>,
        exports: Option<&Path>,
    ) -> Self {
        let or = |p: Option<&Path>, default: PathBuf| p.map_or(default, Path::to_path_buf);
        Self {
            data: data.to_path_buf(),
            db: or(db, data.to_path_buf()),
            blobs: or(blobs, data.join(BLOBS)),
            derived: or(derived, data.join(DERIVED)),
            exports: or(exports, data.join(EXPORTS)),
        }
    }

    pub fn archive_db(&self) -> PathBuf {
        self.db.join(ARCHIVE_DB)
    }

    pub fn user_db(&self) -> PathBuf {
        self.db.join(USER_DB)
    }

    /// The file a `derivative.path` value (`derived/<kind>/ab/cd/<file>`)
    /// names, under the derived directory wherever it is.
    pub fn derived_file(&self, recorded: &str) -> PathBuf {
        match recorded
            .strip_prefix(DERIVED)
            .and_then(|r| r.strip_prefix('/'))
        {
            Some(rest) => self.derived.join(rest),
            None => self.data.join(recorded),
        }
    }

    /// `text` (a tool's error output) with this machine's store paths
    /// replaced by the names they have under the data directory, so recorded
    /// errors read the same wherever the stores are.
    pub fn shorten(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for (dir, name) in [(&self.derived, DERIVED), (&self.blobs, BLOBS)] {
            out = out.replace(&format!("{}/", dir.display()), &format!("{name}/"));
        }
        out.replace(&format!("{}/", self.data.display()), "")
    }

    /// Refuses a layout with two archives: when the databases live outside
    /// the data directory but the data directory still holds one of them
    /// (left behind by a move, or a copy instead of a move). Using either
    /// would silently ignore the other.
    pub fn check(&self) -> Result<()> {
        if same_dir(&self.db, &self.data) {
            return Ok(());
        }
        for name in [ARCHIVE_DB, USER_DB] {
            let stray = self.data.join(name);
            if stray.exists() {
                anyhow::bail!(
                    "{name} exists in both the data directory ({}) and the database directory ({}); \
                     keep only the one in the database directory (move or delete {})",
                    self.data.display(),
                    self.db.display(),
                    stray.display()
                );
            }
        }
        Ok(())
    }
}

/// Whether two paths name the same folder, following symlinks when both
/// exist.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_under_data() {
        let d = Dirs::under(Path::new("/data"));
        assert_eq!(d.archive_db(), Path::new("/data/archive.db"));
        assert_eq!(d.user_db(), Path::new("/data/user.db"));
        assert_eq!(d.blobs, Path::new("/data/blobs"));
        assert_eq!(d.derived, Path::new("/data/derived"));
        assert_eq!(d.exports, Path::new("/data/exports"));
        assert_eq!(
            d.derived_file("derived/thumb/ab/cd/x.webp"),
            Path::new("/data/derived/thumb/ab/cd/x.webp")
        );
    }

    #[test]
    fn recorded_derived_paths_follow_the_derived_dir() {
        let d = Dirs::new(
            Path::new("/data"),
            Some(Path::new("/ssd/db")),
            None,
            Some(Path::new("/ssd/derived")),
            None,
        );
        assert_eq!(d.archive_db(), Path::new("/ssd/db/archive.db"));
        assert_eq!(d.blobs, Path::new("/data/blobs"));
        assert_eq!(
            d.derived_file("derived/video/ab/cd/x.mp4"),
            Path::new("/ssd/derived/video/ab/cd/x.mp4")
        );
    }

    #[test]
    fn two_archives_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let db = tmp.path().join("db");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&db).unwrap();
        let d = Dirs::new(&data, Some(&db), None, None, None);
        d.check().unwrap();
        std::fs::write(data.join(ARCHIVE_DB), b"").unwrap();
        let err = d.check().unwrap_err().to_string();
        assert!(err.contains("archive.db exists in both"), "{err}");
        // The same folder under two spellings is one archive, not two.
        Dirs::new(&data, Some(&data.join(".")), None, None, None)
            .check()
            .unwrap();
    }
}
