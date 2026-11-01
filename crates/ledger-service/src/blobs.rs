//! The blob store: encrypted receipts, as files on the service's disk.
//!
//! The browser encrypts each receipt with a key of its own before uploading
//! it, and that key travels only inside the books. The service stores and
//! serves ciphertext, addressed by its SHA-256, and never sees a key here
//! (it does see one when it checks the proposal that refers to the blob;
//! see `docs/service.md`).
//!
//! Each business has a directory of its own under the store's root, and
//! each blob is the file named by its hash in it. A file is written to a
//! temporary name, synced, and renamed into place, so a reader never sees
//! half of one. Postgres holds the index (`blobs`): which blobs a business
//! has, their sizes, and whether an accepted record refers to them yet.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use ledger_core::MAX_STORED_BYTES;
use protocol::BusinessId;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// The largest blob, in bytes: the largest receipt plus AES-256-GCM's tag.
pub const MAX_BLOB_BYTES: u64 = MAX_STORED_BYTES;
/// Each business's storage, unless `--blob-quota` says otherwise: 5 GB.
pub const DEFAULT_QUOTA: u64 = 5 * 1024 * 1024 * 1024;
/// A blob no accepted record refers to is deleted after this long.
pub const PENDING_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Error)]
pub enum BlobError {
    #[error("Receipt storage isn't set up on this service.")]
    Unavailable,
    #[error("No such business.")]
    NotFound,
    #[error("The upload's SHA-256 doesn't match the address it was sent to.")]
    Mismatch,
    #[error("The receipt is larger than 10 MB.")]
    TooLarge,
    #[error("The upload is empty.")]
    Empty,
    #[error("This business's receipt storage is full: its limit is {quota} bytes.")]
    Quota { quota: u64 },
    #[error("The service's disk is full, so the receipt wasn't stored. Try again later.")]
    DiskFull,
    #[error("storage error: {0}")]
    Io(io::Error),
    #[error(transparent)]
    Ledger(#[from] crate::ledger::LedgerError),
}

impl From<io::Error> for BlobError {
    fn from(e: io::Error) -> BlobError {
        match e.kind() {
            io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => BlobError::DiskFull,
            _ => BlobError::Io(e),
        }
    }
}

/// A blob's address: SHA-256 of its bytes, 64 lowercase hex characters.
pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// SHA-256 of `bytes`, lowercase hex.
pub fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The directory blobs are kept in, and each business's quota.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
    quota: u64,
}

impl BlobStore {
    /// The store at `root`, which is created (mode 0700) if it doesn't
    /// exist. Temporary files a crash left behind are removed: no upload is
    /// in flight yet.
    pub fn open(root: impl Into<PathBuf>, quota: u64) -> io::Result<BlobStore> {
        let root = root.into();
        create_dir(&root)?;
        for business in fs::read_dir(&root)? {
            let business = business?;
            if !business.file_type()?.is_dir() {
                continue;
            }
            for entry in fs::read_dir(business.path())? {
                let entry = entry?;
                if is_temporary(&entry.file_name().to_string_lossy()) {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(BlobStore { root, quota })
    }

    pub fn quota(&self) -> u64 {
        self.quota
    }

    fn dir(&self, business: &BusinessId) -> PathBuf {
        self.root.join(business.to_string())
    }

    /// Where the blob `hash` of `business` is kept. `hash` must pass
    /// [`valid_hash`], so it can't name anything outside the directory.
    pub fn path(&self, business: &BusinessId, hash: &str) -> PathBuf {
        self.dir(business).join(hash)
    }

    /// Writes `bytes` to a temporary file beside where the blob `hash` goes,
    /// with mode 0600, and syncs it. Nothing is left behind on failure.
    pub fn write_temporary(
        &self,
        business: &BusinessId,
        hash: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, BlobError> {
        let dir = self.dir(business);
        create_dir(&dir)?;
        let temporary = dir.join(format!(
            ".{hash}.{}.tmp",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let written = write_new(&temporary, bytes);
        if written.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        written?;
        Ok(temporary)
    }

    /// Moves a file from [`BlobStore::write_temporary`] into place as the
    /// blob `hash`, atomically.
    pub fn install(
        &self,
        business: &BusinessId,
        hash: &str,
        temporary: &Path,
    ) -> Result<(), BlobError> {
        let dir = self.dir(business);
        fs::rename(temporary, dir.join(hash))?;
        // The rename is durable once the directory is synced.
        File::open(&dir)?.sync_all()?;
        Ok(())
    }

    pub fn read(&self, business: &BusinessId, hash: &str) -> io::Result<Vec<u8>> {
        fs::read(self.path(business, hash))
    }

    /// Deletes the blob `hash` of `business`; one already gone is fine.
    pub fn remove(&self, business: &BusinessId, hash: &str) -> io::Result<()> {
        match fs::remove_file(self.path(business, hash)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

fn is_temporary(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(".tmp")
}

fn create_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn store() -> (BlobStore, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "zb-blobs-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        (BlobStore::open(&root, DEFAULT_QUOTA).unwrap(), root)
    }

    #[test]
    fn a_blob_is_written_whole_with_owner_only_permissions() {
        let (store, root) = store();
        let business = BusinessId::generate();
        let bytes = b"invented ciphertext";
        let h = hash(bytes);
        let temporary = store.write_temporary(&business, &h, bytes).unwrap();
        assert!(!store.path(&business, &h).exists());
        store.install(&business, &h, &temporary).unwrap();
        assert!(!temporary.exists());
        assert_eq!(store.read(&business, &h).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(store.path(&business, &h))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        store.remove(&business, &h).unwrap();
        store.remove(&business, &h).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn temporary_files_a_crash_left_are_removed_at_start() {
        let (store, root) = store();
        let business = BusinessId::generate();
        let h = hash(b"x");
        let temporary = store.write_temporary(&business, &h, b"x").unwrap();
        BlobStore::open(&root, DEFAULT_QUOTA).unwrap();
        assert!(!temporary.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_full_disk_is_its_own_error() {
        let full = io::Error::from(io::ErrorKind::StorageFull);
        assert!(matches!(BlobError::from(full), BlobError::DiskFull));
    }
}
