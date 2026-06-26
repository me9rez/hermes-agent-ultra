//! User-facing report scrubbing — hide internal gap keys and web-deferred dims.

use crate::research::analyze::AnalyzeStockResult;
use crate::research::fetchers::dim_keys;

/// Dimensions not collected over HTTP (web/LLM path); never show as user-visible gaps.
pub const WEB_ONLY_DIMS: &[&str] = &[
    dim_keys::MACRO,
    dim_keys::CHAIN,
    dim_keys::MATERIALS,
    dim_keys::FUTURES,
    dim_keys::GOVERNANCE,
    dim_keys::POLICY,
    dim_keys::MOAT,
    dim_keys::SENTIMENT,
    dim_keys::CONTESTS,
    dim_keys::TRAP,
];

#[must_use]
pub fn is_web_only_dim(key: &str) -> bool {
    WEB_ONLY_DIMS.contains(&key)
}

/// User reports omit internal missing-dimension lists (chips / 数据缺口 section).
#[must_use]
pub fn user_missing_dims(raw: &[String]) -> Vec<String> {
    raw.iter()
        .filter(|k| !is_web_only_dim(k))
        .filter(|k| !is_internal_subfield_key(k))
        .map(|k| k.to_string())
        .filter(|k| show_dim_in_gap_section(k))
        .collect()
}

#[must_use]
pub fn user_missing_highlights(
    _confidence_missing: &[String],
    _missing_dims: &[String],
) -> Vec<String> {
    Vec::new()
}

#[must_use]
pub fn show_gaps_section(missing_dims: &[String], highlights: &[String]) -> bool {
    !missing_dims.is_empty() || !highlights.is_empty()
}

pub fn scrub_user_report(result: &mut AnalyzeStockResult) {
    result.missing_dims = user_missing_dims(&result.missing_dims);
    result.synthesis.missing_highlights =
        user_missing_highlights(&result.data_confidence.missing, &result.missing_dims);
}

#[must_use]
pub fn scrub_dim_label(label: &str) -> String {
    label
        .replace("（待 web 补数）", "")
        .replace("(待 web 补数)", "")
        .replace(" · 分位缺数", "")
        .replace("龙虎榜数据缺失", "暂无近期龙虎榜")
        .trim()
        .to_string()
}

fn is_internal_subfield_key(key: &str) -> bool {
    matches!(
        key,
        "pe" | "pe_percentile"
            | "pe_quantile_5y"
            | "fcf_latest_yi"
            | "total_debt_yi"
            | "cash_yi"
            | "main_fund_flow"
            | "holder_change_ratio"
            | "announcements"
            | "news"
            | "research_reports"
            | "lhb_count_30d"
    )
}

fn show_dim_in_gap_section(_key: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_missing_dims_empty_for_web_keys() {
        let raw = vec!["3_macro".into(), "9_futures".into(), "10_valuation".into()];
        assert!(user_missing_dims(&raw).is_empty());
    }

    #[test]
    fn scrub_dim_label_removes_web_suffix() {
        assert_eq!(
            scrub_dim_label("宏观环境中性（待 web 补数）"),
            "宏观环境中性"
        );
        assert_eq!(scrub_dim_label("PE 65.5 · 分位缺数"), "PE 65.5");
    }
}
