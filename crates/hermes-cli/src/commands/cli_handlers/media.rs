//! `hermes media` — image/video generation setup and model discovery.

use super::media_config;
use hermes_config::{GatewayConfig, load_config};
use hermes_core::AgentError;
use hermes_server_client::{
    AuthManager, ClawModelEntry, MODEL_CATEGORY_IMAGE, MODEL_CATEGORY_VIDEO,
};

pub async fn handle_cli_media(
    action: Option<String>,
    rest: Vec<String>,
    config_dir: Option<&str>,
) -> Result<(), AgentError> {
    let config = load_config(config_dir).map_err(|e| AgentError::Config(e.to_string()))?;
    let action = action
        .as_deref()
        .unwrap_or("show")
        .trim()
        .to_ascii_lowercase();

    match action.as_str() {
        "config" => media_config::handle_media_config(&rest, config_dir, &config).await,
        "init" | "setup" => {
            media_config::handle_media_config(&["init".to_string()], config_dir, &config).await
        }
        "models" => handle_media_models(&rest, config_dir, &config).await,
        "workflows" => {
            print_workflow_templates();
            Ok(())
        }
        "show" | "status" => {
            let cfg_path = media_config::config_yaml_path(config_dir);
            media_config::print_media_config(&config.media, &cfg_path, config.server.enabled);
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print_media_help();
            Ok(())
        }
        other => Err(AgentError::Config(format!(
            "unknown media subcommand '{other}'. Try: hermes media help"
        ))),
    }
}

fn print_media_help() {
    println!("Image & video generation (Flowy server APIs + workflows)");
    println!();
    println!("Usage:");
    println!("  hermes media                     Show current settings");
    println!("  hermes media init                Interactive setup wizard");
    println!("  hermes media config [show|set|get|init]");
    println!("  hermes media models              List cloud image + video models");
    println!("  hermes media models pick image   Pick default image model");
    println!("  hermes media models pick video   Pick default video model");
    println!("  hermes media workflows           List builtin workflow templates");
    println!();
    println!("Run `hermes media config help` for all configuration keys.");
}

async fn handle_media_models(
    rest: &[String],
    config_dir: Option<&str>,
    config: &GatewayConfig,
) -> Result<(), AgentError> {
    match rest.first().map(|s| s.as_str()) {
        None | Some("list") => list_models(config, None).await,
        Some("image") => list_models(config, Some(MODEL_CATEGORY_IMAGE)).await,
        Some("video") => list_models(config, Some(MODEL_CATEGORY_VIDEO)).await,
        Some("pick") => {
            let kind = rest.get(1).map(|s| s.as_str()).ok_or_else(|| {
                AgentError::Config("usage: hermes media models pick image|video".into())
            })?;
            match kind.to_ascii_lowercase().as_str() {
                "image" => pick_and_save_model(config_dir, config, "image").await,
                "video" => pick_and_save_model(config_dir, config, "video").await,
                other => Err(AgentError::Config(format!(
                    "unknown model kind '{other}' (use image or video)"
                ))),
            }
        }
        Some(other) => Err(AgentError::Config(format!(
            "unknown models subcommand '{other}'. Try: list, image, video, pick"
        ))),
    }
}

async fn list_models(config: &GatewayConfig, category: Option<i32>) -> Result<(), AgentError> {
    let manager = require_logged_in(&config.server).await?;
    if config.media.image.model.trim().is_empty() {
        println!("  current image model: (auto — first available)");
    } else {
        println!("  current image model: {}", config.media.image.model);
    }
    if config.media.video.model.trim().is_empty() {
        println!("  current video model: (auto — first available)");
    } else {
        println!("  current video model: {}", config.media.video.model);
    }
    println!();

    let models = match category {
        Some(cat) => {
            println!("Cloud models (category={cat}):");
            manager
                .list_claw_models(Some(cat))
                .await
                .map_err(server_client_err)?
        }
        None => {
            println!("Cloud image models:");
            let mut image = manager
                .list_claw_models(Some(MODEL_CATEGORY_IMAGE))
                .await
                .map_err(server_client_err)?;
            for entry in &image {
                print_model_entry(entry);
            }
            println!();
            println!("Cloud video models:");
            let video = manager
                .list_claw_models(Some(MODEL_CATEGORY_VIDEO))
                .await
                .map_err(server_client_err)?;
            for entry in &video {
                print_model_entry(entry);
            }
            image.extend(video);
            image
        }
    };

    if category.is_some() {
        if models.is_empty() {
            println!("No models returned.");
        } else {
            for entry in models {
                print_model_entry(&entry);
            }
        }
    } else if models.is_empty() {
        println!("No models returned.");
    }

    println!();
    println!("Set default: hermes media models pick image|video");
    Ok(())
}

