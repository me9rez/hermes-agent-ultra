//! Offline SenseVoice ASR via sherpa-onnx (Windows / x86 CPU).

use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig};
use tokio::sync::mpsc as async_mpsc;
use tracing::{error, info, warn};

use crate::asr::{AsrEngine, AsrEvent};
use crate::config::SherpaAsrConfig;
use crate::error::{DemoError, Result};

enum AsrCommand {
    FinishUtterance,
    SetPaused(bool),
    SetGate(bool),
    ResetBuffer,
}

struct PcmState {
    samples: Vec<i16>,
    paused: bool,
    gated: bool,
    last_partial_decode: Instant,
    last_partial_text: String,
    min_partial_samples: usize,
}

struct SherpaAsrInner {
    pcm: Mutex<PcmState>,
    cmd_tx: mpsc::SyncSender<AsrCommand>,
}

/// How often to emit interim partial transcripts while the user is still speaking.
const PARTIAL_DECODE_INTERVAL: Duration = Duration::from_millis(250);
/// Skip expensive partial decodes once the utterance exceeds this length.
const MAX_PARTIAL_SAMPLES: usize = 16000 * 8; // ~8 s @ 16 kHz
const THREAD_POLL: Duration = Duration::from_millis(50);

pub struct SherpaAsr {
    inner: Arc<SherpaAsrInner>,
    _thread: JoinHandle<()>,
}

impl SherpaAsr {
    pub async fn connect(
        cfg: &SherpaAsrConfig,
        sample_rate: u32,
        start_paused: bool,
    ) -> Result<(Self, async_mpsc::Receiver<AsrEvent>)> {
        let mut recognizer_config = OfflineRecognizerConfig::default();
        recognizer_config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(cfg.model.clone()),
            language: Some(cfg.language.clone()),
            use_itn: cfg.use_itn,
        };
        recognizer_config.model_config.tokens = Some(cfg.tokens.clone());
        recognizer_config.model_config.provider = Some(cfg.provider.clone());
        recognizer_config.model_config.num_threads = cfg.num_threads;

        let recognizer = OfflineRecognizer::create(&recognizer_config).ok_or_else(|| {
            DemoError::Config(format!(
                "failed to create SenseVoice recognizer (check asr.sherpa model paths): model={}",
                cfg.model
            ))
        })?;

        let (cmd_tx, cmd_rx) = mpsc::sync_channel::<AsrCommand>(32);
        let (event_tx, event_rx) = async_mpsc::channel(64);

        let pcm = Mutex::new(PcmState {
            samples: Vec::new(),
            paused: start_paused,
            gated: false,
            last_partial_decode: Instant::now(),
            last_partial_text: String::new(),
            min_partial_samples: sample_rate as usize / 4, // ~250ms @ 16kHz
        });

        let inner = Arc::new(SherpaAsrInner { pcm, cmd_tx });

        let thread = thread::spawn({
            let inner = Arc::clone(&inner);
            move || {
                if let Err(e) = run_asr_loop(&inner, recognizer, sample_rate, cmd_rx, event_tx) {
                    error!(error = %e, "sherpa asr thread exited");
                }
            }
        });

        info!(
            model = %cfg.model,
            language = %cfg.language,
            sample_rate,
            "sherpa SenseVoice ASR ready"
        );

        Ok((
            Self {
                inner,
                _thread: thread,
            },
            event_rx,
        ))
    }

    fn send_cmd(&self, cmd: AsrCommand) -> Result<()> {
        self.inner
            .cmd_tx
            .send(cmd)
            .map_err(|e| DemoError::Asr(format!("sherpa asr command channel: {e}")))
    }
}

#[async_trait]
impl AsrEngine for SherpaAsr {
    async fn send_audio(&self, pcm: Vec<u8>) -> Result<()> {
        let mut pcm_state = self
            .inner
            .pcm
            .lock()
            .map_err(|e| DemoError::Asr(format!("sherpa asr pcm lock: {e}")))?;
        if pcm_state.paused || pcm_state.gated {
            return Ok(());
        }
        append_pcm_i16(&mut pcm_state.samples, &pcm);
        Ok(())
    }

    async fn pause(&self) -> Result<()> {
        self.send_cmd(AsrCommand::SetPaused(true))
    }

