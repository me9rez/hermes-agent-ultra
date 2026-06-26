//! 06 / VALUATION · DCF block (UZI `_render_dcf_block` parity).

use crate::research::report::sections::util::escape_html;

#[must_use]
pub fn render_dcf_section(dcf: &serde_json::Value) -> String {
    let Some(intrinsic) = dcf.get("intrinsic_per_share").and_then(|v| v.as_f64()) else {
        return String::new();
    };

    let wacc_info = dcf.get("wacc_breakdown").and_then(|v| v.as_object());
    let wacc_pct = wacc_info
        .and_then(|o| o.get("wacc"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        * 100.0;
    let ke_pct = wacc_info
        .and_then(|o| o.get("cost_of_equity"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        * 100.0;
    let kd_pct = wacc_info
        .and_then(|o| o.get("after_tax_kd"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        * 100.0;

    let cur_px = dcf
        .get("current_price")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let sm = dcf
        .get("safety_margin_pct")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let verdict = dcf.get("verdict").and_then(|v| v.as_str()).unwrap_or("—");
    let tv_pct = dcf
        .get("tv_pct_of_ev")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let sm_class = safety_margin_class(sm);

    let log_items: String = dcf
        .get("methodology_log")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .take(7)
                .filter_map(|v| v.as_str())
                .map(|line| format!("<li>{}</li>", escape_html(line)))
                .collect()
        })
        .unwrap_or_default();

    let heatmap = render_sensitivity_heatmap(dcf, cur_px);

    format!(
        r#"<section class="card" id="section-valuation">
<div class="section-head">
<div class="section-tag">06 / VALUATION</div>
<h2 class="section-title">DCF 估值建模</h2>
<div class="section-line"></div>
</div>
<div class="dcf-block">
<div class="dcf-head">
<div>
<span class="dcf-badge">DCF VALUATION</span>
<span class="dcf-subtitle">2-Stage FCF + Gordon Growth Terminal</span>
</div>
</div>
<div class="dcf-summary">
<div class="dcf-kpi"><div class="k">WACC</div><div class="v">{wacc_pct:.2}%</div><div class="hint">k_e {ke_pct:.1}% · k_d {kd_pct:.1}%</div></div>
<div class="dcf-kpi"><div class="k">内在价值 / 股</div><div class="v">¥{intrinsic:.2}</div><div class="hint">vs 当前 ¥{cur_px:.2}</div></div>
<div class="dcf-kpi"><div class="k">安全边际</div><div class="v {sm_class}">{sm:+.1}%</div><div class="hint">{verdict_esc}</div></div>
<div class="dcf-kpi"><div class="k">终值占 EV</div><div class="v">{tv_pct:.0}%</div><div class="hint">高度依赖 g</div></div>
</div>
<details class="dcf-methodology">
<summary>📐 计算推导（7 步）</summary>
<ol>{log_items}</ol>
</details>
<div class="dcf-sens-wrap">
<div class="dcf-sens-title">📊 5×5 敏感性表（WACC × 终值 g）· 中心 = 基础案例</div>
{heatmap}
</div>
</div>
</section>"#,
        verdict_esc = escape_html(verdict),
        log_items = log_items,
        heatmap = heatmap,
    )
}

fn render_sensitivity_heatmap(dcf: &serde_json::Value, current_price: f64) -> String {
    let Some(table) = dcf.get("sensitivity_table") else {
        return String::new();
    };
    let Some(wacc_axis) = table.get("wacc_axis").and_then(|v| v.as_array()) else {
        return String::new();
    };
    let Some(g_axis) = table.get("g_axis").and_then(|v| v.as_array()) else {
        return String::new();
    };
    let Some(values) = table.get("values_per_share").and_then(|v| v.as_array()) else {
        return String::new();
    };
    if wacc_axis.is_empty() || g_axis.is_empty() || values.is_empty() {
        return String::new();
    }

    let mut html = String::from(r#"<table class="sens-heatmap"><tr><th></th>"#);
    for g in g_axis {
        let label = g.as_str().unwrap_or("—");
        html.push_str(&format!("<th>g={}</th>", escape_html(label)));
    }
    html.push_str("</tr>");

    for (i, row) in values.iter().enumerate() {
        let Some(row_vals) = row.as_array() else {
            continue;
        };
        let wacc_label = wacc_axis.get(i).and_then(|v| v.as_str()).unwrap_or("—");
        html.push_str(&format!("<tr><th>WACC {}</th>", escape_html(wacc_label)));
        for val in row_vals {
            let v = val.as_f64().unwrap_or(0.0);
            let class = heat_cell_class(v, current_price);
            html.push_str(&format!(r#"<td class="{class}">¥{v:.0}</td>"#,));
        }
        html.push_str("</tr>");
    }
    html.push_str("</table>");
    html
}

fn heat_cell_class(val: f64, current_price: f64) -> &'static str {
    if current_price <= 0.0 {
        return "sens-fair";
    }
    let ratio = val / current_price;
    if ratio >= 1.3 {
        "sens-deep-under"
    } else if ratio >= 1.1 {
        "sens-under"
    } else if ratio >= 0.9 {
        "sens-fair"
    } else if ratio >= 0.7 {
        "sens-over"
    } else {
        "sens-deep-over"
    }
}

fn safety_margin_class(sm: f64) -> &'static str {
    if sm > 10.0 {
        "sm-pos"
    } else if sm > -10.0 {
        "sm-mid"
    } else {
        "sm-neg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::models::dcf::compute_dcf;
    use crate::research::types::FeatureVector;

    fn sample_dcf_json() -> serde_json::Value {
        let f = FeatureVector {
            symbol: "TEST".into(),
            price: Some(18.5),
            market_cap_yi: Some(260.0),
            shares_outstanding_yi: Some(14.0),
            revenue_latest_yi: Some(52.0),
            net_margin: Some(12.5),
            pe: Some(35.0),
            pb: Some(2.8),
            total_debt_yi: Some(10.0),
            cash_yi: Some(40.0),
            fcf_latest_yi: Some(6.5),
            ebitda_yi: Some(10.0),
            equity_yi: Some(92.0),
            ..Default::default()
        };
        serde_json::to_value(compute_dcf(&f, None)).expect("dcf json")
    }

    #[test]
    fn dcf_section_renders_kpi_and_heatmap() {
        let dcf = sample_dcf_json();
        let html = render_dcf_section(&dcf);
        assert!(html.contains("06 / VALUATION"));
        assert!(html.contains("DCF VALUATION"));
        assert!(html.contains("WACC"));
        assert!(html.contains("终值占 EV"));
        assert!(html.contains("计算推导"));
        assert!(html.contains("sens-heatmap"));
        assert!(html.contains("Step 1 · WACC"));
        let cell_count = html.matches("<td class=\"sens-").count();
        assert_eq!(cell_count, 25, "expected 5x5 heatmap cells");
    }

    #[test]
    fn dcf_heatmap_uses_current_price_ratio() {
        let dcf = sample_dcf_json();
        let html = render_dcf_section(&dcf);
        assert!(
            html.contains("sens-fair") || html.contains("sens-under") || html.contains("sens-over"),
            "heatmap should color cells vs current price"
        );
    }

    #[test]
    fn dcf_section_empty_when_no_intrinsic() {
        assert!(render_dcf_section(&serde_json::json!({})).is_empty());
    }
}
