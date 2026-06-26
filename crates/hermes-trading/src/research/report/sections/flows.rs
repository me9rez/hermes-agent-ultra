//! Capital flow and events section.

use crate::research::report::content::FlowsEventsBlock;
use crate::research::report::sections::util::render_bullet_list;

#[must_use]
pub fn render_flows_section(block: &FlowsEventsBlock) -> String {
    if block.bullets.is_empty() {
        return String::new();
    }
    format!(
        r#"<section class="card"><h2>资金与事件</h2>{}</section>"#,
        render_bullet_list(&block.bullets)
    )
}
