//! Offline sherpa-onnx TTS: Kokoro or ZipVoice (Windows / x86 CPU).

use async_trait::async_trait;
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsZipvoiceModelConfig, Wave,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info};

use crate::config::{SherpaKokoroTtsConfig, SherpaTtsRuntime, SherpaZipvoiceTtsConfig};
use crate::error::{DemoError, Result};
use crate::tts::{TtsEngine, bailian::TtsAudio};

enum TtsCommand {
    AppendText {
        text: String,
        done: oneshot::Sender<Result<()>>,
    },
    FinishTurn(oneshot::Sender<Result<()>>),
    InterruptTurn(oneshot::Sender<Result<()>>),
}

struct ZipvoiceReference {
    samples: Vec<f32>,
    sample_rate: i32,
    text: String,
}

pub struct SherpaTts {
    cmd_tx: mpsc::Sender<TtsCommand>,
}

impl SherpaTts {
    pub async fn connect(cfg: &SherpaTtsRuntime) -> Result<(Self, mpsc::Receiver<TtsAudio>)> {
        let (audio_tx, audio_rx) = mpsc::channel(128);
        let (cmd_tx, cmd_rx) = mpsc::channel::<TtsCommand>(32);
        let cfg = cfg.clone();

        tokio::task::spawn_blocking(move || {
            let result = if cfg.is_zipvoice() {
                run_zipvoice_driver(cfg, cmd_rx, audio_tx)
            } else {
                run_kokoro_driver(cfg, cmd_rx, audio_tx)
            };
            if let Err(e) = result {
                error!(error = %e, "sherpa tts driver exited");
            }
        });

        Ok((Self { cmd_tx }, audio_rx))
    }
}

#[async_trait]
impl TtsEngine for SherpaTts {
    async fn warmup(&self) -> Result<()> {
        Ok(())
    }

    async fn append_text(&self, text: &str) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(TtsCommand::AppendText {
                text: text.to_string(),
                done: tx,
            })
            .await
            .map_err(|e| DemoError::Tts(e.to_string()))?;
        rx.await.map_err(|e| DemoError::Tts(e.to_string()))?
    }

    async fn finish_turn(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(TtsCommand::FinishTurn(tx))
            .await
            .map_err(|e| DemoError::Tts(e.to_string()))?;
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => Err(DemoError::Tts(e.to_string())),
            Err(_) => Err(DemoError::Tts("sherpa tts finish-turn timeout".into())),
        }
    }

    async fn interrupt_turn(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(TtsCommand::InterruptTurn(tx))
            .await
            .map_err(|e| DemoError::Tts(e.to_string()))?;
        rx.await.map_err(|e| DemoError::Tts(e.to_string()))?
    }
}

fn run_driver_loop(
    mut cmd_rx: mpsc::Receiver<TtsCommand>,
    audio_tx: mpsc::Sender<TtsAudio>,
    tts: &OfflineTts,
    synthesize: impl Fn(&OfflineTts, &str, &mpsc::Sender<TtsAudio>) -> Result<()>,
) -> Result<()> {
    let mut text_buf = String::new();

    while let Some(cmd) = cmd_rx.blocking_recv() {
        match cmd {
            TtsCommand::AppendText { text, done } => {
                text_buf.push_str(&text);
                let _ = done.send(Ok(()));
            }
            TtsCommand::FinishTurn(done) => {
                if text_buf.is_empty() {
                    let _ = done.send(Ok(()));
                    continue;
                }
                let text = std::mem::take(&mut text_buf);
                let result = synthesize(tts, &text, &audio_tx);
                let _ = done.send(result);
            }
            TtsCommand::InterruptTurn(done) => {
                text_buf.clear();
                let _ = done.send(Ok(()));
            }
        }
    }
    Ok(())
}

