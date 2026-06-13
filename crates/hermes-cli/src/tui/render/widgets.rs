use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};

use super::super::TuiReadHost;
use super::super::state::TuiState;
use super::super::text::{fit_status_line, transcript_divider, truncate_chars};
use super::super::transcript_cache::{
    TranscriptCache, TranscriptRefreshPlan, expanded_tool_cards_signature, plan_transcript_refresh,
};
use super::super::types::{ActivityLaneMode, InputMode, PickerKind, PickerModal, ViewDensity};
use super::transcript::{
    append_streaming_transcript_tail, append_transcript_message_lines, approximate_visual_rows,
    build_transcript_lines, count_renderable_messages_before, finalize_transcript_cache,
    find_anchor_line_index, project_transcript_window, streaming_transcript_active,
    transcript_fingerprint, transcript_message_fingerprints, transcript_wrap_width,
};
use crate::app::{SessionRuntime, TranscriptRuntime};
pub(crate) fn render_header(
    frame: &mut Frame,
    app: &impl SessionRuntime,
    area: Rect,
    colors: &crate::theme::RatatuiColors,
) {
    let session_short = &app.session_id()[..8.min(app.session_id().len())];
    let chrome = format!(
        "  •  session {}  •  Enter send  •  Shift+Enter/Ctrl+J newline  •  / commands  •  Ctrl+L lane  •  Ctrl+O cockpit  •  Ctrl+G refresh-tail",
        session_short
    );
    let available = area.width.saturating_sub(28) as usize;
    let text = Text::from(vec![Line::from(vec![
        Span::styled(
            " ▓ HERMES ",
            Style::default()
                .fg(colors.status_bar_strong)
                .bg(colors.status_bar_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "x FlowyAIPC",
            Style::default()
                .fg(colors.accent)
                .bg(colors.status_bar_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            truncate_chars(&chrome, available),
            Style::default()
                .fg(colors.status_bar_text)
                .bg(colors.status_bar_bg)
                .add_modifier(Modifier::BOLD),
        ),
    ])]);
    let title = Paragraph::new(text)
        .block(Block::default().style(Style::default().bg(colors.status_bar_bg)));
    frame.render_widget(title, area);
}

pub(crate) fn render_live_details(
    frame: &mut Frame,
    app: &impl TuiReadHost,
    state: &TuiState,
    area: Rect,
    colors: &crate::theme::RatatuiColors,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let lane_title = match state.activity_lane_mode {
        ActivityLaneMode::Live => " Activity Lane ",
        ActivityLaneMode::Cockpit => " Ops Cockpit ",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(lane_title)
        .style(Style::default().bg(colors.background))
        .border_style(Style::default().fg(colors.status_bar_dim));
    let mut rows: Vec<Line<'static>> = Vec::new();

    if matches!(state.activity_lane_mode, ActivityLaneMode::Cockpit) {
        rows.push(Line::from(vec![
            Span::styled(
                " mode: ",
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.background),
            ),
            Span::styled(
                if state.phase.is_processing() {
                    format!(
                        "processing ({:.1}s)",
                        state.processing_elapsed().as_secs_f64()
                    )
                } else {
                    "idle".to_string()
                },
                Style::default()
                    .fg(colors.status_bar_strong)
                    .bg(colors.background)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        rows.push(Line::from(vec![
            Span::styled(
                " model: ",
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.background),
            ),
            Span::styled(
                truncate_chars(app.current_model(), area.width.saturating_sub(10) as usize),
                Style::default()
                    .fg(colors.status_bar_text)
                    .bg(colors.background),
            ),
        ]));
        rows.push(Line::from(vec![
            Span::styled(
                " planner: ",
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.background),
            ),
            Span::styled(
                std::env::var("HERMES_PLAN_CAPABILITY_ROUTER")
                    .unwrap_or_else(|_| "off".to_string()),
                Style::default()
                    .fg(colors.status_bar_text)
                    .bg(colors.background),
            ),
            Span::styled(
                "  compaction: ",
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.background),
            ),
            Span::styled(
                std::env::var("HERMES_CONTEXTLATTICE_COMPACTION_GOVERNANCE")
                    .unwrap_or_else(|_| "advisory".to_string()),
                Style::default()
                    .fg(colors.status_bar_text)
                    .bg(colors.background),
            ),
        ]));
        rows.push(Line::from(vec![
            Span::styled(
                " policy: ",
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.background),
            ),
            Span::styled(
                format!(
                    "preset={} mode={} skills={}",
                    std::env::var("HERMES_TOOL_POLICY_PRESET")
                        .unwrap_or_else(|_| "balanced".to_string()),
                    std::env::var("HERMES_TOOL_POLICY_MODE")
                        .unwrap_or_else(|_| "enforce".to_string()),
                    std::env::var("HERMES_SKILLS_EXECUTION_TIER")
                        .unwrap_or_else(|_| "balanced".to_string()),
                ),
                Style::default()
                    .fg(colors.status_bar_text)
                    .bg(colors.background),
            ),
        ]));
        if let Some((prompt, completion, total)) = state.last_usage {
            rows.push(Line::from(vec![
                Span::styled(
                    " usage: ",
                    Style::default()
                        .fg(colors.status_bar_dim)
                        .bg(colors.background),
                ),
                Span::styled(
                    format!("in={} out={} total={}", prompt, completion, total),
                    Style::default()
                        .fg(colors.status_bar_text)
                        .bg(colors.background),
                ),
            ]));
        }
        rows.push(Line::from(vec![Span::styled(
            " Ctrl+O live lane",
            Style::default()
                .fg(colors.status_bar_dim)
                .bg(colors.background)
                .add_modifier(Modifier::ITALIC),
        )]));

        let paragraph = Paragraph::new(Text::from(rows))
            .block(block)
            .wrap(Wrap { trim: true });
        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
        return;
    }

    if state.phase.is_processing() {
        let processing = state.phase.processing().expect("processing");
        let elapsed = state.processing_elapsed().as_secs_f64();
        rows.push(Line::from(vec![
            Span::styled(
                " ⟳ processing ",
                Style::default()
                    .fg(colors.status_bar_strong)
                    .bg(colors.background)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{elapsed:.1}s"),
                Style::default()
                    .fg(colors.accent)
                    .bg(colors.background)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" • {}", state.processing_stage_label()),
                Style::default()
                    .fg(colors.status_bar_text)
                    .bg(colors.background),
            ),
        ]));
        rows.push(Line::from(vec![Span::styled(
            format!(
                " [{}] chunks:{} chars:{} phase:{}% {}",
                animated_processing_bar(state.spinner_frame, 18),
                processing.stream_chunk_count,
                processing.stream_char_count,
                processing.processing_phase_progress,
                truncate_chars(
                    if processing.processing_phase_label.is_empty() {
                        processing.processing_phase.as_str()
                    } else {
                        processing.processing_phase_label.as_str()
                    },
                    38
                )
            ),
            Style::default().fg(colors.accent).bg(colors.background),
        )]));
        if processing.processing_degraded {
            rows.push(Line::from(vec![Span::styled(
                format!(
                    " ⚠ fallback active: {}",
                    truncate_chars(&processing.degraded_notes.join(" | "), 120)
                ),
                Style::default()
                    .fg(colors.status_bar_warn)
                    .bg(colors.background),
            )]));
        }
    }

    if let Some(processing) = state.phase.processing()
        && !processing.active_tools.is_empty()
    {
        rows.push(Line::from(vec![
            Span::styled(
                " tools: ",
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.background),
            ),
            Span::styled(
                truncate_chars(&processing.active_tools.join(", "), 120),
                Style::default()
                    .fg(colors.status_bar_strong)
                    .bg(colors.background)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    } else if state.phase.is_processing() {
        rows.push(Line::from(vec![Span::styled(
            " tools: awaiting tool events…",
            Style::default()
                .fg(colors.status_bar_dim)
                .bg(colors.background),
        )]));
    }

    if let Some(processing) = state.phase.processing()
        && !processing.live_thinking.is_empty()
    {
        rows.push(Line::from(vec![
            Span::styled(
                " thinking: ",
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.background),
            ),
            Span::styled(
                truncate_chars(&processing.live_thinking, 140),
                Style::default().fg(colors.accent).bg(colors.background),
            ),
        ]));
    }

    if let Some((prompt, completion, total)) = state.last_usage {
        rows.push(Line::from(vec![
            Span::styled(
                " usage: ",
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.background),
            ),
            Span::styled(
                format!("in={} out={} total={}", prompt, completion, total),
                Style::default()
                    .fg(colors.status_bar_text)
                    .bg(colors.background),
            ),
        ]));
    }

    let recent_cap = area.height.saturating_sub(rows.len() as u16 + 3) as usize;
    for event in state
        .recent_activity
        .iter()
        .rev()
        .take(recent_cap.max(2))
        .rev()
    {
        rows.push(Line::from(vec![
            Span::styled(
                " • ",
                Style::default().fg(colors.accent).bg(colors.background),
            ),
            Span::styled(
                truncate_chars(event, area.width.saturating_sub(8) as usize),
                Style::default()
                    .fg(colors.status_bar_text)
                    .bg(colors.background),
            ),
        ]));
    }

    if rows.is_empty() {
        rows.push(Line::from(vec![Span::styled(
            " waiting for activity…",
            Style::default()
                .fg(colors.status_bar_dim)
                .bg(colors.background)
                .add_modifier(Modifier::ITALIC),
        )]));
    }
    rows.push(Line::from(vec![Span::styled(
        " Ctrl+L toggle lane • Ctrl+O cockpit",
        Style::default()
            .fg(colors.status_bar_dim)
            .bg(colors.background)
            .add_modifier(Modifier::ITALIC),
    )]));

    let paragraph = Paragraph::new(Text::from(rows))
        .block(block)
        .wrap(Wrap { trim: true });
    frame.render_widget(Clear, area);
    frame.render_widget(paragraph, area);
}

/// Render the message history area.

pub(crate) fn render_messages(
    frame: &mut Frame,
    app: &impl TranscriptRuntime,
    state: &mut TuiState,
    area: Rect,
    styles: &crate::theme::ResolvedStyles,
    colors: &crate::theme::RatatuiColors,
) {
    let base_block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(colors.background))
        .border_style(Style::default().fg(colors.status_bar_dim));
    let inner = base_block.inner(area);
    if inner.width == 0 || inner.height == 0 {
        frame.render_widget(Clear, area);
        frame.render_widget(base_block.title(" Conversation "), area);
        return;
    }
    let reserved_scrollbar_col = if inner.width > 1 { 1 } else { 0 };
    let transcript_width = inner.width.saturating_sub(reserved_scrollbar_col).max(1);
    let wrap_width = transcript_wrap_width(transcript_width);
    let content_area = Rect {
        x: inner.x,
        y: inner.y,
        width: wrap_width.min(transcript_width),
        height: inner.height,
    };
    let transcript = app.transcript_messages();
    let viewport_rows = usize::from(inner.height.max(1));
    let fingerprint = transcript_fingerprint(&transcript, state, wrap_width);
    let message_fingerprints = transcript_message_fingerprints(&transcript);
    let streaming_active = streaming_transcript_active(state);
    let plan = plan_transcript_refresh(
        &state.transcript_cache,
        fingerprint,
        &message_fingerprints,
        wrap_width,
        state,
        streaming_active,
    );

    if !matches!(plan, TranscriptRefreshPlan::CacheHit) {
        match plan {
            TranscriptRefreshPlan::CacheHit => {}
            TranscriptRefreshPlan::AppendFrom { message_index } => {
                let mut lines = std::mem::take(&mut state.transcript_cache.lines);
                let mut message_line_ends =
                    std::mem::take(&mut state.transcript_cache.message_line_ends);
                let mut rendered_messages = state.transcript_cache.rendered_messages;
                let divider = transcript_divider(wrap_width);
                for (msg_idx, msg) in transcript.iter().enumerate().skip(message_index) {
                    append_transcript_message_lines(
                        &mut lines,
                        msg,
                        msg_idx,
                        &mut rendered_messages,
                        state,
                        styles,
                        colors,
                        &divider,
                    );
                    message_line_ends.push(lines.len());
                }
                let messages_only_len = lines.len();
                state.transcript_cache = finalize_transcript_cache(
                    fingerprint,
                    wrap_width,
                    lines,
                    message_line_ends,
                    messages_only_len,
                    rendered_messages,
                    message_fingerprints,
                    transcript.len(),
                    state,
                    false,
                );
            }
            TranscriptRefreshPlan::RebuildFrom { message_index } => {
                let truncate_at = state.transcript_cache.line_start_for_message(message_index);
                let mut lines = state.transcript_cache.lines[..truncate_at].to_vec();
                let mut message_line_ends = state.transcript_cache.message_line_ends
                    [..message_index.min(state.transcript_cache.message_line_ends.len())]
                    .to_vec();
                let mut rendered_messages =
                    count_renderable_messages_before(&transcript, message_index);
                let divider = transcript_divider(wrap_width);
                for (msg_idx, msg) in transcript.iter().enumerate().skip(message_index) {
                    append_transcript_message_lines(
                        &mut lines,
                        msg,
                        msg_idx,
                        &mut rendered_messages,
                        state,
                        styles,
                        colors,
                        &divider,
                    );
                    message_line_ends.push(lines.len());
                }
                let messages_only_len = lines.len();
                append_streaming_transcript_tail(
                    &mut lines, state, styles, colors, wrap_width, &divider,
                );
                state.transcript_cache = finalize_transcript_cache(
                    fingerprint,
                    wrap_width,
                    lines,
                    message_line_ends,
                    messages_only_len,
                    rendered_messages,
                    message_fingerprints,
                    transcript.len(),
                    state,
                    streaming_active,
                );
            }
            TranscriptRefreshPlan::StreamTailOnly => {
                let mut lines = state.transcript_cache.lines
                    [..state.transcript_cache.messages_only_len]
                    .to_vec();
                let divider = transcript_divider(wrap_width);
                append_streaming_transcript_tail(
                    &mut lines, state, styles, colors, wrap_width, &divider,
                );
                state.transcript_cache = finalize_transcript_cache(
                    fingerprint,
                    wrap_width,
                    lines,
                    state.transcript_cache.message_line_ends.clone(),
                    state.transcript_cache.messages_only_len,
                    state.transcript_cache.rendered_messages,
                    message_fingerprints,
                    transcript.len(),
                    state,
                    streaming_active,
                );
            }
            TranscriptRefreshPlan::FullRebuild => {
                let prev_width = state.transcript_cache.width;
                let prev_len = state.transcript_cache.lines.len();
                let prev_anchor_line = if prev_width != 0
                    && prev_width != wrap_width
                    && state.scroll_offset > 0
                    && prev_len > 0
                {
                    let old_view_rows = viewport_rows.min(prev_len.max(1));
                    let max_hidden = prev_len.saturating_sub(old_view_rows);
                    let hidden = state.scroll_offset.min(max_hidden);
                    let old_end = prev_len.saturating_sub(hidden);
                    let old_start = old_end.saturating_sub(old_view_rows);
                    state
                        .transcript_cache
                        .lines
                        .get(old_start)
                        .map(Line::to_string)
                        .map(|text| (text, old_start, prev_len))
                } else {
                    None
                };

                let built = build_transcript_lines(&transcript, state, styles, colors, wrap_width);
                let new_visual_rows = approximate_visual_rows(&built.lines, wrap_width);
                if let Some((anchor_text, old_start, old_len)) = prev_anchor_line {
                    let new_len = built.lines.len();
                    let expected_idx = if old_len > 0 {
                        old_start.saturating_mul(new_len) / old_len
                    } else {
                        0
                    };
                    if let Some(new_idx) =
                        find_anchor_line_index(&built.lines, &anchor_text, expected_idx)
                    {
                        let new_len = built.lines.len();
                        let visible = viewport_rows.min(new_len.max(1));
                        let new_hidden = new_len.saturating_sub((new_idx + visible).min(new_len));
                        state.scroll_offset = new_hidden;
                    }
                }
                state.transcript_cache = TranscriptCache {
                    fingerprint,
                    width: wrap_width,
                    visual_rows: new_visual_rows,
                    total_messages: transcript.len(),
                    rendered_messages: built.rendered_messages,
                    message_fingerprints,
                    message_line_ends: built.message_line_ends,
                    messages_only_len: built.messages_only_len,
                    show_timestamps: state.show_timestamps,
                    view_density: state.view_density,
                    had_streaming: streaming_active,
                    expanded_tool_cards_sig: expanded_tool_cards_signature(
                        &state.expanded_tool_cards,
                    ),
                    lines: built.lines,
                };
            }
        }
    }
    let lines = &state.transcript_cache.lines;
    if state.auto_follow_transcript {
        state.scroll_offset = 0;
    }
    let total_visual_rows = state.transcript_cache.visual_rows.max(1);
    let max_hidden_from_bottom = total_visual_rows.saturating_sub(viewport_rows);
    let hidden_from_bottom = state.scroll_offset.min(max_hidden_from_bottom);
    if state.scroll_offset != hidden_from_bottom {
        state.scroll_offset = hidden_from_bottom;
    }
    let top_visual_row = total_visual_rows.saturating_sub(viewport_rows + hidden_from_bottom);

    let (render_lines, scroll_rows_in_window) =
        project_transcript_window(lines, wrap_width, top_visual_row, viewport_rows);
    let text = Text::from(render_lines);
    let top_visual_row_u16 = scroll_rows_in_window.min(u16::MAX as usize) as u16;

    let title = if hidden_from_bottom > 0 {
        format!(" Conversation (+{}) ", hidden_from_bottom)
    } else {
        " Conversation ".to_string()
    };
    let block = base_block.title(title);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((top_visual_row_u16, 0)),
        content_area,
    );

    if total_visual_rows > viewport_rows {
        let scrollbar_area = Rect {
            x: content_area.x + content_area.width,
            y: content_area.y,
            width: 1,
            height: content_area.height,
        };
        let mut scrollbar_state = ScrollbarState::new(total_visual_rows)
            .position(top_visual_row)
            .viewport_content_length(viewport_rows);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .track_symbol(Some("│"))
            .thumb_symbol("█")
            .begin_symbol(None)
            .end_symbol(None)
            .track_style(
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.background),
            )
            .thumb_style(
                Style::default()
                    .fg(colors.status_bar_strong)
                    .bg(colors.background),
            );
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}

