//! Full clap command tree for shell completion generation only.

use clap::Command;

use super::commands;
use super::globals;

/// Build the complete CLI command tree for `clap_complete` (not used during normal startup).
pub fn completion_command() -> Command {
    let mut cmd = globals::root_with_global_flags();
    cmd = cmd
        .subcommand(Command::new("hermes"))
        .subcommand(Command::new("setup").about("Run the interactive setup wizard"))
        .subcommand(Command::new("status").about("Show agent and gateway status"))
        .subcommand(Command::new("version").about("Print version information"));

    for sub in commands::all_subcommand_commands() {
        cmd = cmd.subcommand(sub);
    }

    cmd.allow_external_subcommands(true)
}
