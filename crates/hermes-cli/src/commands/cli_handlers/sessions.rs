//! Sessions CLI handler.

use super::super::session;
pub async fn handle_cli_sessions(
    action: Option<String>,
    id: Option<String>,
    name: Option<String>,
) -> Result<(), hermes_core::AgentError> {
    let sessions_dir = hermes_config::hermes_home().join("sessions");

    match action.as_deref().unwrap_or("list") {
        "list" => {
            if !sessions_dir.exists() {
                println!("No sessions directory found.");
                return Ok(());
            }
            let mut entries: Vec<(String, u64, std::time::SystemTime, bool, bool, usize)> =
                Vec::new();
            if let Ok(rd) = std::fs::read_dir(&sessions_dir) {
                for entry in rd.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.extension().map(|e| e == "json").unwrap_or(false) {
                        let stem = path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        let meta = std::fs::metadata(&path);
                        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                        let modified = meta
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        let integrity = session::inspect_snapshot_integrity(&path);
                        let canonical = session::is_canonical_snapshot_name(&stem, &integrity);
                        entries.push((
                            stem,
                            size,
                            modified,
                            canonical,
                            integrity.valid,
                            integrity.message_count,
                        ));
                    }
                }
            }
            entries.sort_by(|a, b| {
                b.3.cmp(&a.3)
                    .then_with(|| b.5.cmp(&a.5))
                    .then_with(|| b.2.cmp(&a.2))
                    .then_with(|| a.0.cmp(&b.0))
            });
            if entries.is_empty() {
                println!("No saved sessions.");
            } else {
                let canonical_count = entries.iter().filter(|entry| entry.3).count();
                let artifact_count = entries.len().saturating_sub(canonical_count);
                println!(
                    "Saved sessions ({} total; {} canonical; {} artifacts):",
                    entries.len(),
                    canonical_count,
                    artifact_count
                );
                for (name, size, _, canonical, valid, messages) in &entries {
                    let kind = if *canonical {
                        "session"
                    } else if *valid {
                        "artifact"
                    } else {
                        "invalid"
                    };
                    println!("  • {} ({} bytes, {} msgs, {})", name, size, messages, kind);
                }
            }
        }
        "export" => {
            let session_id = id.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing session ID. Usage: hermes sessions export --id <id>".into(),
                )
            })?;
            let path = sessions_dir.join(format!("{}.json", session_id));
            if !path.exists() {
                println!("Session '{}' not found.", session_id);
                return Ok(());
            }
            let content = std::fs::read_to_string(&path)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            println!("{}", content);
        }
        "delete" => {
            let session_id = id.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing session ID. Usage: hermes sessions delete --id <id>".into(),
                )
            })?;
            let path = sessions_dir.join(format!("{}.json", session_id));
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
                println!("Session '{}' deleted.", session_id);
            } else {
                println!("Session '{}' not found.", session_id);
            }
        }
        "stats" => {
            if !sessions_dir.exists() {
                println!("No sessions directory.");
                return Ok(());
            }
            let mut total_files = 0u32;
            let mut total_size = 0u64;
            if let Ok(rd) = std::fs::read_dir(&sessions_dir) {
                for entry in rd.filter_map(|e| e.ok()) {
                    if entry
                        .path()
                        .extension()
                        .map(|e| e == "json")
                        .unwrap_or(false)
                    {
                        total_files += 1;
                        total_size += std::fs::metadata(entry.path())
                            .map(|m| m.len())
                            .unwrap_or(0);
                    }
                }
            }
            println!("Session statistics:");
            println!("  Total sessions: {}", total_files);
            println!("  Total size:     {} KB", total_size / 1024);
            println!("  Directory:      {}", sessions_dir.display());
        }
        "prune" => {
            let max_age_days: u64 = name
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(30);
            println!("Pruning sessions older than {} days...", max_age_days);
            if !sessions_dir.exists() {
                println!("No sessions directory.");
                return Ok(());
            }
            let cutoff = std::time::SystemTime::now()
                .checked_sub(std::time::Duration::from_secs(max_age_days * 86400))
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let mut pruned = 0u32;
            if let Ok(rd) = std::fs::read_dir(&sessions_dir) {
                for entry in rd.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if !path.extension().map(|e| e == "json").unwrap_or(false) {
                        continue;
                    }
                    if let Ok(meta) = std::fs::metadata(&path) {
                        let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        if modified < cutoff {
                            if std::fs::remove_file(&path).is_ok() {
                                let name = path.file_stem().unwrap_or_default().to_string_lossy();
                                println!("  Pruned: {}", name);
                                pruned += 1;
                            }
                        }
                    }
                }
            }
            println!("Pruned {} session(s).", pruned);
        }
        "rename" => {
            let session_id = id.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing session ID. Usage: hermes sessions rename --id <id> --name <new>"
                        .into(),
                )
            })?;
            let new_name = name.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing new name. Usage: hermes sessions rename --id <id> --name <new>".into(),
                )
            })?;
            let old_path = sessions_dir.join(format!("{}.json", session_id));
            let new_path = sessions_dir.join(format!("{}.json", new_name));
            if !old_path.exists() {
                println!("Session '{}' not found.", session_id);
                return Ok(());
            }
            std::fs::rename(&old_path, &new_path)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            println!("Session renamed: {} -> {}", session_id, new_name);
        }
        "browse" => {
            if !sessions_dir.exists() {
                println!("No sessions directory found.");
                return Ok(());
            }
            println!("Session Browser");
            println!("===============\n");
            let mut entries: Vec<(String, u64, std::time::SystemTime, usize)> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&sessions_dir) {
                for entry in rd.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if !path.extension().map(|e| e == "json").unwrap_or(false) {
                        continue;
                    }
                    let stem = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let meta = std::fs::metadata(&path);
                    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                    let modified = meta
                        .as_ref()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    let msg_count = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                        .and_then(|v| {
                            v.get("messages")
                                .and_then(|m| m.as_array())
                                .map(|a| a.len())
                        })
                        .unwrap_or(0);
                    entries.push((stem, size, modified, msg_count));
                }
            }
            entries.sort_by(|a, b| b.2.cmp(&a.2));
            if entries.is_empty() {
                println!("No sessions found.");
            } else {
                println!(
                    "{:3} {:30} {:>8} {:>6}  {}",
                    "#", "Session ID", "Size", "Msgs", "Modified"
                );
                println!("{}", "-".repeat(75));
                for (idx, (name, size, modified, msgs)) in entries.iter().enumerate() {
                    let age = modified.elapsed().unwrap_or_default();
                    let age_str = if age.as_secs() < 3600 {
                        format!("{}m ago", age.as_secs() / 60)
                    } else if age.as_secs() < 86400 {
                        format!("{}h ago", age.as_secs() / 3600)
                    } else {
                        format!("{}d ago", age.as_secs() / 86400)
                    };
                    println!(
                        "{:3} {:30} {:>6}KB {:>6}  {}",
                        idx + 1,
                        &name[..name.len().min(30)],
                        size / 1024,
                        msgs,
                        age_str,
                    );
                }
                println!("\nUse `hermes sessions export --id <id>` to view a session.");
            }
        }
        other => {
            println!("Sessions action '{}' is not recognized.", other);
            println!("Available actions: list, export, delete, prune, stats, rename, browse");
        }
    }
    Ok(())
}
