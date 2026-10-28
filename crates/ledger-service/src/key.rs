//! Loading the service key: the signer of record. It is never logged, and
//! no error message contains it.

use std::path::{Path, PathBuf};

use nostr::prelude::Keys;
use thiserror::Error;

/// The systemd credential name (`LoadCredential=service-key:…`).
pub const CREDENTIAL: &str = "service-key";

#[derive(Debug, Error)]
pub enum KeyError {
    #[error(
        "No service key: pass --key-file, or run under systemd with the {CREDENTIAL} credential."
    )]
    Missing,
    #[error("The service key file {path} can't be read: {why}")]
    Unreadable { path: PathBuf, why: String },
    #[error("The service key file {path} has permissions {mode:o}; it must be 600 (owner read and write only). Run: chmod 600 {path}")]
    Permissions { path: PathBuf, mode: u32 },
    #[error(
        "The service key file {path} doesn't hold a key (a 64-character hex secret or an nsec)."
    )]
    Invalid { path: PathBuf },
}

/// Loads the key from the systemd credential if one is present, otherwise
/// from `file`, whose permissions must be exactly 0600.
pub fn load(file: Option<&Path>) -> Result<Keys, KeyError> {
    if let Some(dir) = std::env::var_os("CREDENTIALS_DIRECTORY") {
        let path = Path::new(&dir).join(CREDENTIAL);
        if path.exists() {
            // systemd makes credentials readable by the service alone.
            return read(&path);
        }
    }
    let path = file.ok_or(KeyError::Missing)?;
    check_permissions(path)?;
    read(path)
}

#[cfg(unix)]
fn check_permissions(path: &Path) -> Result<(), KeyError> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|e| KeyError::Unreadable {
        path: path.to_path_buf(),
        why: e.to_string(),
    })?;
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(KeyError::Permissions {
            path: path.to_path_buf(),
            mode,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_permissions(_: &Path) -> Result<(), KeyError> {
    Ok(())
}

fn read(path: &Path) -> Result<Keys, KeyError> {
    let text = std::fs::read_to_string(path).map_err(|e| KeyError::Unreadable {
        path: path.to_path_buf(),
        why: e.to_string(),
    })?;
    Keys::parse(text.trim()).map_err(|_| KeyError::Invalid {
        path: path.to_path_buf(),
    })
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn the_key_file_must_be_0600() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("zb-key-{}", crate::clock::uuid()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("service.key");
        std::fs::write(&path, format!("{}\n", "01".repeat(32)))?;

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
        assert!(matches!(
            load(Some(&path)),
            Err(KeyError::Permissions { mode: 0o644, .. })
        ));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640))?;
        assert!(matches!(
            load(Some(&path)),
            Err(KeyError::Permissions { .. })
        ));

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let keys = load(Some(&path))?;
        assert_eq!(keys.secret_key().to_secret_hex(), "01".repeat(32));
        // No error message ever contains the key.
        std::fs::write(&path, "not a key 0101")?;
        let message = load(Some(&path))
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(message.contains("doesn't hold a key") && !message.contains("0101"));
        assert!(matches!(load(None), Err(KeyError::Missing)));
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
