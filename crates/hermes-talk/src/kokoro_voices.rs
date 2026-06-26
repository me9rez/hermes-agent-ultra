//! Kokoro multi-lang v1.0 speaker names (53 voices).
//!
//! Mapping from https://k2-fsa.github.io/sherpa/onnx/tts/pretrained_models/kokoro.html
//! (model bundled by `scripts/talk/download_models.*`).

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::error::{DemoError, Result};

static KOKORO_MULTI_LANG_V1_0: LazyLock<HashMap<&'static str, i32>> = LazyLock::new(|| {
    HashMap::from([
        ("af_alloy", 0),
        ("af_aoede", 1),
        ("af_bella", 2),
        ("af_heart", 3),
        ("af_jessica", 4),
        ("af_kore", 5),
        ("af_nicole", 6),
        ("af_nova", 7),
        ("af_river", 8),
        ("af_sarah", 9),
        ("af_sky", 10),
        ("am_adam", 11),
        ("am_echo", 12),
        ("am_eric", 13),
        ("am_fenrir", 14),
        ("am_liam", 15),
        ("am_michael", 16),
        ("am_onyx", 17),
        ("am_puck", 18),
        ("am_santa", 19),
        ("bf_alice", 20),
        ("bf_emma", 21),
        ("bf_isabella", 22),
        ("bf_lily", 23),
        ("bm_daniel", 24),
        ("bm_fable", 25),
        ("bm_george", 26),
        ("bm_lewis", 27),
        ("ef_dora", 28),
        ("em_alex", 29),
        ("ff_siwis", 30),
        ("hf_alpha", 31),
        ("hf_beta", 32),
        ("hm_omega", 33),
        ("hm_psi", 34),
        ("if_sara", 35),
        ("im_nicola", 36),
        ("jf_alpha", 37),
        ("jf_gongitsune", 38),
        ("jf_nezumi", 39),
        ("jf_tebukuro", 40),
        ("jm_kumo", 41),
        ("pf_dora", 42),
        ("pm_alex", 43),
        ("pm_santa", 44),
        ("zf_xiaobei", 45),
        ("zf_xiaoni", 46),
        ("zf_xiaoxiao", 47),
        ("zf_xiaoyi", 48),
        ("zm_yunjian", 49),
        ("zm_yunxi", 50),
        ("zm_yunxia", 51),
        ("zm_yunyang", 52),
    ])
});

/// Resolve Kokoro speaker: `voice` name or numeric string overrides `sid`.
pub fn resolve_kokoro_sid(voice: Option<&str>, sid: i32) -> Result<i32> {
    let Some(raw) = voice.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(sid);
    };
    if let Ok(n) = raw.parse::<i32>() {
        if (0..=52).contains(&n) {
            return Ok(n);
        }
        return Err(DemoError::Config(format!(
            "tts.sherpa.kokoro.voice sid '{n}' out of range (0-52 for kokoro-multi-lang-v1_0)"
        )));
    }
    let key = raw.to_ascii_lowercase();
    KOKORO_MULTI_LANG_V1_0
        .get(key.as_str())
        .copied()
        .ok_or_else(|| {
            DemoError::Config(format!(
            "unknown Kokoro voice '{raw}' (see kokoro-multi-lang-v1_0 speaker list in sherpa docs)"
        ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_voice_name_and_numeric_sid() {
        assert_eq!(resolve_kokoro_sid(Some("zf_xiaoyi"), 0).unwrap(), 48);
        assert_eq!(resolve_kokoro_sid(Some("48"), 0).unwrap(), 48);
        assert_eq!(resolve_kokoro_sid(None, 3).unwrap(), 3);
    }

    #[test]
    fn rejects_unknown_voice() {
        assert!(resolve_kokoro_sid(Some("not_a_voice"), 0).is_err());
    }
}
