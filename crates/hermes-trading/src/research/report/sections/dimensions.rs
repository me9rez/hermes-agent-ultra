//! DEEP SCAN · 19-dimension card grid (FloatFu-style layout).

use serde_json::Value;

use crate::research::report::content::ExternalBlock;
use crate::research::report::dim_charts::render_dim_chart;
use crate::research::report::dim_viz::{
    render_deep_scan_bar, render_weight_stars, score_tier_class,
};
use crate::research::report::labels::{
    SCAN_CATEGORIES, dimension_dim_index, dimension_display_name, dimension_english_name,
};
use crate::research::report::sections::util::{escape_html, truncate_for_display};
use crate::research::report_filter::{
    deep_scan_dim_label, deep_scan_stub_footnote, is_placeholder_web_dim, is_web_only_dim,
    show_dim_in_deep_scan,
};
use crate::research::scoring::{DimScore, ScoreDimensionsResult};

const RAW_DUMP_MAX_BYTES: usize = 1500;

/// Render DEEP SCAN with configurable raw JSON dump limit (`0` = omit dumps).
#[must_use]
pub fn render_dimensions_section_with_raw_limit(
    scored: &ScoreDimensionsResult,
    external: &ExternalBlock,
    raw_dims: &Value,
    raw_dump_max_bytes: usize,
) -> String {
    let mut visible = 0usize;
    let mut stub_count = 0usize;
    let mut cards = String::new();

    for (_cat_id, cat_label, keys) in SCAN_CATEGORIES {
        let mut row = String::new();
        for key in *keys {
            let Some(d) = scored.dimensions.get(*key) else {
                continue;
            };
            if !show_dim_in_deep_scan(key, d, external.coverage) {
                continue;
            }
            if is_web_only_dim(key)
                && is_placeholder_web_dim(key, d)
                && external.coverage
                    != crate::research::report::content::ExternalCoverage::WebFilled
            {
                stub_count += 1;
            }
            visible += 1;
            row.push_str(&render_dim_card(
                key,
                d,
                external,
                raw_dims,
                raw_dump_max_bytes,
            ));
        }
        if row.is_empty() {
            continue;
        }
        cards.push_str(&format!(
            r#"<div class="cat-label">{cat}</div><div class="dim-row">{row}</div>"#,
            cat = escape_html(cat_label),
            row = row,
        ));
    }

    let footnote = deep_scan_stub_footnote(stub_count)
        .map(|n| format!(r#"<p class="scan-footnote">{}</p>"#, escape_html(&n)))
        .unwrap_or_default();

    format!(
        r#"<section class="card" id="section-scan">
<div class="section-head">
<div class="section-tag">05 / DEEP SCAN</div>
<h2 class="section-title">全维深度透视</h2>
<div class="section-line"></div>
</div>
<p class="scan-summary">已展示 {visible} 维 · 基本面综合 {score:.1}/100</p>
<div class="deep-scan">{cards}</div>
{footnote}
</section>"#,
        visible = visible,
        score = scored.fundamental_score,
        cards = cards,
        footnote = footnote,
    )
}

#[must_use]
pub fn render_dimensions_section(
    scored: &ScoreDimensionsResult,
    external: &ExternalBlock,
    raw_dims: &Value,
) -> String {
    render_dimensions_section_with_raw_limit(scored, external, raw_dims, RAW_DUMP_MAX_BYTES)
}

fn render_dim_card(
    key: &str,
    dim: &DimScore,
    external: &ExternalBlock,
    raw_dims: &Value,
    raw_dump_max_bytes: usize,
) -> String {
    let title = if dim.display_name.is_empty() {
        dimension_display_name(key)
    } else {
        dim.display_name.clone()
    };
    let label = deep_scan_dim_label(key, dim, external);
    let tier = score_tier_class(dim.score);
    let dim_idx = dimension_dim_index(key);
    let stars = render_weight_stars(dim.weight);
    let pass_fail = render_pass_fail(dim);
    let source = render_source_badge(key, external);
    let viz = render_dim_chart(key, raw_dims, external);
    let raw_dump = if raw_dump_max_bytes == 0 {
        String::new()
    } else {
        render_raw_dim_dump(key, raw_dims, raw_dump_max_bytes)
    };

    format!(
        r#"<div class="dim-card" data-dim="{idx}">
<div class="dim-head">
<div>
<div class="dim-num">DIM {idx} · WEIGHT {stars}</div>
<div class="dim-title">{title}</div>
<div class="dim-en">{en}</div>
</div>
<div class="dim-score"><div class="num {tier}">{score}</div></div>
</div>
{bar}
<div class="dim-label">{label}</div>
{viz}
{pass_fail}
<div class="dim-source">数据来源: {source}</div>
{raw_dump}
</div>"#,
        idx = dim_idx,
        title = escape_html(&title),
        en = escape_html(dimension_english_name(key)),
        tier = tier,
        score = dim.score,
        bar = render_deep_scan_bar(dim.score, 10),
        label = escape_html(&label),
        pass_fail = pass_fail,
        source = source,
        viz = viz,
        raw_dump = raw_dump,
    )
}

fn render_raw_dim_dump(key: &str, raw_dims: &Value, max_bytes: usize) -> String {
    let data = raw_dims
        .get(key)
        .and_then(|v| v.get("data"))
        .cloned()
        .unwrap_or(Value::Null);
    let pretty = serde_json::to_string_pretty(&data).unwrap_or_else(|_| "{}".into());
    let escaped = escape_html(&truncate_for_display(&pretty, max_bytes));
    format!(
        r#"<details class="dim-raw"><summary>查看原始数据 ▼</summary><pre>{escaped}</pre></details>"#
    )
}

fn render_pass_fail(dim: &DimScore) -> String {
    let pass: Vec<_> = dim.reasons_pass.iter().take(3).collect();
    let fail: Vec<_> = dim.reasons_fail.iter().take(3).collect();
    if pass.is_empty() && fail.is_empty() {
        return String::new();
    }
    let mut out = String::from(r#"<div class="dim-pass-fail">"#);
    if !pass.is_empty() {
        out.push_str(r#"<div class="pass"><ul>"#);
        for r in pass {
            out.push_str(&format!("<li>{}</li>", escape_html(r)));
        }
        out.push_str("</ul></div>");
    }
    if !fail.is_empty() {
        out.push_str(r#"<div class="fail"><ul>"#);
        for r in fail {
            out.push_str(&format!("<li>{}</li>", escape_html(r)));
        }
        out.push_str("</ul></div>");
    }
    out.push_str("</div>");
    out
}

use crate::research::report::content::ExternalCoverage;

fn render_source_badge(key: &str, external: &ExternalBlock) -> String {
    let web_filled = external.coverage == ExternalCoverage::WebFilled;
    if web_filled && matches!(key, "3_macro" | "13_policy" | "17_sentiment") {
        return r#"<span class="badge-web">Web 检索</span>"#.into();
    }
    if is_web_only_dim(key) {
        r#"<span class="badge-web">公开信息</span>"#.into()
    } else {
        r#"<span class="badge-live">官方接口</span>"#.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::report::labels::DIM_ORDER;
    use crate::research::scoring::DimScore;

    fn dim(score: u8, label: &str) -> DimScore {
        DimScore {
            score,
            weight: 5,
            display_name: String::new(),
            label: label.into(),
            missing: vec![],
            reasons_pass: vec!["PE 在 5 年中位数以下".into()],
            reasons_fail: vec![],
        }
    }

    fn stub_dim() -> DimScore {
        DimScore {
            score: 5,
            weight: 3,
            display_name: String::new(),
            label: "宏观环境中性".into(),
            missing: vec![],
            reasons_pass: vec![],
            reasons_fail: vec![],
        }
    }

    #[test]
    fn deep_scan_section_has_floatfu_header() {
        let mut dimensions = std::collections::BTreeMap::new();
        dimensions.insert("10_valuation".into(), dim(9, "PE 28.5 · 5 年 35 分位"));
        dimensions.insert("1_financials".into(), dim(7, "ROE 32%"));
        let scored = ScoreDimensionsResult {
            ticker: "600519.SH".into(),
            fundamental_score: 72.5,
            dimensions,
        };
        let external = ExternalBlock::default();
        let raw = serde_json::json!({
            "1_financials": { "data": { "revenue_history": [10.0, 12.0, 15.0], "roe_history": [5.0, 6.0, 7.0] } },
            "10_valuation": { "data": { "pe_percentile": 35.0 } }
        });
        let html = render_dimensions_section(&scored, &external, &raw);
        assert!(html.contains("05 / DEEP SCAN"));
        assert!(html.contains("全维深度透视"));
        assert!(html.contains("dim-card"));
        assert!(html.contains("dim-viz"));
        assert!(html.contains("💰 财务面 · FUNDAMENTALS"));
        assert!(html.contains("DIM 10"));
        assert!(html.contains("badge-live"));
        assert!(html.contains("查看原始数据"));
    }

    #[test]
    fn deep_scan_shows_stub_web_dims() {
        let mut dimensions = std::collections::BTreeMap::new();
        dimensions.insert("3_macro".into(), stub_dim());
        dimensions.insert("15_events".into(), dim(7, "公告 10 条 · 新闻 8 条"));
        let scored = ScoreDimensionsResult {
            ticker: "600525.SH".into(),
            fundamental_score: 50.0,
            dimensions,
        };
        let html =
            render_dimensions_section(&scored, &ExternalBlock::default(), &serde_json::json!({}));
        assert!(html.contains("宏观环境"));
        assert!(html.contains("待 web 补数"));
        assert!(html.contains("事件驱动"));
        assert!(html.contains("web 待补维度"));
    }

    #[test]
    fn deep_scan_renders_all_nineteen_dims_when_scored() {
        let mut dimensions = std::collections::BTreeMap::new();
        for key in DIM_ORDER {
            dimensions.insert((*key).into(), stub_dim());
        }
        let scored = ScoreDimensionsResult {
            ticker: "600528.SH".into(),
            fundamental_score: 55.0,
            dimensions,
        };
        let html =
            render_dimensions_section(&scored, &ExternalBlock::default(), &serde_json::json!({}));
        let card_count = html.matches("dim-card").count();
        assert_eq!(card_count, 19, "expected 19 dim cards in DEEP SCAN");
        assert!(html.contains("已展示 19 维"));
    }

    #[test]
    fn raw_dump_escapes_script_tags() {
        let raw = serde_json::json!({
            "18_trap": { "data": { "note": "<script>alert(1)</script>" } }
        });
        let html = render_dimensions_section(
            &ScoreDimensionsResult {
                ticker: "X".into(),
                fundamental_score: 50.0,
                dimensions: [("18_trap".into(), dim(8, "ok"))].into(),
            },
            &ExternalBlock::default(),
            &raw,
        );
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn raw_dump_omitted_when_limit_zero() {
        let raw = serde_json::json!({ "1_financials": { "data": { "x": 1 } } });
        let html = render_dimensions_section_with_raw_limit(
            &ScoreDimensionsResult {
                ticker: "X".into(),
                fundamental_score: 50.0,
                dimensions: [("1_financials".into(), dim(7, "ok"))].into(),
            },
            &ExternalBlock::default(),
            &raw,
            0,
        );
        assert!(!html.contains("查看原始数据"));
    }

    #[test]
    fn raw_dump_truncates_long_json() {
        let big = "x".repeat(2000);
        let raw = serde_json::json!({ "1_financials": { "data": { "blob": big } } });
        let html = render_dimensions_section(
            &ScoreDimensionsResult {
                ticker: "X".into(),
                fundamental_score: 50.0,
                dimensions: [("1_financials".into(), dim(7, "ok"))].into(),
            },
            &ExternalBlock::default(),
            &raw,
        );
        assert!(html.contains("(truncated)"));
    }
}
