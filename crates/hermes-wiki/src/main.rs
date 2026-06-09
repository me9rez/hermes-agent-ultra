//! hwiki — Fast LLM Wiki CLI.
//!
//! CLI binary for the hermes-wiki library. Subcommands:
//!
//! - `init`    — Initialize a new wiki
//! - `lint`    — Health-check a wiki
//! - `search`  — Search wiki content
//! - `hash`    — Compute SHA256 of a file
//! - `stats`   — Show wiki statistics

use clap::{Parser, Subcommand};
use hermes_wiki::{
    build_search_index, ensure_wiki, hash::hash_file, hash::hash_file_body, init_wiki, lint_wiki,
    search_wiki, should_build_index, types::LintSeverity,
};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "hwiki", about = "Fast LLM Wiki tool", version, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new wiki at the given path
    Init {
        /// Path to create the wiki
        path: PathBuf,
        /// Domain description (optional)
        #[arg(short, long)]
        domain: Option<String>,
    },
    /// Run health checks on a wiki
    Lint {
        /// Path to the wiki (defaults to $WIKI_PATH or ~/wiki)
        #[arg(default_value = "")]
        path: PathBuf,
        /// Only show errors (suppress warnings and info)
        #[arg(short, long)]
        errors_only: bool,
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },
    /// Search wiki content
    Search {
        /// Pattern to search for (regex or literal)
        pattern: String,
        /// Path to the wiki (defaults to $WIKI_PATH or ~/wiki)
        #[arg(default_value = "")]
        path: PathBuf,
    },
    /// Compute SHA256 hash of a file
    Hash {
        /// File to hash
        path: PathBuf,
        /// Hash body only (exclude YAML frontmatter)
        #[arg(short, long)]
        body_only: bool,
    },
    /// Show wiki statistics
    Stats {
        /// Path to the wiki (defaults to $WIKI_PATH or ~/wiki)
        #[arg(default_value = "")]
        path: PathBuf,
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },
    /// Watch wiki for changes and re-lint automatically
    Watch {
        /// Path to the wiki (defaults to $WIKI_PATH or ~/wiki)
        #[arg(default_value = "")]
        path: PathBuf,
    },
}

fn resolve_wiki_path(path: &Path) -> PathBuf {
    if !path.as_os_str().is_empty() {
        return path.to_path_buf();
    }
    // Try WIKI_PATH env var, then default to ~/wiki
    if let Ok(env_path) = std::env::var("WIKI_PATH")
        && !env_path.is_empty()
    {
        return PathBuf::from(env_path);
    }
    dirs::home_dir()
        .map(|h| h.join("wiki"))
        .unwrap_or_else(|| PathBuf::from("wiki"))
}

