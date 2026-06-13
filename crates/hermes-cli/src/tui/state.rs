use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use tui_textarea::{CursorMove, TextArea};

use super::text::{is_submit_shortcut, truncate_chars};
use super::transcript_cache::TranscriptCache;
use super::types::{
    ActivityLaneMode, InputMode, ModalAction, PickerKind, PickerModal, ToolOutputSection,
    ViewDensity,
};
use super::ui_phase::UiPhase;
use crate::app::{SessionRuntime, TranscriptRuntime};
use crate::commands;
/// Mutable state for the TUI rendering loop.
pub struct TuiState {
    pub(crate) phase: UiPhase,
    /// Scroll offset from newest transcript content (0 = newest).
    pub scroll_offset: usize,
    /// Keep transcript pinned to newest content unless user scrolls away.
    pub(crate) auto_follow_transcript: bool,
    /// Status message shown in the status bar.
    pub status_message: String,
    /// Spinner frame counter for tool execution indicator.
    pub spinner_frame: usize,
    /// Tool output sections with fold state (tool_name, output, is_expanded).
    pub tool_outputs: Vec<ToolOutputSection>,
    /// Recent lifecycle/activity rows (newest at end).
    pub recent_activity: Vec<String>,
    /// Last known token usage (prompt, completion, total).
    pub last_usage: Option<(u64, u64, u64)>,
    /// Last total-token value emitted into activity lane.
    pub(crate) last_usage_total_emitted: Option<u64>,
    /// Monotonic sequence for activity-timeline rows.
    timeline_seq: u64,
    /// Sticky prompt hint shown while scrolling history.
    pub sticky_prompt: String,
    /// Number of queued/running background jobs.
    pub background_jobs_running: usize,
    /// Whether the right-side live activity lane is open.
    pub activity_lane_open: bool,
    /// Right-side lane mode.
    pub activity_lane_mode: ActivityLaneMode,
    /// Whether transcript headers show timestamp labels.
    pub show_timestamps: bool,
    /// Transcript density mode.
    pub view_density: ViewDensity,
    /// Cached transcript render to reduce full rebuild churn.
    pub(crate) transcript_cache: TranscriptCache,
    /// Expand state for tool cards by transcript key.
    pub(crate) expanded_tool_cards: HashSet<String>,
    /// Stable timestamp labels keyed by message fingerprint.
    pub(crate) message_time_labels: HashMap<u64, String>,
    /// Animation frame index for companion pet rendering.
    pub(crate) pet_frame: usize,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            phase: UiPhase::idle_default(),
            scroll_offset: 0,
            auto_follow_transcript: true,
            status_message: String::new(),
            spinner_frame: 0,
            tool_outputs: Vec::new(),
            recent_activity: Vec::new(),
            last_usage: None,
            last_usage_total_emitted: None,
            timeline_seq: 0,
            sticky_prompt: String::new(),
            background_jobs_running: 0,
            activity_lane_open: true,
            activity_lane_mode: ActivityLaneMode::Live,
            show_timestamps: false,
            view_density: ViewDensity::Detailed,
            transcript_cache: TranscriptCache::default(),
            expanded_tool_cards: HashSet::new(),
            message_time_labels: HashMap::new(),
            pet_frame: 0,
        }
    }
}

impl TuiState {
    pub(crate) fn reset_input_paint_cache(&mut self) {
        self.phase.composer_mut().reset_input_paint_cache();
    }

    pub(crate) fn scroll_history_up(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(usize::from(lines.max(1)));
        self.auto_follow_transcript = false;
    }

    pub(crate) fn scroll_history_down(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(usize::from(lines.max(1)));
        if self.scroll_offset == 0 {
            self.auto_follow_transcript = true;
        }
    }

    pub(crate) fn jump_to_latest(&mut self) {
        self.scroll_offset = 0;
        self.auto_follow_transcript = true;
    }

    pub(crate) fn jump_to_oldest(&mut self) {
        // Render path clamps this to current transcript max hidden rows.
        self.scroll_offset = usize::MAX;
        self.auto_follow_transcript = false;
    }

