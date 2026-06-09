//! SHA256 hashing utilities for source file drift detection.
//!
//! All file hashing is streaming (BufReader, no full file alloc) to keep
//! L1 cache hot and heap pressure minimal even on large raw sources.
//!
//! ## Design rationale
//! - `hash_file` hashes the entire file byte-for-byte.
//! - `hash_file_body` skips YAML frontmatter by scanning for `\n---` in the
//!   first 64 KB, then streaming the remainder through SHA256.
//!   This avoids loading the entire file into a String just to find the
//!   frontmatter boundary.
//!
//! ## When to use each
//! - **Source drift detection** (lint): use `hash_body()` from `frontmatter.rs`
//!   when you've already read the file into a String for YAML parsing.
//!   Combine the read + hash in one pass when performance matters.
//! - **CLI** (`hwiki hash --body-only`): use `hash_file_body` — no String alloc.

use crate::error::{WikiError, WikiResult};
use sha2::{Digest, Sha256};
use std::io::{BufReader, Read};
use std::path::Path;

/// Buffer size for streaming hash reads — fits in L1 cache on modern CPUs.
const BUF_SIZE: usize = 8192;

/// Maximum frontmatter size we'll scan for the closing `---`.
/// Typical YAML frontmatter is < 4 KB; 64 KB is a generous upper bound.
const MAX_FRONTMATTER_SCAN: usize = 65536;

/// Compute SHA256 hash of a file's entire contents (streaming, no full alloc).
pub fn hash_file(path: &Path) -> WikiResult<String> {
    let file = std::fs::File::open(path)
        .map_err(|e| WikiError::Other(format!("Failed to open {}: {}", path.display(), e)))?;
    let mut reader = BufReader::with_capacity(BUF_SIZE, file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; BUF_SIZE];

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| WikiError::Other(format!("Failed to read {}: {}", path.display(), e)))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>())
}

