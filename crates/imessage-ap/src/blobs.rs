//! Content-addressed file store: `/data/blobs/ab/cd/<sha256>` (decision 0005).
//!
//! A file is streamed from the backup into a temporary file inside the store
//! while it is hashed, flushed to disk, then renamed into place. A rename is
//! atomic, so the store never holds a partly written blob under its final
//! name, whatever interrupts the copy. Backup files are opened with
//! [`File::open`], which is read-only.

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Original attachment files, by content hash.
pub const BLOBS_DIR: &str = "blobs";
/// Thumbnails, previews and conversions, by content hash of their blob.
pub const DERIVED_DIR: &str = "derived";
/// Work-in-progress files inside `blobs/` and `derived/`. Anything here is
/// left over from an interrupted run and is deleted at the start of the next.
pub const TMP_DIR: &str = ".tmp";
/// Lock file that keeps two file runs from working on one data directory.
pub const LOCK_FILE: &str = ".imessage-ap.lock";

const COPY_BUFFER: usize = 1 << 20;

/// Where the blob with `sha256` is stored.
pub fn blob_path(data_dir: &Path, sha256: &str) -> PathBuf {
    data_dir
        .join(BLOBS_DIR)
        .join(&sha256[..2])
        .join(&sha256[2..4])
        .join(sha256)
}

/// Path, relative to the data directory, of a derivative of `sha256`.
pub fn derived_relative_path(kind: &str, sha256: &str, ext: &str) -> String {
    format!(
        "{DERIVED_DIR}/{kind}/{}/{}/{sha256}.{ext}",
        &sha256[..2],
        &sha256[2..4]
    )
}

/// Temporary folder inside `blobs/` or `derived/`, on the same file system
/// as the final paths so a rename can move files into place.
pub fn tmp_dir(data_dir: &Path, store: &str) -> PathBuf {
    data_dir.join(store).join(TMP_DIR)
}

/// A unique path in `dir` for a new temporary file.
pub fn tmp_path(dir: &Path, ext: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("{}-{n}{ext}", std::process::id()))
}

/// Deletes what an interrupted run left in the temporary folders, and makes
/// sure they exist. Returns how many files were removed.
pub fn reset_tmp(data_dir: &Path) -> Result<usize> {
    let mut removed = 0;
    for store in [BLOBS_DIR, DERIVED_DIR] {
        let dir = tmp_dir(data_dir, store);
        if dir.is_dir() {
            for entry in fs::read_dir(&dir)? {
                let path = entry?.path();
                if path.is_file() {
                    fs::remove_file(&path)
                        .with_context(|| format!("removing {}", path.display()))?;
                    removed += 1;
                }
            }
        }
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    Ok(removed)
}

/// A file copied into the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub sha256: String,
    pub size: u64,
    /// A file with this hash was already in the store, so nothing was added.
    pub already_stored: bool,
}

/// Copies `src` into the store under its SHA-256, unless that blob is
/// already there. `src` is only ever opened for reading.
pub fn store(src: &Path, data_dir: &Path) -> io::Result<Stored> {
    let tmp = tmp_path(&tmp_dir(data_dir, BLOBS_DIR), "");
    let result = copy_and_hash(src, &tmp).and_then(|(sha256, size)| {
        let dest = blob_path(data_dir, &sha256);
        let already_stored = fs::metadata(&dest).is_ok_and(|m| m.is_file() && m.len() == size);
        if already_stored {
            fs::remove_file(&tmp)?;
        } else {
            let parent = dest.parent().expect("blob paths have a parent");
            fs::create_dir_all(parent)?;
            fs::rename(&tmp, &dest)?;
            // Make the new directory entry durable. Some file systems (FUSE
            // shares among them) cannot sync a directory; the rename stands.
            if let Ok(dir) = File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(Stored {
            sha256,
            size,
            already_stored,
        })
    });
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Streams `src` into a new file `tmp`, flushes it to disk, and returns the
/// hex SHA-256 and length of what was read.
fn copy_and_hash(src: &Path, tmp: &Path) -> io::Result<(String, u64)> {
    let mut input = File::open(src)?;
    let mut output = File::options().write(true).create_new(true).open(tmp)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; COPY_BUFFER];
    let mut size = 0u64;
    loop {
        let n = match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&buf[..n]);
        output.write_all(&buf[..n])?;
        size += n as u64;
    }
    output.sync_all()?;
    Ok((hex(&hasher.finalize()), size))
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Holds the data directory's lock until dropped.
#[derive(Debug)]
pub struct Lock {
    _file: Option<File>,
}

/// Takes the data directory's exclusive lock, failing if another run holds
/// it. Where the file system cannot lock files, this warns and goes on.
pub fn lock(data_dir: &Path) -> Result<Lock> {
    fs::create_dir_all(data_dir)
        .with_context(|| format!("creating data directory {}", data_dir.display()))?;
    let path = data_dir.join(LOCK_FILE);
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Lock { _file: Some(file) }),
        Err(fs::TryLockError::WouldBlock) => anyhow::bail!(
            "another imessage-ap run is using {}; wait for it to finish",
            data_dir.display()
        ),
        Err(fs::TryLockError::Error(e)) => {
            tracing::warn!(error = %e, "this file system cannot lock files; make sure only one run uses the data directory");
            Ok(Lock { _file: None })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_once_by_content() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        reset_tmp(&data).unwrap();
        let src = dir.path().join("a");
        fs::write(&src, b"abc").unwrap();
        let first = store(&src, &data).unwrap();
        // SHA-256 of "abc".
        let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!((first.sha256.as_str(), first.size), (sha, 3));
        assert!(!first.already_stored);
        assert_eq!(fs::read(blob_path(&data, sha)).unwrap(), b"abc");
        assert!(blob_path(&data, sha).ends_with(format!("ba/78/{sha}")));
        assert!(store(&src, &data).unwrap().already_stored);
        assert_eq!(
            fs::read_dir(tmp_dir(&data, BLOBS_DIR)).unwrap().count(),
            0,
            "temporary files left behind"
        );
    }

    #[test]
    fn a_failed_read_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        reset_tmp(&data).unwrap();
        assert!(store(&dir.path().join("missing"), &data).is_err());
        assert_eq!(fs::read_dir(tmp_dir(&data, BLOBS_DIR)).unwrap().count(), 0);
    }

    #[test]
    fn leftover_temporary_files_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        reset_tmp(dir.path()).unwrap();
        fs::write(tmp_dir(dir.path(), BLOBS_DIR).join("partial"), b"x").unwrap();
        fs::write(tmp_dir(dir.path(), DERIVED_DIR).join("partial.webp"), b"x").unwrap();
        assert_eq!(reset_tmp(dir.path()).unwrap(), 2);
    }

    #[test]
    fn a_second_lock_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let held = lock(dir.path()).unwrap();
        if held._file.is_some() {
            assert!(lock(dir.path()).is_err());
        }
        drop(held);
        lock(dir.path()).unwrap();
    }
}
