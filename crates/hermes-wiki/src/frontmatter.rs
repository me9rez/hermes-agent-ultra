//! YAML frontmatter parsing and serialization.
//!
//! ## Two-tier parsing strategy
//!
//! Full YAML deserialization (`serde_yaml`) is expensive — it builds a parse tree
//! and validates the entire schema. For the 90% use case (linting a page), we only
//! need 6 fields: `type`, `updated`, `tags`, `sources`, `confidence`, `contested`.
//!
//! - **Fast path** (`scan_lint_frontmatter`): key-value line scanner, no YAML parser.
//!   Handles `key: value` and `key: [a, b, c]` syntax. Runs in O(n) with a single
//!   pass over the frontmatter text. CPU cost: ~1 µs for a 10-line frontmatter.
//! - **Full path** (`parse_wiki_frontmatter` / `parse_raw_frontmatter`): delegates
//!   to `serde_yaml`. Used only when writing pages or when the CLI needs structured
//!   access (e.g., `hwiki frontmatter`). Cost: ~50 µs per page.
//!
//! ## SHA256 body hashing
//!
//! `hash_body()` computes SHA256 of content after the closing `---`.
//! This is used for source drift detection in raw/ files. The hash is over the
//! **body only** — frontmatter changes (e.g., updating `ingested` date) don't
//! trigger false drift warnings.
//!
//! ## File format
//!
//! ```markdown
//! ---
//! title: My Page
//! created: 2026-01-01
//! ---
//!
//! Page body here...
//! ```

use crate::error::{WikiError, WikiResult};
use crate::types::{RawFrontmatter, WikiFrontmatter, WikiPage};
use chrono::NaiveDate;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Parse YAML frontmatter and body from a markdown byte slice.
///
/// Returns `(Some(frontmatter_text), body_text)` if frontmatter delimiters are found,
/// or `(None, full_text)` if there's no frontmatter.
pub fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return (None, content);
    }

    // Find the closing `---` after the first line
    // We search for `\n---` (newline followed by closing delimiter)
    if let Some(end) = content.find("\n---") {
        let yaml_text = &content[3..end]; // skip opening `---`, up to the `\n` before closing `---`
        let body_start = end + 4; // skip `\n---`
        // Skip the newline(s) after closing `---`
        let body = &content[body_start..];
        let body = body.trim_start_matches(['\n', '\r']);
        (Some(yaml_text.trim()), body)
    } else {
        (None, content)
    }
}

/// Parse `WikiFrontmatter` from markdown content.
pub fn parse_wiki_frontmatter(content: &str) -> WikiResult<Option<WikiFrontmatter>> {
    let (yaml_text, _body) = split_frontmatter(content);
    match yaml_text {
        Some(yaml) => {
            let fm: WikiFrontmatter =
                serde_yaml::from_str(yaml).map_err(|e| WikiError::InvalidFrontmatter {
                    path: "<content>".into(),
                    detail: e.to_string(),
                })?;
            Ok(Some(fm))
        }
        None => Ok(None),
    }
}

/// Parse `RawFrontmatter` from markdown content.
pub fn parse_raw_frontmatter(content: &str) -> WikiResult<Option<RawFrontmatter>> {
    let (yaml_text, _body) = split_frontmatter(content);
    match yaml_text {
        Some(yaml) => {
            let fm: RawFrontmatter =
                serde_yaml::from_str(yaml).map_err(|e| WikiError::InvalidFrontmatter {
                    path: "<content>".into(),
                    detail: e.to_string(),
                })?;
            Ok(Some(fm))
        }
        None => Ok(None),
    }
}

/// Lightweight frontmatter fields extracted for lint — avoids full serde_yaml.
#[derive(Debug, Default, PartialEq)]
pub struct LintFrontmatter {
    pub page_type: Option<String>,
    pub updated: Option<NaiveDate>,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    pub confidence: Option<String>,
    pub contested: Option<bool>,
}

