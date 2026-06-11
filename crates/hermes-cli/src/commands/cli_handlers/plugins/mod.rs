//! Plugin CLI handlers.

mod external;
mod install;
mod manage;
mod security;
mod surface;

pub use external::handle_cli_external_plugin_subcommand;
pub use manage::handle_cli_plugins;
pub(crate) use surface::{
    PluginSurfaceEntry, PluginSurfaceSource, discover_plugin_surface, render_plugin_surface_table,
};
