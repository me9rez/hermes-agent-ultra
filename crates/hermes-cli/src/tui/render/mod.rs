mod transcript;
mod widgets;

pub(crate) use transcript::*;

#[cfg(test)]
pub(crate) use widgets::{animated_processing_bar, pet_frame_token, status_message_style};

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::Block;

use hermes_core::Message;

use hermes_core::AgentError;

use super::Tui;
use super::TuiReadHost;
use super::event::Event;
use super::state::TuiState;
use super::types::InputMode;
use crate::theme::Theme;
use widgets::{
    render_completions_popup, render_header, render_input, render_live_details, render_messages,
    render_picker_modal, render_status,
};
pub fn render(frame: &mut Frame, app: &impl TuiReadHost, state: &mut TuiState, theme: &Theme) {
    let resolved = theme.resolved_styles();
    let colors = theme.colors.to_ratatui_colors();

    let size = frame.area();
    if size.width == 0 || size.height == 0 {
        return;
    }
    frame.render_widget(
        Block::default().style(Style::default().bg(colors.background)),
        size,
    );

    // Layout: header, body, input, status bar
    let header_height = 1;
    let composer_lines = state.phase.composer_mut().input.matches('\n').count() as u16 + 1;
    let input_height = (composer_lines + 2).clamp(3, 12);
    let status_height = 1;

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height), // header
            Constraint::Min(5),                // body
            Constraint::Length(input_height),  // input
            Constraint::Length(status_height), // status
        ])
        .split(size);

    let header_area = vertical[0];
    let body_area = vertical[1];
    let input_area = vertical[2];
    let status_area = vertical[3];

    let details_enabled = state.activity_lane_open && body_area.width >= 86;
    let body_split = if details_enabled {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(38)])
            .split(body_area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20)])
            .split(body_area)
    };
    let messages_area = body_split[0];
    let details_area = if details_enabled {
        Some(body_split[1])
    } else {
        None
    };

    render_header(frame, app, header_area, &colors);

    // --- Render message history ---
    render_messages(frame, app, state, messages_area, &resolved, &colors);

    if let Some(details_area) = details_area {
        render_live_details(frame, app, state, details_area, &colors);
    }

    // --- Render input area ---
    render_input(frame, state, input_area, &colors);

    // --- Render completions as popup above composer ---
    if should_render_completions_popup(state) {
        let composer = state.phase.composer();
        render_completions_popup(
            frame,
            &composer.completions,
            composer.completion_index,
            messages_area,
            input_area,
            &colors,
        );
    }

    if let Some(modal) = state.phase.modal().as_ref() {
        render_picker_modal(frame, modal, &colors);
    }

    // --- Render status bar ---
    render_status(frame, app, state, status_area, &colors);
}

pub(crate) fn draw_frame_now(
    tui: &mut Tui,
    app: &impl TuiReadHost,
    state: &mut TuiState,
) -> Result<(), AgentError> {
    state.refresh_sticky_prompt(app);
    let active_theme = tui.theme().clone();
    tui.terminal
        .draw(|f| render(f, app, state, &active_theme))
        .map(|_| ())
        .map_err(|e| AgentError::Config(e.to_string()))
}

pub(crate) fn stream_event_completes_background_task(event: &Event) -> bool {
    matches!(
        event,
        Event::AgentRunComplete { .. } | Event::ManagedAppRunComplete { .. }
    )
}

pub(crate) fn should_render_completions_popup(state: &TuiState) -> bool {
    state.phase.composer().mode != InputMode::Normal
        && !state.phase.is_processing()
        && !state.phase.modal_active()
        && state.phase.composer().input.starts_with('/')
        && !state.phase.composer().input.contains('\n')
        && !state.phase.composer().history_search_active
        && !state.phase.composer().completions.is_empty()
}

pub(crate) fn should_route_prompt_via_managed_agent(
    quorum_armed_once: bool,
    messages: &[Message],
) -> bool {
    if quorum_armed_once {
        return true;
    }
    messages.iter().any(|message| {
        message.role == hermes_core::MessageRole::System
            && message
                .content
                .as_deref()
                .unwrap_or_default()
                .starts_with("[QUORUM_MODE] ")
    })
}
