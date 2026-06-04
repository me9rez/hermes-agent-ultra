//! WhatsApp session paths and pairing state (wa-rs SQLite backend).

use std::path::{Path, PathBuf};

const PAIRED_MARKER: &str = ".paired";
const LEGACY_CREDS: &str = "creds.json";

pub fn session_db_path(session_path: &Path) -> PathBuf {
    session_path.join("whatsapp.db")
}

pub fn paired_marker_path(session_path: &Path) -> PathBuf {
    session_path.join(PAIRED_MARKER)
}

/// Legacy Baileys credential file (Node bridge). Kept for migration hints only.
pub fn legacy_creds_path(session_path: &Path) -> PathBuf {
    session_path.join(LEGACY_CREDS)
}

pub fn is_paired(session_path: &Path) -> bool {
    paired_marker_path(session_path).exists()
}

pub fn mark_paired(session_path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(session_path)?;
    std::fs::write(paired_marker_path(session_path), "1")
}

pub fn has_legacy_baileys_session(session_path: &Path) -> bool {
    legacy_creds_path(session_path).exists()
}

pub fn ensure_session_dir(session_path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(session_path)
}

/// Remove wa-rs SQLite session and pairing markers so QR pairing can start fresh.
pub fn clear_pairing_session(session_path: &Path) -> std::io::Result<()> {
    const MAX_ATTEMPTS: u32 = 5;
    for attempt in 0..MAX_ATTEMPTS {
        match clear_pairing_session_once(session_path) {
            Ok(()) => return Ok(()),
            Err(e) if attempt + 1 < MAX_ATTEMPTS && is_file_locked_error(&e) => {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn is_file_locked_error(err: &std::io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(32) | Some(13) // Windows sharing violation / access denied
    )
}

fn clear_pairing_session_once(session_path: &Path) -> std::io::Result<()> {
    if !session_path.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(session_path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn paired_marker_roundtrip() {
        let dir = TempDir::new().unwrap();
        let session = dir.path().join("session");
        assert!(!is_paired(&session));
        mark_paired(&session).unwrap();
        assert!(is_paired(&session));
        assert!(session_db_path(&session).ends_with("whatsapp.db"));
    }

    #[test]
    fn clear_pairing_session_removes_db_and_marker() {
        let dir = TempDir::new().unwrap();
        let session = dir.path().join("session");
        std::fs::create_dir_all(&session).unwrap();
        mark_paired(&session).unwrap();
        std::fs::write(session_db_path(&session), b"db").unwrap();
        clear_pairing_session(&session).unwrap();
        assert!(!is_paired(&session));
        assert!(!session_db_path(&session).exists());
    }
}
