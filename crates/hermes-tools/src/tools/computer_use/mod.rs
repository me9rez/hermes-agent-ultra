//! Computer use toolset.
//!
//! Mirrors Python package layout:
//! - `backend`    abstract backend types
//! - `cua_backend` cua-driver MCP backend (macOS + Windows when installed)
//! - `schema`     tool schema builder
//! - `tool`       dispatch + safety + fallback backend

pub mod backend;
pub mod cua_backend;
pub mod schema;
pub mod tool;

pub use tool::{ComputerUseHandler, check_computer_use_requirements};
pub use cua_backend::ensure_cua_driver_daemon_running;
