//! Voice dialog CLI handler (`hermes talk`).

pub async fn handle_cli_talk(
    action: Option<String>,
    config: Option<String>,
    seconds: u64,
) -> Result<(), hermes_core::AgentError> {
    use std::path::PathBuf;

    use hermes_talk::audio::{list_devices, probe_capture, probe_playback};
    use hermes_talk::{Config, Session, init_talk_home, run_enroll};

    let action = action.as_deref().unwrap_or("run");
    let cfg_path: PathBuf = config
        .map(PathBuf::from)
        .unwrap_or_else(hermes_config::talk_config_path);
    let base = cfg_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(hermes_config::talk_dir);

    match action {
        "init" => init_talk_home().map_err(map_talk_error),
        "list-devices" => list_devices().map_err(map_talk_error),
        "run" => {
            let cfg = Config::load_with_base(&cfg_path, &base).map_err(map_talk_error)?;
            Session::new(cfg).run().await.map_err(map_talk_error)?;
        }
        "probe-capture" => {
            let cfg = Config::load_with_base(&cfg_path, &base).map_err(map_talk_error)?;
            probe_capture(&cfg.audio, cfg.asr.chunk_ms, seconds).map_err(map_talk_error)?;
        }
        "probe-playback" => {
            let cfg = Config::load_with_base(&cfg_path, &base).map_err(map_talk_error)?;
            probe_playback(&cfg.audio, cfg.tts.sample_rate).map_err(map_talk_error)?;
        }
        "enroll" => {
            let cfg = Config::load_with_base(&cfg_path, &base).map_err(map_talk_error)?;
            run_enroll(&cfg, seconds).map_err(map_talk_error)?;
        }
        other => Err(hermes_core::AgentError::Config(format!(
            "unknown talk action '{other}'. Available: run, init, list-devices, probe-capture, probe-playback, enroll"
        ))),
    }
}

fn map_talk_error(e: hermes_talk::DemoError) -> hermes_core::AgentError {
    hermes_core::AgentError::Config(e.to_string())
}
