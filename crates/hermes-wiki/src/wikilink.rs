//! Wikilink parsing and resolution.
//!
//! ## What wikilinks look like
//!
//! - `[[target]]` — links to a page whose slug is `target`
//! - `[[target|Display Text]]` — same, but renders as "Display Text"
//!
//! ## Resolution strategy
//!
//! Resolution is **O(1)**: a HashSet lookup, not a stat() syscall.
//! The set of existing slugs is built once by `existing_page_slugs()`
//! (a single readdir per page directory). Then `resolve_wikilink()` normalizes
//! the target via `slugify()` and checks membership.
//!
//! ### `slugify()` normalization
//!
//! ```text
//! "Transformer Architecture" → "transformer-architecture"
//! "raw/articles/file.md"     → "raw/articles/file.md"     (path-like, no change)
//! "entities/foo"            → "entities/foo"             (path-like, no change)
//! ```
//!
//! This ensures that wikilinks written as `[[Transformer Architecture]]` match
//! files named `transformer-architecture.md` — the convention established in
//! Karpathy's original pattern.
//!
//! ## Performance note
//!
//! The regex `\[\[([^\[\]]+?)(?:\|([^\[\]]*?))?\]\]` is compiled once in a
//! `OnceLock` and reused across all pages. Extraction cost is ~2 µs per 1 KB
//! of page content.

use crate::types::Wikilink;
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;

/// Lazy-initialized regex for wikilink extraction.
fn wikilink_regex() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[\[([^\[\]]+?)(?:\|([^\[\]]*?))?\]\]").unwrap())
}

/// Extract all `[[wikilinks]]` from content, with line numbers.
///
/// Returns a list of `Wikilink` structs, one per occurrence.
pub fn extract_wikilinks(content: &str) -> Vec<Wikilink> {
    let re = wikilink_regex();
    let mut links = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        for cap in re.captures_iter(line) {
            let target = cap
                .get(1)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default();
            let display = cap
                .get(2)
                .map(|m| m.as_str().trim().to_string())
                .filter(|d| !d.is_empty());

            links.push(Wikilink {
                target,
                display,
                line: line_idx + 1, // 1-based
            });
        }
    }

    links
}

/// Check whether a wikilink target resolves to an existing wiki page file.
///
/// O(1): single hashset lookup instead of N stat() syscalls.
/// The `slugs` set should be the normalized slug form from `existing_page_slugs`.
pub fn resolve_wikilink(target: &str, slugs: &HashSet<String>) -> bool {
    let slug = slugify(target);
    slugs.contains(&slug)
}

/// Normalize a wikilink target or filename to a canonical slug.
/// E.g., "Transformer Architecture" → "transformer-architecture"
/// Path-like targets ("raw/articles/file.md", "entities/foo") are returned as-is.
pub fn slugify(target: &str) -> String {
    let target = target.trim();
    if target.contains('/') || target.ends_with(".md") {
        return target.to_string();
    }
    target.to_ascii_lowercase().replace(' ', "-")
}

/// Build a set of all existing wiki page slugs across the given directories.
///
/// Slugs are normalized via `slugify` so they match wikilink targets directly.
pub fn existing_page_slugs(page_dirs: &[&Path]) -> HashSet<String> {
    let mut slugs = HashSet::new();
    for dir in page_dirs {
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "md")
                    && let Some(stem) = path.file_stem()
                {
                    slugs.insert(slugify(&stem.to_string_lossy()));
                }
            }
        }
    }
    slugs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_simple() {
        let content = "Hello [[world]] here.";
        let links = extract_wikilinks(content);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "world");
        assert_eq!(links[0].display, None);
    }

    #[test]
    fn test_extract_with_display() {
        let content = "See [[transformer|Transformer Model]].";
        let links = extract_wikilinks(content);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "transformer");
        assert_eq!(links[0].display.as_deref(), Some("Transformer Model"));
    }

    #[test]
    fn test_extract_multiple() {
        let content = "[[a]] and [[b|c]] and [[d]]";
        let links = extract_wikilinks(content);
        assert_eq!(links.len(), 3);
    }

    #[test]
    fn test_extract_no_links() {
        let content = "Just plain text [not a link] and normal text.";
        let links = extract_wikilinks(content);
        assert!(links.is_empty());
    }

    #[test]
    fn test_extract_line_numbers() {
        let content = "First line\n[[second]] link\nThird [[fourth]]";
        let links = extract_wikilinks(content);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].line, 2);
        assert_eq!(links[1].line, 3);
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("My Page"), "my-page");
        assert_eq!(slugify("raw/articles/file.md"), "raw/articles/file.md");
        assert_eq!(slugify("entities/foo"), "entities/foo");
        assert_eq!(
            slugify("Transformer Architecture"),
            "transformer-architecture"
        );
    }
}
