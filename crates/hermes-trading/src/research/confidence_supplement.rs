//! Derive missing `DataConfidence` fields from fields already on the snapshot.

use crate::research::types::{FundamentalsSnapshot, ProvenanceSource};

/// Fill computed confidence fields (price/pe/pb/market_cap) without overwriting provider data.
pub fn supplement_snapshot_confidence(snap: &mut FundamentalsSnapshot) {
    if snap.shares_outstanding_yi.is_none()
        && let (Some(mc), Some(price)) = (snap.market_cap_yi, snap.price)
        && price > 0.0
    {
        snap.shares_outstanding_yi = Some(mc / price);
        snap.provenance
            .insert("shares_outstanding_yi".into(), ProvenanceSource::Computed);
    }

    if snap.eps.is_none()
        && let (Some(price), Some(pe)) = (snap.price, snap.pe)
        && pe > 0.0
    {
        snap.eps = Some(price / pe);
        snap.provenance
            .insert("eps".into(), ProvenanceSource::Computed);
    }

    if snap.bvps.is_none()
        && let (Some(price), Some(pb)) = (snap.price, snap.pb)
        && pb > 0.0
    {
        snap.bvps = Some(price / pb);
        snap.provenance
            .insert("bvps".into(), ProvenanceSource::Computed);
    }

    if snap.ebitda_yi.is_none()
        && let (Some(rev), Some(margin)) = (snap.revenue_latest_yi, snap.net_margin)
        && rev > 0.0
        && margin > 0.0
    {
        // Rough EBITDA proxy when provider omits explicit EBITDA (revenue × margin × 1.25).
        snap.ebitda_yi = Some(rev * margin / 100.0 * 1.25);
        snap.provenance
            .insert("ebitda_yi".into(), ProvenanceSource::Computed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::types::DataConfidence;

    #[test]
    fn supplement_derives_shares_eps_bvps() {
        let mut snap = FundamentalsSnapshot {
            symbol: "600519.SH".into(),
            industry: Some("白酒".into()),
            price: Some(1680.0),
            pe: Some(28.0),
            pb: Some(8.0),
            market_cap_yi: Some(21000.0),
            revenue_latest_yi: Some(1500.0),
            net_margin: Some(52.0),
            roe_latest: Some(32.0),
            fcf_latest_yi: Some(600.0),
            equity_yi: Some(2200.0),
            cash_yi: Some(1500.0),
            total_debt_yi: Some(30.0),
            debt_ratio: Some(18.0),
            pe_quantile_5y: Some(35.0),
            ..Default::default()
        };
        supplement_snapshot_confidence(&mut snap);
        assert!((snap.shares_outstanding_yi.unwrap() - 12.5).abs() < 0.01);
        assert!((snap.eps.unwrap() - 60.0).abs() < 0.01);
        assert!((snap.bvps.unwrap() - 210.0).abs() < 0.01);
        assert!(snap.ebitda_yi.is_some());
        let conf = DataConfidence::from_snapshot(&snap);
        assert!(
            conf.score >= 0.65,
            "expected G2 target after supplement, got {:.3} missing={:?}",
            conf.score,
            conf.missing
        );
    }

    #[test]
    fn supplement_does_not_overwrite_provider_fields() {
        let mut snap = FundamentalsSnapshot {
            symbol: "600519.SH".into(),
            price: Some(100.0),
            pe: Some(20.0),
            eps: Some(5.0),
            ..Default::default()
        };
        supplement_snapshot_confidence(&mut snap);
        assert_eq!(snap.eps, Some(5.0));
    }
}
