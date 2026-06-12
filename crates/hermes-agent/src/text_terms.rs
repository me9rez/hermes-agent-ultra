//! Shared tokenization helpers for memory fusion and recall keyword extraction.

/// Lowercase alphanumeric tokens of length >= 3 (whitespace split, punctuation trimmed).
pub(crate) fn query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|tok| tok.trim_matches(|c: char| !c.is_alphanumeric()))
        .map(|tok| tok.to_ascii_lowercase())
        .filter(|tok| tok.len() >= 3)
        .collect()
}
