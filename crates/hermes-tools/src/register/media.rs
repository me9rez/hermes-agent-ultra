//! Media tool registrations: vision, image/video generation, TTS, transcription.
//!
//! Preconditions:
//! - vision_analyze / video_analyze: injected `VisionBackend` (auxiliary LLM).
//! - image_gen: FAL_KEY env var or Nous-managed fal-queue gateway.
//! - video_gen: backend-dependent env vars resolved at startup.
//! - tts / tts_premium: TtsConfig from gateway; ELEVENLABS_API_KEY for premium.
//! - transcription: VOICE_TOOLS_OPENAI_KEY / OPENAI_API_KEY / STT_OPENAI_BASE_URL.

use std::sync::Arc;

use super::{RegistryContext, reg, reg_with_check};

pub fn register(ctx: &RegistryContext<'_>) {
    if let Some(vision_backend) = &ctx.vision_backend {
        reg(
            ctx,
            "vision",
            Arc::new(crate::tools::vision::VisionAnalyzeHandler::new(
                vision_backend.clone(),
            )),
            "👁️",
            vec![],
        );
        reg(
            ctx,
            "vision",
            Arc::new(crate::tools::video::VideoAnalyzeHandler::new(Arc::new(
                crate::backends::video::VisionFrameSamplingVideoBackend::new(
                    vision_backend.clone(),
                ),
            ))),
            "🎬",
            vec![],
        );
    } else {
        tracing::debug!("Skipping vision_analyze/video_analyze — no VisionBackend injected");
    }

    {
        let backend = crate::backends::image_gen::FalImageGenBackend::from_env_or_managed()
            .unwrap_or_else(|_| crate::backends::image_gen::FalImageGenBackend::unconfigured());
        reg_with_check(
            ctx,
            "image_gen",
            Arc::new(crate::tools::image_gen::ImageGenerateHandler::new(
                Arc::new(backend),
            )),
            "🎨",
            vec!["FAL_KEY".into()],
            Arc::new(crate::backends::image_gen::FalImageGenBackend::image_gen_is_configured),
        );
    }

    {
        let backend = crate::backends::video_gen::VideoGenBackend::from_env_or_managed();
        let env_deps = backend.required_env_vars();
        reg_with_check(
            ctx,
            "video_gen",
            Arc::new(crate::tools::video::VideoGenerateHandler::new(Arc::new(
                backend,
            ))),
            "🎞️",
            env_deps,
            Arc::new(crate::backends::video_gen::VideoGenBackend::video_gen_is_configured),
        );
    }

    let tts_backend = Arc::new(crate::backends::tts::MultiTtsBackend::with_config(
        ctx.tts_cfg.clone(),
    ));
    reg(
        ctx,
        "tts",
        Arc::new(crate::tools::tts::TextToSpeechHandler::new(
            tts_backend.clone(),
        )),
        "🔊",
        vec![],
    );
    reg(
        ctx,
        "tts",
        Arc::new(crate::tools::tts_premium::TtsPremiumHandler::new(
            tts_backend,
        )),
        "🎵",
        vec!["ELEVENLABS_API_KEY".into()],
    );

    reg(
        ctx,
        "voice",
        Arc::new(
            crate::tools::transcription::TranscriptionHandler::with_config(ctx.stt_cfg.clone()),
        ),
        "🎙️",
        vec![
            "VOICE_TOOLS_OPENAI_KEY".into(),
            "HERMES_OPENAI_API_KEY".into(),
            "OPENAI_API_KEY".into(),
            "STT_OPENAI_BASE_URL".into(),
        ],
    );
    reg(
        ctx,
        "voice",
        Arc::new(crate::tools::voice_mode::VoiceModeHandler::default()),
        "🎤",
        vec![],
    );
}
