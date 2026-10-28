//! Loading the service key: the signer of record. It is never logged, and
//! no error message contains it.

use std::path::{Path, PathBuf};

use nostr::prelude::Keys;
use thiserror::Error;

/// The systemd credential name (`LoadCredential=service-key:…`). Further
/// keys, for businesses moving to a new service key, are credentials named
/// `service-key.<anything>`.
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

/// Loads every service key: from the systemd credentials if `service-key`
/// is present (it comes first, then `service-key.*` by name), otherwise from
/// `files`, in order, each with the 0600 check. The first is the primary
/// key.
pub fn load_all(files: &[PathBuf]) -> Result<Vec<Keys>, KeyError> {
    if let Some(dir) = std::env::var_os("CREDENTIALS_DIRECTORY") {
        let dir = Path::new(&dir);
        if dir.join(CREDENTIAL).exists() {
            let mut extra: Vec<PathBuf> = std::fs::read_dir(dir)
                .map_err(|e| KeyError::Unreadable {
                    path: dir.to_path_buf(),
                    why: e.to_string(),
                })?
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("service-key."))
                })
                .collect();
            extra.sort();
            return std::iter::once(dir.join(CREDENTIAL))
                .chain(extra)
                .map(|p| read(&p))
                .collect();
        }
    }
    if files.is_empty() {
        return Err(KeyError::Missing);
    }
    files
        .iter()
        .map(|path| {
            check_permissions(path)?;
            read(path)
        })
        .collect()
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

        // Several key files: each must be 0600, and order is kept.
        let second = dir.join("next.key");
        std::fs::write(&second, format!("{}\n", "02".repeat(32)))?;
        std::fs::write(&path, format!("{}\n", "01".repeat(32)))?;
        std::fs::set_permissions(&second, std::fs::Permissions::from_mode(0o644))?;
        assert!(matches!(
            load_all(&[path.clone(), second.clone()]),
            Err(KeyError::Permissions { mode: 0o644, .. })
        ));
        std::fs::set_permissions(&second, std::fs::Permissions::from_mode(0o600))?;
        let all = load_all(&[path.clone(), second.clone()])?;
        let hexes: Vec<String> = all.iter().map(|k| k.secret_key().to_secret_hex()).collect();
        assert_eq!(hexes, ["01".repeat(32), "02".repeat(32)]);
        assert!(matches!(load_all(&[]), Err(KeyError::Missing)));
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
