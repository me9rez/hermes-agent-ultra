use std::time::Instant;

/// Wake gate orthogonal to conversation SessionState (Listening / Thinking / Speaking).
#[derive(Debug, Clone)]
pub enum WakePhase {
    /// Waiting for sherpa-onnx KWS; ASR paused.
    Dormant,
    /// KWS hit; user must speak within grace window.
    AwakeGrace { deadline: Instant },
    /// Normal dialog allowed.
    Active,
    /// After a turn completes; idle timeout before dormant.
    IdleAfterTurn { deadline: Instant },
}

impl WakePhase {
    pub fn allows_asr(&self) -> bool {
        !matches!(self, WakePhase::Dormant)
    }

    /// Dialog (flush ASR, trigger LLM, barge-in) allowed in every phase except dormant.
    pub fn allows_dialog(&self) -> bool {
        !matches!(self, WakePhase::Dormant)
    }

    pub fn check_timeout(&self, now: Instant) -> bool {
        match self {
            WakePhase::AwakeGrace { deadline } | WakePhase::IdleAfterTurn { deadline } => {
                now >= *deadline
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn allows_dialog_except_dormant() {
        assert!(!WakePhase::Dormant.allows_dialog());
        assert!(WakePhase::Active.allows_dialog());
        assert!(
            WakePhase::AwakeGrace {
                deadline: Instant::now() + Duration::from_secs(5)
            }
            .allows_dialog()
        );
        assert!(
            WakePhase::IdleAfterTurn {
                deadline: Instant::now() + Duration::from_secs(30)
            }
            .allows_dialog()
        );
    }
}
