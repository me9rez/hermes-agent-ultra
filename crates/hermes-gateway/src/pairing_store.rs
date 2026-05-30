//! Python-compatible DM pairing store (`~/.hermes/platforms/pairing`).
//!
//! Supports code-based DM approval flow:
//! - `generate_code(platform, user_id, user_name)`
//! - `approve_code(platform, code)`
//! - `is_approved(platform, user_id)`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LENGTH: usize = 8;
const CODE_TTL_SECONDS: f64 = 3600.0;
const RATE_LIMIT_SECONDS: f64 = 600.0;
const LOCKOUT_SECONDS: f64 = 3600.0;
const MAX_PENDING_PER_PLATFORM: usize = 3;
const MAX_FAILED_ATTEMPTS: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingEntry {
    hash: String,
    salt: String,
    user_id: String,
    #[serde(default)]
    user_name: String,
    created_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApprovedEntry {
    #[serde(default)]
    user_name: String,
    approved_at: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedUser {
    pub user_id: String,
    pub user_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPairing {
    pub platform: String,
    pub code: String,
    pub user_id: String,
    pub user_name: String,
    pub age_minutes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovedPairing {
    pub platform: String,
    pub user_id: String,
    pub user_name: String,
    pub approved_at: f64,
}

#[derive(Debug, Clone)]
pub struct DmPairingStore {
    root_dir: PathBuf,
}

impl DmPairingStore {
    fn encode_hex(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{:02x}", b);
        }
        out
    }

    fn decode_hex(raw: &str) -> Option<Vec<u8>> {
        let raw = raw.trim();
        if raw.len() % 2 != 0 {
            return None;
        }
        let mut out = Vec::with_capacity(raw.len() / 2);
        let bytes = raw.as_bytes();
        let to_val = |c: u8| -> Option<u8> {
            match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                b'A'..=b'F' => Some(c - b'A' + 10),
                _ => None,
            }
        };
        for idx in (0..bytes.len()).step_by(2) {
            let hi = to_val(bytes[idx])?;
            let lo = to_val(bytes[idx + 1])?;
            out.push((hi << 4) | lo);
        }
        Some(out)
    }

    pub fn new(root_dir: PathBuf) -> Self {
        Self { root_dir }
    }

    pub fn default_root() -> PathBuf {
        hermes_config::hermes_home()
            .join("platforms")
            .join("pairing")
    }

    pub fn open_default() -> Self {
        Self::new(Self::default_root())
    }

    fn pending_path(&self, platform: &str) -> PathBuf {
        self.root_dir.join(format!("{platform}-pending.json"))
    }

    fn approved_path(&self, platform: &str) -> PathBuf {
        self.root_dir.join(format!("{platform}-approved.json"))
    }

    fn rate_limit_path(&self) -> PathBuf {
        self.root_dir.join("_rate_limits.json")
    }

    fn now_ts() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    fn ensure_parent(path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create dir failed: {e}"))?;
        }
        Ok(())
    }

    fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> T
    where
        T: Default,
    {
        if !path.exists() {
            return T::default();
        }
        let Ok(raw) = std::fs::read_to_string(path) else {
            return T::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
        Self::ensure_parent(path)?;
        let data =
            serde_json::to_string_pretty(value).map_err(|e| format!("json encode failed: {e}"))?;
        std::fs::write(path, data).map_err(|e| format!("write {} failed: {e}", path.display()))
    }

    fn hash_code(code: &str, salt: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(salt);
        hasher.update(code.as_bytes());
        let digest = hasher.finalize();
        Self::encode_hex(&digest)
    }

    fn random_code() -> String {
        let mut out = String::with_capacity(CODE_LENGTH);
        let bytes = Uuid::new_v4().as_bytes().to_vec();
        for i in 0..CODE_LENGTH {
            let b = bytes.get(i % bytes.len()).copied().unwrap_or(0);
            out.push(ALPHABET[(b as usize) % ALPHABET.len()] as char);
        }
        out
    }

    fn cleanup_expired(&self, platform: &str) -> Result<(), String> {
        let path = self.pending_path(platform);
        let mut pending: HashMap<String, PendingEntry> = Self::read_json(&path);
        let before = pending.len();
        let now = Self::now_ts();
        pending.retain(|_, v| (now - v.created_at) <= CODE_TTL_SECONDS);
        if pending.len() != before {
            Self::write_json(&path, &pending)?;
        }
        Ok(())
    }

    fn is_rate_limited(&self, platform: &str, user_id: &str) -> bool {
        let limits: HashMap<String, f64> = Self::read_json(&self.rate_limit_path());
        let key = format!("{platform}:{}", user_id.trim());
        limits
            .get(&key)
            .is_some_and(|last| (Self::now_ts() - *last) < RATE_LIMIT_SECONDS)
    }

    fn record_rate_limit(&self, platform: &str, user_id: &str) -> Result<(), String> {
        let path = self.rate_limit_path();
        let mut limits: HashMap<String, f64> = Self::read_json(&path);
        limits.insert(format!("{platform}:{}", user_id.trim()), Self::now_ts());
        Self::write_json(&path, &limits)
    }

    fn is_locked_out(&self, platform: &str) -> bool {
        let limits: HashMap<String, f64> = Self::read_json(&self.rate_limit_path());
        let key = format!("_lockout:{platform}");
        limits
            .get(&key)
            .is_some_and(|until| Self::now_ts() < *until)
    }

    fn record_failed_attempt(&self, platform: &str) -> Result<(), String> {
        let path = self.rate_limit_path();
        let mut limits: HashMap<String, f64> = Self::read_json(&path);
        let fail_key = format!("_failures:{platform}");
        let failures = limits.get(&fail_key).copied().unwrap_or(0.0) as u64 + 1;
        limits.insert(fail_key.clone(), failures as f64);
        if failures >= MAX_FAILED_ATTEMPTS {
            limits.insert(
                format!("_lockout:{platform}"),
                Self::now_ts() + LOCKOUT_SECONDS,
            );
            limits.insert(fail_key, 0.0);
        }
        Self::write_json(&path, &limits)
    }

    pub fn is_approved(&self, platform: &str, user_id: &str) -> bool {
        let approved: HashMap<String, ApprovedEntry> =
            Self::read_json(&self.approved_path(platform));
        approved.contains_key(user_id.trim())
    }

    pub fn generate_code(
        &self,
        platform: &str,
        user_id: &str,
        user_name: &str,
    ) -> Result<Option<String>, String> {
        let platform = platform.trim().to_ascii_lowercase();
        let user_id = user_id.trim();
        if platform.is_empty() || user_id.is_empty() {
            return Ok(None);
        }
        self.cleanup_expired(&platform)?;
        if self.is_locked_out(&platform) || self.is_rate_limited(&platform, user_id) {
            return Ok(None);
        }

        let pending_path = self.pending_path(&platform);
        let mut pending: HashMap<String, PendingEntry> = Self::read_json(&pending_path);
        if pending.len() >= MAX_PENDING_PER_PLATFORM {
            return Ok(None);
        }

        let code = Self::random_code();
        let salt = Uuid::new_v4().as_bytes().to_vec();
        let entry_id = Uuid::new_v4().simple().to_string();
        pending.insert(
            entry_id,
            PendingEntry {
                hash: Self::hash_code(&code, &salt),
                salt: Self::encode_hex(&salt),
                user_id: user_id.to_string(),
                user_name: user_name.to_string(),
                created_at: Self::now_ts(),
            },
        );

        Self::write_json(&pending_path, &pending)?;
        self.record_rate_limit(&platform, user_id)?;
        Ok(Some(code))
    }

    pub fn approve_code(&self, platform: &str, code: &str) -> Result<Option<ApprovedUser>, String> {
        let platform = platform.trim().to_ascii_lowercase();
        let code = code.trim().to_ascii_uppercase();
        if platform.is_empty() || code.is_empty() {
            return Ok(None);
        }
        self.cleanup_expired(&platform)?;
        if self.is_locked_out(&platform) {
            return Ok(None);
        }

        let pending_path = self.pending_path(&platform);
        let mut pending: HashMap<String, PendingEntry> = Self::read_json(&pending_path);
        let mut matched_id: Option<String> = None;
        let mut matched: Option<PendingEntry> = None;
        for (k, v) in &pending {
            let Some(salt) = Self::decode_hex(&v.salt) else {
                continue;
            };
            if Self::hash_code(&code, &salt) == v.hash {
                matched_id = Some(k.clone());
                matched = Some(v.clone());
                break;
            }
        }

        let Some(entry_id) = matched_id else {
            self.record_failed_attempt(&platform)?;
            return Ok(None);
        };
        let Some(entry) = matched else {
            return Ok(None);
        };

        pending.remove(&entry_id);
        Self::write_json(&pending_path, &pending)?;

        let approved_path = self.approved_path(&platform);
        let mut approved: HashMap<String, ApprovedEntry> = Self::read_json(&approved_path);
        approved.insert(
            entry.user_id.clone(),
            ApprovedEntry {
                user_name: entry.user_name.clone(),
                approved_at: Self::now_ts(),
            },
        );
        Self::write_json(&approved_path, &approved)?;

        Ok(Some(ApprovedUser {
            user_id: entry.user_id,
            user_name: entry.user_name,
        }))
    }

    pub fn revoke(&self, platform: &str, user_id: &str) -> Result<bool, String> {
        let platform = platform.trim().to_ascii_lowercase();
        let user_id = user_id.trim();
        if platform.is_empty() || user_id.is_empty() {
            return Ok(false);
        }
        let path = self.approved_path(&platform);
        let mut approved: HashMap<String, ApprovedEntry> = Self::read_json(&path);
        let existed = approved.remove(user_id).is_some();
        if existed {
            Self::write_json(&path, &approved)?;
        }
        Ok(existed)
    }

    pub fn list_pending(&self, platform: Option<&str>) -> Vec<PendingPairing> {
        let mut out = Vec::new();
        let now = Self::now_ts();
        let platforms = self.all_platforms("pending", platform);
        for p in platforms {
            let _ = self.cleanup_expired(&p);
            let pending: HashMap<String, PendingEntry> = Self::read_json(&self.pending_path(&p));
            for entry in pending.values() {
                let code = entry.hash.chars().take(8).collect::<String>();
                out.push(PendingPairing {
                    platform: p.clone(),
                    code,
                    user_id: entry.user_id.clone(),
                    user_name: entry.user_name.clone(),
                    age_minutes: ((now - entry.created_at).max(0.0) / 60.0) as u64,
                });
            }
        }
        out
    }

    pub fn list_approved(&self, platform: Option<&str>) -> Vec<ApprovedPairing> {
        let mut out = Vec::new();
        for p in self.all_platforms("approved", platform) {
            let approved: HashMap<String, ApprovedEntry> = Self::read_json(&self.approved_path(&p));
            for (user_id, info) in approved {
                out.push(ApprovedPairing {
                    platform: p.clone(),
                    user_id,
                    user_name: info.user_name,
                    approved_at: info.approved_at,
                });
            }
        }
        out
    }

    pub fn clear_pending(&self, platform: Option<&str>) -> Result<usize, String> {
        let mut removed = 0usize;
        for p in self.all_platforms("pending", platform) {
            let path = self.pending_path(&p);
            let pending: HashMap<String, PendingEntry> = Self::read_json(&path);
            removed += pending.len();
            Self::write_json(&path, &HashMap::<String, PendingEntry>::new())?;
        }
        Ok(removed)
    }

    fn all_platforms(&self, suffix: &str, only: Option<&str>) -> Vec<String> {
        if let Some(platform) = only {
            let p = platform.trim().to_ascii_lowercase();
            if p.is_empty() {
                return Vec::new();
            }
            return vec![p];
        }
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.root_dir) else {
            return out;
        };
        let marker = format!("-{suffix}.json");
        for item in rd.flatten() {
            let name = item.file_name().to_string_lossy().to_string();
            if name.ends_with(&marker) {
                let p = name.trim_end_matches(&marker).to_string();
                if !p.starts_with('_') && !p.is_empty() {
                    out.push(p);
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_roundtrip_approves_user() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = DmPairingStore::new(tmp.path().to_path_buf());
        let code = store
            .generate_code("telegram", "123", "alice")
            .expect("generate")
            .expect("code");
        assert!(!store.is_approved("telegram", "123"));
        let approved = store
            .approve_code("telegram", &code)
            .expect("approve")
            .expect("approved");
        assert_eq!(approved.user_id, "123");
        assert_eq!(approved.user_name, "alice");
        assert!(store.is_approved("telegram", "123"));
    }

    #[test]
    fn invalid_code_is_rejected() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = DmPairingStore::new(tmp.path().to_path_buf());
        let _ = store
            .generate_code("telegram", "123", "alice")
            .expect("generate");
        let approved = store.approve_code("telegram", "BADCODE1").expect("approve");
        assert!(approved.is_none());
    }
}
