//! Core data types for the LLM Wiki system.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Page classification for wiki pages (Layer 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageType {
    /// A notable entity (person, org, product, model).
    Entity,
    /// A concept or topic.
    Concept,
    /// A side-by-side analysis.
    Comparison,
    /// A filed query result.
    Query,
    /// A summary page.
    Summary,
}

impl PageType {
    /// Returns the directory name under the wiki root where pages of this type live.
    pub fn dir_name(&self) -> &'static str {
        match self {
            PageType::Entity => "entities",
            PageType::Concept => "concepts",
            PageType::Comparison => "comparisons",
            PageType::Query => "queries",
            PageType::Summary => "concepts", // summaries co-locate with concepts
        }
    }

    /// Parse from a string (from YAML frontmatter `type:` field).
    pub fn parse_type(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "entity" => Some(PageType::Entity),
            "concept" => Some(PageType::Concept),
            "comparison" => Some(PageType::Comparison),
            "query" => Some(PageType::Query),
            "summary" => Some(PageType::Summary),
            _ => None,
        }
    }
}

/// Confidence level for claims on a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// Well-supported across multiple sources.
    High,
    /// Moderately supported.
    Medium,
    /// Single-source or speculative.
    Low,
}

impl Confidence {
    pub fn parse_confidence(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "high" => Some(Confidence::High),
            "medium" => Some(Confidence::Medium),
            "low" => Some(Confidence::Low),
            _ => None,
        }
    }
}

/// YAML frontmatter for wiki pages (Layer 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiFrontmatter {
    pub title: String,
    pub created: NaiveDate,
    pub updated: NaiveDate,
    #[serde(rename = "type")]
    pub page_type: String,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contested: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contradictions: Option<Vec<String>>,
}

/// YAML frontmatter for raw source files (Layer 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawFrontmatter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    pub ingested: NaiveDate,
    pub sha256: String,
}

/// Represents a parsed wiki page with its frontmatter and body.
#[derive(Debug, Clone)]
pub struct WikiPage {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// Relative path from wiki root.
    pub rel_path: PathBuf,
    /// Parsed frontmatter, if present and valid.
    pub frontmatter: Option<WikiFrontmatter>,
    /// Raw page body (everything after the closing `---`).
    pub body: String,
    /// Line count of the full file.
    pub line_count: usize,
}

/// Represents a raw source file with its frontmatter.
#[derive(Debug, Clone)]
pub struct RawSource {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// Relative path from raw/ dir.
    pub rel_path: PathBuf,
    /// Parsed frontmatter.
    pub frontmatter: RawFrontmatter,
    /// Raw body (everything after the closing `---`).
    pub body: String,
}

/// A `[[wikilink]]` extracted from page content.
#[derive(Debug, Clone)]
pub struct Wikilink {
    /// The target slug (e.g., "transformer-architecture" from [[transformer-architecture]]).
    pub target: String,
    /// The display text, if specified via `[[page|display text]]`.
    pub display: Option<String>,
    /// Line number where this link appears (1-based).
    pub line: usize,
}

/// A lint finding.
#[derive(Debug, Clone)]
pub struct LintFinding {
    /// Severity level.
    pub severity: LintSeverity,
    /// Category of the finding.
    pub category: LintCategory,
    /// Human-readable description.
    pub message: String,
    /// Path of the affected file, relative to wiki root.
    pub file: Option<String>,
    /// Suggested action.
    pub suggestion: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintCategory {
    BrokenLink,
    OrphanPage,
    MissingFromIndex,
    InvalidFrontmatter,
    StaleContent,
    OversizedPage,
    InvalidTag,
    SourceDrift,
    LogRotationNeeded,
    MissingConfidence,
}

/// Summary statistics for a wiki.
#[derive(Debug, Clone, Default)]
pub struct WikiStats {
    pub total_pages: usize,
    pub total_raw_sources: usize,
    pub total_wikilinks: usize,
    pub orphan_pages: usize,
    pub broken_links: usize,
    pub pages_by_type: std::collections::HashMap<String, usize>,
    pub tags_in_use: std::collections::HashMap<String, usize>,
}

/// The wiki directory structure.
#[derive(Debug, Clone)]
pub struct WikiLayout {
    pub root: PathBuf,
    pub raw: PathBuf,
    pub raw_articles: PathBuf,
    pub raw_papers: PathBuf,
    pub raw_transcripts: PathBuf,
    pub raw_assets: PathBuf,
    pub entities: PathBuf,
    pub concepts: PathBuf,
    pub comparisons: PathBuf,
    pub queries: PathBuf,
    pub schema: PathBuf,
    pub index: PathBuf,
    pub log: PathBuf,
}

impl WikiLayout {
    /// Resolve all paths relative to the wiki root.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: root.clone(),
            raw: root.join("raw"),
            raw_articles: root.join("raw").join("articles"),
            raw_papers: root.join("raw").join("papers"),
            raw_transcripts: root.join("raw").join("transcripts"),
            raw_assets: root.join("raw").join("assets"),
            entities: root.join("entities"),
            concepts: root.join("concepts"),
            comparisons: root.join("comparisons"),
            queries: root.join("queries"),
            schema: root.join("SCHEMA.md"),
            index: root.join("index.md"),
            log: root.join("log.md"),
        }
    }

    /// Returns all layer-2 (wiki page) directories.
    pub fn page_dirs(&self) -> Vec<&PathBuf> {
        vec![
            &self.entities,
            &self.concepts,
            &self.comparisons,
            &self.queries,
        ]
    }

    /// Returns the directory for a given page type.
    pub fn dir_for(&self, page_type: &PageType) -> &PathBuf {
        match page_type {
            PageType::Entity => &self.entities,
            PageType::Concept | PageType::Summary => &self.concepts,
            PageType::Comparison => &self.comparisons,
            PageType::Query => &self.queries,
        }
    }
}
