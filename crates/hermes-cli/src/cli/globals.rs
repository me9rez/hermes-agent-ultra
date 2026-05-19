//! Global CLI flags and root command wiring.

use std::ffi::OsString;

use clap::{Args, Command, CommandFactory, FromArgMatches, Parser};

use super::shallow::ShallowCommand;

pub const BINARY_NAME: &str = "hermes-agent-ultra";

const ABOUT: &str = "Hermes Agent Ultra — autonomous AI agent with tool use";
const LONG_ABOUT: &str = "Hermes Agent Ultra is an autonomous AI agent that can use tools, execute code, \
    and interact with various platforms. Start an interactive session with \
    `hermes-agent-ultra` (or legacy alias `hermes`) or use subcommands for \
    specific tasks.";

/// Global flags shared by both parse passes (no subcommands).
#[derive(Debug, Clone, Args)]
pub struct GlobalFlags {
    /// Enable verbose / debug logging.
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,
    /// Override the configuration directory path.
    #[arg(short = 'C', long, global = true)]
    pub config_dir: Option<String>,
    /// Override the default model (e.g. "openai:gpt-4o").
    #[arg(short = 'm', long, global = true)]
    pub model: Option<String>,
    /// Override the provider for this invocation (e.g. "nous", "anthropic").
    #[arg(long, global = true)]
    pub provider: Option<String>,
    /// One-shot prompt mode (non-interactive), aliasing upstream `-z`.
    #[arg(short = 'z', long, global = true)]
    pub oneshot: Option<String>,
    /// Force-enable tools in one-shot/query mode (`-z` / `chat --query`).
    #[arg(long, global = true)]
    pub allow_tools: bool,
    /// Override the personality / persona.
    #[arg(short = 'p', long, global = true)]
    pub personality: Option<String>,
    /// Ignore user config files for this run.
    #[arg(long, global = true)]
    pub ignore_user_config: bool,
    /// Ignore local instruction/rules context injection for this run.
    #[arg(long, global = true)]
    pub ignore_rules: bool,
    /// Auto-approve config shell hooks without a TTY prompt (also sets HERMES_ACCEPT_HOOKS).
    #[arg(long, global = true)]
    pub accept_hooks: bool,
}

/// First-pass root parser: global flags + shallow subcommand names only.
#[derive(Debug, Clone, Parser)]
#[command(name = BINARY_NAME, about = ABOUT, long_about = LONG_ABOUT, disable_help_subcommand = true)]
pub struct GlobalCli {
    #[command(flatten)]
    pub flags: GlobalFlags,
    #[command(subcommand)]
    pub command: Option<ShallowCommand>,
}

impl GlobalCli {
    pub fn root_command() -> Command {
        Self::command()
    }

    pub fn parse_shallow<I, T>(args: I) -> Result<(Self, Vec<OsString>), clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
        let matches = Self::root_command().try_get_matches_from(&args)?;
        let globals = Self::from_arg_matches(&matches)?;
        Ok((globals, args))
    }
}

/// Second-pass root: global flags + one fully-specified subcommand (no shallow duplicates).
pub fn root_with_global_flags() -> Command {
    let cmd = Command::new(BINARY_NAME)
        .about(ABOUT)
        .long_about(LONG_ABOUT)
        .disable_help_subcommand(true);
    GlobalFlags::augment_args(cmd)
}

pub fn command_with_subcommand(subcommand: Command) -> Command {
    root_with_global_flags().subcommand(subcommand)
}
