//! Meeting notes CLI handler.

pub async fn handle_cli_meeting(
    action: Option<String>,
    audio: Option<String>,
    title: Option<String>,
    mode: Option<String>,
    diarize: bool,
) -> Result<(), hermes_core::AgentError> {
    use hermes_config::{DiarizationProvider, MeetingConfig, MeetingTranscriptionMode, SttConfig};
    use hermes_tools::tools::meeting_notes::run_offline_pipeline;

    let hermes_home = hermes_config::hermes_home();
    let action = action.as_deref().unwrap_or("notes");

    match action {
        "notes" => {
            let audio_path = audio.ok_or_else(|| {
                hermes_core::AgentError::Config("meeting notes requires --audio <path>".into())
            })?;
            let title = title.unwrap_or_else(|| "会议".to_string());

            let mut meeting_cfg = MeetingConfig::default();
            if let Some(m) = mode.as_deref() {
                meeting_cfg.transcription_mode = Some(match m {
                    "realtime" => MeetingTranscriptionMode::Realtime,
                    _ => MeetingTranscriptionMode::Offline,
                });
            }
            if diarize {
                meeting_cfg.diarization_provider = Some(DiarizationProvider::Pyannote);
            }

            let llm_base = std::env::var("MEETING_LLM_BASE_URL")
                .or_else(|_| std::env::var("OPENAI_BASE_URL"))
                .unwrap_or_else(|_| "https://api.openai.com/v1".into());
            let llm_key = std::env::var("MEETING_LLM_API_KEY")
                .or_else(|_| std::env::var("OPENAI_API_KEY"))
                .unwrap_or_default();
            let llm_model =
                std::env::var("MEETING_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());

            println!("▶ Generating meeting notes for: {}", audio_path);
            let notes = run_offline_pipeline(
                &audio_path,
                &title,
                SttConfig::default(),
                meeting_cfg,
                &llm_base,
                &llm_key,
                &llm_model,
                &hermes_home,
                |state| {
                    use hermes_tools::tools::meeting_notes::SummarizeState;
                    match &state {
                        SummarizeState::Transcribing => println!("  ⟳ 转录中…"),
                        SummarizeState::Diarizing => println!("  ⟳ 说话人识别中…"),
                        SummarizeState::SummarizingChunk(i, n) => println!("  ⟳ 总结片段 {i}/{n}…"),
                        SummarizeState::MergingSummaries => println!("  ⟳ 合并摘要…"),
                        SummarizeState::WritingMemory => println!("  ⟳ 写入记忆…"),
                        SummarizeState::Done => println!("  ✓ 完成"),
                        SummarizeState::Warning(w) => println!("  ⚠ {w}"),
                    }
                },
            )
            .await
            .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;

            println!("\n# {}\n", notes.title);
            println!("**日期**: {}", notes.date);
            println!("\n## 摘要\n{}", notes.summary);

            if !notes.key_decisions.is_empty() {
                println!("\n## 关键决策");
                for d in &notes.key_decisions {
                    println!("- {d}");
                }
            }
            if !notes.action_items.is_empty() {
                println!("\n## 行动项");
                for a in &notes.action_items {
                    println!("- {a}");
                }
            }
            if !notes.risks.is_empty() {
                println!("\n## 风险");
                for r in &notes.risks {
                    println!("- {r}");
                }
            }
            if let Some(tf) = &notes.transcript_file {
                println!("\n📁 转录文件: {tf}");
            }
            println!("\n✓ 已写入记忆系统 (holographic facts + MEMORY.md)");
        }
        "record" => {
            println!("⚠ `hermes meeting record` requires a microphone source (Phase 2 runtime).");
            println!("  Run `hermes meeting notes --audio <recorded.wav>` after recording.");
        }
        _ => {
            println!("Unknown meeting action '{action}'. Available: notes, record");
        }
    }

    Ok(())
}