    async fn resume(&self) -> Result<()> {
        self.send_cmd(AsrCommand::SetPaused(false))
    }

    async fn set_gate(&self, on: bool) -> Result<()> {
        self.send_cmd(AsrCommand::SetGate(on))
    }

    async fn reconnect(&self) -> Result<()> {
        self.send_cmd(AsrCommand::ResetBuffer)
    }

    async fn finish_utterance(&self) -> Result<()> {
        self.send_cmd(AsrCommand::FinishUtterance)
    }
}

fn run_asr_loop(
    inner: &SherpaAsrInner,
    recognizer: OfflineRecognizer,
    sample_rate: u32,
    cmd_rx: mpsc::Receiver<AsrCommand>,
    event_tx: async_mpsc::Sender<AsrEvent>,
) -> Result<()> {
    loop {
        match cmd_rx.recv_timeout(THREAD_POLL) {
            Ok(AsrCommand::FinishUtterance) => {
                finish_utterance(&inner.pcm, &recognizer, sample_rate, &event_tx);
            }
            Ok(AsrCommand::SetPaused(on)) => {
                if let Ok(mut pcm) = inner.pcm.lock() {
                    pcm.paused = on;
                }
            }
            Ok(AsrCommand::SetGate(on)) => {
                if let Ok(mut pcm) = inner.pcm.lock() {
                    pcm.gated = on;
                    if on {
                        pcm.samples.clear();
                        pcm.last_partial_text.clear();
                    }
                }
            }
            Ok(AsrCommand::ResetBuffer) => {
                if let Ok(mut pcm) = inner.pcm.lock() {
                    pcm.samples.clear();
                    pcm.last_partial_text.clear();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                maybe_emit_partial(&inner.pcm, &recognizer, sample_rate, &event_tx);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn finish_utterance(
    pcm: &Mutex<PcmState>,
    recognizer: &OfflineRecognizer,
    sample_rate: u32,
    event_tx: &async_mpsc::Sender<AsrEvent>,
) {
    let mut state = match pcm.lock() {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "sherpa asr pcm lock poisoned on finish");
            return;
        }
    };
    state.last_partial_text.clear();
    if state.samples.is_empty() {
        return;
    }
    let sample_count = state.samples.len();
    let text = decode_buffer(recognizer, sample_rate, &state.samples).unwrap_or_default();
    state.samples.clear();

    if text.is_empty() {
        warn!(
            samples = sample_count,
            "sherpa asr: empty decode result on finish"
        );
        return;
    }

    info!(text = %text, samples = sample_count, "sherpa asr final");
    let _ = event_tx.blocking_send(AsrEvent::Final { text });
}

fn maybe_emit_partial(
    pcm: &Mutex<PcmState>,
    recognizer: &OfflineRecognizer,
    sample_rate: u32,
    event_tx: &async_mpsc::Sender<AsrEvent>,
) {
    let mut state = match pcm.lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    if state.paused || state.gated {
        return;
    }
    if state.samples.len() < state.min_partial_samples {
        return;
    }
    if state.samples.len() > MAX_PARTIAL_SAMPLES {
        return;
    }
    let now = Instant::now();
    if now.duration_since(state.last_partial_decode) < PARTIAL_DECODE_INTERVAL {
        return;
    }
    state.last_partial_decode = now;
    let Some(text) = decode_buffer(recognizer, sample_rate, &state.samples) else {
        return;
    };
    if text.is_empty() || text == state.last_partial_text {
        return;
    }
    info!(text = %text, samples = state.samples.len(), "sherpa asr partial");
    state.last_partial_text = text.clone();
    let _ = event_tx.blocking_send(AsrEvent::Partial { text, full: None });
}

fn decode_buffer(
    recognizer: &OfflineRecognizer,
    sample_rate: u32,
    buffer: &[i16],
) -> Option<String> {
    if buffer.is_empty() {
        return None;
    }
    let samples: Vec<f32> = buffer.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
    let stream = recognizer.create_stream();
    stream.accept_waveform(sample_rate as i32, &samples);
    recognizer.decode(&stream);
    stream.get_result().map(|r| r.text.trim().to_string())
}

fn append_pcm_i16(buf: &mut Vec<i16>, pcm: &[u8]) {
    let mut iter = pcm.chunks_exact(2);
    for chunk in iter.by_ref() {
        buf.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
}