/// Render slash-command completions as a popup over the conversation panel.
pub(crate) fn render_completions_popup(
    frame: &mut Frame,
    completions: &[String],
    selected: Option<usize>,
    messages_area: Rect,
    input_area: Rect,
    colors: &crate::theme::RatatuiColors,
) {
    if completions.is_empty() {
        return;
    }
    let max_inner_rows = 10usize;
    let visible_rows = completions.len().min(max_inner_rows).max(1);
    let mut start = 0usize;
    if let Some(sel) = selected {
        if sel >= visible_rows {
            start = sel + 1 - visible_rows;
        }
    }
    let end = (start + visible_rows).min(completions.len());
    let max_item_width = completions[start..end]
        .iter()
        .map(|c| {
            let desc = crate::commands::help_for(c).unwrap_or("");
            if desc.is_empty() {
                c.chars().count()
            } else {
                format!("{c} — {desc}").chars().count()
            }
        })
        .max()
        .unwrap_or(0);
    let popup_max_width = messages_area.width.saturating_sub(2).max(1);
    let popup_min_width = 36u16.min(popup_max_width);
    let popup_width = (max_item_width as u16 + 8).clamp(popup_min_width, popup_max_width);
    let popup_height = (end.saturating_sub(start) as u16 + 2).max(3);
    if popup_width == 0 || popup_height == 0 {
        return;
    }
    let right_bound = messages_area.x + messages_area.width.saturating_sub(1);
    let mut x = input_area.x.saturating_add(1);
    if x + popup_width > right_bound {
        x = right_bound.saturating_sub(popup_width);
    }
    let min_y = messages_area.y.saturating_add(1);
    let y = input_area.y.saturating_sub(popup_height).max(min_y);
    let popup = Rect {
        x,
        y,
        width: popup_width,
        height: popup_height,
    };

    let inner_width = popup_width.saturating_sub(4) as usize;
    let items: Vec<Line<'static>> = completions
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|(i, cmd)| {
            let active = selected.or(if completions.is_empty() {
                None
            } else {
                Some(0)
            });
            let style = if active == Some(i) {
                Style::default()
                    .fg(Color::Black)
                    .bg(colors.status_bar_strong)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(colors.status_bar_text)
                    .bg(colors.status_bar_bg)
            };
            let desc = crate::commands::help_for(cmd).unwrap_or("");
            let text = if desc.is_empty() {
                cmd.to_string()
            } else {
                format!("{:<18} {}", cmd, desc)
            };
            Line::from(Span::styled(truncate_chars(&text, inner_width), style))
        })
        .collect();

    let title = if completions.len() > visible_rows {
        format!(
            " Slash Commands ({}/{}) ↑↓ scroll Enter/Tab accept ",
            end,
            completions.len()
        )
    } else {
        " Slash Commands ".to_string()
    };

    let paragraph = Paragraph::new(Text::from(items))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().bg(colors.status_bar_bg))
                .border_style(Style::default().fg(colors.status_bar_strong))
                .title(title),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(Clear, popup);
    frame.render_widget(paragraph, popup);

    if completions.len() > visible_rows {
        let inner = Rect {
            x: popup.x.saturating_add(1),
            y: popup.y.saturating_add(1),
            width: popup.width.saturating_sub(2),
            height: popup.height.saturating_sub(2),
        };
        let mut scrollbar_state = ScrollbarState::new(completions.len())
            .position(start)
            .viewport_content_length(visible_rows);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .track_symbol(Some("│"))
            .thumb_symbol("█")
            .begin_symbol(None)
            .end_symbol(None)
            .track_style(
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.status_bar_bg),
            )
            .thumb_style(
                Style::default()
                    .fg(colors.status_bar_strong)
                    .bg(colors.status_bar_bg),
            );
        frame.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
    }
}

