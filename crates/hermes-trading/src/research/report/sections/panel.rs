//! 66-investor panel section (Top 20 + collapsible full table).

use crate::research::personas::investors::find_investor;
use crate::research::report::sections::util::escape_html;
use crate::research::scoring::PanelResult;

const PANEL_TOP_N: usize = 20;

#[must_use]
pub fn render_panel_section(panel: &PanelResult, include_full_table: bool) -> String {
    let vd = &panel.vote_distribution;
    let mut sorted = panel.investors.clone();
    sorted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut out = format!(
        r#"<section class="card"><h2>66 位评委</h2>
<p>共识 <strong>{:.1}/10</strong> · 买入 {} · 回避 {} · 共 {} 位</p>
<table><tr><th>类别</th><th>人数</th></tr>
<tr><td>强烈买入</td><td>{}</td></tr>
<tr><td>买入</td><td>{}</td></tr>
<tr><td>关注</td><td>{}</td></tr>
<tr><td>观望</td><td>{}</td></tr>
<tr><td>回避</td><td>{}</td></tr>
<tr><td>跳过</td><td>{}</td></tr>
</table>
<h3>Top {PANEL_TOP_N} 评委</h3>
<table><tr><th>评委</th><th>结论</th><th>分数</th></tr>"#,
        panel.panel_consensus,
        vd.strongly_buy + vd.buy,
        vd.avoid,
        panel.investors.len(),
        vd.strongly_buy,
        vd.buy,
        vd.watch,
        vd.wait,
        vd.avoid,
        vd.skip + vd.n_a,
    );
    for vote in sorted.iter().take(PANEL_TOP_N) {
        out.push_str(&panel_row(vote));
    }
    out.push_str("</table>");
    if include_full_table && panel.investors.len() > PANEL_TOP_N {
        out.push_str(r#"<details class="panel-details"><summary>展开全部评委</summary><table><tr><th>评委</th><th>结论</th><th>分数</th></tr>"#);
        for vote in &sorted {
            out.push_str(&panel_row(vote));
        }
        out.push_str("</table></details>");
    }
    out.push_str("</section>");
    out
}

fn panel_row(vote: &crate::research::personas::PersonaVote) -> String {
    let name = find_investor(&vote.id)
        .map(|m| m.name)
        .unwrap_or(vote.id.as_str());
    format!(
        "<tr><td>{}</td><td>{}</td><td>{:.0}</td></tr>",
        escape_html(name),
        escape_html(&vote.vote),
        vote.score,
    )
}
