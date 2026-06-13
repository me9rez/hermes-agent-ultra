//! Terminal UI using ratatui + crossterm (Requirement 9.1-9.6).
//!
//! Implements the interactive terminal interface with:
//! - Message history rendering (9.1, 9.4)
//! - Input area with slash command auto-completion (9.2)
//! - Ctrl+C immediate exit back to parent terminal (with interrupt signal) (9.3)
//! - Streaming output display (9.5)
//! - Status bar with model/session info (9.6)
//! - Theme/skin engine support (9.8)

use std::io::{Stdout, Write};

use crossterm::ExecutableCommand;
use crossterm::cursor::{Hide, Show};
use crossterm::style::ResetColor;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::app::{
    AcpServerRuntime, AgentCoordinator, ModelRuntime, SessionRuntime, SessionSnapshotRuntime,
    SlashCommandHost, TranscriptRuntime, UiChromeRuntime,
};
use crate::theme::Theme;

mod event;
mod pipeline;
mod render;
mod run_loop;
mod state;
mod text;
mod types;

mod transcript_cache;
mod ui_phase;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use types::{PickerItem, PickerKind};

pub use event::{Event, StreamHandle};
pub use run_loop::run;
pub use state::TuiState;
pub use types::{ActivityLaneMode, InputMode, ToolOutputSection, ViewDensity};

pub(crate) use types::{PickerModal, StreamMarkdownCache};

pub use render::render;

pub trait TuiReadHost:
    SessionRuntime + ModelRuntime + TranscriptRuntime + UiChromeRuntime + AgentCoordinator
{
}
impl<T> TuiReadHost for T where
    T: SessionRuntime + ModelRuntime + TranscriptRuntime + UiChromeRuntime + AgentCoordinator
{
}

#[allow(dead_code)]
trait TuiLoopHost: SlashCommandHost + SessionSnapshotRuntime + AcpServerRuntime {}
impl<T> TuiLoopHost for T where T: SlashCommandHost + SessionSnapshotRuntime + AcpServerRuntime {}

/// The terminal UI wrapper.
///
/// Owns the ratatui Terminal and provides methods for rendering,
/// event handling, and theme management.
pub struct Tui {
    /// The ratatui terminal backend.
    pub terminal: ratatui::Terminal<CrosstermBackend<Stdout>>,
    /// Channel receiver for control/UI events (keys, mouse, resize, app control).
    pub events: mpsc::UnboundedReceiver<Event>,
    /// Channel receiver for high-volume stream events (tokens/chunks).
    pub stream_events: mpsc::UnboundedReceiver<Event>,
    /// Channel sender for control/UI events.
    event_sender: mpsc::UnboundedSender<Event>,
    /// Channel sender for stream events.
    stream_sender: mpsc::UnboundedSender<Event>,
    /// The active color theme.
    theme: Theme,
    /// Whether terminal cleanup has already run.
    restored: bool,
    /// Whether mouse capture is currently enabled on the terminal backend.
    mouse_capture_enabled: bool,
}

impl Tui {
    /// Create a new Tui instance, initializing the terminal.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        stdout.execute(crossterm::event::EnableBracketedPaste)?;
        stdout.execute(EnterAlternateScreen)?;
        stdout.execute(Hide)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = ratatui::Terminal::new(backend)?;
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        let (stream_sender, stream_receiver) = mpsc::unbounded_channel();
        let requested_theme =
            std::env::var("HERMES_THEME").unwrap_or_else(|_| "ultra-sunburst".to_string());
        Ok(Self {
            terminal,
            events: event_receiver,
            stream_events: stream_receiver,
            event_sender,
            stream_sender,
            theme: crate::skin_engine::resolve_theme(requested_theme.as_str()),
            restored: false,
            mouse_capture_enabled: false,
        })
    }

    pub fn set_mouse_capture(&mut self, enabled: bool) -> Result<(), Box<dyn std::error::Error>> {
        if self.mouse_capture_enabled == enabled {
            return Ok(());
        }
        if enabled {
            self.terminal
                .backend_mut()
                .execute(crossterm::event::EnableMouseCapture)?;
        } else {
            self.terminal
                .backend_mut()
                .execute(crossterm::event::DisableMouseCapture)?;
        }
        self.mouse_capture_enabled = enabled;
        Ok(())
    }

    /// Restore the terminal to its original state.
    pub fn restore(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.restored {
            return Ok(());
        }
        disable_raw_mode()?;
        if self.mouse_capture_enabled {
            self.terminal
                .backend_mut()
                .execute(crossterm::event::DisableMouseCapture)?;
            self.mouse_capture_enabled = false;
        }
        self.terminal
            .backend_mut()
            .execute(crossterm::event::DisableBracketedPaste)?;
        self.terminal.backend_mut().execute(LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        let mut stdout = std::io::stdout();
        let _ = stdout.execute(ResetColor);
        let _ = stdout.flush();
        self.restored = true;
        Ok(())
    }

    /// Get a sender for injecting events (used by async tasks).
    pub fn event_sender(&self) -> mpsc::UnboundedSender<Event> {
        self.event_sender.clone()
    }

    /// Get a sender for injecting high-volume stream events.
    pub fn stream_sender(&self) -> mpsc::UnboundedSender<Event> {
        self.stream_sender.clone()
    }

    /// Set the active theme.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// Get a reference to the current theme.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        if self.restored {
            return;
        }
        let _ = disable_raw_mode();
        let mut stdout = std::io::stdout();
        let _ = stdout.execute(crossterm::event::DisableBracketedPaste);
        let _ = stdout.execute(crossterm::event::DisableMouseCapture);
        let _ = stdout.execute(LeaveAlternateScreen);
        let _ = stdout.execute(ResetColor);
        let _ = stdout.execute(Show);
        let _ = stdout.flush();
        self.restored = true;
    }
}
