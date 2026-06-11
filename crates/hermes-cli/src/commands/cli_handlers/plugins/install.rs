//! Plugin install logic.

use std::path::Path;

use hermes_core::AgentError;

use super::security::{git_checkout_ref, plugin_git_host_allowed, scan_plugin_security};
pub(super) async fn install_plugin(
    plugin_name: String,
    git_ref: Option<String>,
    allow_untrusted_git_host: bool,
    plugins_dir: &Path,
) -> Result<(), AgentError> {
    println!("Installing plugin: {}...", plugin_name);

    let is_git_url = plugin_name.starts_with("http://")
        || plugin_name.starts_with("https://")
        || plugin_name.starts_with("git@");

    if is_git_url {
        if !plugin_git_host_allowed(&plugin_name, allow_untrusted_git_host) {
            println!(
                "  ✗ Git host is not on the default allow-list (github.com, gitlab.com, codeberg.org, …)."
            );
            println!(
                "    Set comma-separated HERMES_PLUGIN_GIT_EXTRA_HOSTS or pass --allow-untrusted-git-host after you trust the source."
            );
            return Ok(());
        }
        // Extract repo name from URL for target directory
        let repo_name = plugin_name
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .rsplit('/')
            .next()
            .unwrap_or("unknown-plugin")
            .to_string();

        // Also handle git@ SSH URLs like git@github.com:user/repo.git
        let repo_name = if repo_name.contains(':') {
            repo_name
                .rsplit(':')
                .next()
                .unwrap_or(&repo_name)
                .trim_end_matches(".git")
                .rsplit('/')
                .next()
                .unwrap_or(&repo_name)
                .to_string()
        } else {
            repo_name
        };

        let target = plugins_dir.join(&repo_name);
        if target.exists() {
            println!(
                "Plugin '{}' is already installed at {}",
                repo_name,
                target.display()
            );
            return Ok(());
        }

        std::fs::create_dir_all(&plugins_dir).map_err(|e| {
            hermes_core::AgentError::Io(format!("Failed to create plugins dir: {}", e))
        })?;

        println!("  Cloning {} ...", plugin_name);
        let output = tokio::process::Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                &plugin_name,
                &target.to_string_lossy(),
            ])
            .output()
            .await
            .map_err(|e| hermes_core::AgentError::Io(format!("git clone failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("  ✗ git clone failed: {}", stderr.trim());
            return Ok(());
        }

        if let Some(gr) = git_ref.as_deref() {
            println!("  Checking out ref: {} ...", gr);
            if let Err(e) = git_checkout_ref(&target, gr).await {
                println!("  ✗ {}", e);
                let _ = std::fs::remove_dir_all(&target);
                return Ok(());
            }
        }

        // Verify plugin.yaml exists
        let manifest_path = target.join("plugin.yaml");
        if !manifest_path.exists() {
            println!("  ✗ No plugin.yaml found in cloned repository.");
            println!("    Removing {}...", target.display());
            let _ = std::fs::remove_dir_all(&target);
            return Ok(());
        }

        // Parse and display plugin info
        let manifest_content = std::fs::read_to_string(&manifest_path)
            .map_err(|e| hermes_core::AgentError::Io(format!("Read error: {}", e)))?;
        let manifest: serde_json::Value =
            serde_yaml::from_str(&manifest_content).unwrap_or(serde_json::json!({}));

        let p_name = manifest
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&repo_name);
        let p_version = manifest
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let p_desc = manifest
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Security scan of cloned files
        let suspicious = scan_plugin_security(&target);
        let hard_block = suspicious.iter().any(|s| {
            s.contains("curl piped to shell")
                || s.contains("wget piped to shell")
                || s.contains("curl|sh style install")
        });
        if hard_block && !allow_untrusted_git_host {
            println!("\n  ✗ High-risk install patterns detected — clone removed.");
            for warning in &suspicious {
                println!("    - {}", warning);
            }
            println!(
                "\n  If you reviewed the code manually, re-run with --allow-untrusted-git-host."
            );
            let _ = std::fs::remove_dir_all(&target);
            return Ok(());
        }
        if !suspicious.is_empty() {
            println!("\n  ⚠ Security warnings found ({}):", suspicious.len());
            for warning in &suspicious {
                println!("    - {}", warning);
            }
            println!("\n  Review the warnings above before enabling this plugin.");
        }

        println!("  ✓ Plugin installed successfully!");
        println!("    Name:        {}", p_name);
        println!("    Version:     {}", p_version);
        println!("    Description: {}", p_desc);
        println!("    Path:        {}", target.display());
    } else if plugin_name.starts_with("gh:") || plugin_name.contains('/') {
        // Convert gh:user/repo or user/repo to a GitHub HTTPS URL
        let repo_path = plugin_name.trim_start_matches("gh:");
        let git_url = format!("https://github.com/{}.git", repo_path);
        let repo_name = repo_path.rsplit('/').next().unwrap_or("unknown-plugin");
        let target = plugins_dir.join(repo_name);
        if target.exists() {
            println!("Plugin '{}' is already installed.", repo_name);
            return Ok(());
        }

        std::fs::create_dir_all(&plugins_dir).map_err(|e| {
            hermes_core::AgentError::Io(format!("Failed to create plugins dir: {}", e))
        })?;

        println!("  Cloning from GitHub: {}", git_url);
        let output = tokio::process::Command::new("git")
            .args(["clone", "--depth", "1", &git_url, &target.to_string_lossy()])
            .output()
            .await
            .map_err(|e| hermes_core::AgentError::Io(format!("git clone failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("  ✗ git clone failed: {}", stderr.trim());
            return Ok(());
        }

        if let Some(gr) = git_ref.as_deref() {
            println!("  Checking out ref: {} ...", gr);
            if let Err(e) = git_checkout_ref(&target, gr).await {
                println!("  ✗ {}", e);
                let _ = std::fs::remove_dir_all(&target);
                return Ok(());
            }
        }

        let manifest_path = target.join("plugin.yaml");
        if !manifest_path.exists() {
            println!("  ✗ No plugin.yaml found in cloned repository.");
            let _ = std::fs::remove_dir_all(&target);
            return Ok(());
        }

        let manifest_content = std::fs::read_to_string(&manifest_path).unwrap_or_default();
        let manifest: serde_json::Value =
            serde_yaml::from_str(&manifest_content).unwrap_or(serde_json::json!({}));

        let p_name = manifest
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(repo_name);
        let p_version = manifest
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let p_desc = manifest
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let suspicious = scan_plugin_security(&target);
        let hard_block = suspicious.iter().any(|s| {
            s.contains("curl piped to shell")
                || s.contains("wget piped to shell")
                || s.contains("curl|sh style install")
        });
        if hard_block && !allow_untrusted_git_host {
            println!("\n  ✗ High-risk install patterns detected — clone removed.");
            for warning in &suspicious {
                println!("    - {}", warning);
            }
            println!(
                "\n  If you reviewed the code manually, re-run with --allow-untrusted-git-host."
            );
            let _ = std::fs::remove_dir_all(&target);
            return Ok(());
        }
        if !suspicious.is_empty() {
            println!("\n  ⚠ Security warnings found ({}):", suspicious.len());
            for warning in &suspicious {
                println!("    - {}", warning);
            }
        }

        println!("  ✓ Plugin installed successfully!");
        println!("    Name:        {}", p_name);
        println!("    Version:     {}", p_version);
        println!("    Description: {}", p_desc);
        println!("    Path:        {}", target.display());
    } else {
        let target = plugins_dir.join(&plugin_name);
        if target.exists() {
            println!("Plugin '{}' is already installed.", plugin_name);
            return Ok(());
        }
        // Registry lookup
        println!("  Looking up '{}' in plugin registry...", plugin_name);
        match reqwest::Client::new()
            .get(&format!(
                "https://plugins.hermes.run/api/v1/{}",
                plugin_name
            ))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    let version = data
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("latest");
                    let git_url = data.get("git_url").and_then(|v| v.as_str());
                    println!("  Found {} v{}", plugin_name, version);

                    if let Some(url) = git_url {
                        if !plugin_git_host_allowed(url, allow_untrusted_git_host) {
                            println!(
                                "  ✗ Registry git_url host is not allow-listed. Use --allow-untrusted-git-host or HERMES_PLUGIN_GIT_EXTRA_HOSTS."
                            );
                            return Ok(());
                        }
                        std::fs::create_dir_all(&plugins_dir)
                            .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;

                        let output = tokio::process::Command::new("git")
                            .args(["clone", "--depth", "1", url, &target.to_string_lossy()])
                            .output()
                            .await
                            .map_err(|e| {
                                hermes_core::AgentError::Io(format!("git clone failed: {}", e))
                            })?;

                        if output.status.success() {
                            if let Some(gr) = git_ref.as_deref() {
                                println!("  Checking out ref: {} ...", gr);
                                if let Err(e) = git_checkout_ref(&target, gr).await {
                                    println!("  ✗ {}", e);
                                    let _ = std::fs::remove_dir_all(&target);
                                    return Ok(());
                                }
                            }
                            let suspicious = scan_plugin_security(&target);
                            let hard_block = suspicious.iter().any(|s| {
                                s.contains("curl piped to shell")
                                    || s.contains("wget piped to shell")
                                    || s.contains("curl|sh style install")
                            });
                            if hard_block && !allow_untrusted_git_host {
                                println!("  ✗ High-risk patterns — removed clone.");
                                let _ = std::fs::remove_dir_all(&target);
                                return Ok(());
                            }
                            if !suspicious.is_empty() {
                                println!("  ⚠ Security warnings: {}", suspicious.len());
                                for w in &suspicious {
                                    println!("    - {}", w);
                                }
                            }
                            println!("  ✓ Plugin '{}' v{} installed.", plugin_name, version);
                        } else {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            println!("  ✗ Clone failed: {}", stderr.trim());
                        }
                    } else {
                        println!("  No git_url in registry response. Cannot install.");
                    }
                }
            }
            _ => {
                println!("  Plugin '{}' not found in registry.", plugin_name);
                println!("  Try installing from a URL or GitHub repo instead:");
                println!("    hermes plugins install https://github.com/user/repo");
                println!("    hermes plugins install gh:user/repo");
            }
        }
    }
    Ok(())
}
