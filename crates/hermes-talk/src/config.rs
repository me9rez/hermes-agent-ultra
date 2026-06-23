use std::path::Path;

use serde::Deserialize;

use crate::error::{DemoError, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub dashscope: DashscopeConfig,
    #[serde(default)]
    pub asr: AsrConfig,
    #[serde(default)]
    pub tts: TtsConfig,
    pub llm: LlmConfig,
    #[serde(default)]
    pub orchestrator: OrchestratorConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub wake: WakeConfig,
    #[serde(default)]
    pub denoise: DenoiseConfig,
    #[serde(default)]
    pub speaker: SpeakerConfig,
    #[serde(default)]
    pub vad: VadConfig,
    #[serde(default)]
    pub aec: AecConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DashscopeConfig {
    pub api_key: String,
    #[serde(default = "default_ws_url")]
    pub ws_url: String,
}

impl Default for DashscopeConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            ws_url: default_ws_url(),
        }
    }
}

fn default_ws_url() -> String {
    "wss://dashscope.aliyuncs.com/api-ws/v1/inference".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct AsrConfig {
    /// "bailian" (default) or "local"/"rockchip"
    #[serde(default = "default_asr_backend")]
    pub backend: String,
    #[serde(default = "default_asr_model")]
    pub model: String,
    #[serde(default = "default_16k")]
    pub sample_rate: u32,
    #[serde(default = "default_chunk_ms")]
    pub chunk_ms: u32,
    #[serde(default = "default_pcm")]
    pub format: String,
    /// Language hints for recognition (e.g. ["zh","en","ja","yue","ko"])
    #[serde(default)]
    pub language_hints: Option<Vec<String>>,
    /// Configuration for local Rockchip ASR backend
    #[serde(default)]
    pub local: Option<RockchipAsrConfig>,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            backend: default_asr_backend(),
            model: default_asr_model(),
            sample_rate: default_16k(),
            chunk_ms: default_chunk_ms(),
            format: default_pcm(),
            language_hints: None,
            local: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RockchipAsrConfig {
    /// Path to ASR SDK data directory (containing llmasr.rkllm, etc.)
    #[serde(default = "default_rkasr_data_path")]
    pub data_path: String,
    /// Inline JSON auth config, e.g. `{ "trial": 1, "license_path": "key.lic" }`
    /// Written to a temp file and passed to ROCKX2 at init.
    #[serde(default = "default_rkasr_auth_config")]
    pub auth_config: String,
}

impl Default for RockchipAsrConfig {
    fn default() -> Self {
        Self {
            data_path: default_rkasr_data_path(),
            auth_config: default_rkasr_auth_config(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TtsConfig {
    /// "bailian" (default) or "local"/"rockchip"
    #[serde(default = "default_tts_backend")]
    pub backend: String,
    #[serde(default = "default_tts_model")]
    pub model: String,
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default = "default_24k")]
    pub sample_rate: u32,
    #[serde(default = "default_pcm")]
    pub format: String,
    /// Language hints for synthesis quality (e.g. ["zh","en","ja","yue","ko"])
    #[serde(default)]
    pub language_hints: Option<Vec<String>>,
    /// Configuration for local Rockchip TTS backend
    #[serde(default)]
    pub local: Option<RockchipTtsConfig>,
    /// Deprecated alias for local config
    #[serde(default)]
    pub rockchip: Option<RockchipTtsConfig>,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            backend: default_tts_backend(),
            model: default_tts_model(),
            voice: default_voice(),
            sample_rate: default_24k(),
            format: default_pcm(),
            language_hints: None,
            local: None,
            rockchip: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RockchipTtsConfig {
    /// JSON auth config, e.g. `{ "trial": 1, "license_path": "key.lic" }`
    #[serde(default = "default_rktts_auth")]
    pub auth_config: String,
    /// Path to directory containing model files (am_part1.data, am_part2.data, voc.data)
    #[serde(default = "default_rktts_model_path")]
    pub model_path: String,
    /// Path to directory containing frontend dictionary files
    #[serde(default = "default_rktts_dicts_path")]
    pub dicts_path: String,
    /// Voice timbre: 0, 1, or 2 (only 3 voices supported)
    #[serde(default)]
    pub speaker_id: i32,
    /// Speed adjustment (1.0 = normal)
    #[serde(default = "default_rktts_alpha")]
    pub alpha: f32,
}

impl Default for RockchipTtsConfig {
    fn default() -> Self {
        Self {
            auth_config: default_rktts_auth(),
            model_path: default_rktts_model_path(),
            dicts_path: default_rktts_dicts_path(),
            speaker_id: 0,
            alpha: default_rktts_alpha(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_warmup_on_start")]
    pub warmup_on_start: bool,
    #[serde(default)]
    pub thinking_enabled: bool,
    #[serde(default)]
    pub thinking_budget: Option<u32>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default = "default_user_id")]
    pub user_id: String,
    #[serde(default)]
    pub tools_enabled: bool,
    #[serde(default = "default_execute_allowlist")]
    pub execute_allowlist: Vec<String>,
    #[serde(default)]
    pub aipc_talk: AipcTalkConfig,
}

fn default_user_id() -> String {
    "user".to_string()
}

fn default_execute_allowlist() -> Vec<String> {
    vec![
        "date",
        "uptime",
        "uname",
        "whoami",
        "pwd",
        "ls",
        "cat",
        "head",
        "tail",
        "echo",
        "wc",
        "du",
        "df",
        "free",
        "ps",
        "ping",
        "curl",
        "which",
        "hostname",
        "id",
        "env",
        "grep",
        "find",
        "sort",
        // Windows executables
        "powershell",
        "cmd",
        "findstr",
        "ipconfig",
        "systeminfo",
        "tasklist",
        "where",
        "netstat",
        "nslookup",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect()
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrchestratorConfig {
    /// Legacy; used only if `endpoint_silence_ms` is not set in old configs.
    #[serde(default = "default_min_silence")]
    pub min_silence_ms: u32,
    #[serde(default = "default_endpoint_silence")]
    pub endpoint_silence_ms: u32,
    #[serde(default = "default_trigger_on_asr_final")]
    pub trigger_on_asr_final: bool,
    #[serde(default = "default_cold_start")]
    pub cold_start_sec: u64,
    #[serde(default = "default_min_final")]
    pub min_final_chars: usize,
    /// During AwakeGrace / IdleAfterTurn, require this many chars before promoting to Active
    #[serde(default = "default_grace_min_final")]
    pub grace_min_final_chars: usize,
    #[serde(default = "default_sentence_len")]
    pub sentence_min_len: usize,
    #[serde(default = "default_tts_first_chunk")]
    pub tts_first_chunk_chars: usize,
    #[serde(default = "default_barge_frames")]
    pub barge_in_frames: u32,
    #[serde(default = "default_true")]
    pub barge_in_enabled: bool,
    #[serde(default)]
    pub speculative_llm: bool,
    #[serde(default = "default_speculative_stable")]
    pub speculative_stable_ms: u32,
    /// Phase 1: near-field RMS energy threshold for triggering new turns (0=disabled)
    #[serde(default)]
    pub min_rms_trigger: f32,
    /// Phase 1: near-field RMS energy threshold for barge-in (0=disabled)
    #[serde(default)]
    pub min_rms_barge_in: f32,
    /// Phase 2: consecutive speech frames required before barge-in fires
    #[serde(default = "default_barge_sustain")]
    pub barge_in_sustain_frames: u32,
    /// Phase 2: cooldown after barge-in before another barge-in is allowed (ms)
    #[serde(default = "default_barge_cooldown")]
    pub barge_in_cooldown_ms: u64,
    /// When wake is enabled, require wake word for barge-in (false = VAD also works)
    #[serde(default = "default_true")]
    pub barge_in_requires_wake: bool,
    /// Max conversation messages to keep in LLM context (0 = unlimited)
    #[serde(default = "default_max_context_messages")]
    pub max_context_messages: usize,
    /// WebRTC VAD aggressiveness: 0=Quality, 1=LowBitrate, 2=Aggressive, 3=VeryAggressive
    #[serde(default = "default_vad_mode")]
    pub vad_mode: u8,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            min_silence_ms: default_min_silence(),
            endpoint_silence_ms: default_endpoint_silence(),
            trigger_on_asr_final: default_trigger_on_asr_final(),
            cold_start_sec: default_cold_start(),
            min_final_chars: default_min_final(),
            grace_min_final_chars: default_grace_min_final(),
            sentence_min_len: default_sentence_len(),
            tts_first_chunk_chars: default_tts_first_chunk(),
            barge_in_frames: default_barge_frames(),
            barge_in_enabled: true,
            speculative_llm: false,
            speculative_stable_ms: default_speculative_stable(),
            min_rms_trigger: 0.0,
            min_rms_barge_in: 0.0,
            barge_in_sustain_frames: default_barge_sustain(),
            barge_in_cooldown_ms: default_barge_cooldown(),
            barge_in_requires_wake: true,
            max_context_messages: default_max_context_messages(),
            vad_mode: default_vad_mode(),
        }
    }
}

impl OrchestratorConfig {
    pub fn endpoint_silence_ms(&self) -> u32 {
        self.endpoint_silence_ms
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WakeConfig {
    #[serde(default = "default_wake_enabled")]
    pub enabled: bool,
    /// Spoken immediately after wake word is detected (empty to disable).
    #[serde(default = "default_wake_ack_reply")]
    pub ack_reply: String,
    /// Wake phrases; encoded at startup via sherpa-onnx text2token.
    #[serde(default)]
    pub phrases: Vec<String>,
    /// Deprecated: use `phrases = ["…"]`; merged in [`WakeConfig::normalize`].
    #[serde(default)]
    pub phrase: Option<String>,
    #[serde(default = "default_wake_model_dir")]
    pub model_dir: String,
    #[serde(default)]
    pub encoder: String,
    #[serde(default)]
    pub decoder: String,
    #[serde(default)]
    pub joiner: String,
    #[serde(default)]
    pub tokens: String,
    /// Modeling units for text2token (`phone+ppinyin` for zh-en KWS model).
    #[serde(default = "default_wake_tokens_type")]
    pub tokens_type: String,
    #[serde(default)]
    pub bpe_model: String,
    #[serde(default)]
    pub lexicon: String,
    #[serde(default = "default_wake_boost")]
    pub boost_score: f32,
    #[serde(default = "default_wake_threshold")]
    pub trigger_threshold: f32,
    #[serde(default = "default_grace_after_wake")]
    pub grace_after_wake_sec: u64,
    #[serde(default = "default_idle_after_turn")]
    pub idle_after_turn_sec: u64,
    #[serde(default = "default_kws_threads")]
    pub num_threads: i32,
}

impl Default for WakeConfig {
    fn default() -> Self {
        Self {
            enabled: default_wake_enabled(),
            ack_reply: default_wake_ack_reply(),
            phrases: vec![default_wake_phrase()],
            phrase: None,
            model_dir: "models/kws-zh-en".to_string(),
            encoder: String::new(),
            decoder: String::new(),
            joiner: String::new(),
            tokens: String::new(),
            tokens_type: default_wake_tokens_type(),
            bpe_model: String::new(),
            lexicon: String::new(),
            boost_score: default_wake_boost(),
            trigger_threshold: default_wake_threshold(),
            grace_after_wake_sec: default_grace_after_wake(),
            idle_after_turn_sec: default_idle_after_turn(),
            num_threads: default_kws_threads(),
        }
    }
}

impl WakeConfig {
    pub fn normalize(&mut self) {
        if self.phrases.is_empty() {
            if let Some(p) = self.phrase.take() {
                if !p.trim().is_empty() {
                    self.phrases.push(p);
                }
            }
        }
        self.resolve_paths();
    }

    pub fn effective_phrases(&self) -> Vec<String> {
        self.phrases
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn resolve_paths(&mut self) {
        if self.model_dir.is_empty() {
            return;
        }
        let dir = self.model_dir.trim_end_matches(['/', '\\']);
        if self.encoder.is_empty() {
            self.encoder = format!("{dir}/encoder.onnx");
        }
        if self.decoder.is_empty() {
            self.decoder = format!("{dir}/decoder.onnx");
        }
        if self.joiner.is_empty() {
            self.joiner = format!("{dir}/joiner.onnx");
        }
        if self.tokens.is_empty() {
            self.tokens = format!("{dir}/tokens.txt");
        }
        if self.lexicon.is_empty() && self.tokens_type == "phone+ppinyin" {
            self.lexicon = format!("{dir}/en.phone");
        }
        if self.bpe_model.is_empty()
            && (self.tokens_type == "bpe" || self.tokens_type == "cjkchar+bpe")
        {
            self.bpe_model = format!("{dir}/bpe.model");
        }
    }

    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.effective_phrases().is_empty() {
            return Err(DemoError::Config(
                "wake.phrases is empty; add at least one phrase".into(),
            ));
        }
        if self.grace_after_wake_sec == 0 {
            return Err(DemoError::Config(
                "wake.grace_after_wake_sec must be >= 1".into(),
            ));
        }
        if self.idle_after_turn_sec == 0 {
            return Err(DemoError::Config(
                "wake.idle_after_turn_sec must be >= 1".into(),
            ));
        }
        for (name, path) in [
            ("encoder", &self.encoder),
            ("decoder", &self.decoder),
            ("joiner", &self.joiner),
            ("tokens", &self.tokens),
        ] {
            if path.is_empty() {
                return Err(DemoError::Config(format!(
                    "wake.{name} is empty; set wake.model_dir or explicit paths"
                )));
            }
            if !std::path::Path::new(path).exists() {
                return Err(DemoError::Config(format!("wake.{name} not found: {path}")));
            }
        }
        if (self.tokens_type == "bpe" || self.tokens_type == "cjkchar+bpe")
            && !self.bpe_model.is_empty()
            && !std::path::Path::new(&self.bpe_model).exists()
        {
            return Err(DemoError::Config(format!(
                "wake.bpe_model not found: {}",
                self.bpe_model
            )));
        }
        if self.tokens_type == "phone+ppinyin"
            && !self.lexicon.is_empty()
            && !std::path::Path::new(&self.lexicon).exists()
        {
            return Err(DemoError::Config(format!(
                "wake.lexicon not found: {}",
                self.lexicon
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AudioConfig {
    #[serde(default)]
    pub input_device: String,
    #[serde(default)]
    pub output_device: String,
}

fn default_asr_backend() -> String {
    "bailian".to_string()
}
fn default_tts_backend() -> String {
    "bailian".to_string()
}
fn default_rkasr_data_path() -> String {
    "data".to_string()
}
fn default_rkasr_auth_config() -> String {
    r#"{ "trial": 1, "rkauth_modules_config": [{ "module": "asr", "license_path": "key.lic" }] }"#
        .to_string()
}
fn default_asr_model() -> String {
    "fun-asr-realtime".to_string()
}
fn default_tts_model() -> String {
    "cosyvoice-v3-flash".to_string()
}
fn default_voice() -> String {
    "longanyang".to_string()
}
fn default_rktts_auth() -> String {
    r#"{ "trial": 1, "license_path": "key.lic" }"#.to_string()
}
fn default_rktts_model_path() -> String {
    "models".to_string()
}
fn default_rktts_dicts_path() -> String {
    "frontend_extras".to_string()
}
fn default_rktts_alpha() -> f32 {
    1.0
}
fn default_16k() -> u32 {
    16000
}
fn default_24k() -> u32 {
    24000
}
fn default_chunk_ms() -> u32 {
    100
}
fn default_pcm() -> String {
    "pcm".to_string()
}
fn default_min_silence() -> u32 {
    450
}
fn default_endpoint_silence() -> u32 {
    150
}
fn default_trigger_on_asr_final() -> bool {
    true
}
fn default_cold_start() -> u64 {
    3
}
fn default_min_final() -> usize {
    2
}
fn default_grace_min_final() -> usize {
    3
}
fn default_sentence_len() -> usize {
    12
}
fn default_tts_first_chunk() -> usize {
    6
}
fn default_true() -> bool {
    true
}
fn default_barge_frames() -> u32 {
    2
}
fn default_speculative_stable() -> u32 {
    300
}
fn default_system_prompt() -> String {
    "口语助手，先短答。".to_string()
}
fn default_max_tokens() -> u32 {
    80
}
fn default_temperature() -> f32 {
    0.7
}
fn default_warmup_on_start() -> bool {
    true
}
fn default_wake_enabled() -> bool {
    false
}
fn default_wake_ack_reply() -> String {
    "哎，我在！".to_string()
}
fn default_wake_model_dir() -> String {
    "models/kws-zh-en".to_string()
}
fn default_wake_phrase() -> String {
    "小智小智".to_string()
}
fn default_wake_tokens_type() -> String {
    "phone+ppinyin".to_string()
}
fn default_wake_boost() -> f32 {
    2.0
}
fn default_wake_threshold() -> f32 {
    0.35
}
fn default_grace_after_wake() -> u64 {
    5
}
fn default_idle_after_turn() -> u64 {
    30
}
fn default_kws_threads() -> i32 {
    1
}
fn default_barge_sustain() -> u32 {
    4
}
fn default_barge_cooldown() -> u64 {
    1000
}
fn default_max_context_messages() -> usize {
    20
}
fn default_vad_mode() -> u8 {
    3
}
fn default_denoise_model_dir() -> String {
    "models/denoise".to_string()
}
fn default_speaker_model_dir() -> String {
    "models/speaker".to_string()
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DenoiseConfig {
    #[serde(default)]
    pub enabled: bool,
    /// DPDFNet: "dpdfnet_baseline", "dpdfnet2", "dpdfnet4", "dpdfnet8"
    /// GTCRN: "gtcrn_simple"
    #[serde(default = "default_denoise_variant")]
    pub variant: String,
    /// Path to ONNX model file; auto-generated from model_dir + variant if empty
    #[serde(default)]
    pub model_path: String,
    #[serde(default = "default_denoise_model_dir")]
    pub model_dir: String,
}

impl DenoiseConfig {
    pub fn resolve_model_path(&self) -> String {
        if !self.model_path.is_empty() {
            return self.model_path.clone();
        }
        let dir = self.model_dir.trim_end_matches(['/', '\\']);
        format!("{dir}/{}.onnx", self.variant)
    }
}

fn default_denoise_variant() -> String {
    "dpdfnet_baseline".to_string()
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SpeakerConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Cosine similarity threshold (0.0-1.0), higher = stricter
    #[serde(default = "default_speaker_threshold")]
    pub threshold: f32,
    /// Path to ONNX model file
    #[serde(default)]
    pub model_path: String,
    #[serde(default = "default_speaker_model_dir")]
    pub model_dir: String,
    /// Saved voiceprint file
    #[serde(default = "default_voiceprint_path")]
    pub voiceprint_path: String,
}

impl SpeakerConfig {
    pub fn resolve_model_path(&self) -> String {
        if !self.model_path.is_empty() {
            return self.model_path.clone();
        }
        let dir = self.model_dir.trim_end_matches(['/', '\\']);
        format!("{dir}/3dspeaker.onnx")
    }
}

fn default_speaker_threshold() -> f32 {
    0.6
}

fn default_voiceprint_path() -> String {
    "voiceprint.bin".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct VadConfig {
    /// Path to silero_vad.onnx model
    #[serde(default = "default_vad_model_path")]
    pub model_path: String,
    /// Speech probability threshold (0.0-1.0); lower = more sensitive
    #[serde(default = "default_vad_threshold")]
    pub threshold: f32,
    /// Minimum silence duration to end a speech segment (seconds)
    #[serde(default = "default_vad_min_silence")]
    pub min_silence_duration: f32,
    /// Minimum speech duration to start a segment (seconds)
    #[serde(default = "default_vad_min_speech")]
    pub min_speech_duration: f32,
    /// Maximum speech segment duration (seconds)
    #[serde(default = "default_vad_max_speech")]
    pub max_speech_duration: f32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            model_path: default_vad_model_path(),
            threshold: default_vad_threshold(),
            min_silence_duration: default_vad_min_silence(),
            min_speech_duration: default_vad_min_speech(),
            max_speech_duration: default_vad_max_speech(),
        }
    }
}

fn default_vad_model_path() -> String {
    "models/vad/silero_vad.onnx".to_string()
}
fn default_vad_threshold() -> f32 {
    0.5
}
fn default_vad_min_silence() -> f32 {
    0.4
}
fn default_vad_min_speech() -> f32 {
    0.25
}
fn default_vad_max_speech() -> f32 {
    15.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct AecConfig {
    /// Enable acoustic echo cancellation (requires aec-rs/speexdsp)
    #[serde(default)]
    pub enabled: bool,
    /// Frame size in samples (must be power of 2). Default 256 = 16ms @16kHz.
    #[serde(default = "default_aec_frame_size")]
    pub frame_size: usize,
    /// Echo tail length in samples. Default 2048 ≈ 128ms @16kHz.
    #[serde(default = "default_aec_filter_length")]
    pub filter_length: i32,
    /// Enable Speex preprocessor (noise suppression + AGC + dereverb) after AEC
    #[serde(default = "default_aec_preprocess")]
    pub enable_preprocess: bool,
    /// Delay in ms from reference playback to mic capture. 0 = auto-detect.
    #[serde(default)]
    pub delay_ms: u32,
}

impl Default for AecConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            frame_size: default_aec_frame_size(),
            filter_length: default_aec_filter_length(),
            enable_preprocess: default_aec_preprocess(),
            delay_ms: 0,
        }
    }
}

fn default_aec_frame_size() -> usize {
    256
}
fn default_aec_filter_length() -> i32 {
    2048
}
fn default_aec_preprocess() -> bool {
    false
}

#[derive(Debug, Clone, Deserialize)]
pub struct AipcTalkConfig {
    #[serde(default = "default_aipc_talk_url")]
    pub url: String,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    pub model: Option<String>,
    pub provider: Option<String>,
}

impl Default for AipcTalkConfig {
    fn default() -> Self {
        Self {
            url: default_aipc_talk_url(),
            timeout_secs: None,
            model: None,
            provider: None,
        }
    }
}

fn default_aipc_talk_url() -> String {
    "ws://127.0.0.1:9100".to_string()
}

impl Config {
    /// Load config from `$HERMES_HOME/hermes-talk/config.toml`, resolving relative paths against that directory.
    pub fn load_from_home() -> Result<Self> {
        let home = hermes_config::talk_dir();
        let path = hermes_config::talk_config_path();
        Self::load_with_base(&path, &home)
    }

    /// Load config from `path`, resolving relative paths against `base`.
    pub fn load_with_base(path: impl AsRef<Path>, base: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let base = base.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|e| {
            let hint = if path == hermes_config::talk_config_path() {
                format!(
                    "read {}: {e}. Run `hermes talk init` to create the talk home directory.",
                    path.display()
                )
            } else {
                format!("read {}: {e}", path.display())
            };
            DemoError::Config(hint)
        })?;
        let mut cfg: Config =
            toml::from_str(&raw).map_err(|e| DemoError::Config(format!("parse toml: {e}")))?;

        cfg.resolve_paths_against(base);
        cfg.wake.normalize();
        cfg.wake.validate()?;
        Ok(cfg)
    }

    /// Load config from `path` without a base directory (paths used as-is; for tests).
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let base = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        Self::load_with_base(path, base)
    }

    fn resolve_paths_against(&mut self, base: &Path) {
        if let Some(ref mut local) = self.asr.local {
            local.data_path = join_if_relative(base, &local.data_path);
        }
        if let Some(ref mut local) = self.tts.local {
            local.model_path = join_if_relative(base, &local.model_path);
            local.dicts_path = join_if_relative(base, &local.dicts_path);
        }
        if let Some(ref mut rockchip) = self.tts.rockchip {
            rockchip.model_path = join_if_relative(base, &rockchip.model_path);
            rockchip.dicts_path = join_if_relative(base, &rockchip.dicts_path);
        }
        self.wake.model_dir = join_if_relative(base, &self.wake.model_dir);
        self.wake.encoder = join_if_relative(base, &self.wake.encoder);
        self.wake.decoder = join_if_relative(base, &self.wake.decoder);
        self.wake.joiner = join_if_relative(base, &self.wake.joiner);
        self.wake.tokens = join_if_relative(base, &self.wake.tokens);
        self.wake.bpe_model = join_if_relative(base, &self.wake.bpe_model);
        self.wake.lexicon = join_if_relative(base, &self.wake.lexicon);
        self.denoise.model_path = join_if_relative(base, &self.denoise.model_path);
        self.denoise.model_dir = join_if_relative(base, &self.denoise.model_dir);
        self.speaker.model_path = join_if_relative(base, &self.speaker.model_path);
        self.speaker.model_dir = join_if_relative(base, &self.speaker.model_dir);
        self.speaker.voiceprint_path = join_if_relative(base, &self.speaker.voiceprint_path);
        self.vad.model_path = join_if_relative(base, &self.vad.model_path);
    }
}

fn join_if_relative(base: &Path, path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let p = Path::new(path);
    if p.is_absolute() {
        path.to_string()
    } else {
        base.join(p).to_string_lossy().into_owned()
    }
}
