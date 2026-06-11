use std::time::{Duration, Instant};

use super::{InputMode, PickerModal, StreamMarkdownCache};

/// Snapshot of composer state used to skip redundant clears/cursor updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InputPaintSnapshot {
    pub input: String,
    pub cursor_position: usize,
    pub mode: InputMode,
    pub history_search_active: bool,
}

/// Composer fields valid while the user can edit input (idle or modal-over-idle).
#[derive(Debug, Clone)]
pub struct ComposerState {
    pub mode: InputMode,
    pub input: String,
    pub cursor_position: usize,
    pub completions: Vec<String>,
    pub completion_index: Option<usize>,
    pub history_search_active: bool,
    pub history_search_query: String,
    pub message_browse_index: Option<usize>,
    pub selection_anchor: Option<usize>,
    pub last_input_paint: Option<InputPaintSnapshot>,
}

impl Default for ComposerState {
    fn default() -> Self {
        Self {
            mode: InputMode::Insert,
            input: String::new(),
            cursor_position: 0,
            completions: Vec::new(),
            completion_index: None,
            history_search_active: false,
            history_search_query: String::new(),
            message_browse_index: None,
            selection_anchor: None,
            last_input_paint: None,
        }
    }
}

impl ComposerState {
    pub fn input_paint_snapshot(&self) -> InputPaintSnapshot {
        InputPaintSnapshot {
            input: self.input.clone(),
            cursor_position: self.cursor_position,
            mode: self.mode,
            history_search_active: self.history_search_active,
        }
    }

    pub fn reset_input_paint_cache(&mut self) {
        self.last_input_paint = None;
    }
}

/// Processing-only fields while an agent run is in flight.
#[derive(Debug, Clone)]
pub struct ProcessingState {
    pub composer: ComposerState,
    pub started_at: Option<Instant>,
    pub awaiting_run_complete: bool,
    pub stream_buffer: String,
    pub stream_muted: bool,
    pub stream_needs_break: bool,
    pub stream_md_cache: StreamMarkdownCache,
    pub stream_chunk_count: usize,
    pub stream_char_count: usize,
    pub saw_first_token: bool,
    pub processing_phase: String,
    pub processing_phase_label: String,
    pub processing_phase_progress: u8,
    pub processing_degraded: bool,
    pub degraded_notes: Vec<String>,
    pub last_progress_pulse_at: Option<Instant>,
    pub active_tools: Vec<String>,
    pub live_thinking: String,
}

impl ProcessingState {
    fn new(composer: ComposerState) -> Self {
        Self {
            composer,
            started_at: None,
            awaiting_run_complete: false,
            stream_buffer: String::new(),
            stream_muted: false,
            stream_needs_break: false,
            stream_md_cache: StreamMarkdownCache::default(),
            stream_chunk_count: 0,
            stream_char_count: 0,
            saw_first_token: false,
            processing_phase: String::new(),
            processing_phase_label: String::new(),
            processing_phase_progress: 0,
            processing_degraded: false,
            degraded_notes: Vec::new(),
            last_progress_pulse_at: None,
            active_tools: Vec::new(),
            live_thinking: String::new(),
        }
    }
}

/// Algebraic UI phase: illegal combinations are unrepresentable.
#[derive(Debug, Clone)]
pub enum UiPhase {
    Idle(ComposerState),
    Processing(ProcessingState),
    Modal {
        modal: PickerModal,
        underlying: Box<UiPhase>,
    },
}

impl Default for UiPhase {
    fn default() -> Self {
        Self::idle_default()
    }
}

impl UiPhase {
    pub fn idle_default() -> Self {
        Self::Idle(ComposerState::default())
    }

    pub fn is_processing(&self) -> bool {
        matches!(self, Self::Processing(_))
    }

    pub fn composer(&self) -> &ComposerState {
        match self {
            Self::Idle(composer) => composer,
            Self::Processing(processing) => &processing.composer,
            Self::Modal { underlying, .. } => underlying.composer(),
        }
    }

    pub fn composer_mut(&mut self) -> &mut ComposerState {
        match self {
            Self::Idle(composer) => composer,
            Self::Processing(processing) => &mut processing.composer,
            Self::Modal { underlying, .. } => underlying.composer_mut(),
        }
    }

