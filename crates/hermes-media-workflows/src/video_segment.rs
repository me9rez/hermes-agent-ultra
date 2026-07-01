//! Long-video segmentation and ffmpeg post-processing (Seedance max ~10s per clip).
//!
//! Mirrors mainstream creative apps (即梦 / CapCut-style): split target duration into
//! API-sized clips, chain via last-frame → first-frame, then concat locally.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use hermes_core::ToolError;
use tokio::process::Command;

use crate::assets::persist_bytes;

/// Per-model maximum seconds for a single Seedance generation request.
pub fn max_clip_duration_for_model(model: &str) -> u32 {
    let _ = model.to_ascii_lowercase();
    // Seedance (Flowy default video backend) caps at ~10s per task today.
    10
}

/// True when target duration exceeds a single upstream clip.
pub fn needs_long_video_pipeline(target_secs: u32, max_clip_secs: u32) -> bool {
    target_secs > max_clip_secs.max(1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentPlan {
    pub target_duration_secs: u32,
    pub max_clip_secs: u32,
    pub segment_durations: Vec<u32>,
}

impl SegmentPlan {
    pub fn segment_count(&self) -> usize {
        self.segment_durations.len()
    }

    pub fn total_duration_secs(&self) -> u32 {
        self.segment_durations.iter().sum()
    }
}

/// Parse a target duration from natural language (e.g. "约20秒", "20s", "20-second clip").
pub fn parse_duration_secs_from_text(text: &str) -> Option<u32> {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let num: u32 = lower[start..i].parse().ok()?;
        if num == 0 || num > 600 {
            continue;
        }
        let rest = lower[i..].trim_start();
        if rest.starts_with('秒')
            || rest.starts_with("s")
            || rest.starts_with("sec")
            || rest.starts_with('-')
            || rest.starts_with(" second")
            || rest.starts_with(" seconds")
        {
            return Some(num);
        }
    }
    None
}

/// When target exceeds single-clip limit, map short-video templates to long-video workflows.
pub fn route_long_video_template(template_id: &str, target_secs: u32, model: &str) -> String {
    let max_clip = max_clip_duration_for_model(model);
    if !needs_long_video_pipeline(target_secs, max_clip) {
        return template_id.to_string();
    }
    match template_id {
        "long_txt2video" | "long_img2video_direct" | "long_img2video" => template_id.to_string(),
        "img2video_direct" => "long_img2video_direct".to_string(),
        "img2video" | "storyboard_to_video" => "long_img2video".to_string(),
        "prompt_refine_txt2video" => "long_txt2video".to_string(),
        _ => template_id.to_string(),
    }
}

/// Split `target_secs` into clips of at most `max_clip_secs` (last clip may be shorter).
pub fn plan_segment_durations(target_secs: u32, max_clip_secs: u32) -> SegmentPlan {
    let target = target_secs.max(1);
    let max_clip = max_clip_secs.max(1);
    if target <= max_clip {
        return SegmentPlan {
            target_duration_secs: target,
            max_clip_secs: max_clip,
            segment_durations: vec![target],
        };
    }
    let mut remaining = target;
    let mut durations = Vec::new();
    while remaining > 0 {
        let clip = remaining.min(max_clip);
        durations.push(clip);
        remaining -= clip;
    }
    SegmentPlan {
        target_duration_secs: target,
        max_clip_secs: max_clip,
        segment_durations: durations,
    }
}

/// Motion/scene prompt tweak for continuation segments (after the first clip).
pub fn segment_video_prompt(base: &str, segment_index: usize, total: usize) -> String {
    if segment_index == 0 || total <= 1 {
        return base.trim().to_string();
    }
    let chinese = base.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    if chinese {
        format!(
            "{}。与上一段镜头无缝衔接，主体与场景连续，运动自然流畅（第 {}/{} 段）",
            base.trim(),
            segment_index + 1,
            total
        )
    } else {
        format!(
            "{}. Seamless continuation from the previous clip; consistent subject and scene; smooth motion (part {}/{})",
            base.trim(),
            segment_index + 1,
            total
        )
    }
}

pub fn require_ffmpeg() -> Result<(), ToolError> {
    if which_ffmpeg().is_some() {
        Ok(())
    } else {
        Err(ToolError::ExecutionFailed(
            "ffmpeg is required for long video concat — install ffmpeg and ensure it is on PATH"
                .into(),
        ))
    }
}

fn which_ffmpeg() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join(if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            });
            candidate.is_file().then_some(candidate)
        })
    })
}

