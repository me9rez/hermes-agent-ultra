use std::path::{Path, PathBuf};

use hermes_core::AgentError;
use regex::Regex;

use crate::commands::skills_infra;

pub(crate) async fn run_publish(name: Option<String>, skills_dir: &Path) -> Result<(), AgentError> {
    let skill_name = name.ok_or_else(|| {
        AgentError::Config("Missing skill name. Usage: hermes skills publish <name>".into())
    })?;
    let skill_path = skills_dir.join(&skill_name);
    if !skill_path.exists() {
        return Err(AgentError::Config(format!(
            "Skill '{}' not found.",
            skill_name
        )));
    }
    println!("Publishing skill '{}' to Skills Hub...", skill_name);
    println!("  Source: {}", skill_path.display());

    let skill_md = skill_path.join("SKILL.md");
    if !skill_md.exists() {
        println!("  ✗ Missing SKILL.md — required for publishing.");
        return Ok(());
    }

    let content = std::fs::read_to_string(&skill_md)
        .map_err(|e| AgentError::Io(format!("Read error: {}", e)))?;
    let (frontmatter, _body) = hermes_tools::tools::skill_utils::parse_frontmatter(&content);

    let fm_name = frontmatter.get("name").and_then(|v| v.as_str());
    let fm_version = frontmatter.get("version").and_then(|v| v.as_str());
    let fm_desc = frontmatter.get("description").and_then(|v| v.as_str());
    let fm_category = frontmatter.get("category").and_then(|v| v.as_str());

    if fm_name.is_none() || fm_version.is_none() || fm_desc.is_none() || fm_category.is_none() {
        println!("  ✗ SKILL.md frontmatter must include: name, version, description, category");
        let mut missing = Vec::new();
        if fm_name.is_none() {
            missing.push("name");
        }
        if fm_version.is_none() {
            missing.push("version");
        }
        if fm_desc.is_none() {
            missing.push("description");
        }
        if fm_category.is_none() {
            missing.push("category");
        }
        println!("    Missing: {}", missing.join(", "));
        return Ok(());
    }

    let publish_name = fm_name.unwrap();
    let publish_version = fm_version.unwrap();
    let publish_desc = fm_desc.unwrap();
    let publish_category = fm_category.unwrap();
    println!(
        "  ✓ name={}, version={}, category={}",
        publish_name, publish_version, publish_category
    );
    println!("  ✓ description: {}", publish_desc);

    let mut tar_buf = Vec::new();
    {
        let enc = flate2::write::GzEncoder::new(&mut tar_buf, flate2::Compression::default());
        let mut tar_builder = tar::Builder::new(enc);
        tar_builder
            .append_dir_all(&skill_name, &skill_path)
            .map_err(|e| AgentError::Io(format!("Tar error: {}", e)))?;
        tar_builder
            .finish()
            .map_err(|e| AgentError::Io(format!("Tar finish error: {}", e)))?;
    }
    println!("  ✓ Packaged {} bytes", tar_buf.len());

    let token_path = hermes_config::hermes_home().join("hub_token");
    if !token_path.exists() {
        println!("  ✗ No hub token found at {}", token_path.display());
        println!("    Run `hermes login hub` to authenticate with Skills Hub.");
        return Ok(());
    }
    let hub_token = std::fs::read_to_string(&token_path)
        .map_err(|e| AgentError::Io(format!("Token read error: {}", e)))?
        .trim()
        .to_string();

    let metadata = serde_json::json!({
        "name": publish_name,
        "version": publish_version,
        "description": publish_desc,
        "category": publish_category,
    });

    let tarball_part = reqwest::multipart::Part::bytes(tar_buf)
        .file_name(format!("{}-{}.tar.gz", publish_name, publish_version))
        .mime_str("application/gzip")
        .unwrap();
    let metadata_part = reqwest::multipart::Part::text(metadata.to_string())
        .mime_str("application/json")
        .unwrap();
    let form = reqwest::multipart::Form::new()
        .part("tarball", tarball_part)
        .part("metadata", metadata_part);

    println!("  Uploading to Skills Hub...");
    match reqwest::Client::new()
        .post("https://agentskills.io/api/v1/skills")
        .bearer_auth(&hub_token)
        .multipart(form)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let url = format!("https://agentskills.io/skills/{}", publish_name);
            println!("  ✓ Published successfully!");
            println!("  URL: {}", url);
        }
        Ok(resp) if resp.status() == reqwest::StatusCode::CONFLICT => {
            println!(
                "  ✗ Version {} already exists on Skills Hub.",
                publish_version
            );
            println!("    Bump the version in SKILL.md frontmatter and try again.");
        }
        Ok(resp) if resp.status() == reqwest::StatusCode::UNAUTHORIZED => {
            println!("  ✗ Unauthorized. Hub token may be expired.");
            println!("    Run `hermes login hub` to re-authenticate.");
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            println!("  ✗ Upload failed (HTTP {}): {}", status, body);
        }
        Err(e) => {
            println!("  ✗ Could not reach Skills Hub: {}", e);
        }
    }
    Ok(())
}

