//! hermes-wiki — Karpathy's LLM Wiki: fast markdown knowledge base tooling.
//!
//! A Rust library and CLI for building, maintaining, and querying interlinked
//! markdown knowledge bases. Based on [Andrej Karpathy's LLM Wiki pattern](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f).
//!
//! ## Architecture
//!
//! ```text
//! wiki/
//! ├── SCHEMA.md        # Conventions, structure rules, domain config
//! ├── index.md         # Sectioned content catalog
//! ├── log.md           # Chronological action log
//! ├── raw/             # Layer 1: Immutable source material
//! │   ├── articles/
//! │   ├── papers/
//! │   ├── transcripts/
//! │   └── assets/
//! ├── entities/        # Layer 2: Entity pages
//! ├── concepts/        # Layer 2: Concept/topic pages
//! ├── comparisons/     # Layer 2: Side-by-side analyses
//! └── queries/         # Layer 2: Filed query results
//! ```
//!
//! ## Core Operations
//!
//! - **Init** — Create a new wiki directory structure with SCHEMA.md, index.md, log.md
//! - **Lint** — Health-check: orphan pages, broken wikilinks, stale content, source drift, etc.
//! - **Search** — Full-text regex search across all markdown files
//! - **Frontmatter** — Parse and validate YAML frontmatter
//! - **Hash** — SHA256 computation for source drift detection

pub mod error;
pub mod frontmatter;
pub mod hash;
pub mod init;
pub mod lint;
pub mod search;
pub mod types;
pub mod wikilink;

// Re-export commonly used types at the crate root.
pub use error::{WikiError, WikiResult};
pub use init::{ensure_wiki, init_wiki, is_wiki};
pub use lint::lint_wiki;
pub use search::{
    SearchIndex, SearchResult, build_search_index, search_titles, search_wiki, should_build_index,
};
pub use types::{LintFinding, WikiLayout, WikiPage, WikiStats};
