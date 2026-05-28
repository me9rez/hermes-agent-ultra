//! hermes-audio — shared audio capture abstractions.
//!
//! This crate provides the `AudioCaptureSource` trait and common types used by
//! both the real-time voice-mode pipeline (`voice_mode.rs`) and the meeting
//! recorder (`meeting_notes.rs`).
//!
//! # Architecture
//!
//! ```text
//!   MicSource          LoopbackSource
//!       └──────┬────────────┘
//!           DualTrackMixer (TaggedFrame)
//!                 │
//!            VAD / STT pipeline
//! ```
//!
//! `voice_mode.rs` uses `MicSource` directly (single-track, real-time dialogue).
//! `meeting_notes.rs` uses `DualTrackMixer` (two-track, meeting recording).

//! hermes-audio — shared audio capture abstractions.
//!
//! # Use Cases
//!
//! | Scenario | Configuration |
//! |----------|--------------|
//! | Internal team meetings | Dual-track (mic + loopback), Chinese notes, holographic facts |
//! | Sales calls / customer conversations | `loopback_only` mode, push to CRM (Linear/Notion) |
//! | Board / executive briefings | Strict grounding prompt, hallucination-free, source-line audit |
//! | Regulated / sensitive environments | Offline mode: faster-whisper local + local LLM, no network |
//! | Offline / low-connectivity workflows | Full offline: faster-whisper + ollama, transcript to file |
//!
//! # Architecture
//!
//! ```text
//!   MicSource          LoopbackSource
//!       └──────┬────────────┘
//!           DualTrackMixer (TaggedFrame)
//!                 │
//!         SilenceGuard (warn if no audio)
//!                 │
//!           VAD (Energy / Silero)
//!                 │
//!    [optional: filler word cleaner]
//!                 │
//!            STT callback
//!                 │
//!         TranscriptSegment (speaker, text, offset_s, audio_file)
//! ```
//!
//! `voice_mode.rs` uses `MicSource` directly (single-track, real-time dialogue).
//! `meeting_notes.rs` uses `DualTrackMixer` (two-track, meeting recording).

pub mod capture;
pub mod devices;
pub mod frame;
pub mod keepawake;
pub mod loopback;
pub mod mixer;
pub mod process_watch;
pub mod recorder;
pub mod vad;

pub use capture::AudioCaptureSource;
pub use devices::{AudioDeviceInfo, AudioDeviceManager, DeviceKind};
pub use frame::{AudioChannel, TaggedFrame};
pub use keepawake::KeepAwakeGuard;
pub use loopback::LoopbackSource;
pub use mixer::DualTrackMixer;
pub use process_watch::{detect_meeting_process, ProcessWatcher};
pub use recorder::{
    pcm_to_wav, MeetingRecorder, NodeStats, PipelineStats, SilenceGuard, StatsHandle,
    SttCallback, TranscriptSegment,
};
pub use vad::{create_vad, EnergyVad, VadBackend, VadConfig};
