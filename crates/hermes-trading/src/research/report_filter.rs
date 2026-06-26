//! User-facing report scrubbing — hide internal gap keys and web-deferred dims.

use crate::research::analyze::AnalyzeStockResult;
use crate::research::fetchers::dim_keys;
use crate::research::report::content::{ExternalBlock, ExternalCoverage};
use crate::research::scoring::DimScore;

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
        .replace("（待检索）", "")
        .replace("(待检索)", "")
        .replace(" · 分位缺数", "")
        .replace("龙虎榜数据缺失", "暂无近期龙虎榜")
        .trim()
        .to_string()
}

/// Web-only dims with HTTP-collected signal (events/trap cross-read) stay visible.
#[must_use]
pub fn is_placeholder_web_dim(key: &str, dim: &DimScore) -> bool {
    if !is_web_only_dim(key) {
        return false;
    }
    if dim.score >= 7 || dim.score <= 4 {
        return false;
    }
    let label = dim.label.as_str();
    let has_signal = label.contains("公告")
        || label.contains("新闻")
        || label.contains("推广")
        || label.contains("研报")
        || label.contains("龙虎榜");
    !has_signal
}

/// DEEP SCAN always renders scored dimensions (web stubs included).
#[must_use]
pub fn show_dim_in_deep_scan(_key: &str, _dim: &DimScore, _coverage: ExternalCoverage) -> bool {
    true
}

/// Hide neutral 5/10 web-deferred rows in chat brief / legacy paths; policy/macro/sentiment live in the external section.
#[must_use]
pub fn show_dim_in_user_report(key: &str, dim: &DimScore, coverage: ExternalCoverage) -> bool {
    if !is_web_only_dim(key) {
        return true;
    }
    if is_placeholder_web_dim(key, dim) {
        return false;
    }
    if coverage == ExternalCoverage::WebFilled {
        return matches!(key, "3_macro" | "13_policy" | "17_sentiment") || dim.score != 5;
    }
    true
}

#[must_use]
pub fn user_dim_label(key: &str, dim: &DimScore, external: &ExternalBlock) -> String {
    let base = scrub_dim_label(&dim.label);
    if external.coverage != ExternalCoverage::WebFilled {
        return base;
    }
    let bullets = match key {
        "3_macro" => &external.macro_bullets,
        "13_policy" => &external.policy_bullets,
        "17_sentiment" => &external.sentiment_bullets,
        _ => return base,
    };
    bullets
        .first()
        .map(|s| truncate_for_dim_label(s))
        .unwrap_or(base)
}

/// Label for DEEP SCAN cards; stub web-only dims note pending web fill.
#[must_use]
pub fn deep_scan_dim_label(key: &str, dim: &DimScore, external: &ExternalBlock) -> String {
    if is_web_only_dim(key)
        && is_placeholder_web_dim(key, dim)
        && external.coverage != ExternalCoverage::WebFilled
    {
        let base = scrub_dim_label(&dim.label);
        return format!("{base} · 待 web 补数 · 见上方「政策 / 宏观 / 舆情」");
    }
    user_dim_label(key, dim, external)
}

#[must_use]
pub fn deep_scan_stub_footnote(stub_count: usize) -> Option<String> {
    if stub_count == 0 {
        return None;
    }
    Some(format!(
        "{stub_count} 张卡片为 web 待补维度（非 HTTP 数据缺失）；详见上方「政策 / 宏观 / 舆情」专节。"
    ))
}

#[must_use]
pub fn external_dims_footnote(coverage: ExternalCoverage, hidden_count: usize) -> Option<String> {
    if hidden_count == 0 {
        return None;
    }
    Some(match coverage {
        ExternalCoverage::WebFilled => {
            format!("另有 {hidden_count} 项外部维度未单独展开，见本章已检索要点。")
        }
        ExternalCoverage::HttpPartial | ExternalCoverage::NotRetrieved => {
            format!(
                "另有 {hidden_count} 项（宏观/产业链/政策/舆情等）未单独检索，见上方「政策 / 宏观 / 舆情」专节。"
            )
        }
    })
}

fn truncate_for_dim_label(s: &str) -> String {
    const MAX: usize = 120;
    let t = s.trim();
    if t.chars().count() <= MAX {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(MAX).collect::<String>())
    }
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
        assert_eq!(scrub_dim_label("舆情（待检索）"), "舆情");
    }

    #[test]
    fn placeholder_web_dim_neutral_macro() {
        use crate::research::scoring::DimScore;

        let dim = DimScore {
            score: 5,
            weight: 3,
            display_name: String::new(),
            label: "宏观环境中性".into(),
            missing: vec![],
            reasons_pass: vec![],
            reasons_fail: vec![],
        };
        assert!(is_placeholder_web_dim("3_macro", &dim));
    }

    #[test]
    fn deep_scan_shows_placeholder_web_dims() {
        use crate::research::scoring::DimScore;

        let dim = DimScore {
            score: 5,
            weight: 3,
            display_name: String::new(),
            label: "宏观环境中性".into(),
            missing: vec![],
            reasons_pass: vec![],
            reasons_fail: vec![],
        };
        assert!(show_dim_in_deep_scan(
            "3_macro",
            &dim,
            ExternalCoverage::NotRetrieved
        ));
        assert!(!show_dim_in_user_report(
            "3_macro",
            &dim,
            ExternalCoverage::NotRetrieved
        ));
        let label = deep_scan_dim_label("3_macro", &dim, &ExternalBlock::default());
        assert!(label.contains("待 web 补数"));
    }

    #[test]
    fn hides_placeholder_web_dims_when_not_retrieved() {
        use crate::research::scoring::DimScore;

        let dim = DimScore {
            score: 5,
            weight: 3,
            display_name: String::new(),
            label: "宏观环境中性".into(),
            missing: vec![],
            reasons_pass: vec![],
            reasons_fail: vec![],
        };
        assert!(!show_dim_in_user_report(
            "3_macro",
            &dim,
            ExternalCoverage::NotRetrieved
        ));
        let trap = DimScore {
            score: 9,
            label: "未发现推广痕迹".into(),
            ..dim.clone()
        };
        assert!(show_dim_in_user_report(
            "18_trap",
            &trap,
            ExternalCoverage::NotRetrieved
        ));
    }
}
