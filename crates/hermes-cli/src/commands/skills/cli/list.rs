use std::path::Path;

use hermes_core::AgentError;

pub(crate) fn run_list(skills_dir: &Path) -> Result<(), AgentError> {
    if !skills_dir.exists() {
        println!(
            "No skills directory found at {}. Run `hermes setup` first.",
            skills_dir.display()
        );
        return Ok(());
    }
    let mut count = 0u32;
    println!("Installed skills ({}):", skills_dir.display());
    if let Ok(entries) = std::fs::read_dir(skills_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let skill_md = path.join("SKILL.md");
            if path.is_dir() && skill_md.exists() {
                let dir_name = path.file_name().unwrap_or_default().to_string_lossy();
                let first_line = std::fs::read_to_string(&skill_md)
                    .ok()
                    .and_then(|c| {
                        c.lines()
                            .find(|l| l.starts_with('#'))
                            .map(|l| l.trim_start_matches('#').trim().to_string())
                    })
                    .unwrap_or_else(|| "(no description)".to_string());
                println!("  • {} — {}", dir_name, first_line);
                count += 1;
            }
        }
    }
    if count == 0 {
        println!("  (no skills installed)");
    }
    Ok(())
}

pub(crate) fn run_browse(skills_dir: &Path) -> Result<(), AgentError> {
    if !skills_dir.exists() {
        println!("No skills directory found.");
        return Ok(());
    }
    println!("Skills Browser");
    println!("==============\n");
    let mut categories: std::collections::HashMap<String, Vec<(String, String)>> =
        std::collections::HashMap::new();
    if let Ok(entries) = std::fs::read_dir(skills_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let skill_md = path.join("SKILL.md");
            if path.is_dir() && skill_md.exists() {
                let dir_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let content = std::fs::read_to_string(&skill_md).unwrap_or_default();
                let first_line = content
                    .lines()
                    .find(|l| l.starts_with('#'))
                    .map(|l| l.trim_start_matches('#').trim().to_string())
                    .unwrap_or_else(|| "(no description)".to_string());
                let category = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "general".to_string());
                categories
                    .entry(category)
                    .or_default()
                    .push((dir_name, first_line));
            }
        }
    }
    for (category, skills) in &categories {
        println!("[{}]", category);
        for (name, desc) in skills {
            println!("  • {} — {}", name, desc);
        }
        println!();
    }
    if categories.is_empty() {
        println!("  (no skills installed)");
    }
    Ok(())
}