pub(crate) fn run_snapshot(
    name: Option<String>,
    extra: Option<String>,
    skills_dir: &Path,
) -> Result<(), AgentError> {
    let sub = name.as_deref().unwrap_or("export");
    match sub {
        "export" => {
            let output = extra.unwrap_or_else(|| {
                format!(
                    "skills-snapshot-{}.tar.gz",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                )
            });
            println!("Exporting skills snapshot to: {}", output);
            if !skills_dir.exists() {
                println!("No skills directory found.");
                return Ok(());
            }
            let tar_gz = std::fs::File::create(&output)
                .map_err(|e| AgentError::Io(format!("Failed to create archive: {}", e)))?;
            let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
            let mut tar = tar::Builder::new(enc);
            tar.append_dir_all("skills", skills_dir)
                .map_err(|e| AgentError::Io(format!("Failed to archive: {}", e)))?;
            tar.finish()
                .map_err(|e| AgentError::Io(format!("Failed to finalize archive: {}", e)))?;
            println!("Snapshot exported to: {}", output);
        }
        "import" => {
            let input = extra.ok_or_else(|| {
                AgentError::Config(
                    "Missing snapshot path. Usage: hermes skills snapshot import <path>".into(),
                )
            })?;
            println!("Importing skills snapshot from: {}", input);
            let tar_gz = std::fs::File::open(&input)
                .map_err(|e| AgentError::Io(format!("Failed to open archive: {}", e)))?;
            let dec = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(dec);
            std::fs::create_dir_all(skills_dir)
                .map_err(|e| AgentError::Io(format!("Failed to create skills dir: {}", e)))?;
            archive
                .unpack(hermes_config::hermes_home())
                .map_err(|e| AgentError::Io(format!("Failed to extract archive: {}", e)))?;
            println!("Snapshot imported successfully.");
        }
        _ => {
            println!("Usage: hermes skills snapshot export|import [path]");
        }
    }
    Ok(())
}

pub(crate) fn run_tap(
    name: Option<String>,
    extra: Option<String>,
    skills_dir: &Path,
) -> Result<(), AgentError> {
    let sub = name.as_deref().unwrap_or("list");
    let taps_file = hermes_config::hermes_home().join("skill_taps.json");
    let subscriptions_file = skills_dir.join("subscriptions.json");
    match sub {
        "list" => {
            let taps = skills_infra::effective_skill_taps(&taps_file, &subscriptions_file);
            if taps.is_empty() {
                println!("No skill taps configured.");
            } else {
                println!("Skill taps:");
                for tap in &taps {
                    println!("  • {}", tap);
                }
            }
        }
        "add" => {
            let url = extra.ok_or_else(|| {
                AgentError::Config("Missing tap URL. Usage: hermes skills tap add <url>".into())
            })?;
            let mut taps: Vec<String> = skills_infra::read_skill_taps(&taps_file);
            if skills_infra::effective_skill_taps(&taps_file, &subscriptions_file).contains(&url) {
                println!("Tap already exists: {}", url);
            } else {
                taps.push(url.clone());
                skills_infra::write_skill_taps(&taps_file, &taps)?;
                println!("Added tap: {}", url);
            }
        }
        "remove" => {
            let url = extra.ok_or_else(|| {
                AgentError::Config("Missing tap URL. Usage: hermes skills tap remove <url>".into())
            })?;
            if skills_infra::DEFAULT_SKILL_TAPS
                .iter()
                .any(|default_tap| default_tap == &url.as_str())
            {
                println!("Tap '{}' is a built-in default and cannot be removed.", url);
                println!(
                    "Add custom taps with `hermes skills tap add <url>`; defaults remain active."
                );
                return Ok(());
            }

            let mut taps: Vec<String> = skills_infra::read_skill_taps(&taps_file);
            let before_len = taps.len();
            taps.retain(|t| t != &url);
            if taps.len() < before_len {
                skills_infra::write_skill_taps(&taps_file, &taps)?;
                println!("Removed tap: {}", url);
            } else {
                println!("Tap not found: {}", url);
            }
        }
        _ => {
            println!("Usage: hermes skills tap list|add|remove [url]");
        }
    }
    Ok(())
}

