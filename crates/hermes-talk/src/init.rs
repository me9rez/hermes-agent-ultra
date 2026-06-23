//! Initialize `$HERMES_HOME/hermes-talk` layout.

use std::fs;
use std::path::Path;

use crate::error::{DemoError, Result};

const CONFIG_EXAMPLE: &str = include_str!("../config.example.toml");

const SUBDIRS: &[&str] = &[
    "data",
    "frontend_extras",
    "models/vad",
    "models/denoise",
    "models/speaker",
    "models/kws-zh-en",
    "models/rk3588",
];

/// Create talk home directory tree and default `config.toml` if missing.
pub fn init_talk_home() -> Result<()> {
    let home = hermes_config::talk_dir();
    fs::create_dir_all(&home)
        .map_err(|e| DemoError::Config(format!("mkdir {}: {e}", home.display())))?;

    for sub in SUBDIRS {
        let dir = home.join(sub);
        fs::create_dir_all(&dir)
            .map_err(|e| DemoError::Config(format!("mkdir {}: {e}", dir.display())))?;
    }

    let config_path = hermes_config::talk_config_path();
    if !config_path.exists() {
        fs::write(&config_path, CONFIG_EXAMPLE)
            .map_err(|e| DemoError::Config(format!("write {}: {e}", config_path.display())))?;
        println!("Created {}", config_path.display());
    } else {
        println!("Config already exists: {}", config_path.display());
    }

    print_post_init_notes(&home);
    Ok(())
}

fn print_post_init_notes(home: &Path) {
    println!();
    println!("Talk home: {}", home.display());
    println!();
    println!("Next steps:");
    println!(
        "  1. Edit {} with your API keys and backends.",
        hermes_config::talk_config_path().display()
    );
    println!(
        "  2. Place ONNX models under {}/models/ (vad, denoise, speaker, kws).",
        home.display()
    );
    println!(
        "  3. For Rockchip local ASR/TTS, copy SDK data to {}/data and {}/models/rk3588.",
        home.display(),
        home.display()
    );
    println!("  4. Run `hermes talk list-devices` to verify audio devices.");
    println!("  5. Run `hermes talk` to start the voice dialog loop.");
    println!();
    println!(
        "Note: `call_hermes` requires gateway aipc_talk (ws://127.0.0.1:9100) — not yet bundled in Hermes Ultra."
    );
}
