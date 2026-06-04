//! Re-exports from [`crate::error_classifier`] (legacy module path).

pub use crate::error_classifier::FailoverReason;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parse_expected(label: &str) -> FailoverReason {
        match label {
            "Auth" => FailoverReason::Auth,
            "Billing" => FailoverReason::Billing,
            "RateLimit" => FailoverReason::RateLimit,
            "ThinkingSignature" => FailoverReason::ThinkingSignature,
            "ImageTooLarge" => FailoverReason::ImageTooLarge,
            "ProviderPolicyBlocked" => FailoverReason::ProviderPolicyBlocked,
            "LlamaCppGrammarPattern" => FailoverReason::LlamaCppGrammarPattern,
            "OAuthLongContextBetaForbidden" => FailoverReason::OAuthLongContextBetaForbidden,
            "InvalidEncryptedReplay" => FailoverReason::InvalidEncryptedReplay,
            "Unknown" => FailoverReason::Unknown,
            other => panic!("unknown expected label: {other}"),
        }
    }

    /// Golden matrix for string-based failover classification used in `chat_completion_helpers`.
    #[test]
    fn classify_matrix_golden_fixture() {
        let raw = include_str!("../tests/fixtures/retry_failover/classify_matrix.json");
        let doc: Value = serde_json::from_str(raw).expect("classify_matrix.json");
        let cases = doc["cases"]
            .as_array()
            .expect("cases array");
        for case in cases {
            let id = case["id"].as_str().unwrap_or("?");
            let err = case["error"].as_str().expect("error");
            let provider = case["provider"].as_str().unwrap_or("");
            let expected = parse_expected(case["expected"].as_str().expect("expected"));
            assert_eq!(
                classify_failover_reason_with_provider(err, provider),
                expected,
                "case {id}"
            );
        }
    }
}