pub(crate) fn run_config(
    name: Option<String>,
    extra: Option<String>,
    skills_dir: &Path,
) -> Result<(), AgentError> {
    let skill_name = name.ok_or_else(|| {
        AgentError::Config(
            "Missing skill name. Usage: hermes skills config <name> [key] [value]".into(),
        )
    })?;
    let config_file = skills_dir.join(&skill_name).join("config.json");
    if let Some(key) = extra {
        let parts: Vec<&str> = key.splitn(2, '=').collect();
        if parts.len() == 2 {
            let mut config: serde_json::Value = if config_file.exists() {
                let c = std::fs::read_to_string(&config_file).unwrap_or_else(|_| "{}".to_string());
                serde_json::from_str(&c).unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };
            config[parts[0]] = serde_json::Value::String(parts[1].to_string());
            let json = serde_json::to_string_pretty(&config)
                .map_err(|e| AgentError::Config(e.to_string()))?;
            std::fs::write(&config_file, json).map_err(|e| AgentError::Io(e.to_string()))?;
            println!("Set {} = {} for skill '{}'", parts[0], parts[1], skill_name);
        } else if config_file.exists() {
            let c = std::fs::read_to_string(&config_file).unwrap_or_else(|_| "{}".to_string());
            let config: serde_json::Value =
                serde_json::from_str(&c).unwrap_or(serde_json::json!({}));
            match config.get(&key) {
                Some(v) => println!("{} = {}", key, v),
                None => println!("Key '{}' not found in skill config.", key),
            }
        } else {
            println!("No config for skill '{}'.", skill_name);
        }
    } else if config_file.exists() {
        let content =
            std::fs::read_to_string(&config_file).map_err(|e| AgentError::Io(e.to_string()))?;
        println!("Config for skill '{}':", skill_name);
        println!("{}", content);
    } else {
        println!("No config for skill '{}'.", skill_name);
    }
    Ok(())
}

pub(crate) fn run_quality(skills_dir: &Path) -> Result<(), AgentError> {
    println!("Skill quality scorecard");
    println!("======================\n");
    if !skills_dir.exists() {
        println!("No skills installed.");
        return Ok(());
    }

    #[derive(Debug)]
    struct SkillQualityRow {
        name: String,
        score: i32,
        tier: &'static str,
        notes: Vec<String>,
    }

    let mut rows: Vec<SkillQualityRow> = Vec::new();
    let weak_regex = Regex::new(r"(?i)\b(todo|fixme|placeholder|stub)\b")
        .map_err(|e| AgentError::Config(format!("quality regex error: {}", e)))?;
    let risky_regex = Regex::new(r"(?i)\b(rm\s+-rf|mkfs|dd\s+if=|eval\s*\(|exec\s*\()")
        .map_err(|e| AgentError::Config(format!("quality regex error: {}", e)))?;

    if let Ok(entries) = std::fs::read_dir(skills_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let skill_md = path.join("SKILL.md");
            if !path.is_dir() || !skill_md.exists() {
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let mut score = 100i32;
            let mut notes = Vec::new();
            let content = std::fs::read_to_string(&skill_md).unwrap_or_default();
            let (frontmatter, _) = hermes_tools::tools::skill_utils::parse_frontmatter(&content);
            for required in ["name", "version", "description", "category"] {
                if frontmatter.get(required).and_then(|v| v.as_str()).is_none() {
                    score -= 8;
                    notes.push(format!("missing_frontmatter:{}", required));
                }
            }

            let line_count = content.lines().count();
            if line_count < 20 {
                score -= 10;
                notes.push("short_skill_doc".to_string());
            } else if line_count > 80 {
                score += 4;
            }

            let scripts_dir = path.join("scripts");
            if scripts_dir.exists() {
                score += 6;
            } else {
                score -= 4;
                notes.push("no_scripts".to_string());
            }
            if path.join("examples").exists() {
                score += 4;
            } else {
                notes.push("no_examples".to_string());
            }
            if path.join("templates").exists() {
                score += 3;
            }
            if path.join("tests").exists() {
                score += 4;
            }

            if weak_regex.is_match(&content) {
                score -= 8;
                notes.push("contains_placeholder_markers".to_string());
            }
            if risky_regex.is_match(&content) {
                score -= 20;
                notes.push("contains_risky_exec_pattern".to_string());
            }

            score = score.clamp(0, 100);
            let tier = if score >= 85 {
                "excellent"
            } else if score >= 70 {
                "good"
            } else if score >= 55 {
                "watch"
            } else {
                "fallback"
            };
            rows.push(SkillQualityRow {
                name,
                score,
                tier,
                notes,
            });
        }
    }

    if rows.is_empty() {
        println!("No skills installed.");
        return Ok(());
    }
    rows.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
    println!("{:28} {:>5} {:>10}  notes", "skill", "score", "tier");
    println!("{}", "-".repeat(84));
    for row in &rows {
        let notes = if row.notes.is_empty() {
            "-".to_string()
        } else {
            row.notes.join(",")
        };
        println!(
            "{:28} {:>5} {:>10}  {}",
            row.name, row.score, row.tier, notes
        );
    }

    let fallback: Vec<&SkillQualityRow> = rows.iter().filter(|row| row.score < 55).collect();
    if !fallback.is_empty() {
        println!("\nFallback recommendations:");
        for row in fallback {
            println!(
                "- {}: run `hermes skills update --apply` or reinstall from a trusted registry source.",
                row.name
            );
        }
    } else {
        println!("\nFallback recommendations: none (all tracked skills >= watch tier).");
    }
    Ok(())
}

