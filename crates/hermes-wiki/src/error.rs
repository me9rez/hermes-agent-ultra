//! Error types for wiki operations.

use thiserror::Error;

/// Errors that can occur during wiki operations.
#[derive(Debug, Error)]
pub enum WikiError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("Not a wiki directory: {0}")]
    NotAWiki(String),

    #[error("Wiki already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid frontmatter in {path}: {detail}")]
    InvalidFrontmatter { path: String, detail: String },

    #[error("Page not found: {0}")]
    PageNotFound(String),

    #[error("Invalid page type: {0}")]
    InvalidPageType(String),

    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),

    #[error("{0}")]
    Other(String),
}

pub type WikiResult<T> = std::result::Result<T, WikiError>;
