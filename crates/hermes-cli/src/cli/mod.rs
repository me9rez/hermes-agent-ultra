//! CLI argument parsing using clap (Requirement 9.7).
//!
//! Uses a staged parser so debug builds do not need a multi-megabyte main-thread
//! stack when constructing the full command tree.

pub(crate) mod commands;
mod completion_tree;
mod globals;
mod parse;
mod shallow;
mod types;

#[cfg(test)]
mod tests;

pub use completion_tree::completion_command;
pub use globals::GlobalCli;
pub use types::{Cli, CliCommand};

impl clap::CommandFactory for Cli {
    fn command() -> clap::Command {
        GlobalCli::command()
    }

    fn command_for_update() -> clap::Command {
        GlobalCli::command_for_update()
    }
}