    pub(crate) fn push_activity(&mut self, text: impl Into<String>) {
        let trimmed = text.into().trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        self.timeline_seq = self.timeline_seq.saturating_add(1);
        self.recent_activity
            .push(format!("{:02}. {}", self.timeline_seq, trimmed));
        const MAX_EVENTS: usize = 16;
        if self.recent_activity.len() > MAX_EVENTS {
            let remove = self.recent_activity.len() - MAX_EVENTS;
            self.recent_activity.drain(0..remove);
        }
    }

    pub(crate) fn append_live_thinking(&mut self, chunk: &str) {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            return;
        }
        let processing = self.phase.processing_mut().expect("processing");
        if !processing.live_thinking.is_empty() {
            processing.live_thinking.push(' ');
        }
        processing.live_thinking.push_str(chunk);
        const MAX_CHARS: usize = 260;
        if processing.live_thinking.chars().count() > MAX_CHARS {
            let tail: String = processing
                .live_thinking
                .chars()
                .rev()
                .take(MAX_CHARS.saturating_sub(1))
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            processing.live_thinking = format!("…{}", tail);
        }
    }

    pub(crate) fn begin_processing_cycle(&mut self, model: &str) {
        self.last_usage_total_emitted = None;
        self.timeline_seq = 0;
        if !self.phase.begin_processing(model) {
            return;
        }
        let processing = self.phase.processing_mut().expect("processing started");
        processing.awaiting_run_complete = true;
        processing.started_at = Some(Instant::now());
        processing.last_progress_pulse_at = None;
        processing.stream_chunk_count = 0;
        processing.stream_char_count = 0;
        processing.saw_first_token = false;
        processing.processing_degraded = false;
        processing.degraded_notes.clear();
        processing.stream_buffer.clear();
        processing.stream_md_cache.clear();
        processing.stream_muted = false;
        processing.stream_needs_break = false;
        processing.active_tools.clear();
        processing.live_thinking.clear();
        processing.clarify_awaiting = false;
        processing.pending_clarify_prompt = None;
        processing.processing_phase = "preflight".to_string();
        processing.processing_phase_label = "preparing request".to_string();
        processing.processing_phase_progress = 0;
        self.push_activity(format!("⟳ dispatching request to {model}"));
    }

    pub(crate) fn mark_blocking_action(&mut self, label: impl AsRef<str>) {
        let label = truncate_chars(label.as_ref().trim(), 100);
        if label.is_empty() {
            return;
        }
        self.phase
            .processing_mut()
            .expect("processing")
            .processing_phase = "command".to_string();
        self.phase
            .processing_mut()
            .expect("processing")
            .processing_phase_label = label.clone();
        self.phase
            .processing_mut()
            .expect("processing")
            .processing_phase_progress = self
            .phase
            .processing_mut()
            .expect("processing")
            .processing_phase_progress
            .max(5);
        self.push_activity(format!("◈ {}", label));
        self.maybe_emit_progress_pulse();
    }

    pub(crate) fn finish_processing_cycle(&mut self, label: &str) {
        if !self.phase.is_processing() {
            return;
        }
        let summary = {
            let processing = self.phase.processing().expect("processing");
            let resolved_label = if processing.processing_degraded && label.starts_with('✔') {
                "⚠ completed with fallback in"
            } else {
                label
            };
            let elapsed = processing
                .started_at
                .map(|t| t.elapsed().as_secs_f64())
                .unwrap_or_default();
            let activity = format!(
                "{} {:.2}s • {} chunks • {} chars",
                resolved_label,
                elapsed,
                processing.stream_chunk_count,
                processing.stream_char_count
            );
            let fallback =
                if processing.processing_degraded && !processing.degraded_notes.is_empty() {
                    Some(format!(
                        "fallback notes: {}",
                        truncate_chars(&processing.degraded_notes.join(" | "), 220)
                    ))
                } else {
                    None
                };
            (activity, fallback)
        };
        self.phase.finish_processing();
        self.push_activity(summary.0);
        if let Some(note) = summary.1 {
            self.push_activity(note);
        }
        self.jump_to_latest();
    }

    pub(crate) fn maybe_emit_progress_pulse(&mut self) {
        if !self.phase.is_processing() {
            return;
        }
        let now = Instant::now();
        let should_emit = self
            .phase
            .processing()
            .and_then(|processing| processing.last_progress_pulse_at)
            .map(|t| now.duration_since(t) >= Duration::from_millis(1250))
            .unwrap_or(true);
        if !should_emit {
            return;
        }
        let elapsed = self
            .phase
            .processing()
            .and_then(|processing| processing.started_at)
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or_default();
        let processing = self.phase.processing_mut().expect("processing");
        let tool_state = if processing.active_tools.is_empty() {
            "no active tools".to_string()
        } else {
            format!("{} active tool(s)", processing.active_tools.len())
        };
        let phase_state = if processing.processing_phase_label.is_empty() {
            "phase: n/a".to_string()
        } else {
            format!(
                "phase: {} ({}%)",
                truncate_chars(&processing.processing_phase_label, 64),
                processing.processing_phase_progress
            )
        };
        let chunk_count = processing.stream_chunk_count;
        let char_count = processing.stream_char_count;
        processing.last_progress_pulse_at = Some(now);
        self.push_activity(format!(
            "… working {:.1}s • {} chunks • {} chars • {} • {}",
            elapsed, chunk_count, char_count, tool_state, phase_state
        ));
    }

    pub(crate) fn processing_elapsed(&self) -> Duration {
        self.phase.processing_elapsed()
    }

    pub(crate) fn processing_stage_label(&self) -> &'static str {
        if !self.phase.is_processing() {
            return "idle";
        }
        let processing = self.phase.processing().expect("processing");
        if !processing.processing_phase_label.is_empty() {
            return "phase-driven";
        }
        if !processing.saw_first_token {
            if processing.active_tools.is_empty() {
                "awaiting first token"
            } else {
                "running tools (pre-token)"
            }
        } else if processing.active_tools.is_empty() {
            "streaming response"
        } else {
            "running tools + streaming"
        }
    }

    pub(crate) fn update_processing_phase(
        &mut self,
        phase: &str,
        label: &str,
        progress_pct: Option<u8>,
    ) {
        if !self.phase.is_processing() {
            return;
        }
        let phase = phase.trim().to_ascii_lowercase();
        let label = label.trim();
        if phase.is_empty() && label.is_empty() && progress_pct.is_none() {
            return;
        }
        let processing = self.phase.processing_mut().expect("processing");
        if !phase.is_empty() {
            processing.processing_phase = phase;
        }
        if !label.is_empty() {
            processing.processing_phase_label = truncate_chars(label, 120);
        }
        if let Some(progress) = progress_pct {
            processing.processing_phase_progress = progress.min(100);
        }
        let activity_label = if processing.processing_phase_label.is_empty() {
            processing.processing_phase.clone()
        } else {
            processing.processing_phase_label.clone()
        };
        let progress = processing.processing_phase_progress;
        self.push_activity(format!("◈ phase {}% • {}", progress, activity_label));
    }

    pub(crate) fn refresh_sticky_prompt(&mut self, app: &impl TranscriptRuntime) {
        if self.scroll_offset == 0 {
            self.sticky_prompt.clear();
            return;
        }
        let transcript = app.transcript_messages();
        let prompt = transcript
            .iter()
            .rev()
            .find(|m| m.role == hermes_core::MessageRole::User)
            .and_then(|m| m.content.as_deref())
            .unwrap_or("")
            .trim();
        self.sticky_prompt = if prompt.is_empty() {
            String::new()
        } else {
            truncate_chars(prompt, 120)
        };
    }

    pub(crate) fn open_modal(&mut self, modal: PickerModal) {
        self.phase.open_modal(modal);
    }

    pub(crate) fn close_modal(&mut self) {
        self.phase.close_modal();
    }

    pub(crate) fn modal_active(&self) -> bool {
        self.phase.modal_active()
    }

    pub(crate) fn handle_modal_key(&mut self, key: KeyEvent) -> ModalAction {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(modal) = self.phase.modal_mut() else {
            return ModalAction::None;
        };
        let is_interactive_question = matches!(modal.kind, PickerKind::InteractiveQuestion { .. });
        match key.code {
            KeyCode::Esc => ModalAction::Close,
            KeyCode::Enter => ModalAction::Confirm,
            KeyCode::Up => {
                modal.move_selection(-1);
                ModalAction::None
            }
            KeyCode::Down => {
                modal.move_selection(1);
                ModalAction::None
            }
            KeyCode::PageUp => {
                modal.page_move(-1);
                ModalAction::None
            }
            KeyCode::PageDown => {
                modal.page_move(1);
                ModalAction::None
            }
            KeyCode::Home => {
                modal.selected_filtered = 0;
                ModalAction::None
            }
            KeyCode::End => {
                if !modal.filtered_indices.is_empty() {
                    modal.selected_filtered = modal.filtered_indices.len() - 1;
                }
                ModalAction::None
            }
            KeyCode::Char(' ') => {
                modal.toggle_selected();
                ModalAction::None
            }
            KeyCode::Backspace if !is_interactive_question => {
                modal.query.pop();
                modal.refresh_filter();
                ModalAction::None
            }
            KeyCode::Char('u')
                if key.modifiers.contains(KeyModifiers::CONTROL) && !is_interactive_question =>
            {
                modal.query.clear();
                modal.refresh_filter();
                ModalAction::None
            }
            KeyCode::Char('d')
                if key.modifiers.is_empty()
                    && modal.query.trim().is_empty()
                    && matches!(modal.kind, PickerKind::ModelProvider) =>
            {
                ModalAction::DisconnectProvider
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty()
                    && modal.query.trim().is_empty()
                    && ch.is_ascii_digit() =>
            {
                let nth = if ch == '0' {
                    10usize
                } else {
                    ch.to_digit(10).unwrap_or(0) as usize
                };
                if nth >= 1 && nth <= modal.filtered_indices.len() {
                    modal.selected_filtered = nth - 1;
                    ModalAction::Confirm
                } else {
                    ModalAction::None
                }
            }
            KeyCode::Char(ch)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !is_interactive_question =>
            {
                modal.query.push(ch);
                modal.refresh_filter();
                ModalAction::None
            }
            _ => ModalAction::None,
        }
    }

    /// Handle a key event and return whether the app should quit.
    pub fn handle_key(&mut self, key: KeyEvent, app: &mut impl SessionRuntime) -> bool {
        match self.phase.composer_mut().mode {
            InputMode::Normal => self.handle_normal_key(key, app),
            InputMode::Insert => self.handle_insert_key(key, app),
            InputMode::Command => self.handle_command_key(key, app),
        }
    }

    pub(crate) fn handle_normal_key(
        &mut self,
        key: KeyEvent,
        _app: &mut impl SessionRuntime,
    ) -> bool {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::PageUp => {
                self.scroll_history_up(8);
            }
            KeyCode::PageDown => {
                self.scroll_history_down(8);
            }
            KeyCode::Home => {
                self.jump_to_oldest();
            }
            KeyCode::End => {
                self.jump_to_latest();
            }
            KeyCode::Char('i') => {
                self.phase.composer_mut().mode = InputMode::Insert;
            }
            KeyCode::Char(':') => {
                self.phase.composer_mut().mode = InputMode::Command;
                self.phase.composer_mut().input.clear();
                self.phase.composer_mut().cursor_position = 0;
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                return true; // quit
            }
            _ => {}
        }
        false
    }

    pub(crate) fn handle_insert_key(
        &mut self,
        key: KeyEvent,
        app: &mut impl SessionRuntime,
    ) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mods = key.modifiers;
        if mods.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('l') => {
                    self.activity_lane_open = !self.activity_lane_open;
                    self.status_message = if self.activity_lane_open {
                        "Activity lane enabled".to_string()
                    } else {
                        "Activity lane hidden".to_string()
                    };
                    return false;
                }
                KeyCode::Char('o') => {
                    self.activity_lane_mode = match self.activity_lane_mode {
                        ActivityLaneMode::Live => ActivityLaneMode::Cockpit,
                        ActivityLaneMode::Cockpit => ActivityLaneMode::Live,
                    };
                    self.status_message = match self.activity_lane_mode {
                        ActivityLaneMode::Live => "Activity lane mode: live".to_string(),
                        ActivityLaneMode::Cockpit => "Activity lane mode: ops cockpit".to_string(),
                    };
                    return false;
                }
                KeyCode::Char('d') => {
                    self.view_density = match self.view_density {
                        ViewDensity::Compact => ViewDensity::Detailed,
                        ViewDensity::Detailed => ViewDensity::Compact,
                    };
                    self.status_message = match self.view_density {
                        ViewDensity::Compact => "Compact transcript mode".to_string(),
                        ViewDensity::Detailed => "Detailed transcript mode".to_string(),
                    };
                    return false;
                }
                KeyCode::Char('t') => {
                    self.show_timestamps = !self.show_timestamps;
                    self.status_message = if self.show_timestamps {
                        "Timestamps visible".to_string()
                    } else {
                        "Timestamps hidden".to_string()
                    };
                    return false;
                }
                KeyCode::Char('e') => {
                    if self.expanded_tool_cards.insert("__all__".to_string()) {
                        self.status_message = "Expanded tool cards".to_string();
                    } else {
                        self.expanded_tool_cards.remove("__all__");
                        self.status_message = "Collapsed tool cards".to_string();
                    }
                    return false;
                }
                KeyCode::Left => {
                    self.move_cursor_word_left();
                    return false;
                }
                KeyCode::Right => {
                    self.move_cursor_word_right();
                    return false;
                }
                _ => {}
            }
        }
        let completion_nav_active = self.phase.composer_mut().input.starts_with('/')
            && !self.phase.composer_mut().completions.is_empty()
            && !self.phase.composer_mut().history_search_active;

        if completion_nav_active && mods.is_empty() {
            match key.code {
                KeyCode::Up => {
                    self.move_completion_selection(-1);
                    return false;
                }
                KeyCode::Down => {
                    self.move_completion_selection(1);
                    return false;
                }
                KeyCode::PageUp => {
                    self.move_completion_selection(-6);
                    return false;
                }
                KeyCode::PageDown => {
                    self.move_completion_selection(6);
                    return false;
                }
                KeyCode::Home => {
                    self.phase.composer_mut().completion_index = Some(0);
                    return false;
                }
                KeyCode::End => {
                    if !self.phase.composer_mut().completions.is_empty() {
                        self.phase.composer_mut().completion_index =
                            Some(self.phase.composer_mut().completions.len() - 1);
                    }
                    return false;
                }
                _ => {}
            }
        }

        match key.code {
            // Scroll transcript without leaving insert mode.
            KeyCode::PageUp => {
                self.scroll_history_up(8);
                false
            }
            KeyCode::PageDown => {
                self.scroll_history_down(8);
                false
            }
            KeyCode::Home => {
                self.jump_to_oldest();
                false
            }
            KeyCode::End if mods.contains(KeyModifiers::CONTROL) => {
                self.jump_to_latest();
                false
            }
            KeyCode::End => {
                self.jump_to_latest();
                false
            }
            // Fine-grained transcript scroll.
            KeyCode::Up if mods.contains(KeyModifiers::CONTROL) => {
                self.scroll_history_up(1);
                false
            }
            KeyCode::Down if mods.contains(KeyModifiers::CONTROL) => {
                self.scroll_history_down(1);
                false
            }
            // Fallback fine-grained scroll chords when terminals reserve Ctrl+Up/Down.
            KeyCode::Up
                if mods.contains(KeyModifiers::ALT) || mods.contains(KeyModifiers::SHIFT) =>
            {
                self.scroll_history_up(1);
                false
            }
            KeyCode::Down
                if mods.contains(KeyModifiers::ALT) || mods.contains(KeyModifiers::SHIFT) =>
            {
                self.scroll_history_down(1);
                false
            }
            // Force refresh + pin to newest transcript content.
            KeyCode::Char('g') if mods.contains(KeyModifiers::CONTROL) => {
                self.jump_to_latest();
                self.transcript_cache = TranscriptCache::default();
                self.status_message = "Jumped to latest transcript (forced refresh)".to_string();
                false
            }
            // Explicit multiline shortcuts.
            KeyCode::Enter if mods.contains(KeyModifiers::SHIFT) => {
                self.insert_newline_at_cursor();
                self.phase.composer_mut().selection_anchor = None;
                self.refresh_completions();
                false
            }
            KeyCode::Char('j') if mods.contains(KeyModifiers::CONTROL) => {
                self.insert_newline_at_cursor();
                self.phase.composer_mut().selection_anchor = None;
                self.refresh_completions();
                false
            }
            // Submit shortcuts are handled in the run-loop after key handling.
            _ if is_submit_shortcut(&key, &self.phase.composer_mut().input) => false,
            KeyCode::Tab => {
                // Accept completion
                self.accept_completion();
                self.phase.composer_mut().completions.clear();
                self.phase.composer_mut().completion_index = None;
                false
            }
            // Ctrl+R toggles reverse-search across message input history.
            KeyCode::Char('r') if mods.contains(KeyModifiers::CONTROL) => {
                self.phase.composer_mut().history_search_active =
                    !self.phase.composer_mut().history_search_active;
                if !self.phase.composer_mut().history_search_active {
                    self.phase.composer_mut().history_search_query.clear();
                }
                false
            }
            KeyCode::Char(c) if self.phase.composer_mut().history_search_active => {
                self.phase.composer_mut().history_search_query.push(c);
                if let Some(found) = app
                    .input_history()
                    .iter()
                    .rev()
                    .find(|h| h.contains(&self.phase.composer_mut().history_search_query))
                {
                    self.phase.composer_mut().input = found.clone();
                    self.phase.composer_mut().cursor_position =
                        self.phase.composer_mut().input.len();
                }
                false
            }
            KeyCode::Backspace if self.phase.composer_mut().history_search_active => {
                self.phase.composer_mut().history_search_query.pop();
                false
            }
            // On single-line inputs without completion menus, Up/Down browse previous prompts.
            KeyCode::Up
                if !self.phase.composer_mut().input.contains('\n')
                    && !completion_nav_active
                    && mods.is_empty() =>
            {
                if let Some(prev) = app.history_prev() {
                    self.phase.composer_mut().input = prev.to_string();
                    self.phase.composer_mut().cursor_position =
                        self.phase.composer_mut().input.len();
                }
                self.refresh_completions();
                false
            }
            KeyCode::Down
                if !self.phase.composer_mut().input.contains('\n')
                    && !completion_nav_active
                    && mods.is_empty() =>
            {
                if let Some(next) = app.history_next() {
                    self.phase.composer_mut().input = next.to_string();
                    self.phase.composer_mut().cursor_position =
                        self.phase.composer_mut().input.len();
                }
                self.refresh_completions();
                false
            }
            KeyCode::Esc => {
                if self.phase.composer_mut().history_search_active {
                    self.phase.composer_mut().history_search_active = false;
                    self.phase.composer_mut().history_search_query.clear();
                    return false;
                }
                if !self.phase.composer_mut().input.is_empty() {
                    self.phase.composer_mut().input.clear();
                    self.phase.composer_mut().cursor_position = 0;
                    self.phase.composer_mut().selection_anchor = None;
                }
                self.phase.composer_mut().completions.clear();
                self.phase.composer_mut().completion_index = None;
                if self.scroll_offset > 0 {
                    self.jump_to_latest();
                }
                // Keep insert mode so Esc never appears to "freeze" typing.
                self.phase.composer_mut().mode = InputMode::Insert;
                false
            }
            _ => {
                self.apply_textarea_input(key);
                self.phase.composer_mut().selection_anchor = None;
                self.refresh_completions();
                false
            }
        }
    }

    pub(crate) fn handle_command_key(
        &mut self,
        key: KeyEvent,
        _app: &mut impl SessionRuntime,
    ) -> bool {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Enter => {
                let input = std::mem::take(&mut self.phase.composer_mut().input);
                self.phase.composer_mut().cursor_position = 0;
                self.phase.composer_mut().mode = InputMode::Insert;
                self.phase.composer_mut().completions.clear();
                self.phase.composer_mut().completion_index = None;
                let _ = input; // Processed outside
                false
            }
            KeyCode::Esc => {
                self.phase.composer_mut().mode = InputMode::Insert;
                self.phase.composer_mut().input.clear();
                self.phase.composer_mut().cursor_position = 0;
                self.phase.composer_mut().completions.clear();
                self.phase.composer_mut().completion_index = None;
                false
            }
            KeyCode::Tab => {
                // Cycle through completions
                if !self.phase.composer_mut().completions.is_empty() {
                    let idx = self
                        .phase
                        .composer_mut()
                        .completion_index
                        .map(|i| (i + 1) % self.phase.composer_mut().completions.len())
                        .unwrap_or(0);
                    self.phase.composer_mut().completion_index = Some(idx);
                    self.phase.composer_mut().input =
                        self.phase.composer_mut().completions[idx].clone();
                    self.phase.composer_mut().cursor_position =
                        self.phase.composer_mut().input.len();
                }
                false
            }
            _ => {
                // Delegate to insert handler for typing
                self.handle_insert_key(key, _app)
            }
        }
    }

    /// Update auto-completion suggestions based on current input.
    pub(crate) fn update_completions(&mut self) {
        if self.phase.composer_mut().input.starts_with('/') {
            self.phase.composer_mut().completions =
                commands::autocomplete_contextual(&self.phase.composer_mut().input);
            self.phase.composer_mut().completion_index =
                if self.phase.composer_mut().completions.is_empty() {
                    None
                } else {
                    Some(0)
                };
        } else {
            self.phase.composer_mut().completions.clear();
            self.phase.composer_mut().completion_index = None;
        }
    }

    pub(crate) fn refresh_completions(&mut self) {
        if self.phase.composer_mut().input.starts_with('/') {
            self.update_completions();
        } else {
            self.phase.composer_mut().completions.clear();
            self.phase.composer_mut().completion_index = None;
        }
    }

    pub(crate) fn clamp_char_boundary(input: &str, cursor_byte: usize) -> usize {
        let mut clamped = cursor_byte.min(input.len());
        while clamped > 0 && !input.is_char_boundary(clamped) {
            clamped = clamped.saturating_sub(1);
        }
        clamped
    }

    pub(crate) fn cursor_row_col(input: &str, cursor_byte: usize) -> (usize, usize) {
        let clamped = Self::clamp_char_boundary(input, cursor_byte);
        let before = &input[..clamped];
        let row = before.bytes().filter(|b| *b == b'\n').count();
        let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = input[line_start..clamped].chars().count();
        (row, col)
    }

    pub(crate) fn row_col_to_byte_offset(input: &str, row: usize, col: usize) -> usize {
        let mut current_row = 0usize;
        let mut line_start = 0usize;
        for (idx, ch) in input.char_indices() {
            if current_row == row {
                break;
            }
            if ch == '\n' {
                current_row += 1;
                line_start = idx + ch.len_utf8();
            }
        }
        if current_row < row {
            line_start = input.len();
        }
        let line_end = input[line_start..]
            .find('\n')
            .map(|i| line_start + i)
            .unwrap_or(input.len());
        let mut byte = line_start;
        for (taken, (idx, ch)) in input[line_start..line_end].char_indices().enumerate() {
            if taken == col {
                return line_start + idx;
            }
            byte = line_start + idx + ch.len_utf8();
        }
        byte.min(line_end)
    }

    pub(crate) fn textarea_from_input(&self) -> TextArea<'static> {
        let composer = self.phase.composer();
        let lines: Vec<String> = if composer.input.is_empty() {
            vec![String::new()]
        } else {
            composer
                .input
                .split('\n')
                .map(ToString::to_string)
                .collect()
        };
        let mut textarea = TextArea::from(lines);
        let (row, col) = Self::cursor_row_col(&composer.input, composer.cursor_position);
        let row_u16 = row.min(u16::MAX as usize) as u16;
        let col_u16 = col.min(u16::MAX as usize) as u16;
        textarea.move_cursor(CursorMove::Jump(row_u16, col_u16));
        textarea
    }

    pub(crate) fn sync_from_textarea(&mut self, textarea: &TextArea<'_>) {
        self.phase.composer_mut().input = textarea.lines().join("\n");
        let (row, col) = textarea.cursor();
        self.phase.composer_mut().cursor_position =
            Self::row_col_to_byte_offset(&self.phase.composer_mut().input, row, col);
    }

    pub(crate) fn apply_textarea_input(&mut self, key: KeyEvent) {
        let mut textarea = self.textarea_from_input();
        let _ = textarea.input(key);
        self.sync_from_textarea(&textarea);
    }

    pub(crate) fn insert_newline_at_cursor(&mut self) {
        let composer = self.phase.composer_mut();
        let at = Self::clamp_char_boundary(&composer.input, composer.cursor_position);
        composer.input.insert(at, '\n');
        composer.cursor_position = at.saturating_add(1);
    }

    pub(crate) fn insert_paste_at_cursor(&mut self, pasted: &str) {
        let normalized = pasted.replace("\r\n", "\n").replace('\r', "\n");
        if normalized.is_empty() {
            return;
        }
        let composer = self.phase.composer_mut();
        let at = Self::clamp_char_boundary(&composer.input, composer.cursor_position);
        composer.input.insert_str(at, &normalized);
        composer.cursor_position = at.saturating_add(normalized.len());
        composer.selection_anchor = None;
        self.refresh_completions();
    }

    pub(crate) fn move_cursor_word_left(&mut self) {
        if self.phase.composer_mut().cursor_position == 0
            || self.phase.composer_mut().input.is_empty()
        {
            self.phase.composer_mut().cursor_position = 0;
            return;
        }
        let chars: Vec<(usize, char)> = self.phase.composer_mut().input.char_indices().collect();
        let mut idx = chars
            .iter()
            .position(|(byte, _)| *byte >= self.phase.composer_mut().cursor_position)
            .unwrap_or(chars.len());
        if idx > 0 && chars[idx - 1].1.is_whitespace() {
            while idx > 0 && chars[idx - 1].1.is_whitespace() {
                idx -= 1;
            }
        }
        while idx > 0 && !chars[idx - 1].1.is_whitespace() {
            idx -= 1;
        }
        self.phase.composer_mut().cursor_position = chars.get(idx).map(|(b, _)| *b).unwrap_or(0);
    }

    pub(crate) fn move_cursor_word_right(&mut self) {
        if self.phase.composer_mut().input.is_empty() {
            self.phase.composer_mut().cursor_position = 0;
            return;
        }
        let chars: Vec<(usize, char)> = self.phase.composer_mut().input.char_indices().collect();
        let mut idx = chars
            .iter()
            .position(|(byte, _)| *byte > self.phase.composer_mut().cursor_position)
            .unwrap_or(chars.len());
        while idx < chars.len() && chars[idx].1.is_whitespace() {
            idx += 1;
        }
        while idx < chars.len() && !chars[idx].1.is_whitespace() {
            idx += 1;
        }
        self.phase.composer_mut().cursor_position = if idx >= chars.len() {
            self.phase.composer_mut().input.len()
        } else {
            chars[idx].0
        };
    }

    pub(crate) fn move_completion_selection(&mut self, delta: isize) {
        if self.phase.composer_mut().completions.is_empty() {
            self.phase.composer_mut().completion_index = None;
            return;
        }
        let len = self.phase.composer_mut().completions.len() as isize;
        let current = self.phase.composer_mut().completion_index.unwrap_or(0) as isize;
        let mut next = current + delta;
        while next < 0 {
            next += len;
        }
        next %= len;
        self.phase.composer_mut().completion_index = Some(next as usize);
    }

    pub(crate) fn accept_completion(&mut self) {
        if let Some(idx) = self.phase.composer_mut().completion_index {
            if idx < self.phase.composer_mut().completions.len() {
                self.phase.composer_mut().input =
                    self.phase.composer_mut().completions[idx].clone();
                self.phase.composer_mut().cursor_position = self.phase.composer_mut().input.len();
                return;
            }
        }
        if let Some(first) = self.phase.composer_mut().completions.first() {
            self.phase.composer_mut().input = first.clone();
            self.phase.composer_mut().cursor_position = self.phase.composer_mut().input.len();
        }
    }

    /// When slash-command completions are visible, plain Enter should accept the
    /// highlighted suggestion instead of submitting a partial command.
    ///
    /// Returns `true` when Enter was consumed to fill the input (caller should
    /// skip submit). Returns `false` when the input already matches the selected
    /// completion and Enter should proceed to submit.
    pub(crate) fn try_accept_completion_on_enter(&mut self) -> bool {
        let completion_nav_active = self.phase.composer_mut().input.starts_with('/')
            && !self.phase.composer_mut().completions.is_empty()
            && !self.phase.composer_mut().history_search_active;
        if !completion_nav_active {
            return false;
        }
        let idx = self.phase.composer_mut().completion_index.unwrap_or(0);
        if idx >= self.phase.composer_mut().completions.len() {
            return false;
        }
        let selected = self.phase.composer_mut().completions[idx].clone();
        if self.phase.composer_mut().input.trim() == selected.trim() {
            return false;
        }
        self.phase.composer_mut().input = selected;
        self.phase.composer_mut().cursor_position = self.phase.composer_mut().input.len();
        self.refresh_completions();
        true
    }

    /// Get the spinner character for the current frame.
    pub fn spinner_char(&self) -> char {
        const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        SPINNER[self.spinner_frame % SPINNER.len()]
    }

    /// Advance the spinner frame.
    pub fn tick_spinner(&mut self) {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
    }

    pub fn tick_pet(&mut self) {
        self.pet_frame = self.pet_frame.wrapping_add(1);
    }
}
