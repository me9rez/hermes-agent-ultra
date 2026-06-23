mod bailian;
#[cfg(all(feature = "rockchip", target_arch = "aarch64"))]
pub mod rk_tts;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::config::{DashscopeConfig, TtsConfig};
use crate::error::Result;

pub use bailian::BailianTts;
pub use bailian::TtsAudio;

#[cfg(all(feature = "rockchip", target_arch = "aarch64"))]
pub use rk_tts::RockchipTts;

#[async_trait]
pub trait TtsEngine: Send + Sync {
    async fn warmup(&self) -> Result<()>;
    async fn append_text(&self, text: &str) -> Result<()>;
    async fn finish_turn(&self) -> Result<()>;
    async fn interrupt_turn(&self) -> Result<()>;
}

pub enum TtsBackend {
    Bailian,
    #[cfg(all(feature = "rockchip", target_arch = "aarch64"))]
    Rockchip,
}

impl TtsBackend {
    pub fn from_config(tts_cfg: &TtsConfig) -> Self {
        match tts_cfg.backend.as_str() {
            #[cfg(all(feature = "rockchip", target_arch = "aarch64"))]
            "local" | "rockchip" => TtsBackend::Rockchip,
            _ => TtsBackend::Bailian,
        }
    }
}

pub async fn create_tts(
    dashscope: &DashscopeConfig,
    tts_cfg: &TtsConfig,
    backend: TtsBackend,
) -> Result<(Arc<dyn TtsEngine>, mpsc::Receiver<TtsAudio>)> {
    match backend {
        TtsBackend::Bailian => {
            let (client, rx) = BailianTts::connect(dashscope, tts_cfg).await?;
            Ok((Arc::new(client) as Arc<dyn TtsEngine>, rx))
        }
        #[cfg(all(feature = "rockchip", target_arch = "aarch64"))]
        TtsBackend::Rockchip => {
            let rockchip_cfg = tts_cfg
                .local
                .as_ref()
                .or(tts_cfg.rockchip.as_ref())
                .ok_or_else(|| {
                    crate::error::DemoError::Config(
                        "tts.local config required when backend = \"local\"".into(),
                    )
                })?;
            let (client, rx) = RockchipTts::connect(rockchip_cfg).await?;
            Ok((Arc::new(client) as Arc<dyn TtsEngine>, rx))
        }
    }
}
