//! Telegram DM topic-mode tables — delegates to `hermes_tools::state_db`.

use std::sync::{Arc, Mutex};

use hermes_core::AgentError;
use rusqlite::Connection;

pub use hermes_tools::state_db::{TelegramTopicBinding, UnlinkedTelegramSession};

/// Create Telegram topic-mode tables on explicit `/topic` opt-in (Python parity).
pub fn apply_telegram_topic_migration(
    conn: &Arc<Mutex<Connection>>,
) -> Result<(), AgentError> {
    hermes_tools::state_db::apply_telegram_topic_migration(conn)
        .map_err(|e| AgentError::Io(e.to_string()))
}
