//! Three-statement appendix (DCF moved to `dcf_block` section).

use crate::research::analyze::AnalyzeStockResult;

#[must_use]
pub fn render_appendix(result: &AnalyzeStockResult) -> String {
    render_three_stmt_summary(&result.three_statement)
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
