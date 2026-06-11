use std::path::Path;

use bytes::Bytes;
use hermes_core::AgentError;

use crate::commands::skills_infra;

pub(crate) fn run_reset(name: Option<String>, skills_dir: &Path) -> Result<(), AgentError> {
    let skill_name = name.ok_or_else(|| {
        AgentError::Config("Missing skill name. Usage: hermes skills reset <name>".into())
    })?;
    let target = skills_dir.join(&skill_name);
    if target.exists() {
        std::fs::remove_dir_all(&target)
            .map_err(|e| AgentError::Io(format!("Failed to remove skill dir: {}", e)))?;
    }
    std::fs::create_dir_all(&target)
        .map_err(|e| AgentError::Io(format!("Failed to create skill dir: {}", e)))?;
    std::fs::write(
        target.join("SKILL.md"),
        format!(
            "# {}\n\nReset by CLI. Replace with canonical skill contents.\n",
            skill_name
        ),
    )
    .map_err(|e| AgentError::Io(format!("Failed to write SKILL.md: {}", e)))?;
    println!("Skill '{}' reset at {}", skill_name, target.display());
    Ok(())
}

pub(crate) fn run_subscribe(
    name: Option<String>,
    extra: Option<String>,
    skills_dir: &Path,
) -> Result<(), AgentError> {
    let source = name.ok_or_else(|| {
        AgentError::Config("Missing source. Usage: hermes skills subscribe <name-or-url>".into())
    })?;
    std::fs::create_dir_all(skills_dir).map_err(|e| AgentError::Io(e.to_string()))?;
    let subscriptions_path = skills_dir.join("subscriptions.json");
    let mut subscriptions: Vec<serde_json::Value> = if subscriptions_path.exists() {
        let raw = std::fs::read_to_string(&subscriptions_path)
            .map_err(|e| AgentError::Io(e.to_string()))?;
        serde_json::from_str(&raw).unwrap_or_default()
    } else {
        Vec::new()
    };
    let normalized = source.trim().to_string();
    if normalized.is_empty() {
        return Err(AgentError::Config(
            "skills subscribe: source cannot be empty".into(),
        ));
    }
    let exists = subscriptions.iter().any(|item| {
        item.get("source")
            .and_then(|v| v.as_str())
            .map(|s| s == normalized)
            .unwrap_or(false)
    });
    if exists {
        println!("Skill subscription already exists: {}", normalized);
        return Ok(());
    }
    subscriptions.push(serde_json::json!({
        "source": normalized,
        "added_at": chrono::Utc::now().to_rfc3339(),
        "options": extra.as_deref().unwrap_or(""),
    }));
    std::fs::write(
        &subscriptions_path,
        serde_json::to_string_pretty(&subscriptions)
            .map_err(|e| AgentError::Config(e.to_string()))?,
    )
    .map_err(|e| AgentError::Io(e.to_string()))?;
    println!(
        "Subscribed to skill source '{}'. Registry: {}",
        source,
        subscriptions_path.display()
    );
    Ok(())
}

pub(crate) fn run_inspect(name: Option<String>, skills_dir: &Path) -> Result<(), AgentError> {
    let skill_name = name.unwrap_or_default();
    let skill_md = skills_dir.join(&skill_name).join("SKILL.md");
    if skill_md.exists() {
        let content = std::fs::read_to_string(&skill_md)
            .map_err(|e| AgentError::Io(format!("Read error: {}", e)))?;
        println!("{}", content);
    } else {
        println!("Skill '{}' not found at {}", skill_name, skill_md.display());
    }
    Ok(())
}

pub(crate) fn run_uninstall(name: Option<String>, skills_dir: &Path) -> Result<(), AgentError> {
    let skill_name = name.ok_or_else(|| {
        AgentError::Config("Missing skill name. Usage: hermes skills uninstall <name>".into())
    })?;
    let target = skills_dir.join(&skill_name);
    if target.exists() {
        std::fs::remove_dir_all(&target)
            .map_err(|e| AgentError::Io(format!("Failed to remove skill: {}", e)))?;
        let removed = skills_infra::record_skill_uninstall_in_hub_lock(skills_dir, &skill_name)?;
        if let Some(entry) = removed {
            println!(
                "Skill '{}' uninstalled (source={}, id={}).",
                skill_name, entry.source, entry.identifier
            );
        } else {
            println!("Skill '{}' uninstalled.", skill_name);
        }
    } else if let Some(entry) =
        skills_infra::record_skill_uninstall_in_hub_lock(skills_dir, &skill_name)?
    {
        println!(
            "Skill '{}' not found locally, but removed stale lock entry (source={}, id={}).",
            skill_name, entry.source, entry.identifier
        );
    } else {
        println!("Skill '{}' not found.", skill_name);
    }
    Ok(())
}

