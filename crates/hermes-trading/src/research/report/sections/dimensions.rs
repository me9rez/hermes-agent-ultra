//! Grouped 19-dimension scoring table.

use crate::research::report::dim_viz::render_dim_bar;
use crate::research::report::labels::{
    dimension_display_name, dimension_group_title, dimensions_in_group,
};
use crate::research::report::sections::util::escape_html;
use crate::research::report_filter::scrub_dim_label;
use crate::research::scoring::ScoreDimensionsResult;

#[must_use]
pub fn render_dimensions_section(scored: &ScoreDimensionsResult) -> String {
    let mut out = String::from(r#"<section class="card"><h2>19 维评分</h2>"#);
    for group in ["fundamentals", "market", "external"] {
        out.push_str(&format!(
            "<h3>{}</h3>",
            escape_html(&dimension_group_title(group))
        ));
        out.push_str(r#"<table><tr><th>维度</th><th>得分</th><th>说明</th></tr>"#);
        for key in dimensions_in_group(group) {
            let Some(d) = scored.dimensions.get(*key) else {
                continue;
            };
            let name = if d.display_name.is_empty() {
                dimension_display_name(key)
            } else {
                d.display_name.clone()
            };
            let bar = render_dim_bar(d.score, 10);
            let label = if crate::research::report_filter::is_web_only_dim(key)
                && !d.label.contains('待')
            {
                format!("{}（待检索）", scrub_dim_label(&d.label))
            } else {
                scrub_dim_label(&d.label)
            };
            out.push_str(&format!(
                "<tr><td>{name}</td><td>{bar} {score}/10</td><td>{}</td></tr>",
                escape_html(&label),
                score = d.score,
            ));
        }
        out.push_str("</table>");
    }
    out.push_str("</section>");
    out
}
