use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::types::ArtifactId;

type HmacSha256 = Hmac<Sha256>;

const DEFAULT_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone)]
pub struct SignedUrlConfig {
    pub secret: Vec<u8>,
    pub base_url: String,
    pub ttl: Duration,
}

impl SignedUrlConfig {
    pub fn from_env() -> Self {
        let secret = std::env::var("TERRA_ARTIFACT_SIGNING_SECRET")
            .unwrap_or_else(|_| "dev-insecure-secret".into())
            .into_bytes();
        let base_url =
            std::env::var("TERRA_HTTP_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8787".into());
        Self {
            secret,
            base_url,
            ttl: DEFAULT_TTL,
        }
    }
}

pub fn generate_signed_url(
    cfg: &SignedUrlConfig,
    artifact_id: ArtifactId,
) -> Result<String, String> {
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs()
        + cfg.ttl.as_secs();
    let path = format!("/api/artifacts/{artifact_id}/raw");
    let sig = sign(&cfg.secret, artifact_id, expires)?;
    Ok(format!(
        "{}{}?expires={expires}&sig={sig}",
        cfg.base_url.trim_end_matches('/'),
        path
    ))
}

pub fn verify_signed_url(
    cfg: &SignedUrlConfig,
    artifact_id: ArtifactId,
    expires: u64,
    sig: &str,
) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if expires < now {
        return false;
    }
    match sign(&cfg.secret, artifact_id, expires) {
        Ok(expected) => constant_time_eq(sig.as_bytes(), expected.as_bytes()),
        Err(_) => false,
    }
}

fn sign(secret: &[u8], artifact_id: ArtifactId, expires: u64) -> Result<String, String> {
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|e| e.to_string())?;
    mac.update(format!("{}:{}", artifact_id, expires).as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_url_roundtrip() {
        let cfg = SignedUrlConfig {
            secret: b"test-secret".to_vec(),
            base_url: "http://localhost:8787".into(),
            ttl: Duration::from_secs(60),
        };
        let id = ArtifactId::new();
        let url = generate_signed_url(&cfg, id).unwrap();
        let query = url.split('?').nth(1).unwrap();
        let mut expires = 0u64;
        let mut sig = String::new();
        for part in query.split('&') {
            if let Some(v) = part.strip_prefix("expires=") {
                expires = v.parse().unwrap();
            } else if let Some(v) = part.strip_prefix("sig=") {
                sig = v.to_string();
            }
        }
        assert!(verify_signed_url(&cfg, id, expires, &sig));
    }
}
