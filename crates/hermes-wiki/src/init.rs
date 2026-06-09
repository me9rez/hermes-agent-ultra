//! Wiki initialization: create the directory structure and starter files.

use crate::error::{WikiError, WikiResult};
use crate::types::WikiLayout;
use chrono::Utc;
use std::fs;
use std::path::Path;

/// Default SCHEMA.md content template.
const DEFAULT_SCHEMA: &str = r#"# Wiki Schema

## Domain
[What this wiki covers — e.g., "AI/ML research", "personal knowledge base"]

## Conventions
- File names: lowercase, hyphens, no spaces (e.g., `transformer-architecture.md`)
- Every wiki page starts with YAML frontmatter (see below)
- Use `[[wikilinks]]` to link between pages (minimum 2 outbound links per page)
- When updating a page, always bump the `updated` date
- Every new page must be added to `index.md` under the correct section
- Every action must be appended to `log.md`

## Frontmatter
  ```yaml
  ---
  title: Page Title
  created: YYYY-MM-DD
  updated: YYYY-MM-DD
  type: entity | concept | comparison | query | summary
  tags: [tag1, tag2]
  sources: [raw/articles/source-name.md]
  confidence: high | medium | low
  contested: true
  ---
  ```

## Tag Taxonomy
- Models: model, architecture, benchmark, training
- People/Orgs: person, company, lab, open-source
- Techniques: optimization, fine-tuning, inference, alignment, data
- Meta: comparison, timeline, controversy, prediction

## Page Thresholds
- Create a page when an entity/concept appears in 2+ sources OR is central to one source
- DON'T create for passing mentions
- Split when a page exceeds ~200 lines
"#;

/// Default index.md content template.
fn default_index(today: &str) -> String {
    format!(
        r#"# Wiki Index

> Content catalog. Every wiki page listed under its type with a one-line summary.
> Last updated: {} | Total pages: 0

## Entities

<!-- Alphabetical within section -->

## Concepts

## Comparisons

## Queries
"#,
        today
    )
}

/// Default log.md content template.
fn default_log(today: &str, domain: &str) -> String {
    format!(
        r#"# Wiki Log

> Chronological record of all wiki actions. Append-only.
> Format: `## [YYYY-MM-DD] action | subject`

## [{}] create | Wiki initialized
- Domain: {}
- Structure created with SCHEMA.md, index.md, log.md
"#,
        today, domain
    )
}

/// Initialize a new wiki at the given path.
///
/// Creates the full directory structure and starter files (SCHEMA.md, index.md, log.md).
/// Returns an error if the path already exists and is non-empty.
pub fn init_wiki(path: &Path, domain: Option<&str>) -> WikiResult<WikiLayout> {
    if path.exists() {
        // Check if it's already a wiki or non-empty
        let layout = WikiLayout::new(path.to_path_buf());
        if layout.schema.exists() || layout.index.exists() {
            return Err(WikiError::AlreadyExists(
                path.display().to_string(),
            ));
        }
        let has_files = fs::read_dir(path)
            .map(|mut d| d.any(|e| e.is_ok()))
            .unwrap_or(false);
        if has_files {
            return Err(WikiError::AlreadyExists(
                format!("{} is not empty", path.display()),
            ));
        }
    }

    let layout = WikiLayout::new(path.to_path_buf());
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let domain = domain.unwrap_or("Personal knowledge base");

    // Create directory structure
    fs::create_dir_all(&layout.raw_articles)?;
    fs::create_dir_all(&layout.raw_papers)?;
    fs::create_dir_all(&layout.raw_transcripts)?;
    fs::create_dir_all(&layout.raw_assets)?;
    fs::create_dir_all(&layout.entities)?;
    fs::create_dir_all(&layout.concepts)?;
    fs::create_dir_all(&layout.comparisons)?;
    fs::create_dir_all(&layout.queries)?;

    // Write starter files
    fs::write(&layout.schema, DEFAULT_SCHEMA)?;
    fs::write(&layout.index, default_index(&today))?;
    fs::write(&layout.log, default_log(&today, domain))?;

    Ok(layout)
}

/// Check if a directory looks like a wiki (has SCHEMA.md and index.md).
pub fn is_wiki(path: &Path) -> bool {
    let layout = WikiLayout::new(path.to_path_buf());
    layout.schema.exists() && layout.index.exists()
}

/// Validate that a path is an initialized wiki.
pub fn ensure_wiki(path: &Path) -> WikiResult<WikiLayout> {
    if !path.exists() {
        return Err(WikiError::NotAWiki(path.display().to_string()));
    }
    let layout = WikiLayout::new(path.to_path_buf());
    if !layout.schema.exists() {
        return Err(WikiError::NotAWiki(format!(
            "{}: missing SCHEMA.md",
            path.display()
        )));
    }
    if !layout.index.exists() {
        return Err(WikiError::NotAWiki(format!(
            "{}: missing index.md",
            path.display()
        )));
    }
    Ok(layout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_wiki() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-wiki");
        let layout = init_wiki(&path, Some("Test domain")).unwrap();

        assert!(layout.schema.exists());
        assert!(layout.index.exists());
        assert!(layout.log.exists());
        assert!(layout.entities.exists());
        assert!(layout.concepts.exists());
        assert!(layout.comparisons.exists());
        assert!(layout.queries.exists());
        assert!(layout.raw_articles.exists());
        assert!(layout.raw_papers.exists());
        assert!(layout.raw_transcripts.exists());
        assert!(layout.raw_assets.exists());
    }

    #[test]
    fn test_init_wiki_twice_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-wiki");
        init_wiki(&path, None).unwrap();
        let result = init_wiki(&path, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_is_wiki() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-wiki");
        assert!(!is_wiki(&path));
        init_wiki(&path, None).unwrap();
        assert!(is_wiki(&path));
    }

    #[test]
    fn test_ensure_wiki() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-wiki");
        let result = ensure_wiki(&path);
        assert!(result.is_err());
        init_wiki(&path, None).unwrap();
        assert!(ensure_wiki(&path).is_ok());
    }
}