pub(crate) fn run_audit(name: Option<String>, skills_dir: &Path) -> Result<(), AgentError> {
    let scan_dir = name
        .as_ref()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| skills_dir.to_path_buf());
    println!(
        "Security audit of installed skills ({})",
        scan_dir.display()
    );
    println!("==================================\n");
    if !scan_dir.exists() {
        println!("No skills directory at {}.", scan_dir.display());
        return Ok(());
    }

    struct AuditFinding {
        file: String,
        pattern: String,
        severity: &'static str,
    }

    let shell_injection_patterns: &[(&str, &str)] = &[
        (
            r"(?i)\b(rm\s+-rf|mkfs|dd\s+if=)",
            "Shell command injection (destructive command)",
        ),
        (r"(?i)(:\(\)\{.*;\}|fork\s+bomb)", "Fork bomb pattern"),
        (r"(?i)\b(sudo\s+|su\s+-\s)", "Privilege escalation attempt"),
        (
            r"(?i)(export\s+PATH|PATH\s*=\s*/)",
            "PATH environment manipulation",
        ),
        (
            r"(?i)chmod\s+[0-7]*777",
            "Overly permissive file permissions",
        ),
        (r"(?i)\beval\s*\(", "Dynamic code evaluation (eval)"),
        (r"(?i)\bexec\s*\(", "Dynamic code execution (exec)"),
        (
            r"(?i)(os\.system|subprocess\.call|subprocess\.run|subprocess\.Popen)",
            "Subprocess execution",
        ),
    ];

    let path_traversal_patterns: &[(&str, &str)] = &[(r"\.\.[\\/]", "Path traversal (../)")];

    let network_patterns: &[(&str, &str)] = &[
        (r"(?i)://127\.0\.0\.1", "Internal network URL (127.0.0.1)"),
        (r"(?i)://localhost", "Internal network URL (localhost)"),
        (
            r"(?i)://10\.\d+\.\d+\.\d+",
            "Internal network URL (10.x.x.x)",
        ),
        (
            r"(?i)://192\.168\.\d+\.\d+",
            "Internal network URL (192.168.x.x)",
        ),
        (r"(?i)://0\.0\.0\.0", "Internal network URL (0.0.0.0)"),
        (r"(?i)://\[::1\]", "Internal network URL (::1)"),
    ];

    let credential_patterns: &[(&str, &str)] = &[
        (
            r#"(?i)(password\s*=\s*['"][^'"]{3,}['"])"#,
            "Hardcoded password",
        ),
        (
            r#"(?i)(api[_-]?key\s*=\s*['"][^'"]{3,}['"])"#,
            "Hardcoded API key",
        ),
        (
            r#"(?i)(secret\s*=\s*['"][^'"]{3,}['"])"#,
            "Hardcoded secret",
        ),
        (r"(?i)(sk-[a-zA-Z0-9]{20,})", "Exposed API key (sk-...)"),
        (r"(?i)(ghp_[a-zA-Z0-9]{30,})", "Exposed GitHub PAT"),
    ];

    let base64_suspicious: &[(&str, &str)] = &[
        (
            r"(?i)(base64[._-]?decode|atob)\s*\(",
            "Base64 decode invocation (potential obfuscation)",
        ),
        (
            r"[A-Za-z0-9+/]{100,}={0,2}",
            "Long base64-encoded content (potential obfuscation)",
        ),
    ];

    let mut total = 0u32;
    let mut total_warnings = 0u32;
    let mut total_critical = 0u32;

    fn scan_dir_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.is_dir() {
                    scan_dir_recursive(&p, files);
                } else if p.is_file() {
                    files.push(p);
                }
            }
        }
    }

    if let Ok(entries) = std::fs::read_dir(&scan_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            total += 1;
            let dir_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let mut findings: Vec<AuditFinding> = Vec::new();

            let mut all_files = Vec::new();
            scan_dir_recursive(&path, &mut all_files);

            for fp in &all_files {
                let Ok(content) = std::fs::read_to_string(fp) else {
                    continue;
                };
                let fname = fp
                    .strip_prefix(&path)
                    .unwrap_or(fp)
                    .to_string_lossy()
                    .to_string();

                for (pat, desc) in shell_injection_patterns {
                    if let Ok(re) = Regex::new(pat) {
                        if re.is_match(&content) {
                            findings.push(AuditFinding {
                                file: fname.clone(),
                                pattern: desc.to_string(),
                                severity: "critical",
                            });
                        }
                    }
                }

                for (pat, desc) in path_traversal_patterns {
                    if let Ok(re) = Regex::new(pat) {
                        if re.is_match(&content) {
                            findings.push(AuditFinding {
                                file: fname.clone(),
                                pattern: desc.to_string(),
                                severity: "critical",
                            });
                        }
                    }
                }

                for (pat, desc) in network_patterns {
                    if let Ok(re) = Regex::new(pat) {
                        if re.is_match(&content) {
                            findings.push(AuditFinding {
                                file: fname.clone(),
                                pattern: desc.to_string(),
                                severity: "warning",
                            });
                        }
                    }
                }

                for (pat, desc) in credential_patterns {
                    if let Ok(re) = Regex::new(pat) {
                        if re.is_match(&content) {
                            findings.push(AuditFinding {
                                file: fname.clone(),
                                pattern: desc.to_string(),
                                severity: "critical",
                            });
                        }
                    }
                }

                for (pat, desc) in base64_suspicious {
                    if let Ok(re) = Regex::new(pat) {
                        if re.is_match(&content) {
                            findings.push(AuditFinding {
                                file: fname.clone(),
                                pattern: desc.to_string(),
                                severity: "warning",
                            });
                        }
                    }
                }
            }

            if findings.is_empty() {
                println!("  ✓ {} — clean", dir_name);
            } else {
                let crit_count = findings.iter().filter(|f| f.severity == "critical").count();
                let warn_count = findings.iter().filter(|f| f.severity == "warning").count();
                total_critical += crit_count as u32;
                total_warnings += warn_count as u32;

                let icon = if crit_count > 0 { "✗" } else { "⚠" };
                println!(
                    "  {} {} — {} critical, {} warning(s):",
                    icon, dir_name, crit_count, warn_count
                );
                for f in &findings {
                    let sev_icon = if f.severity == "critical" {
                        "CRIT"
                    } else {
                        "WARN"
                    };
                    println!("    [{}] {} — {}", sev_icon, f.file, f.pattern);
                }
            }
        }
    }

    println!("\n{}", "=".repeat(50));
    println!("Audited {} skill(s)", total);
    println!("  Critical: {}", total_critical);
    println!("  Warnings: {}", total_warnings);
    if total_critical == 0 && total_warnings == 0 {
        println!("  Status:   All clear ✓");
    } else if total_critical > 0 {
        println!("  Status:   Action required — review critical findings");
    } else {
        println!("  Status:   Review recommended");
    }
    Ok(())
}
