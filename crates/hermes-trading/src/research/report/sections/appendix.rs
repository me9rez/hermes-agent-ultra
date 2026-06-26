//! DCF / three-statement / LBO appendix.

use crate::research::analyze::AnalyzeStockResult;
use crate::research::report::sections::util::escape_html;

#[must_use]
pub fn render_appendix(result: &AnalyzeStockResult) -> String {
    let mut out = String::new();
    out.push_str(&render_dcf_detail(&result.dcf));
    out.push_str(&render_three_stmt_summary(&result.three_statement));
    let _ = &result.lbo;
    out
}

fn render_dcf_detail(dcf: &serde_json::Value) -> String {
    let intrinsic = dcf
        .get("intrinsic_per_share")
        .and_then(|v| v.as_f64())
        .map(|v| format!("¥{v:.2}"))
        .unwrap_or_else(|| "—".into());
    let safety = dcf
        .get("safety_margin_pct")
        .and_then(|v| v.as_f64())
        .map(|v| format!("{v:+.1}%"))
        .unwrap_or_else(|| "—".into());
    let verdict = dcf.get("verdict").and_then(|v| v.as_str()).unwrap_or("—");
    let fallbacks = dcf
        .get("used_fallback")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "无".into());

    let mut out = format!(
        r#"<section class="card"><h2>附录 · DCF 估值</h2>
<table>
<tr><th>指标</th><th>值</th></tr>
<tr><td>内在价值 (每股)</td><td>{intrinsic}</td></tr>
<tr><td>安全边际</td><td>{safety}</td></tr>
<tr><td>结论</td><td>{}</td></tr>
<tr><td>模型假设 fallback</td><td>{}</td></tr>
</table>"#,
        escape_html(verdict),
        escape_html(&fallbacks),
    );
    out.push_str(&render_dcf_sensitivity(dcf));
    out.push_str("</section>");
    out
}

fn render_dcf_sensitivity(dcf: &serde_json::Value) -> String {
    let Some(table) = dcf.get("sensitivity_table") else {
        return String::new();
    };
    let Some(rows) = table.get("rows").and_then(|v| v.as_array()) else {
        return String::new();
    };
    let Some(cols) = table.get("cols").and_then(|v| v.as_array()) else {
        return String::new();
    };
    let cells = table.get("cells").and_then(|v| v.as_array());
    let center = table.get("center_cell").and_then(|v| v.as_f64());

    let mut html = String::from(r#"<h3>DCF 敏感性（WACC × 永续增长）</h3><table><tr><th></th>"#);
    for col in cols {
        let label = col.as_str().unwrap_or("—");
        html.push_str(&format!("<th>{}</th>", escape_html(label)));
    }
    html.push_str("</tr>");
    for (ri, row_label) in rows.iter().enumerate() {
        html.push_str(&format!(
            "<tr><th>{}</th>",
            escape_html(row_label.as_str().unwrap_or("—"))
        ));
        for ci in 0..cols.len() {
            let val = cells
                .and_then(|c| c.get(ri))
                .and_then(|r| r.get(ci))
                .and_then(|v| v.as_f64());
            let cell_text = val
                .map(|v| format!("¥{v:.0}"))
                .unwrap_or_else(|| "—".into());
            let class = heat_class(val, center);
            html.push_str(&format!("<td class=\"{class}\">{cell_text}</td>"));
        }
        html.push_str("</tr>");
    }
    html.push_str("</table>");
    html
}

fn heat_class(val: Option<f64>, center: Option<f64>) -> &'static str {
    match (val, center) {
        (Some(v), Some(c)) if c > 0.0 => {
            let ratio = v / c;
            if ratio < 0.85 {
                "heat-low"
            } else if ratio > 1.15 {
                "heat-high"
            } else {
                "heat-mid"
            }
        }
        _ => "",
    }
}

fn render_three_stmt_summary(three_stmt: &serde_json::Value) -> String {
    if three_stmt.get("skipped").is_some() || three_stmt.get("error").is_some() {
        return String::new();
    }
    let rev = three_stmt.get("revenue_yi").and_then(|v| v.as_f64());
    let ni = three_stmt.get("net_income_yi").and_then(|v| v.as_f64());
    let fcf = three_stmt.get("fcf_yi").and_then(|v| v.as_f64());
    if rev.is_none() && ni.is_none() && fcf.is_none() {
        return String::new();
    }
    format!(
        r#"<section class="card"><h2>附录 · 三表摘要</h2>
<p>营收 {} · 净利 {} · FCF {}</p></section>"#,
        fmt_yi(rev),
        fmt_yi(ni),
        fmt_yi(fcf),
    )
}

fn fmt_yi(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.1} 亿"))
        .unwrap_or_else(|| "—".into())
}