fn run_kokoro_driver(
    cfg: SherpaTtsRuntime,
    cmd_rx: mpsc::Receiver<TtsCommand>,
    audio_tx: mpsc::Sender<TtsAudio>,
) -> Result<()> {
    let kokoro_cfg = cfg.kokoro.clone();
    let kokoro = OfflineTtsKokoroModelConfig {
        model: Some(kokoro_cfg.model.clone()),
        voices: Some(kokoro_cfg.voices.clone()),
        tokens: Some(kokoro_cfg.tokens.clone()),
        data_dir: Some(kokoro_cfg.data_dir.clone()),
        dict_dir: Some(kokoro_cfg.dict_dir.clone()),
        lexicon: Some(kokoro_cfg.lexicon.clone()),
        length_scale: kokoro_cfg.length_scale,
        lang: kokoro_cfg.lang.clone(),
    };

    let tts_config = OfflineTtsConfig {
        model: sherpa_onnx::OfflineTtsModelConfig {
            kokoro,
            num_threads: cfg.num_threads,
            provider: Some(cfg.provider.clone()),
            debug: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let tts = OfflineTts::create(&tts_config).ok_or_else(|| {
        DemoError::Config(format!(
            "failed to create Kokoro TTS (check tts.sherpa.kokoro paths): model={}",
            kokoro_cfg.model
        ))
    })?;

    info!(
        engine = "kokoro",
        model = %kokoro_cfg.model,
        provider = %cfg.provider,
        sample_rate = tts.sample_rate(),
        speakers = tts.num_speakers(),
        sid = kokoro_cfg.sid,
        "sherpa Kokoro TTS ready"
    );

    run_driver_loop(cmd_rx, audio_tx, &tts, move |tts, text, audio_tx| {
        synthesize_kokoro_turn(tts, &kokoro_cfg, text, audio_tx)
    })
}

fn run_zipvoice_driver(
    cfg: SherpaTtsRuntime,
    cmd_rx: mpsc::Receiver<TtsCommand>,
    audio_tx: mpsc::Sender<TtsAudio>,
) -> Result<()> {
    let zip_cfg = cfg.zipvoice.clone();
    let zipvoice = OfflineTtsZipvoiceModelConfig {
        tokens: Some(zip_cfg.tokens.clone()),
        encoder: Some(zip_cfg.encoder.clone()),
        decoder: Some(zip_cfg.decoder.clone()),
        vocoder: Some(zip_cfg.vocoder.clone()),
        data_dir: Some(zip_cfg.data_dir.clone()),
        lexicon: Some(zip_cfg.lexicon.clone()),
        ..Default::default()
    };

    let tts_config = OfflineTtsConfig {
        model: sherpa_onnx::OfflineTtsModelConfig {
            zipvoice,
            num_threads: cfg.num_threads,
            provider: Some(cfg.provider.clone()),
            debug: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let tts = OfflineTts::create(&tts_config).ok_or_else(|| {
        DemoError::Config(format!(
            "failed to create ZipVoice TTS (check tts.sherpa.zipvoice paths): encoder={}",
            zip_cfg.encoder
        ))
    })?;

    let wave = Wave::read(&zip_cfg.reference_audio).ok_or_else(|| {
        DemoError::Config(format!(
            "failed to read ZipVoice reference_audio: {}",
            zip_cfg.reference_audio
        ))
    })?;
    let reference = ZipvoiceReference {
        samples: wave.samples().to_vec(),
        sample_rate: wave.sample_rate(),
        text: zip_cfg.reference_text.clone(),
    };

    info!(
        engine = "zipvoice",
        encoder = %zip_cfg.encoder,
        vocoder = %zip_cfg.vocoder,
        provider = %cfg.provider,
        sample_rate = tts.sample_rate(),
        reference_audio = %zip_cfg.reference_audio,
        reference_samples = reference.samples.len(),
        num_steps = zip_cfg.num_steps,
        "sherpa ZipVoice TTS ready"
    );

    run_driver_loop(cmd_rx, audio_tx, &tts, move |tts, text, audio_tx| {
        synthesize_zipvoice_turn(tts, &zip_cfg, &reference, text, audio_tx)
    })
}

fn synthesize_kokoro_turn(
    tts: &OfflineTts,
    cfg: &SherpaKokoroTtsConfig,
    text: &str,
    audio_tx: &mpsc::Sender<TtsAudio>,
) -> Result<()> {
    let gen_config = GenerationConfig {
        sid: cfg.sid,
        speed: cfg.speed,
        ..Default::default()
    };

    let audio = tts
        .generate_with_config(text, &gen_config, Option::<fn(&[f32], f32) -> bool>::None)
        .ok_or_else(|| DemoError::Tts("kokoro generate failed".into()))?;

    emit_pcm(audio.samples(), audio_tx)
}

fn synthesize_zipvoice_turn(
    tts: &OfflineTts,
    cfg: &SherpaZipvoiceTtsConfig,
    reference: &ZipvoiceReference,
    text: &str,
    audio_tx: &mpsc::Sender<TtsAudio>,
) -> Result<()> {
    let gen_config = GenerationConfig {
        speed: cfg.speed,
        reference_audio: Some(reference.samples.clone()),
        reference_sample_rate: reference.sample_rate,
        reference_text: Some(reference.text.clone()),
        num_steps: cfg.num_steps,
        ..Default::default()
    };

    let audio = tts
        .generate_with_config(text, &gen_config, Option::<fn(&[f32], f32) -> bool>::None)
        .ok_or_else(|| DemoError::Tts("zipvoice generate failed".into()))?;

    emit_pcm(audio.samples(), audio_tx)
}

fn emit_pcm(samples: &[f32], audio_tx: &mpsc::Sender<TtsAudio>) -> Result<()> {
    let pcm = f32_to_i16_pcm_bytes(samples);
    if !pcm.is_empty() {
        audio_tx
            .blocking_send(TtsAudio { pcm })
            .map_err(|e| DemoError::Tts(e.to_string()))?;
    }
    Ok(())
}

fn f32_to_i16_pcm_bytes(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|&s| {
            let clamped = s.clamp(-1.0, 1.0);
            let i = (clamped * i16::MAX as f32) as i16;
            i.to_le_bytes()
        })
        .collect()
}
