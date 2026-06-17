//! Symbol format detection and normalization for multi-market routing.

/// Whether a symbol is an A-share (Shenzhen `.SZ` or Shanghai `.SH`).
#[must_use]
pub fn is_a_share(symbol: &str) -> bool {
    let upper = symbol.to_uppercase();
    upper.ends_with(".SZ") || upper.ends_with(".SH")
}

/// Whether a symbol is a Hong Kong listed stock.
///
/// Accepted formats: `0700.HK`, `HK_00700`.
#[must_use]
pub fn is_hk_share(symbol: &str) -> bool {
    let upper = symbol.to_uppercase();
    if upper.ends_with(".HK") {
        return true;
    }
    if let Some(suffix) = upper.strip_prefix("HK_") {
        return !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit());
    }
    false
}

/// Whether a symbol is a US listed stock.
///
/// Accepted formats: `AAPL`, `AAPL.US`.
#[must_use]
pub fn is_us_share(symbol: &str) -> bool {
    let upper = symbol.to_uppercase();
    if upper.ends_with(".US") {
        return true;
    }
    if symbol.contains('.') || symbol.contains('-') {
        return false;
    }
    let len = upper.len();
    (1..=5).contains(&len) && upper.chars().all(|c| c.is_ascii_alphabetic())
}

/// Normalize alternate symbol formats to canonical form (`HK_00700` → `0700.HK`).
#[must_use]
pub fn normalize_symbol(symbol: &str) -> String {
    let trimmed = symbol.trim();
    // ponytail: tiny alias table; extend when agents keep passing bare crypto tickers.
    match trimmed {
        "比特币" => return "BTC-USDT".to_string(),
        "以太坊" => return "ETH-USDT".to_string(),
        _ => {}
    }
    let upper = trimmed.to_uppercase();
    match upper.as_str() {
        "BTC" => return "BTC-USDT".to_string(),
        "ETH" => return "ETH-USDT".to_string(),
        _ => {}
    }
    if let Some(suffix) = upper.strip_prefix("HK_")
        && !suffix.is_empty()
        && suffix.chars().all(|c| c.is_ascii_digit())
    {
        let code: u32 = suffix.parse().unwrap_or(0);
        return format!("{code:04}.HK");
    }
    upper
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hk_formats() {
        assert!(is_hk_share("0700.HK"));
        assert!(is_hk_share("hk_00700"));
        assert!(is_hk_share("HK_09988"));
        assert!(!is_hk_share("00700"));
        assert!(!is_hk_share("HK_ABC"));
    }

    #[test]
    fn us_formats() {
        assert!(is_us_share("AAPL"));
        assert!(is_us_share("aapl.us"));
        assert!(!is_us_share("BTC-USDT"));
        assert!(!is_us_share("000001.SZ"));
    }

    #[test]
    fn normalize_hk_underscore() {
        assert_eq!(normalize_symbol("HK_00700"), "0700.HK");
        assert_eq!(normalize_symbol("0700.HK"), "0700.HK");
    }

    #[test]
    fn normalize_crypto_aliases() {
        assert_eq!(normalize_symbol("BTC"), "BTC-USDT");
        assert_eq!(normalize_symbol("eth"), "ETH-USDT");
        assert_eq!(normalize_symbol("比特币"), "BTC-USDT");
    }
}
