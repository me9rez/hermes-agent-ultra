//! Integration tests for incremental session DB append (Python `_last_flushed_db_idx` parity).

use hermes_agent::{leading_system_prompt_for_persist, SessionFlushCursor, SessionPersistence};
use hermes_core::Message;

fn persist(
    sp: &SessionPersistence,
    session_id: &str,
    messages: &[Message],
    cursor: &mut SessionFlushCursor,
) {
    sp.persist_session(session_id, messages, cursor, None, None, None, None)
        .expect("persist_session");
}

#[test]
fn incremental_append_only_new_messages() {
    let tmp = tempfile::tempdir().unwrap();
    let sp = SessionPersistence::new(tmp.path());
    let mut cursor = SessionFlushCursor::new();

    persist(&sp, "sess-1", &[Message::user("First")], &mut cursor);

    persist(
        &sp,
        "sess-1",
        &[
            Message::user("First"),
            Message::assistant("Response"),
            Message::user("Second"),
        ],
        &mut cursor,
    );

    let loaded = sp.load_session("sess-1").unwrap();
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded[0].content.as_deref(), Some("First"));
    assert_eq!(loaded[1].content.as_deref(), Some("Response"));
    assert_eq!(loaded[2].content.as_deref(), Some("Second"));
}

#[test]
fn replace_session_messages_clears_and_rewrites() {
    let tmp = tempfile::tempdir().unwrap();
    let sp = SessionPersistence::new(tmp.path());
    let mut cursor = SessionFlushCursor::new();

    persist(
        &sp,
        "sess-2",
        &[Message::user("A"), Message::assistant("B")],
        &mut cursor,
    );
    assert_eq!(cursor.last_flushed_db_idx, 2);

    let compressed = vec![Message::user("summary"), Message::user("recent")];
    sp.replace_session_messages("sess-2", &compressed, &mut cursor)
        .unwrap();

    assert_eq!(cursor.last_flushed_db_idx, 2);
    let loaded = sp.load_session("sess-2").unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].content.as_deref(), Some("summary"));
}

#[test]
fn compression_lock_acquire_and_release() {
    let tmp = tempfile::tempdir().unwrap();
    let sp = SessionPersistence::new(tmp.path());
    sp.ensure_db().unwrap();
    assert!(sp
        .try_acquire_compression_lock("sess-lock", "holder-a", 300.0)
        .unwrap());
    assert_eq!(
        sp.get_compression_lock_holder("sess-lock").unwrap().as_deref(),
        Some("holder-a")
    );
    assert!(!sp
        .try_acquire_compression_lock("sess-lock", "holder-b", 300.0)
        .unwrap());
    sp.release_compression_lock("sess-lock", "holder-a").unwrap();
    assert!(sp
        .try_acquire_compression_lock("sess-lock", "holder-b", 300.0)
        .unwrap());
}

#[test]
fn leading_system_prompt_for_persist_joins_prefix() {
    let msgs = vec![
        Message::system("Part A"),
        Message::system("Part B"),
        Message::user("hi"),
    ];
    assert_eq!(
        leading_system_prompt_for_persist(&msgs).as_deref(),
        Some("Part A\n\nPart B")
    );
}
