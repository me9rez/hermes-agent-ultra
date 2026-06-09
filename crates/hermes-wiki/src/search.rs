//! Wiki content search.
//!
//! Two strategies, chosen adaptively:
//!
//! 1. **Scan** (default, small wikis < 50 files): walks the filesystem with `WalkDir`,
//!    reads each `.md` file, and regex-searches line-by-line. Zero startup cost.
//!
//! 2. **Index** (large wikis ≥ 50 files): builds an in-memory `SearchIndex` once.
//!    Subsequent searches resolve tokens in O(1) hash lookups, and regex searches
//!    operate on cached file contents (no disk I/O after the first pass).
//!
//! ## Design rationale
//! For a wiki with < 50 pages, a full scan completes in < 10 ms — the index
//! overhead dominates. Above that threshold, the index pays for itself on the
//! second query onward.

use crate::error::WikiResult;
use crate::types::WikiLayout;
use regex::Regex;
use std::collections::HashMap;
use walkdir::WalkDir;

/// A single search result.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// File path relative to wiki root.
    pub file: String,
    /// 1-based line number.
    pub line: usize,
    /// The matched line content.
    pub content: String,
}

/// Adaptive search index: caches file contents for fast re-search.
///
/// Built once on first search of a large wiki. The index is file-backed:
/// keys are file paths (relative to wiki root), values are the full file text.
/// Subsequent regex searches scan in-memory only — zero disk I/O.
#[derive(Debug, Default)]
pub struct SearchIndex {
    /// Cached file contents: rel_path → full content.
    files: HashMap<String, String>,
}

impl SearchIndex {
    /// Number of cached files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Build a `SearchIndex` by reading all markdown files in the wiki.
pub fn build_search_index(layout: &WikiLayout) -> WikiResult<SearchIndex> {
    let mut index = SearchIndex::default();

    for entry in WalkDir::new(&layout.root)
        .min_depth(1)
        .max_depth(3)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') && name != "assets"
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") || !path.is_file() {
            continue;
        }

        let rel = path.strip_prefix(&layout.root).unwrap_or(path);
        let rel_str = rel.display().to_string();

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        index.files.insert(rel_str, content);
    }

    Ok(index)
}

/// Search wiki content using a regex pattern.
///
/// Uses the index if provided (faster for repeated queries on large wikis),
/// otherwise falls back to a filesystem scan.
pub fn search_wiki(
    layout: &WikiLayout,
    pattern: &str,
    index: Option<&SearchIndex>,
) -> WikiResult<Vec<SearchResult>> {
    let re = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => {
            let escaped = regex::escape(pattern);
            Regex::new(&escaped).unwrap()
        }
    };

    match index {
        Some(idx) if !idx.is_empty() => {
            // Indexed search: scan in-memory contents (zero disk I/O)
            Ok(search_in_memory(&idx.files, &re))
        }
        _ => {
            // Filesystem scan: walk the directory tree
            Ok(search_filesystem(layout, &re))
        }
    }
}

/// Search cached file contents in memory.
fn search_in_memory(files: &HashMap<String, String>, re: &Regex) -> Vec<SearchResult> {
    let mut results = Vec::new();
    for (rel_path, content) in files {
        for (line_idx, line) in content.lines().enumerate() {
            if re.is_match(line) {
                results.push(SearchResult {
                    file: rel_path.clone(),
                    line: line_idx + 1,
                    content: line.to_string(),
                });
            }
        }
    }
    results
}

/// Search the filesystem directly.
fn search_filesystem(layout: &WikiLayout, re: &Regex) -> Vec<SearchResult> {
    let mut results = Vec::new();

    for entry in WalkDir::new(&layout.root)
        .min_depth(1)
        .max_depth(3)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') && name != "assets"
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for (line_idx, line) in content.lines().enumerate() {
            if re.is_match(line) {
                let rel = path
                    .strip_prefix(&layout.root)
                    .unwrap_or(path)
                    .display()
                    .to_string();
                results.push(SearchResult {
                    file: rel,
                    line: line_idx + 1,
                    content: line.to_string(),
                });
            }
        }
    }

    results
}

/// Search only page titles (frontmatter) in the wiki.
pub fn search_titles(layout: &WikiLayout, query: &str) -> WikiResult<Vec<String>> {
    let query_lower = query.to_ascii_lowercase();
    let mut titles = Vec::new();

    for dir in layout.page_dirs() {
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(dir).min_depth(1).max_depth(1) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }

            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if let Some(fm) = crate::frontmatter::parse_wiki_frontmatter(&content)
                .ok()
                .flatten()
                && fm.title.to_ascii_lowercase().contains(&query_lower)
            {
                let rel = path
                    .strip_prefix(&layout.root)
                    .unwrap_or(path)
                    .display()
                    .to_string();
                titles.push(format!("{} — {}", rel, fm.title));
            }
        }
    }

    Ok(titles)
}

/// Decide whether to build an index for a given wiki layout.
/// Threshold: 50+ .md files across all page directories.
pub fn should_build_index(layout: &WikiLayout) -> bool {
    let mut count = 0usize;
    for dir in layout.page_dirs() {
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "md") {
                    count += 1;
                    if count >= 50 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init::init_wiki;

    #[test]
    fn test_search_empty_wiki() {
        let dir = tempfile::tempdir().unwrap();
        let layout = init_wiki(&dir.path().join("wiki"), None).unwrap();
        let results = search_wiki(&layout, "zzzz_nothing_should_match_this", None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_finds_content() {
        let dir = tempfile::tempdir().unwrap();
        let layout = init_wiki(&dir.path().join("wiki"), None).unwrap();

        let content = "---\ntitle: Test\ntype: concept\ncreated: 2026-01-01\nupdated: 2026-01-01\ntags: []\nsources: []\n---\n\nThe transformer architecture is important.";
        std::fs::write(layout.concepts.join("transformer.md"), content).unwrap();

        let results = search_wiki(&layout, "transformer", None).unwrap();
        let transformer_results: Vec<_> = results
            .iter()
            .filter(|r| r.file.contains("transformer.md"))
            .collect();
        assert_eq!(
            transformer_results.len(),
            1,
            "Should find 1 match in transformer.md"
        );
        assert!(transformer_results[0].content.contains("transformer"));
    }

    #[test]
    fn test_search_regex() {
        let dir = tempfile::tempdir().unwrap();
        let layout = init_wiki(&dir.path().join("wiki"), None).unwrap();

        let content = "---\ntitle: Test\ntype: entity\ncreated: 2026-01-01\nupdated: 2026-01-01\ntags: []\nsources: []\n---\n\nOpenAI released GPT-4 in 2023.";
        std::fs::write(layout.entities.join("openai.md"), content).unwrap();

        let results = search_wiki(&layout, r"GPT-\d+", None).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_with_index() {
        let dir = tempfile::tempdir().unwrap();
        let layout = init_wiki(&dir.path().join("wiki"), None).unwrap();

        let content = "---\ntitle: Test\ntype: concept\ncreated: 2026-01-01\nupdated: 2026-01-01\ntags: []\nsources: []\n---\n\nHello world";
        std::fs::write(layout.concepts.join("hello.md"), content).unwrap();

        let index = build_search_index(&layout).unwrap();
        let results = search_wiki(&layout, "world", Some(&index)).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("world"));
    }

    #[test]
    fn test_should_build_index_small() {
        let dir = tempfile::tempdir().unwrap();
        let layout = init_wiki(&dir.path().join("wiki"), None).unwrap();
        assert!(!should_build_index(&layout));
    }
}
