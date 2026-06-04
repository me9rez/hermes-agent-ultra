pub mod platform;
pub mod github;
pub mod download;
pub mod verify;
pub mod replace;
pub mod modelscope;
pub mod probe;
pub mod version;
pub mod manifest;

use hermes_core::errors::AgentError;
use crate::update::platform::Platform;
use crate::update::version::{ChannelPolicy, Channel, VersionPolicy, UpdateMeta, UpdateDecision};

/// 更新选项
pub struct UpdateOptions {
    pub yes: bool,
    pub force: bool,
    pub source: Option<String>,
    pub channel: Option<String>,
}

/// 检查是否有更新可用（兼容旧接口）
pub async fn check_for_updates() -> Result<String, AgentError> {
    let platform = Platform::detect()?;
    let source = probe::select_fastest_source(None).await;
    println!("Checking for updates from {}...", source.name());

    let info = source.fetch_latest(&platform).await?;
    let current = semver::Version::parse(
        env!("CARGO_PKG_VERSION").trim_start_matches('v')
    ).unwrap_or_else(|_| semver::Version::new(0, 0, 0));

    let subscribed = Channel::Stable;
    let policy = ChannelPolicy { subscribed_channel: subscribed };
    let meta = UpdateMeta {
        channel: info.channel,
        forced: info.forced,
        min_supported_version: info.min_version.clone(),
        ..Default::default()
    };

    match policy.evaluate(&current, &info.version, &meta) {
        UpdateDecision::UpToDate => {
            Ok(format!("Already up to date (v{}).", current))
        }
        UpdateDecision::UpdateAvailable { forced } => {
            let mut msg = format!(
                "New version available: v{} (current: v{})\nRun `hermes update` to upgrade.",
                info.version, current
            );
            if forced {
                msg = format!("[FORCED UPDATE REQUIRED]\n{}", msg);
            }
            if let Some(notes) = &info.release_notes {
                let preview: String = notes.lines().take(5).collect::<Vec<_>>().join("\n");
                msg.push_str(&format!("\n\nRelease notes:\n{preview}"));
            }
            Ok(msg)
        }
        UpdateDecision::DoNotUpdate { reason } => {
            Ok(format!("No update available: {}", reason))
        }
    }
}

/// 执行完整的 OTA 更新流程
pub async fn perform_update(opts: UpdateOptions) -> Result<(), AgentError> {
    // 1. Detect platform
    let platform = Platform::detect()?;
    println!("Platform: {}-{}", platform.os, platform.arch);

    // 2. Fetch latest release info
    let source = probe::select_fastest_source(opts.source.as_deref()).await;
    println!("Checking for updates from {}...", source.name());
    let info = source.fetch_latest(&platform).await?;

    // 3. Version comparison using ChannelPolicy
    let current = semver::Version::parse(
        env!("CARGO_PKG_VERSION").trim_start_matches('v')
    ).unwrap_or_else(|_| semver::Version::new(0, 0, 0));

    let subscribed = opts.channel
        .as_deref()
        .map(Channel::from_str)
        .unwrap_or_default();
    let policy = ChannelPolicy { subscribed_channel: subscribed };
    let meta = UpdateMeta {
        channel: info.channel,
        forced: info.forced,
        min_supported_version: info.min_version.clone(),
        ..Default::default()
    };

    let decision = if opts.force {
        // --force 标记总是允许更新
        UpdateDecision::UpdateAvailable { forced: false }
    } else {
        policy.evaluate(&current, &info.version, &meta)
    };

    match decision {
        UpdateDecision::UpToDate => {
            println!("Already up to date (v{}).", current);
            return Ok(());
        }
        UpdateDecision::DoNotUpdate { reason } => {
            println!("No update available: {}", reason);
            return Ok(());
        }
        UpdateDecision::UpdateAvailable { forced } => {
            println!("Current version: v{}", current);
            println!("Latest version:  v{}", info.version);
            if forced {
                println!("[FORCED UPDATE - security or compatibility requirement]");
            }
        }
    }

    if let Some(ref notes) = info.release_notes {
        let preview: String = notes.lines().take(10).collect::<Vec<_>>().join("\n");
        println!("\nRelease notes:\n{preview}\n");
    }

    // 4. Confirm (unless -y)
    if !opts.yes {
        println!("Proceed with update? [y/N] ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)
            .map_err(|e| AgentError::Io(format!("Failed to read input: {e}")))?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Update cancelled.");
            return Ok(());
        }
    }

    // 5. Download and extract
    let (archive_path, new_binary) = download::download_and_extract(
        &info.artifact_url,
        &platform,
        true, // show progress
    ).await?;

    // 6. Verify checksum (on archive, not extracted binary)
    if let Some(ref checksum_url) = info.checksum_url {
        verify::verify_checksum(&archive_path, checksum_url, &platform.artifact_name()).await?;
    } else {
        tracing::warn!("No checksums.sha256 available for this release, skipping verification");
    }

    // Cleanup archive
    let _ = std::fs::remove_file(&archive_path);

    // 7. Self-replace
    replace::self_replace(&new_binary)?;

    // Cleanup temp file
    let _ = std::fs::remove_file(&new_binary);

    // 8. Success message
    println!("\nSuccessfully updated to v{}!", info.version);
    if cfg!(windows) {
        println!("Please restart hermes for the update to take effect.");
    }

    Ok(())
}
