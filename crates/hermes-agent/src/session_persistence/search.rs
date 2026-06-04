//! Full-text search — delegates to shared `hermes_tools::state_db`.

use std::sync::{Arc, Mutex};

use hermes_core::AgentError;
use rusqlite::Connection;

pub use hermes_tools::state_db::{SearchMessageMatch, sanitize_fts5_query};

use hermes_tools::state_db::search_messages as search_messages_impl;

/// Full-text search across session messages (Python `SessionDB.search_messages`).
pub fn search_messages(
    conn: &Arc<Mutex<Connection>>,
    query: &str,
    source_filter: Option<&[&str]>,
    exclude_sources: Option<&[&str]>,
    role_filter: Option<&[&str]>,
    limit: usize,
    offset: usize,
    sort: Option<&str>,
) -> Result<Vec<SearchMessageMatch>, AgentError> {
    search_messages_impl(
        conn,
        query,
        source_filter,
        exclude_sources,
        role_filter,
        limit,
        offset,
        sort,
    )
    .map_err(|e| AgentError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_persistence::queries::{append_messages, create_session};
    use crate::session_persistence::schema::init_schema;
    use hermes_core::Message;
    use std::sync::Mutex;

    fn mem_conn() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[test]
    fn search_finds_ascii_term() {
        let conn = mem_conn();
        create_session(&conn, "s1", "cli", None, None, None, None).unwrap();
        append_messages(
            &conn,
            "s1",
            &[Message::user("docker deployment guide")],
        )
        .unwrap();
        let hits = search_messages(&conn, "docker", None, None, None, 10, 0, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "s1");
    }
}