/// Scan frontmatter yaml text for the fields lint needs — no full YAML parse.
///
/// This is a key-value line scanner that handles:
/// - Simple values: `key: value`
/// - Bracket lists: `key: [a, b, c]`
/// - Absent fields (returns None for the optionals)
///
/// Falls back gracefully: returns partial results on malformed lines.
pub fn scan_lint_frontmatter(yaml_text: &str) -> LintFrontmatter {
    let mut fm = LintFrontmatter::default();
    for line in yaml_text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let colon = match line.find(':') {
            Some(p) => p,
            None => continue,
        };
        let key = line[..colon].trim();
        let raw_val = line[colon + 1..].trim();

        match key {
            "type" => fm.page_type = Some(raw_val.to_string()),
            "updated" => {
                if let Ok(d) = NaiveDate::parse_from_str(raw_val, "%Y-%m-%d") {
                    fm.updated = Some(d);
                }
            }
            "tags" => fm.tags = parse_bracket_list(raw_val),
            "sources" => fm.sources = parse_bracket_list(raw_val),
            "confidence" => {
                let c = raw_val.trim().to_lowercase();
                if matches!(c.as_str(), "high" | "medium" | "low") {
                    fm.confidence = Some(c);
                }
            }
            "contested" => {
                fm.contested = Some(raw_val.trim().eq_ignore_ascii_case("true"));
            }
            _ => {}
        }
    }
    fm
}

/// Parse a simple bracket-delimited list: `[a, b, c]` or `[a]`
fn parse_bracket_list(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    if raw.starts_with('[') && raw.ends_with(']') {
        let inner = &raw[1..raw.len() - 1];
        inner
            .split(',')
            .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    }
}

/// Compute SHA256 hash of the body content (everything after frontmatter).
pub fn hash_body(content: &str) -> String {
    let (_fm, body) = split_frontmatter(content);
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

/// Read and parse a wiki page from a file path.
pub fn read_wiki_page(path: &Path) -> WikiResult<WikiPage> {
    let content = std::fs::read_to_string(path)?;
    let line_count = content.lines().count();
    let (yaml_text, body) = split_frontmatter(&content);
    let frontmatter = match yaml_text {
        Some(yaml) => match serde_yaml::from_str::<WikiFrontmatter>(yaml) {
            Ok(fm) => Some(fm),
            Err(e) => {
                return Err(WikiError::InvalidFrontmatter {
                    path: path.display().to_string(),
                    detail: e.to_string(),
                });
            }
        },
        None => None,
    };

    Ok(WikiPage {
        path: path.to_path_buf(),
        rel_path: Path::new("").to_path_buf(), // caller should set this
        frontmatter,
        body: body.to_string(),
        line_count,
    })
}

/// Serialize a `WikiFrontmatter` back to YAML string with `---` delimiters.
pub fn serialize_frontmatter(fm: &WikiFrontmatter) -> WikiResult<String> {
    let yaml = serde_yaml::to_string(fm)?;
    Ok(format!("---\n{}---\n", yaml))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_frontmatter_basic() {
        let content = "---\ntitle: Test\ncreated: 2026-01-01\n---\n\nBody text here.";
        let (yaml, body) = split_frontmatter(content);
        assert_eq!(yaml, Some("title: Test\ncreated: 2026-01-01"));
        assert_eq!(body, "Body text here.");
    }

    #[test]
    fn test_split_frontmatter_no_frontmatter() {
        let content = "Just body text.";
        let (yaml, body) = split_frontmatter(content);
        assert_eq!(yaml, None);
        assert_eq!(body, "Just body text.");
    }

    #[test]
    fn test_split_frontmatter_incomplete() {
        let content = "---\ntitle: Test\nNo closing delimiter";
        let (yaml, body) = split_frontmatter(content);
        assert_eq!(yaml, None);
        assert_eq!(body, content);
    }

    #[test]
    fn test_parse_wiki_frontmatter() {
        let content = "---\ntitle: Transformer Architecture\ncreated: 2026-01-15\nupdated: 2026-02-01\ntype: concept\ntags: [model, architecture]\nsources: [raw/articles/attention-paper.md]\n---\n\n# Transformer\n\nBody.";
        let fm = parse_wiki_frontmatter(content).unwrap().unwrap();
        assert_eq!(fm.title, "Transformer Architecture");
        assert_eq!(fm.page_type, "concept");
        assert_eq!(fm.tags, vec!["model", "architecture"]);
    }

    #[test]
    fn test_hash_body() {
        let content = "---\ntitle: Test\n---\nHello World";
        let hash = hash_body(content);
        // SHA256 of "Hello World"
        assert_eq!(hash.len(), 64);
        // Same content should produce same hash
        assert_eq!(hash_body(content), hash);
        // Different body should produce different hash
        assert_ne!(hash_body("---\ntitle: Test\n---\nGoodbye World"), hash);
    }

    #[test]
    fn test_hash_ignores_frontmatter() {
        let a = "---\ntitle: A\n---\nBody";
        let b = "---\ntitle: B\n---\nBody";
        assert_eq!(hash_body(a), hash_body(b));
    }
}
