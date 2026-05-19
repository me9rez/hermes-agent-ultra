#[cfg(test)]
mod tests {
    use crate::{Cli, CliCommand};
    use clap::CommandFactory;

    #[test]
    fn cli_parse_default() {
        let cli = Cli::try_parse_from(["hermes"]).unwrap();
        assert!(cli.command.is_none());
        assert!(!cli.verbose);
        assert!(cli.config_dir.is_none());
        assert!(cli.model.is_none());
        assert!(cli.provider.is_none());
        assert!(cli.oneshot.is_none());
        assert!(!cli.allow_tools);
        assert!(!cli.ignore_user_config);
        assert!(!cli.ignore_rules);
    }

    #[test]
    fn cli_parse_model() {
        let cli = Cli::try_parse_from(["hermes", "model", "openai:gpt-4o"]).unwrap();
        match cli.command {
            Some(CliCommand::Model { provider_model }) => {
                assert_eq!(provider_model.as_deref(), Some("openai:gpt-4o"));
            }
            _ => panic!("Expected Model command"),
        }
    }

    #[test]
    fn cli_parse_verbose() {
        let cli = Cli::try_parse_from(["hermes", "-v"]).unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn subcommand_help_uses_second_pass_parser() {
        let err = Cli::try_parse_from(["hermes", "gateway", "--help"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
        let help = err.to_string();
        assert!(
            help.contains("--system"),
            "expected gateway-specific flags in help, got: {help}"
        );
    }

    #[test]
    fn cli_parse_gateway_start() {
        let cli = Cli::try_parse_from(["hermes", "gateway", "start"]).unwrap();
        match cli.command {
            Some(CliCommand::Gateway { action, .. }) => {
                assert_eq!(action.as_deref(), Some("start"));
            }
            _ => panic!("Expected Gateway command"),
        }
    }

    #[test]
    fn cli_effective_command_default() {
        let cli = Cli::try_parse_from(["hermes"]).unwrap();
        assert!(matches!(cli.effective_command(), CliCommand::Hermes));
    }

    #[test]
    fn cli_command_factory_builds() {
        let _ = Cli::command();
    }
}
