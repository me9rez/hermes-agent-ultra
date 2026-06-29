//! System browser CDP driver for Computer Use vertical.

pub mod actions;
pub mod approval;
pub mod driver;

pub use actions::{BrowserAction, BrowserObservation};
pub use approval::ApprovalMode;
pub use driver::{BrowserDriver, BrowserError, detect_system_browser};
