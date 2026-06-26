//! Fundamentals section.

use crate::research::report::content::FundamentalsBlock;
use crate::research::report::sections::util::{escape_html, render_bullet_list};
use crate::research::report::svg::render_svg_percentile;

#[must_use]
pub fn render_fundamentals_section(block: &FundamentalsBlock) -> String {
    let mut out = String::from(r#"<section class="card"><h2>公司基本面</h2>"#);
    out.push_str(&render_bullet_list(&block.bullets));
    if !block.metrics.is_empty() {
        out.push_str(r#"<div class="metrics">"#);
        for m in &block.metrics {
            out.push_str(&format!(
                r#"<div class="metric"><div class="k">{}</div><div class="v">{}</div></div>"#,
                escape_html(&m.label),
                escape_html(&m.value)
            ));
        }
        out.push_str("</div>");
    }
    if let Some(pct) = block.pe_percentile {
        out.push_str(r#"<div class="gauges">"#);
        out.push_str(&render_svg_percentile(pct));
        out.push_str("</div>");
    }
    out.push_str("</section>");
    out
}
