//! Policy / macro / sentiment section.

use crate::research::report::content::{ExternalBlock, ExternalCoverage};
use crate::research::report::sections::util::{escape_html, render_bullet_list};

#[must_use]
pub fn render_external_section(block: &ExternalBlock) -> String {
    let mut out = String::from(r#"<section class="card"><h2>政策 / 宏观 / 舆情</h2>"#);
    match block.coverage {
        ExternalCoverage::NotRetrieved => {
            out.push_str(
                r#"<p class="muted-note">本次未检索政策与舆情；下方「外部与定性」维度评分为框架占位。slash 流程可触发 web_search 补数。</p>"#,
            );
        }
        ExternalCoverage::HttpPartial => {
            out.push_str(
                r#"<p class="muted-note">部分维度来自 HTTP 采集；政策/舆情建议结合 web 检索。</p>"#,
            );
        }
        ExternalCoverage::WebFilled => {}
    }
    if !block.macro_bullets.is_empty() {
        out.push_str("<h3>宏观环境</h3>");
        out.push_str(&render_bullet_list(&block.macro_bullets));
    }
    if !block.policy_bullets.is_empty() {
        out.push_str("<h3>政策影响</h3>");
        out.push_str(&render_bullet_list(&block.policy_bullets));
    }
    if !block.sentiment_bullets.is_empty() {
        out.push_str("<h3>舆情与情绪</h3>");
        out.push_str(&render_bullet_list(&block.sentiment_bullets));
    }
    if !block.sources.is_empty() {
        out.push_str("<h3>参考来源</h3><ul class=\"bullets\">");
        for src in &block.sources {
            out.push_str(&format!("<li>{}</li>", escape_html(src)));
        }
        out.push_str("</ul>");
    }
    out.push_str("</section>");
    out
}
