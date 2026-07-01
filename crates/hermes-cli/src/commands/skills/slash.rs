use std::process::Stdio;

use hermes_core::AgentError;

use crate::commands::{CommandResult, emit_command_output};

/// Read the first markdown heading from a SKILL.md file as its title.
fn read_skill_title(skill_md: &std::path::Path) -> String {
    std::fs::read_to_string(skill_md)
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with('#'))
                .map(|l| l.trim_start_matches('#').trim().to_string())
        })
        .unwrap_or_else(|| "(no description)".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillsSlashInvocation {
    action: Option<String>,
    name: Option<String>,
    extra: Option<String>,
}

fn parse_skills_slash_invocation(args: &[&str]) -> Result<SkillsSlashInvocation, String> {
    if args.is_empty() {
        return Ok(SkillsSlashInvocation {
            action: None,
            name: None,
            extra: None,
        });
    }

    let action = args[0].to_ascii_lowercase();
    let rest = &args[1..];

    let build_joined = |values: &[&str]| -> Option<String> {
        let joined = values.join(" ").trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    };

    let parsed = match action.as_str() {
        "list" | "browse" | "audit" | "quality" => SkillsSlashInvocation {
            action: Some(action),
            name: build_joined(rest),
            extra: None,
        },
        "search" | "install" | "inspect" | "uninstall" | "remove" | "publish" | "subscribe"
        | "reset" => SkillsSlashInvocation {
            action: Some(action),
            name: build_joined(rest),
            extra: None,
        },
        "check" => SkillsSlashInvocation {
            action: Some(action),
            name: rest.first().map(|s| s.to_string()),
            extra: None,
        },
        "update" => {
            let apply = rest
                .iter()
                .any(|v| matches!(v.to_ascii_lowercase().as_str(), "--apply" | "-a"));
            SkillsSlashInvocation {
                action: Some(action),
                name: None,
                extra: if apply {
                    Some("--apply".to_string())
                } else {
                    None
                },
            }
        }
        "snapshot" => SkillsSlashInvocation {
            action: Some(action),
            name: rest.first().map(|s| s.to_string()),
            extra: build_joined(if rest.len() > 1 { &rest[1..] } else { &[] }),
        },
        "tap" => SkillsSlashInvocation {
            action: Some(action),
            name: rest.first().map(|s| s.to_ascii_lowercase()),
            extra: build_joined(if rest.len() > 1 { &rest[1..] } else { &[] }),
        },
        "config" => SkillsSlashInvocation {
            action: Some(action),
            name: rest.first().map(|s| s.to_string()),
            extra: build_joined(if rest.len() > 1 { &rest[1..] } else { &[] }),
        },
        _ => {
            return Err(format!(
                "Unknown /skills subcommand '{}'. Use `/skills list`, `/skills quality`, or `/skills search <query>`.",
                action
            ));
        }
    };

    Ok(parsed)
}

async fn run_skills_subcommand_via_cli(
    invocation: &SkillsSlashInvocation,
) -> Result<String, AgentError> {
    let exe = std::env::current_exe()
        .map_err(|e| AgentError::Io(format!("Could not determine current executable: {}", e)))?;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("skills");
    if let Some(action) = invocation.action.as_deref() {
        cmd.arg(action);
    }
    if let Some(name) = invocation.name.as_deref() {
        cmd.arg(name);
    }
    if let Some(extra) = invocation.extra.as_deref() {
        cmd.arg("--extra").arg(extra);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd
        .output()
        .await
        .map_err(|e| AgentError::Io(format!("Failed to execute skills command: {}", e)))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let mut combined = String::new();
    if !stdout.is_empty() {
        combined.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push_str("\n\n");
        }
        combined.push_str(&format!("stderr:\n{}", stderr));
    }
    if combined.is_empty() {
        combined = if output.status.success() {
            "No output.".to_string()
        } else {
            format!("Command failed with status {}.", output.status)
        };
    }
    if !output.status.success() {
        combined = format!("(exit: {})\n{}", output.status, combined);
    }
    Ok(combined)
}

pub(crate) async fn handle_skills_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if !args.is_empty() {
        let invocation = match parse_skills_slash_invocation(args) {
            Ok(v) => v,
            Err(msg) => {
                emit_command_output(host, msg);
                return Ok(CommandResult::Handled);
            }
        };
        let output = run_skills_subcommand_via_cli(&invocation).await?;
        emit_command_output(host, output);
        return Ok(CommandResult::Handled);
    }

    let skills_dir = hermes_config::hermes_home().join("skills");
    if !skills_dir.exists() {
        emit_command_output(
            host,
            format!(
                "No skills directory found at {}. Run `hermes setup` first.",
                skills_dir.display()
            ),
        );
        return Ok(CommandResult::Handled);
    }

    let mut skills: Vec<(String, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // Case 1: skill at top level — skills/<name>/SKILL.md
            let skill_md = path.join("SKILL.md");
            if skill_md.exists() {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let title = read_skill_title(&skill_md);
                skills.push((name, title));
                continue;
            }
            // Case 2: skill under a category — skills/<category>/<name>/SKILL.md
            if let Ok(cat_entries) = std::fs::read_dir(&path) {
                for cat_entry in cat_entries.flatten() {
                    let cat_path = cat_entry.path();
                    let cat_skill_md = cat_path.join("SKILL.md");
                    if !cat_path.is_dir() || !cat_skill_md.exists() {
                        continue;
                    }
                    let name = cat_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let title = read_skill_title(&cat_skill_md);
                    skills.push((name, title));
                }
            }
        }
    }
    skills.sort_by(|a, b| a.0.cmp(&b.0));

    if skills.is_empty() {
        emit_command_output(
            host,
            format!(
                "No installed skills found in {}.\nInstall skills with `hermes skills install <name>`.",
                skills_dir.display()
            ),
        );
    } else {
        let mut out = format!("Installed skills ({}):\n", skills.len());
        for (name, title) in &skills {
            out.push_str(&format!("- `{}` — {}\n", name, title));
        }
        out.push_str("\nUse `hermes skills inspect <name>` for details.");
        out.push_str("\nUse `/skills quality` for score + fallback recommendations.");
        emit_command_output(host, out.trim_end());
    }
    Ok(CommandResult::Handled)
}