pub(crate) fn run_check(name: Option<String>, skills_dir: &Path) -> Result<(), AgentError> {
    let skill_name = name.unwrap_or_default();
    if skill_name.is_empty() {
        println!("Checking all installed skills...");
        let mut ok = 0u32;
        let mut issues = 0u32;
        if let Ok(entries) = std::fs::read_dir(skills_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let dir_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let skill_md = path.join("SKILL.md");
                if !skill_md.exists() {
                    println!("  ✗ {} — missing SKILL.md", dir_name);
                    issues += 1;
                } else {
                    let content = std::fs::read_to_string(&skill_md).unwrap_or_default();
                    if content.trim().is_empty() {
                        println!("  ⚠ {} — SKILL.md is empty", dir_name);
                        issues += 1;
                    } else {
                        println!("  ✓ {}", dir_name);
                        ok += 1;
                    }
                }
            }
        }
        println!("\n{} healthy, {} with issues.", ok, issues);
    } else {
        let skill_path = skills_dir.join(&skill_name);
        let skill_md = skill_path.join("SKILL.md");
        if !skill_path.exists() {
            println!("Skill '{}' not found.", skill_name);
        } else if !skill_md.exists() {
            println!("Skill '{}': MISSING SKILL.md", skill_name);
        } else {
            let content = std::fs::read_to_string(&skill_md).unwrap_or_default();
            let lines = content.lines().count();
            let has_frontmatter = content.starts_with("---");
            println!("Skill '{}': OK", skill_name);
            println!("  Path: {}", skill_path.display());
            println!("  SKILL.md: {} lines", lines);
            println!(
                "  Frontmatter: {}",
                if has_frontmatter { "yes" } else { "no" }
            );
        }
    }
    Ok(())
}

pub(crate) async fn run_update(extra: Option<String>, skills_dir: &Path) -> Result<(), AgentError> {
    println!("Checking for skill updates...\n");
    if !skills_dir.exists() {
        println!("No skills installed.");
        return Ok(());
    }

    let apply_updates = extra.as_deref() == Some("--apply");
    let lock = skills_infra::read_skills_hub_lock(skills_dir);
    if lock.installed.is_empty() {
        println!(
            "No hub-installed skills tracked in {}.",
            skills_infra::skills_hub_lock_path(skills_dir).display()
        );
        println!(
            "Install skills with `hermes skills install <identifier>` to enable source-aware updates."
        );
        return Ok(());
    }

    println!(
        "{:28} {:14} {:14} {:16} {}",
        "Skill", "Source", "Local Hash", "Upstream Hash", "Status"
    );
    println!("{}", "-".repeat(98));

    let taps_file = hermes_config::hermes_home().join("skill_taps.json");
    let subscriptions_file = skills_dir.join("subscriptions.json");
    let merged_taps = skills_infra::effective_skill_taps(&taps_file, &subscriptions_file);
    let client = reqwest::Client::new();

    struct PendingUpdate {
        entry: skills_infra::SkillHubInstalledEntry,
        files: Vec<(String, Bytes)>,
        upstream_hash: String,
    }
    let mut pending: Vec<PendingUpdate> = Vec::new();

    for entry in lock.installed {
        let local_hash = if skills_dir.join(&entry.install_path).exists() {
            skills_infra::hash_installed_skill_dir(&skills_dir.join(&entry.install_path))
                .unwrap_or_else(|_| entry.content_hash.clone())
        } else {
            entry.content_hash.clone()
        };

        match skills_infra::fetch_bundle_for_lock_entry(&client, &entry, &merged_taps).await {
            Ok(files) => {
                let upstream_hash = skills_infra::hash_skill_bundle(&files);
                let status = if local_hash == upstream_hash {
                    "✓ up-to-date"
                } else {
                    pending.push(PendingUpdate {
                        entry: entry.clone(),
                        files,
                        upstream_hash: upstream_hash.clone(),
                    });
                    "⬆ update available"
                };
                println!(
                    "{:28} {:14} {:14} {:16} {}",
                    entry.name,
                    entry.source,
                    &local_hash.chars().take(14).collect::<String>(),
                    &upstream_hash.chars().take(16).collect::<String>(),
                    status
                );
            }
            Err(err) => {
                println!(
                    "{:28} {:14} {:14} {:16} unavailable ({})",
                    entry.name,
                    entry.source,
                    &local_hash.chars().take(14).collect::<String>(),
                    "-",
                    err
                );
            }
        }
    }

    println!();
    if pending.is_empty() {
        println!("All tracked hub skills are up to date.");
    } else {
        println!("{} update(s) available.", pending.len());
        if apply_updates {
            println!("\nApplying updates...");
            for update in pending {
                let install_name = skills_infra::sanitize_skill_install_name(&update.entry.name);
                let target = skills_infra::install_skill_files(
                    skills_dir,
                    &install_name,
                    &update.files,
                    &update.entry.identifier,
                    false,
                )?;
                let prov = skills_infra::SkillInstallProvenance {
                    source: update.entry.source.clone(),
                    identifier: update.entry.identifier.clone(),
                    trust_level: update.entry.trust_level.clone(),
                    metadata: update.entry.metadata.clone(),
                };
                skills_infra::record_skill_install_in_hub_lock(
                    skills_dir,
                    &install_name,
                    &target,
                    &update.files,
                    &prov,
                )?;
                println!(
                    "  ✓ {} updated (new hash: {})",
                    install_name,
                    &update.upstream_hash.chars().take(16).collect::<String>()
                );
                skills_infra::maybe_run_skill_bootstrap(&install_name, &target, &update.files)
                    .await?;
            }
        } else {
            println!("Run `hermes skills update --apply` to install updates.");
        }
    }
    Ok(())
}