pub(crate) fn render_picker_modal(
    frame: &mut Frame,
    modal: &PickerModal,
    colors: &crate::theme::RatatuiColors,
) {
    let area = frame.area();
    if area.width < 20 || area.height < 8 {
        return;
    }
    let width = (area.width.saturating_sub(6)).min(110).max(48);
    let height = (area.height.saturating_sub(4)).min(22).max(10);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", modal.title))
        .style(Style::default().bg(colors.status_bar_bg))
        .border_style(Style::default().fg(colors.status_bar_strong));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let footer_height = 2u16;
    let query_height = 1u16;
    let rows_height = inner.height.saturating_sub(footer_height + query_height);
    let rows_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: rows_height,
    };
    let query_area = Rect {
        x: inner.x,
        y: inner.y + rows_height,
        width: inner.width,
        height: query_height,
    };
    let footer_area = Rect {
        x: inner.x,
        y: inner.y + rows_height + query_height,
        width: inner.width,
        height: footer_height,
    };

    let (start, end) = modal.visible_window();
    let items: Vec<Line<'static>> = if modal.filtered_indices.is_empty() {
        vec![Line::from(vec![Span::styled(
            "No matches for current search query.",
            Style::default()
                .fg(colors.status_bar_dim)
                .bg(colors.status_bar_bg)
                .add_modifier(Modifier::ITALIC),
        )])]
    } else {
        modal
            .filtered_indices
            .iter()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(|(filtered_idx, item_idx)| {
                let item = &modal.items[*item_idx];
                let selected = filtered_idx == modal.selected_filtered;
                let selected_marker = if selected { "▶" } else { " " };
                let absolute_number = filtered_idx + 1;
                let multi_marker = if modal.allow_multi {
                    if modal.selected_values.contains(&item.value) {
                        "■ "
                    } else {
                        "□ "
                    }
                } else {
                    ""
                };
                let row_style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(colors.status_bar_strong)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(colors.status_bar_text)
                        .bg(colors.status_bar_bg)
                };
                let detail_style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(colors.status_bar_strong)
                } else {
                    Style::default()
                        .fg(colors.status_bar_dim)
                        .bg(colors.status_bar_bg)
                };
                let text = format!(
                    "{selected_marker} {:>3}. {multi_marker}{}",
                    absolute_number, item.label
                );
                let available = rows_area.width.saturating_sub(2) as usize;
                let primary = truncate_chars(&text, available);
                if item.detail.is_empty() {
                    Line::from(vec![Span::styled(primary, row_style)])
                } else {
                    let detail = truncate_chars(
                        &format!("  {}", item.detail),
                        rows_area.width.saturating_sub(2) as usize,
                    );
                    Line::from(vec![
                        Span::styled(primary, row_style),
                        Span::styled("  ", row_style),
                        Span::styled(detail, detail_style),
                    ])
                }
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(Text::from(items))
            .style(Style::default().bg(colors.status_bar_bg))
            .wrap(Wrap { trim: true }),
        rows_area,
    );

    let query_line = match &modal.kind {
        PickerKind::InteractiveQuestion { prompt } => {
            format!("Question: {}", truncate_chars(prompt, 200))
        }
        _ => format!(
            "Search: {}",
            if modal.query.is_empty() {
                "(type to filter)"
            } else {
                modal.query.as_str()
            }
        ),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            truncate_chars(&query_line, query_area.width as usize),
            Style::default()
                .fg(colors.accent)
                .bg(colors.status_bar_bg)
                .add_modifier(Modifier::BOLD),
        )])),
        query_area,
    );

    let footer = if matches!(modal.kind, PickerKind::InteractiveQuestion { .. }) {
        "↑↓ choose • Enter insert answer • Esc close"
    } else if modal.allow_multi {
        "↑↓ move • PgUp/PgDn page • Space toggle • Enter confirm • Esc close"
    } else if matches!(modal.kind, PickerKind::ModelProvider) {
        "↑↓ move • 1-9/0 quick-pick • d disconnect • Enter select • Esc close"
    } else {
        "↑↓ move • PgUp/PgDn page • 1-9/0 quick-pick • Enter select • Esc close"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            truncate_chars(footer, footer_area.width as usize),
            Style::default()
                .fg(colors.status_bar_dim)
                .bg(colors.status_bar_bg),
        )])),
        footer_area,
    );
}