fn main() {
    // Initialize tracing with a minimal filter for the CLI
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { path, domain } => match init_wiki(&path, domain.as_deref()) {
            Ok(layout) => {
                println!("✓ Wiki initialized at {}", layout.root.display());
                println!("  ├── SCHEMA.md");
                println!("  ├── index.md");
                println!("  ├── log.md");
                println!("  ├── raw/ (articles, papers, transcripts, assets)");
                println!("  ├── entities/");
                println!("  ├── concepts/");
                println!("  ├── comparisons/");
                println!("  └── queries/");
                println!();
                println!(
                    "Ready. Customize SCHEMA.md for your domain, then start ingesting sources."
                );
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        },

        Commands::Lint {
            path,
            errors_only,
            json,
        } => {
            let wiki_path = resolve_wiki_path(path.as_ref());
            match ensure_wiki(&wiki_path).and_then(|layout| lint_wiki(&layout)) {
                Ok((findings, stats)) => {
                    if json {
                        // JSON output
                        let output = serde_json::json!({
                            "wiki": wiki_path.display().to_string(),
                            "stats": {
                                "total_pages": stats.total_pages,
                                "total_raw_sources": stats.total_raw_sources,
                                "total_wikilinks": stats.total_wikilinks,
                                "orphan_pages": stats.orphan_pages,
                                "broken_links": stats.broken_links,
                                "pages_by_type": stats.pages_by_type,
                                "tags_in_use": stats.tags_in_use,
                            },
                            "findings": findings.iter().filter(|f| {
                                !errors_only || f.severity == LintSeverity::Error
                            }).map(|f| {
                                serde_json::json!({
                                    "severity": format!("{:?}", f.severity).to_lowercase(),
                                    "category": format!("{:?}", f.category),
                                    "message": f.message,
                                    "file": f.file,
                                    "suggestion": f.suggestion,
                                })
                            }).collect::<Vec<_>>(),
                        });
                        println!("{}", serde_json::to_string_pretty(&output).unwrap());
                    } else {
                        // Human-readable output
                        let error_count = findings
                            .iter()
                            .filter(|f| f.severity == LintSeverity::Error)
                            .count();
                        let warning_count = findings
                            .iter()
                            .filter(|f| f.severity == LintSeverity::Warning)
                            .count();
                        let info_count = findings
                            .iter()
                            .filter(|f| f.severity == LintSeverity::Info)
                            .count();

                        println!("Wiki: {}", wiki_path.display());
                        println!(
                            "Stats: {} pages, {} raw sources, {} wikilinks",
                            stats.total_pages, stats.total_raw_sources, stats.total_wikilinks
                        );
                        println!(
                            "       {} orphan pages, {} broken links",
                            stats.orphan_pages, stats.broken_links
                        );
                        println!(
                            "Issues: {} errors, {} warnings, {} info\n",
                            error_count, warning_count, info_count
                        );

                        // Print findings grouped by severity
                        for finding in &findings {
                            if errors_only && finding.severity != LintSeverity::Error {
                                continue;
                            }
                            let icon = match finding.severity {
                                LintSeverity::Error => "✖",
                                LintSeverity::Warning => "⚠",
                                LintSeverity::Info => "ℹ",
                            };
                            let file = finding.file.as_deref().unwrap_or("");
                            println!("  {} [{:?}] {}", icon, finding.category, finding.message);
                            if !file.is_empty() {
                                println!("     File: {}", file);
                            }
                            println!("     Suggestion: {}", finding.suggestion);
                            println!();
                        }

                        if findings.is_empty() {
                            println!("  ✓ No issues found!");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Search { pattern, path } => {
            let wiki_path = resolve_wiki_path(path.as_ref());
            let layout = match ensure_wiki(&wiki_path) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };
            // Build adaptive index for large wikis
            let index = if should_build_index(&layout) {
                build_search_index(&layout).ok()
            } else {
                None
            };
            match search_wiki(&layout, &pattern, index.as_ref()) {
                Ok(results) => {
                    if results.is_empty() {
                        println!("No matches for '{}'", pattern);
                        return;
                    }
                    println!("{} matches for '{}':\n", results.len(), pattern);
                    for result in &results {
                        println!(
                            "  {}:{}  {}",
                            result.file,
                            result.line,
                            result.content.trim()
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Hash { path, body_only } => {
            let result = if body_only {
                hash_file_body(&path)
            } else {
                hash_file(&path)
            };
            match result {
                Ok(hash) => println!("{}", hash),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Stats { path, json } => {
            let wiki_path = resolve_wiki_path(path.as_ref());
            match ensure_wiki(&wiki_path).and_then(|layout| lint_wiki(&layout)) {
                Ok((findings, stats)) => {
                    if json {
                        let output = serde_json::json!({
                            "wiki": wiki_path.display().to_string(),
                            "total_pages": stats.total_pages,
                            "total_raw_sources": stats.total_raw_sources,
                            "total_wikilinks": stats.total_wikilinks,
                            "orphan_pages": stats.orphan_pages,
                            "broken_links": stats.broken_links,
                            "pages_by_type": stats.pages_by_type,
                            "tags_in_use": stats.tags_in_use,
                            "errors": findings.iter().filter(|f| f.severity == LintSeverity::Error).count(),
                            "warnings": findings.iter().filter(|f| f.severity == LintSeverity::Warning).count(),
                        });
                        println!("{}", serde_json::to_string_pretty(&output).unwrap());
                    } else {
                        println!("Wiki: {}", wiki_path.display());
                        println!("  Total pages:      {}", stats.total_pages);
                        println!("  Raw sources:      {}", stats.total_raw_sources);
                        println!("  Wikilinks:        {}", stats.total_wikilinks);
                        println!("  Orphan pages:     {}", stats.orphan_pages);
                        println!("  Broken links:     {}", stats.broken_links);
                        println!();
                        if !stats.pages_by_type.is_empty() {
                            println!("  Pages by type:");
                            for (ptype, count) in &stats.pages_by_type {
                                println!("    {}: {}", ptype, count);
                            }
                        }
                        if !stats.tags_in_use.is_empty() {
                            println!("  Tags in use:");
                            let mut tags: Vec<_> = stats.tags_in_use.into_iter().collect();
                            tags.sort_by_key(|b| std::cmp::Reverse(b.1));
                            for (tag, count) in tags.iter().take(20) {
                                println!("    {}: {}", tag, count);
                            }
                            if tags.len() > 20 {
                                println!("    ... and {} more", tags.len() - 20);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Watch { path } => {
            let wiki_path = resolve_wiki_path(path.as_ref());
            let layout = match ensure_wiki(&wiki_path) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };

            println!(
                "👀 Watching {} for changes... (Ctrl+C to stop)",
                wiki_path.display()
            );

            // Run initial lint
            println!("\n--- Initial lint ---");
            if let Ok((findings, stats)) = lint_wiki(&layout) {
                let err_count = findings
                    .iter()
                    .filter(|f| f.severity == LintSeverity::Error)
                    .count();
                let warn_count = findings
                    .iter()
                    .filter(|f| f.severity == LintSeverity::Warning)
                    .count();
                println!(
                    "{} pages, {} errors, {} warnings\n",
                    stats.total_pages, err_count, warn_count
                );
            }

            // Set up filesystem watcher
            let (tx, rx) = mpsc::channel::<Result<Event, notify::Error>>();
            let mut watcher = match notify::recommended_watcher(tx) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("Error setting up file watcher: {}", e);
                    std::process::exit(1);
                }
            };

            if let Err(e) = watcher.watch(&wiki_path, RecursiveMode::Recursive) {
                eprintln!("Error watching directory: {}", e);
                std::process::exit(1);
            }

            for event in rx {
                if let Ok(Event {
                    kind: EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_),
                    ..
                }) = event
                {
                    println!("\n--- Change detected, re-linting ---");
                    if let Ok((findings, stats)) = lint_wiki(&layout) {
                        let err_count = findings
                            .iter()
                            .filter(|f| f.severity == LintSeverity::Error)
                            .count();
                        let warn_count = findings
                            .iter()
                            .filter(|f| f.severity == LintSeverity::Warning)
                            .count();
                        println!(
                            "{} pages, {} errors, {} warnings\n",
                            stats.total_pages, err_count, warn_count
                        );
                    }
                }
            }
        }
    }
}
