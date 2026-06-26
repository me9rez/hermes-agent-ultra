//! HTML section renderers for institutional report.

mod appendix;
mod dimensions;
mod external;
mod flows;
mod fundamentals;
mod hero;
mod panel;
mod sector;
mod util;

pub use appendix::render_appendix;
pub use dimensions::render_dimensions_section;
pub use external::render_external_section;
pub use flows::render_flows_section;
pub use fundamentals::render_fundamentals_section;
pub use hero::{render_shell_start, render_warn_banner};
pub use panel::render_panel_section;
pub use sector::render_sector_section;
pub use util::escape_html;