/// Render the input area (supports multi-line display with wrapping).
pub(crate) fn render_input(
    frame: &mut Frame,
    state: &mut TuiState,
    area: Rect,
    colors: &crate::theme::RatatuiColors,
) {
    let paint_snapshot = state.phase.composer_mut().input_paint_snapshot();
    let input_changed =
        state.phase.composer_mut().last_input_paint.as_ref() != Some(&paint_snapshot);
    let mode_label = match state.phase.composer_mut().mode {
        InputMode::Normal => "NORMAL",
        InputMode::Insert => "INSERT",
        InputMode::Command => "COMMAND",
    };
    let mode_color = match state.phase.composer_mut().mode {
        InputMode::Normal => colors.status_bar_dim,
        InputMode::Insert => colors.status_bar_good,
        InputMode::Command => colors.accent,
    };
    let line_count = state.phase.composer_mut().input.matches('\n').count() + 1;

    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(vec![
            Span::styled(" Message  •  ", Style::default().fg(colors.status_bar_dim)),
            Span::styled(
                mode_label.to_string(),
                Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  •  L{}  •  Ctrl+←/→ word-jump ", line_count),
                Style::default().fg(colors.status_bar_dim),
            ),
        ]))
        .style(Style::default().bg(colors.background))
        .border_style(Style::default().fg(colors.status_bar_strong));
    if state.phase.composer_mut().history_search_active {
        block = block.title_bottom(Line::from(Span::styled(
            format!(
                " reverse-i-search: `{}` (Ctrl+R to exit) ",
                state.phase.composer_mut().history_search_query
            ),
            Style::default()
                .fg(colors.status_bar_warn)
                .bg(colors.background)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    let mut textarea = state.textarea_from_input();
    textarea.set_block(block.clone());
    textarea.set_style(Style::default().fg(colors.foreground).bg(colors.background));
    textarea.set_cursor_style(
        Style::default()
            .fg(Color::Black)
            .bg(colors.status_bar_strong)
            .add_modifier(Modifier::BOLD),
    );
    textarea.set_cursor_line_style(Style::default().bg(colors.background));
    if state.phase.composer_mut().input.is_empty()
        && state.phase.composer_mut().mode == InputMode::Insert
        && !state.phase.composer_mut().history_search_active
    {
        let clarify_awaiting = state
            .phase
            .processing()
            .map(|p| p.clarify_awaiting)
            .unwrap_or(false);
        let placeholder = if clarify_awaiting {
            "Clarify: reply with option number or text (Enter sends)"
        } else {
            "Type a message (Enter sends, Shift+Enter/Ctrl+J inserts newline)"
        };
        textarea.set_placeholder_text(placeholder);
        textarea.set_placeholder_style(
            Style::default()
                .fg(colors.status_bar_dim)
                .bg(colors.background)
                .add_modifier(Modifier::ITALIC),
        );
    } else {
        textarea.set_placeholder_text("");
    }

    if input_changed {
        frame.render_widget(Clear, area);
    }
    frame.render_widget(&textarea, area);
    state.phase.composer_mut().last_input_paint = Some(paint_snapshot);
}

/// Render the status bar at the bottom of the screen.
pub(crate) fn status_message_style(message: &str, colors: &crate::theme::RatatuiColors) -> Style {
    let lower = message.to_ascii_lowercase();
    if lower.contains("error") {
        Style::default()
            .fg(colors.status_bar_critical)
            .bg(colors.status_bar_bg)
    } else if lower.contains("warn") {
        Style::default()
            .fg(colors.status_bar_warn)
            .bg(colors.status_bar_bg)
    } else {
        Style::default()
            .fg(colors.status_bar_text)
            .bg(colors.status_bar_bg)
    }
}

/// Render the status bar at the bottom of the screen.
pub(crate) fn render_status(
    frame: &mut Frame,
    app: &impl TuiReadHost,
    state: &TuiState,
    area: Rect,
    colors: &crate::theme::RatatuiColors,
) {
    let processing_indicator = if state.phase.is_processing() {
        format!("⟳{}", state.spinner_char())
    } else {
        "✓".to_string()
    };
    let model = app.current_model();
    let session = &app.session_id()[..8.min(app.session_id().len())];
    let msg_count = state
        .transcript_cache
        .total_messages
        .max(app.messages().len());
    let scroll_hint = if state.scroll_offset > 0 {
        format!(" (history +{})", state.scroll_offset)
    } else {
        String::new()
    };

    let base = Style::default()
        .fg(colors.status_bar_text)
        .bg(colors.status_bar_bg);

    let mut status_text = if state.phase.is_processing() {
        let elapsed = state.processing_elapsed().as_secs_f64();
        format!(
            "{} PROCESSING {:.1}s [{}] {} | {} | {} msgs | {}",
            processing_indicator,
            elapsed,
            animated_processing_bar(state.spinner_frame, 12),
            state.processing_stage_label(),
            state.phase.composer().mode,
            msg_count,
            session
        )
    } else {
        format!(
            "{} {} | {} | {} msgs | {}",
            processing_indicator,
            state.phase.composer().mode,
            model,
            msg_count,
            session
        )
    };
    status_text.push_str(match state.view_density {
        ViewDensity::Compact => " | compact",
        ViewDensity::Detailed => " | detailed",
    });
    if state.show_timestamps {
        status_text.push_str(" | ts:on");
    }
    if state.activity_lane_open {
        status_text.push_str(" | lane:on");
    } else {
        status_text.push_str(" | lane:off");
    }
    status_text.push_str(match state.activity_lane_mode {
        ActivityLaneMode::Live => " (live)",
        ActivityLaneMode::Cockpit => " (cockpit)",
    });
    if state.background_jobs_running > 0 {
        status_text.push_str(&format!(" | bg:{}", state.background_jobs_running));
    }
    status_text.push_str(if app.mouse_enabled() {
        " | mouse:on"
    } else {
        " | mouse:off"
    });
    if !state.sticky_prompt.is_empty() {
        status_text.push_str(&format!(
            " | ↳ {}",
            truncate_chars(&state.sticky_prompt, 40)
        ));
    }
    let usage = app.agent().session_usage_metrics();
    if usage.api_calls > 0 {
        status_text.push_str(&format!(
            " | tok:{} calls:{}",
            usage.total_tokens, usage.api_calls
        ));
    }
    if !state.status_message.is_empty() || !scroll_hint.is_empty() {
        status_text.push_str(" | ");
        status_text.push_str(&state.status_message);
        status_text.push_str(&scroll_hint);
    }
    if let Some(frame_token) = pet_frame_token(
        app.pet_settings(),
        state.pet_frame,
        state.phase.is_processing(),
    ) {
        if matches!(app.pet_settings().dock, crate::app::PetDock::Left) {
            status_text = format!("{frame_token} | {status_text}");
        } else {
            status_text.push_str(&format!(" | {frame_token}"));
        }
    }
    let clipped = fit_status_line(&status_text, area.width.saturating_sub(1) as usize);
    let line_style = if state.status_message.is_empty() {
        base
    } else {
        status_message_style(&state.status_message, colors).bg(colors.status_bar_bg)
    };
    let status_bar = Paragraph::new(Line::from(Span::styled(clipped, line_style)))
        .block(Block::default().style(Style::default().bg(colors.status_bar_bg)));
    frame.render_widget(status_bar, area);
}

pub(crate) fn animated_processing_bar(frame: usize, width: usize) -> String {
    let width = width.max(6);
    let head = frame % width;
    let trail = 3usize;
    let mut out = String::with_capacity(width);
    for i in 0..width {
        let lit = if head >= trail {
            i >= head - trail && i <= head
        } else {
            i <= head || i + width >= head + width - trail
        };
        out.push(if lit { '█' } else { '·' });
    }
    out
}

pub(crate) fn pet_frame_token(
    settings: &crate::app::PetSettings,
    frame: usize,
    processing: bool,
) -> Option<String> {
    if !settings.enabled {
        return None;
    }
    let effective_mood = if processing && settings.mood != "sleepy" {
        "working"
    } else {
        settings.mood.as_str()
    };
    let frames: [&str; 2] = match (settings.species.as_str(), effective_mood) {
        ("boba", "sleepy") => ["(-_- )z", "(-_- )Z"],
        ("boba", "working") => ["(>_< )", "(<_< )"],
        ("boba", "hyped") => ["(o_o)!", "(!o_o)"],
        ("boba", "chill") => ["(u_u )", "(u_U )"],
        ("bytecat", "sleepy") => ["= -.-=z", "= -.-=Z"],
        ("bytecat", "working") => ["=^x^=", "=^_^="],
        ("bytecat", "hyped") => ["=^o^=!", "=^O^=!"],
        ("bytecat", "chill") => ["=^.^=~", "=^.-=~"],
        ("otter", "sleepy") => ["(>< )z", "(>< )Z"],
        ("otter", "working") => ["(>> )~", "(<< )~"],
        ("otter", "hyped") => ["(OO )~", "(oo )~"],
        ("otter", "chill") => ["(~~ )~", "(~_ )~"],
        ("fox", "sleepy") => ["{-- }z", "{-- }Z"],
        ("fox", "working") => ["{^x }", "{x^ }"],
        ("fox", "hyped") => ["{^^ }!", "{oo }!"],
        ("fox", "chill") => ["{.. }", "{._ }"],
        ("owl", "sleepy") => ["(v_v)z", "(v_v)Z"],
        ("owl", "working") => ["(O_O)", "(0_0)"],
        ("owl", "hyped") => ["(O0O)!", "(0O0)!"],
        ("owl", "chill") => ["(o_o)", "(o_O)"],
        ("capy", "sleepy") => ["(._.)z", "(._.)Z"],
        ("capy", "working") => ["(>_.)", "(._<)"],
        ("capy", "hyped") => ["(o_.)!", "(._o)!"],
        ("capy", "chill") => ["(._.)~", "(.._)~"],
        ("boba", _) => ["(o_o )", "(O_O )"],
        ("bytecat", _) => ["=^.^=", "=^o^="],
        ("otter", _) => ["(>< )~", "(~>< )"],
        ("fox", _) => ["{^.^}", "{^o^}"],
        ("owl", _) => ["(OvO)", "(oVo)"],
        ("capy", _) => ["(._.)", "(o_.)"],
        _ => ["(o_o )", "(O_O )"],
    };
    Some(frames[frame % frames.len()].to_string())
}
