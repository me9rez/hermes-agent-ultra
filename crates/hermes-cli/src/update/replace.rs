use hermes_core::errors::AgentError;
use std::path::{Path, PathBuf};

/// 获取当前 binary 路径
fn current_exe_path() -> Result<PathBuf, AgentError> {
    std::env::current_exe()
        .map_err(|e| AgentError::Io(format!("Failed to determine current executable path: {e}")))
}

fn backup_path(exe_path: &Path) -> PathBuf {
    let mut p = exe_path.to_path_buf().into_os_string();
    p.push(".bak");
    PathBuf::from(p)
}

fn old_path(exe_path: &Path) -> PathBuf {
    let mut p = exe_path.to_path_buf().into_os_string();
    p.push(".old");
    PathBuf::from(p)
}

/// 用新 binary 替换当前正在运行的可执行文件
pub fn self_replace(new_binary: &Path) -> Result<(), AgentError> {
    let exe_path = current_exe_path()?;
    let bak = backup_path(&exe_path);

    // Create backup for rollback
    std::fs::copy(&exe_path, &bak)
        .map_err(|e| AgentError::Io(format!("Failed to create backup: {e}")))?;
    tracing::debug!("Backup created at {}", bak.display());

    #[cfg(unix)]
    {
        unix_replace(new_binary, &exe_path, &bak)?;
    }

    #[cfg(windows)]
    {
        windows_replace(new_binary, &exe_path, &bak)?;
    }

    Ok(())
}

#[cfg(unix)]
fn unix_replace(new_binary: &Path, exe_path: &Path, bak: &Path) -> Result<(), AgentError> {
    use std::os::unix::fs::PermissionsExt;

    // Set executable permission on new binary
    let perms = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(new_binary, perms)
        .map_err(|e| AgentError::Io(format!("Failed to set permissions: {e}")))?;

    // Atomic rename (same filesystem required - temp_dir might be different FS)
    // So we copy first, then rename
    let staging = exe_path.with_extension("new");
    std::fs::copy(new_binary, &staging)
        .map_err(|e| AgentError::Io(format!("Failed to stage new binary: {e}")))?;

    let perms = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(&staging, perms)
        .map_err(|e| AgentError::Io(format!("Failed to set permissions on staged binary: {e}")))?;

    // Atomic rename
    if let Err(e) = std::fs::rename(&staging, exe_path) {
        // Rollback: restore from backup
        tracing::error!("Failed to replace binary: {e}, rolling back...");
        let _ = std::fs::rename(bak, exe_path);
        let _ = std::fs::remove_file(&staging);
        return Err(AgentError::Io(format!("Failed to replace binary: {e}")));
    }

    tracing::info!("Binary replaced successfully");
    Ok(())
}

#[cfg(windows)]
fn windows_replace(new_binary: &Path, exe_path: &Path, _bak: &Path) -> Result<(), AgentError> {
    let old = old_path(exe_path);

    // Remove previous .old if exists
    let _ = std::fs::remove_file(&old);

    // Rename running exe to .old (Windows allows renaming a running exe)
    if let Err(e) = std::fs::rename(exe_path, &old) {
        tracing::error!("Failed to rename current exe: {e}");
        return Err(AgentError::Io(format!(
            "Failed to rename current executable: {e}"
        )));
    }

    // Copy new binary to original path
    if let Err(e) = std::fs::copy(new_binary, exe_path) {
        // Rollback: rename .old back
        tracing::error!("Failed to install new binary: {e}, rolling back...");
        let _ = std::fs::rename(&old, exe_path);
        return Err(AgentError::Io(format!("Failed to install new binary: {e}")));
    }

    tracing::info!("Binary replaced successfully (restart required on Windows)");
    Ok(())
}

/// 回滚到上一个版本
pub fn rollback() -> Result<(), AgentError> {
    let exe_path = current_exe_path()?;
    let bak = backup_path(&exe_path);

    if !bak.exists() {
        return Err(AgentError::Io(
            "No backup found. Cannot rollback.".to_string(),
        ));
    }

    #[cfg(unix)]
    {
        std::fs::rename(&bak, &exe_path)
            .map_err(|e| AgentError::Io(format!("Rollback failed: {e}")))?;
    }

    #[cfg(windows)]
    {
        let old = old_path(&exe_path);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe_path, &old)
            .map_err(|e| AgentError::Io(format!("Rollback rename failed: {e}")))?;
        std::fs::rename(&bak, &exe_path)
            .map_err(|e| AgentError::Io(format!("Rollback restore failed: {e}")))?;
    }

    println!("Successfully rolled back to previous version.");
    Ok(())
}

/// 清理上次更新遗留的 .old 文件（应在启动时调用）
pub fn cleanup_old() {
    if let Ok(exe_path) = current_exe_path() {
        let old = old_path(&exe_path);
        if old.exists() {
            match std::fs::remove_file(&old) {
                Ok(()) => tracing::debug!("Cleaned up old binary: {}", old.display()),
                Err(e) => tracing::debug!("Failed to clean up old binary: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_path() {
        let exe = Path::new("/usr/local/bin/hermes");
        let bak = backup_path(exe);
        assert_eq!(bak, Path::new("/usr/local/bin/hermes.bak"));
    }

    #[test]
    fn test_backup_path_windows() {
        let exe = Path::new("C:\\Program Files\\hermes\\hermes.exe");
        let bak = backup_path(exe);
        assert_eq!(bak, Path::new("C:\\Program Files\\hermes\\hermes.exe.bak"));
    }

    #[test]
    fn test_old_path() {
        let exe = Path::new("/usr/local/bin/hermes");
        let old = old_path(exe);
        assert_eq!(old, Path::new("/usr/local/bin/hermes.old"));
    }

    #[test]
    fn test_old_path_windows() {
        let exe = Path::new("C:\\hermes.exe");
        let old = old_path(exe);
        assert_eq!(old, Path::new("C:\\hermes.exe.old"));
    }

    #[test]
    fn test_cleanup_old_no_panic_when_missing() {
        // cleanup_old should not panic even if no .old file exists
        cleanup_old();
    }
}
