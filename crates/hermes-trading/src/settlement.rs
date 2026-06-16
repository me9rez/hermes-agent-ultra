//! Trade settlement rules (T+0 vs A-share T+1).

/// Whether a symbol is an A-share (Shenzhen `.SZ` or Shanghai `.SH`).
#[must_use]
pub fn is_a_share(symbol: &str) -> bool {
    let upper = symbol.to_uppercase();
    upper.ends_with(".SZ") || upper.ends_with(".SH")
}

/// Settlement mode for backtest trade execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementMode {
    /// Same-bar execution at close (crypto and other T+0 markets).
    T0,
    /// A-share T+1: buy fills next open; sell same-day only if not bought today.
    T1,
}

/// Derive settlement mode from symbol format.
#[must_use]
pub fn settlement_for_symbol(symbol: &str) -> SettlementMode {
    if is_a_share(symbol) {
        SettlementMode::T1
    } else {
        SettlementMode::T0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_detection() {
        assert!(is_a_share("000001.SZ"));
        assert!(is_a_share("600519.SH"));
        assert!(!is_a_share("BTC-USDT"));
        assert!(!is_a_share("AAPL"));
    }

    #[test]
    fn settlement_routing() {
        assert_eq!(settlement_for_symbol("000001.SZ"), SettlementMode::T1);
        assert_eq!(settlement_for_symbol("BTC-USDT"), SettlementMode::T0);
    }
}