fn print_model_entry(entry: &ClawModelEntry) {
    let kind = match entry.category {
        MODEL_CATEGORY_IMAGE => "image",
        MODEL_CATEGORY_VIDEO => "video",
        _ => "other",
    };
    println!(
        "  - {} [{}] id={} (or {})",
        entry.name,
        kind,
        entry.id,
        entry.flowy_model_id()
    );
}

async fn pick_and_save_model(
    config_dir: Option<&str>,
    _config: &GatewayConfig,
    kind: &str,
) -> Result<(), AgentError> {
    let Some(id) = interactive_model_pick(config_dir, kind).await? else {
        println!("Model selection cancelled.");
        return Ok(());
    };
    let key = if kind == "video" {
        "video_model"
    } else {
        "image_model"
    };
    let path = media_config::save_media_field(config_dir, key, &id)?;
    println!("Default {kind} model set to: {id}");
    println!("Saved → {}", path.display());
    Ok(())
}

pub(crate) async fn interactive_model_pick(
    config_dir: Option<&str>,
    kind: &str,
) -> Result<Option<String>, AgentError> {
    let config = load_config(config_dir).map_err(|e| AgentError::Config(e.to_string()))?;
    let category = if kind == "video" {
        MODEL_CATEGORY_VIDEO
    } else {
        MODEL_CATEGORY_IMAGE
    };
    let manager = require_logged_in(&config.server).await?;
    let models = manager
        .list_claw_models(Some(category))
        .await
        .map_err(server_client_err)?;

    if models.is_empty() {
        return Err(AgentError::Config(format!(
            "no {kind} models returned from server"
        )));
    }

    println!("Available {kind} models:");
    for (idx, entry) in models.iter().enumerate() {
        println!("  [{}] {} — id={}", idx + 1, entry.name, entry.id);
    }
    let current = if kind == "video" {
        &config.media.video.model
    } else {
        &config.media.image.model
    };
    if !current.trim().is_empty() {
        println!("  Current: {current}");
    }

    let line = media_config::prompt_line(&format!(
        "Select {kind} model [1-{}] (Enter to cancel): ",
        models.len()
    ))
    .await?;
    if line.is_empty() {
        return Ok(None);
    }
    let choice: usize = line
        .parse()
        .map_err(|_| AgentError::Config("enter a number from the list".into()))?;
    if choice == 0 || choice > models.len() {
        return Err(AgentError::Config(format!(
            "selection out of range (1-{})",
            models.len()
        )));
    }
    Ok(Some(models[choice - 1].id.clone()))
}

fn print_workflow_templates() {
    println!("Builtin media workflow templates:");
    for id in hermes_media_workflows::workflows::list_builtin_templates() {
        if let Some(def) = hermes_media_workflows::workflows::builtin_template(id) {
            println!("  - {} (v{}) — {}", def.id, def.version, def.description);
        } else {
            println!("  - {id}");
        }
    }
    println!();
    println!("Use via agent tools: media_workflow_plan / media_workflow_run");
}

async fn require_logged_in(
    config: &hermes_config::ServerConfig,
) -> Result<AuthManager, AgentError> {
    if !config.api_ready() {
        return Err(AgentError::Config(
            "server.base_url not configured — run `hermes server config init`".into(),
        ));
    }
    let manager = AuthManager::new(config.clone(), hermes_config::hermes_home())
        .map_err(server_client_err)?;
    let status = manager.whoami().await.map_err(server_client_err)?;
    if !status.is_logged_in() {
        return Err(AgentError::Config(
            "not logged in — run `hermes server login` first".into(),
        ));
    }
    Ok(manager)
}

fn server_client_err(err: hermes_server_client::ServerClientError) -> AgentError {
    AgentError::Config(err.to_string())
}
