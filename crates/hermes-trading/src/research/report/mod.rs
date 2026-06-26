//! Minimal HTML report (P0.5) + SVG stub (P2).

pub mod chat_brief;
pub mod dim_viz;
pub mod disk;
pub mod html;
pub mod identity;
pub mod institutional;
pub mod labels;
pub mod markdown;
pub mod quick_scan;
pub mod svg;

pub use chat_brief::render_chat_brief_markdown;
pub use disk::{WrittenReportPaths, write_equity_report};
pub use html::render_html_report;
pub use identity::{ReportIdentity, infer_target_name_from_peers};
pub use institutional::render_institutional_html;
pub use markdown::render_summary_markdown;
pub use quick_scan::render_quick_scan_markdown;
pub use svg::{render_svg_gauge, render_svg_percentile};