    pub fn processing(&self) -> Option<&ProcessingState> {
        match self {
            Self::Processing(processing) => Some(processing),
            _ => None,
        }
    }

    pub fn processing_mut(&mut self) -> Option<&mut ProcessingState> {
        match self {
            Self::Processing(processing) => Some(processing),
            _ => None,
        }
    }

    pub fn modal_active(&self) -> bool {
        matches!(self, Self::Modal { .. })
    }

    pub fn modal(&self) -> Option<&PickerModal> {
        match self {
            Self::Modal { modal, .. } => Some(modal),
            _ => None,
        }
    }

    pub fn modal_mut(&mut self) -> Option<&mut PickerModal> {
        match self {
            Self::Modal { modal, .. } => Some(modal),
            _ => None,
        }
    }

    pub fn open_modal(&mut self, modal: PickerModal) {
        if self.is_processing() {
            return;
        }
        let mut underlying = std::mem::replace(self, Self::Idle(ComposerState::default()));
        underlying.composer_mut().mode = InputMode::Insert;
        *self = Self::Modal {
            modal,
            underlying: Box::new(underlying),
        };
    }

    pub fn close_modal(&mut self) {
        let Self::Modal { underlying, .. } =
            std::mem::replace(self, Self::Idle(ComposerState::default()))
        else {
            return;
        };
        *self = *underlying;
    }

    pub fn begin_processing(&mut self, _model: &str) -> bool {
        if self.modal_active() {
            return false;
        }
        let composer = match std::mem::replace(self, Self::Idle(ComposerState::default())) {
            Self::Idle(composer) => composer,
            Self::Processing(_) => return false,
            Self::Modal { .. } => return false,
        };
        *self = Self::Processing(ProcessingState::new(composer));
        true
    }

    pub fn finish_processing(&mut self) -> bool {
        let Self::Processing(processing) =
            std::mem::replace(self, Self::Idle(ComposerState::default()))
        else {
            return false;
        };
        *self = Self::Idle(processing.composer);
        true
    }

    pub fn processing_elapsed(&self) -> Duration {
        self.processing()
            .and_then(|processing| processing.started_at)
            .map(|started| started.elapsed())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::{PickerItem, PickerKind};

    fn sample_modal() -> PickerModal {
        PickerModal::new(
            PickerKind::Personality,
            "personality",
            vec![PickerItem {
                label: "default".to_string(),
                detail: String::new(),
                value: "default".to_string(),
            }],
        )
    }

    #[test]
    fn idle_default_is_idle_with_insert_mode() {
        let phase = UiPhase::idle_default();
        assert!(!phase.is_processing());
        assert!(!phase.modal_active());
        assert_eq!(phase.composer().mode, InputMode::Insert);
    }

    #[test]
    fn idle_to_processing_to_idle() {
        let mut phase = UiPhase::idle_default();
        phase.composer_mut().input = "hello".to_string();
        assert!(phase.begin_processing("nous:test"));
        assert!(phase.is_processing());
        assert_eq!(phase.composer().input, "hello");

        assert!(phase.finish_processing());
        assert!(!phase.is_processing());
        assert_eq!(phase.composer().input, "hello");
    }

    #[test]
    fn idle_to_modal_to_idle() {
        let mut phase = UiPhase::idle_default();
        phase.open_modal(sample_modal());
        assert!(phase.modal_active());
        assert!(!phase.is_processing());

        phase.close_modal();
        assert!(!phase.modal_active());
        assert!(!phase.is_processing());
    }

    #[test]
    fn cannot_open_modal_while_processing() {
        let mut phase = UiPhase::idle_default();
        assert!(phase.begin_processing("nous:test"));
        phase.open_modal(sample_modal());
        assert!(!phase.modal_active());
        assert!(phase.is_processing());
    }

    #[test]
    fn begin_processing_rejects_modal_phase() {
        let mut phase = UiPhase::idle_default();
        phase.open_modal(sample_modal());
        assert!(!phase.begin_processing("nous:test"));
        assert!(phase.modal_active());
    }
}