/// Compute SHA256 hash of a file, excluding YAML frontmatter (streaming).
///
/// Scans for the first `\n---` delimiter to find the start of the body,
/// then streams the remainder through SHA256. Avoids allocating the full file.
///
/// If no valid frontmatter boundary is found within `MAX_FRONTMATTER_SCAN`
/// bytes, falls back to hashing the entire file (matching `split_frontmatter`).
pub fn hash_file_body(path: &Path) -> WikiResult<String> {
    let file = std::fs::File::open(path)
        .map_err(|e| WikiError::Other(format!("Failed to open {}: {}", path.display(), e)))?;
    let mut reader = BufReader::with_capacity(BUF_SIZE, file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; BUF_SIZE];

    // Read the first chunk to check for opening `---`
    let n = reader
        .read(&mut buf)
        .map_err(|e| WikiError::Other(format!("Failed to read {}: {}", path.display(), e)))?;
    if n == 0 {
        return Ok(String::new()); // empty file
    }

    // Check for opening `---` at byte 0
    let has_frontmatter = n >= 3 && buf[0] == b'-' && buf[1] == b'-' && buf[2] == b'-';

    if has_frontmatter {
        // Search for `\n---` in the buffer to find the closing delimiter.
        let mut body_offset = None;
        let scan_end = MAX_FRONTMATTER_SCAN.min(n);
        for i in 1..scan_end {
            if buf[i] == b'\n'
                && i + 3 < n
                && buf[i + 1] == b'-'
                && buf[i + 2] == b'-'
                && buf[i + 3] == b'-'
                // Verify `---` is followed by newline/EOF, not more dashes (avoids matching
                // inside a long dash-line like `----(1000x)---`).
                && (i + 4 >= n || buf[i + 4] != b'-')
            {
                // Found closing `\n---`. Body starts after the `---` and any trailing newline.
                let after_close = i + 4;
                // Skip one trailing \n or \r\n
                let body_start = if after_close < n
                    && (buf[after_close] == b'\n' || buf[after_close] == b'\r')
                {
                    after_close + 1
                } else {
                    after_close
                };
                body_offset = Some(body_start);
                break;
            }
        }

        match body_offset {
            Some(offset) => {
                // Hash the tail of the current buffer from body offset
                hasher.update(&buf[offset..n]);
                // If the closing delimiter was in a later chunk, we'd need to scan more.
                // But frontmatter is almost always < 8 KB, so this is very rare.
                // For correctness, we fall through to streaming the rest.
            }
            None => {
                // Closing `---` not found in first chunk. Either:
                // (a) frontmatter is larger than one buffer, or
                // (b) no valid frontmatter — hash as body (matching split_frontmatter).
                // We scan subsequent chunks up to MAX_FRONTMATTER_SCAN.
                let mut accumulated = n;
                // Track newline state across chunk boundaries for `\n---` detection.
                let mut prev_was_newline = buf[n - 1] == b'\n';

                loop {
                    let n2 = reader.read(&mut buf).map_err(|e| {
                        WikiError::Other(format!("Failed to read {}: {}", path.display(), e))
                    })?;
                    if n2 == 0 {
                        break;
                    }
                    accumulated += n2;
                    if accumulated > MAX_FRONTMATTER_SCAN {
                        break;
                    }

                    for j in 0..n2 {
                        if prev_was_newline
                            && buf[j] == b'-'
                            && j + 2 < n2
                            && buf[j + 1] == b'-'
                            && buf[j + 2] == b'-'
                            && (j + 3 >= n2 || buf[j + 3] != b'-')
                        {
                            // Found `\n---`. Body starts after.
                            let after_close = j + 3;
                            let body_start = if after_close < n2
                                && (buf[after_close] == b'\n' || buf[after_close] == b'\r')
                            {
                                after_close + 1
                            } else {
                                after_close
                            };
                            if body_start < n2 {
                                hasher.update(&buf[body_start..n2]);
                            }
                            // Stream remaining chunks
                            loop {
                                let n3 = reader.read(&mut buf).map_err(|e| {
                                    WikiError::Other(format!(
                                        "Failed to read {}: {}",
                                        path.display(),
                                        e
                                    ))
                                })?;
                                if n3 == 0 {
                                    break;
                                }
                                hasher.update(&buf[..n3]);
                            }
                            return Ok(hasher
                                .finalize()
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<String>());
                        }
                        prev_was_newline = buf[j] == b'\n';
                    }
                }
                // No valid frontmatter boundary found: hash the whole file as body.
                // Re-read from the start (buffered reader may have consumed data).
                // This is the fallback matching split_frontmatter behavior.
                drop(reader);
                let file = std::fs::File::open(path).map_err(|e| {
                    WikiError::Other(format!("Failed to open {}: {}", path.display(), e))
                })?;
                let mut reader = BufReader::with_capacity(BUF_SIZE, file);
                let mut hasher = Sha256::new();
                let mut buf = [0u8; BUF_SIZE];
                loop {
                    let n = reader.read(&mut buf).map_err(|e| {
                        WikiError::Other(format!("Failed to read {}: {}", path.display(), e))
                    })?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                return Ok(hasher
                    .finalize()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>());
            }
        }
    } else {
        // No opening `---` — whole file is body
        hasher.update(&buf[..n]);
    }

    // Stream remaining chunks
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| WikiError::Other(format!("Failed to read {}: {}", path.display(), e)))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_file_streaming() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "Hello World").unwrap();

        let hash = hash_file(&path).unwrap();
        assert_eq!(hash.len(), 64);
        // Verify deterministic
        assert_eq!(hash_file(&path).unwrap(), hash);
    }

    #[test]
    fn test_hash_file_body_excludes_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "---\ntitle: Test\n---\nActual content").unwrap();

        let hash = hash_file_body(&path).unwrap();
        // SHA256 of "Actual content"
        let mut hasher = Sha256::new();
        hasher.update(b"Actual content");
        let expected: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_hash_file_body_no_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.md");
        std::fs::write(&path, "Just body text").unwrap();

        let hash = hash_file_body(&path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(b"Just body text");
        let expected: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_hash_file_body_large_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.md");
        // Create a frontmatter with 1000 dashes (filling a buffer)
        let dashes = "-".repeat(1000);
        let content = format!("---\nkey: value\n{}\n---\nBody content here", dashes);
        std::fs::write(&path, &content).unwrap();

        let streaming_hash = hash_file_body(&path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(b"Body content here");
        let expected: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(streaming_hash, expected);
    }
}
