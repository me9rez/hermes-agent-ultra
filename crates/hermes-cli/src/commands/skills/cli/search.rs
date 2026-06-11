use std::path::Path;

use hermes_core::AgentError;

use crate::commands::skills_infra;

pub(crate) async fn run_search(name: Option<String>, skills_dir: &Path) -> Result<(), AgentError> {
    let query = name.unwrap_or_default();
    if query.is_empty() {
        println!("Usage: hermes skills search <query>");
        return Ok(());
    }
    println!("Searching registries for: \"{}\"...", query);
    let client = reqwest::Client::new();
    let mut displayed_results = false;

    if let Ok(results) = skills_infra::search_multi_registry(&client, &query, 40).await {
        if !results.is_empty() {
            displayed_results = true;
            println!("Multi-registry matches:");
            for rec in results {
                let short_desc = if rec.description.trim().is_empty() {
                    "(no description)"
                } else {
                    rec.description.trim()
                };
                println!("  • [{}] {} — {}", rec.source, rec.identifier, short_desc);
            }
            println!(
                "\nInstall with: hermes skills install <identifier> (example: skills.sh/anthropics/skills/skill-creator)"
            );
        }
    }

    // Legacy hub path retained for compatibility.
    match client
        .get("https://skills.hermes.run/api/search")
        .query(&[("q", &query)])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(results) = data.get("results").and_then(|r| r.as_array()) {
                    if results.is_empty() {
                        if !displayed_results {
                            println!("No skills found matching \"{}\".", query);
                        }
                    } else {
                        displayed_results = true;
                        println!("\nLegacy Skills Hub matches:");
                        for skill in results {
                            let name = skill.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                            let desc = skill
                                .get("description")
                                .and_then(|d| d.as_str())
                                .unwrap_or("");
                            let version =
                                skill.get("version").and_then(|v| v.as_str()).unwrap_or("?");
                            println!("  • {} (v{}) — {}", name, version, desc);
                        }
                        println!("\nInstall with: hermes skills install <name>");
                    }
                } else if !displayed_results {
                    println!("Unexpected response format from Skills Hub.");
                }
            }
        }
        Ok(resp) => {
            if !displayed_results {
                println!("Skills Hub returned status {}", resp.status());
            }
        }
        Err(e) => {
            if !displayed_results {
                println!("Could not reach Skills Hub: {}", e);
            }
        }
    }
    if !displayed_results {
        if let Ok(skills_sh_hits) =
            skills_infra::search_skills_sh_registry(&client, &query, 20).await
        {
            if !skills_sh_hits.is_empty() {
                displayed_results = true;
                println!("\nSkills.sh fallback matches:");
                for (name, identifier) in skills_sh_hits {
                    println!("  • {} — {}", name, identifier);
                }
                println!("\nInstall with: hermes skills install skills.sh/<owner/repo/skill>");
            }
        }
    }
    if !displayed_results {
        let taps_file = hermes_config::hermes_home().join("skill_taps.json");
        let subscriptions_file = skills_dir.join("subscriptions.json");
        let taps = skills_infra::effective_skill_taps(&taps_file, &subscriptions_file);
        let fallback = skills_infra::search_skills_via_taps(&client, &taps, &query, 20).await?;
        if fallback.is_empty() {
            println!("No tap-backed matches found for \"{}\".", query);
        } else {
            println!("\nTap-backed matches:");
            for (name, source) in fallback {
                println!("  • {} — {}", name, source);
            }
            println!(
                "\nInstall with: hermes skills install <name> or hermes skills install <owner/repo/path>"
            );
        }
    }
    Ok(())
}
