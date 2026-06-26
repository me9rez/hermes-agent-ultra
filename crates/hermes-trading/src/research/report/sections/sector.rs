//! Sector and peer comparison section.

use crate::research::report::content::SectorBlock;
use crate::research::report::dim_viz::render_pe_relative_bar;
use crate::research::report::sections::util::escape_html;

#[must_use]
pub fn render_sector_section(block: &SectorBlock) -> String {
    let mut out = String::from(r#"<section class="card"><h2>板块与同业</h2>"#);
    if let Some(name) = &block.industry_name {
        out.push_str(&format!(
            "<p><strong>所属行业</strong> {}</p>",
            escape_html(name)
        ));
    }
    let mut stats = Vec::new();
    if let Some(g) = block.growth_pct {
        stats.push(format!("行业增速 {g:+.1}%"));
    }
    if let Some(ipe) = block.industry_pe {
        stats.push(format!("行业 PE {ipe:.1}"));
    }
    if let Some(cpe) = block.company_pe {
        stats.push(format!("公司 PE {cpe:.1}"));
    }
    if !stats.is_empty() {
        out.push_str(&format!("<p>{}</p>", escape_html(&stats.join(" · "))));
    }
    if let Some(note) = &block.relative_note {
        out.push_str(&format!("<p>{}</p>", escape_html(note)));
    }
    if let (Some(c), Some(i)) = (block.company_pe, block.industry_pe) {
        out.push_str(r#"<div class="gauges">"#);
        out.push_str(&render_pe_relative_bar(c, i));
        out.push_str("</div>");
    }
    if !block.peer_rows.is_empty() {
        out.push_str(
            r#"<h3>同业对比</h3><table><tr><th>公司</th><th>代码</th><th>PE</th><th>PB</th></tr>"#,
        );
        for row in &block.peer_rows {
            out.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape_html(&row.name),
                escape_html(row.ticker.as_deref().unwrap_or("—")),
                fmt_opt(row.pe),
                fmt_opt(row.pb),
            ));
        }
        out.push_str("</table>");
    }
    out.push_str("</section>");
    out
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.2}")).unwrap_or_else(|| "—".into())
}