/// Extract the last frame of a local video to PNG (for next-segment first_frame).
pub async fn extract_last_frame_png(video_path: &Path, output_png: &Path) -> Result<(), ToolError> {
    require_ffmpeg()?;
    if !video_path.is_file() {
        return Err(ToolError::ExecutionFailed(format!(
            "segment video missing: {}",
            video_path.display()
        )));
    }
    if let Some(parent) = output_png.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("create frame dir: {e}")))?;
    }

    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-sseof",
            "-0.05",
            "-i",
        ])
        .arg(video_path)
        .args(["-vframes", "1", "-q:v", "2", "-y"])
        .arg(output_png)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("ffmpeg extract frame: {e}")))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError::ExecutionFailed(format!(
            "ffmpeg extract last frame failed: {err}"
        )));
    }
    Ok(())
}

/// Encode PNG bytes as a data URL for Seedance `first_frame` chaining.
pub fn png_file_to_data_url(path: &Path) -> Result<String, ToolError> {
    let bytes = std::fs::read(path)
        .map_err(|e| ToolError::ExecutionFailed(format!("read frame png: {e}")))?;
    png_bytes_to_data_url(&bytes)
}

pub fn png_bytes_to_data_url(bytes: &[u8]) -> Result<String, ToolError> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:image/png;base64,{b64}"))
}

/// Concatenate segment MP4s with ffmpeg (re-encode for codec consistency).
pub async fn concat_videos(segment_paths: &[PathBuf], output_path: &Path) -> Result<(), ToolError> {
    require_ffmpeg()?;
    if segment_paths.is_empty() {
        return Err(ToolError::ExecutionFailed(
            "no video segments to concat".into(),
        ));
    }
    if segment_paths.len() == 1 {
        tokio::fs::copy(&segment_paths[0], output_path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("copy single segment: {e}")))?;
        return Ok(());
    }

    let list_dir = output_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    tokio::fs::create_dir_all(&list_dir)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("create concat dir: {e}")))?;

    let list_path = list_dir.join(format!("concat_{}.txt", uuid::Uuid::new_v4()));
    let mut list_body = String::new();
    for path in segment_paths {
        let escaped = path.display().to_string().replace('\'', "'\\''");
        list_body.push_str(&format!("file '{escaped}'\n"));
    }
    tokio::fs::write(&list_path, list_body)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("write concat list: {e}")))?;

    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
        ])
        .arg(&list_path)
        .args([
            "-c:v",
            "libx264",
            "-crf",
            "18",
            "-preset",
            "fast",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-an",
            "-y",
        ])
        .arg(output_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("ffmpeg concat: {e}")))?;

    let _ = tokio::fs::remove_file(&list_path).await;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError::ExecutionFailed(format!(
            "ffmpeg concat failed: {err}"
        )));
    }
    Ok(())
}

/// Persist concatenated output as a [`MediaArtifact`].
pub async fn persist_concatenated_video(
    path: &Path,
    provider: &str,
    model: &str,
    duration_secs: u32,
) -> Result<crate::assets::MediaArtifact, ToolError> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("read concat output: {e}")))?;
    let mut artifact = persist_bytes(&bytes, "video/mp4", provider, model).await?;
    artifact.duration_secs = Some(duration_secs as f32);
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_20s_into_two_10s_clips() {
        let plan = plan_segment_durations(20, 10);
        assert_eq!(plan.segment_durations, vec![10, 10]);
        assert!(needs_long_video_pipeline(20, 10));
        assert!(!needs_long_video_pipeline(10, 10));
    }

    #[test]
    fn plan_25s_splits_three_clips() {
        let plan = plan_segment_durations(25, 10);
        assert_eq!(plan.segment_durations, vec![10, 10, 5]);
    }

    #[test]
    fn short_target_single_segment() {
        let plan = plan_segment_durations(8, 10);
        assert_eq!(plan.segment_durations, vec![8]);
    }

    #[test]
    fn parse_duration_from_chinese_text() {
        assert_eq!(
            parse_duration_secs_from_text("生成一段约20秒的产品视频"),
            Some(20)
        );
        assert_eq!(parse_duration_secs_from_text("make a 15s clip"), Some(15));
        assert_eq!(parse_duration_secs_from_text("short cat video"), None);
    }

    #[test]
    fn route_long_templates() {
        assert_eq!(
            route_long_video_template("prompt_refine_txt2video", 20, "seedance"),
            "long_txt2video"
        );
        assert_eq!(
            route_long_video_template("img2video_direct", 20, "seedance"),
            "long_img2video_direct"
        );
        assert_eq!(
            route_long_video_template("prompt_refine_txt2video", 8, "seedance"),
            "prompt_refine_txt2video"
        );
    }

    #[test]
    fn continuation_prompt_adds_segment_marker() {
        let p = segment_video_prompt("一只猫在奔跑", 1, 2);
        assert!(p.contains("2"));
        assert!(p.contains("猫"));
    }

    #[test]
    fn png_data_url_roundtrip_prefix() {
        let url = png_bytes_to_data_url(b"\x89PNG").expect("data url");
        assert!(url.starts_with("data:image/png;base64,"));
    }
}
