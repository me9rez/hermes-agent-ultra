//! Session-scoped log context (Python `hermes_logging.set_session_context`).

use std::cell::RefCell;

thread_local! {
    static SESSION_ID: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Bind the active session id for the current thread (cleared on turn end).
pub fn set_session_context(session_id: Option<&str>) {
    SESSION_ID.with(|slot| {
        *slot.borrow_mut() = session_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
    });
}

pub fn clear_session_context() {
    SESSION_ID.with(|slot| *slot.borrow_mut() = None);
}

/// Current session id for diagnostics (falls back to `"none"`).
pub fn current_session_tag() -> String {
    SESSION_ID
        .with(|slot| slot.borrow().clone())
        .unwrap_or_else(|| "none".to_string())
}

/// Enter a tracing span tagged with the active session.
pub fn conversation_turn_span() -> tracing::Span {
    tracing::info_span!("conversation_turn", session_id = %current_session_tag())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_context_roundtrip() {
        set_session_context(Some("sess-abc"));
        assert_eq!(current_session_tag(), "sess-abc");
        clear_session_context();
        assert_eq!(current_session_tag(), "none");
    }
}
