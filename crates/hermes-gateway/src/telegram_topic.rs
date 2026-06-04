//! Telegram DM multi-session topic mode (`/topic` command parity).

use hermes_tools::state_db::StateDb;

use crate::gateway::IncomingMessage;

const LOBBY_PROMPT: &str = "This main chat is reserved for system commands.\n\n\
To chat with Hermes, create a new topic using the + button in this bot interface. \
Each topic works as an independent Hermes session.";

const ONBOARDING: &str = "Multi-session mode is enabled.\n\n\
Create new Hermes chats with the + button in this bot interface. Each Telegram topic is an \
independent Hermes session, so you can work on different tasks in parallel.\n\n\
This main chat is reserved for system commands, status, and session management.\n\n\
To restore an old session:\n\
1. Use /topic here to see unlinked sessions.\n\
2. Create a new topic with the + button.\n\
3. Send /topic <session_id> inside that topic.";

fn open_db() -> Option<StateDb> {
    StateDb::open_default().ok()
}

fn is_root_dm(incoming: &IncomingMessage) -> bool {
    incoming.platform == "telegram" && incoming.is_dm && incoming.message_thread_id.is_none()
}

fn is_topic_dm(incoming: &IncomingMessage) -> bool {
    incoming.platform == "telegram"
        && incoming.is_dm
        && incoming.message_thread_id.is_some()
}

/// Compose session key including Telegram DM topic thread when present.
pub fn compose_telegram_session_key(incoming: &IncomingMessage) -> Option<String> {
    if incoming.platform != "telegram" {
        return None;
    }
    let thread = incoming.message_thread_id.as_deref()?;
    Some(format!("{}:{}:{}", incoming.platform, incoming.chat_id, thread))
}

/// Returns `Some(reply)` when the message was fully handled as a topic command.
pub fn try_handle_topic_command(incoming: &IncomingMessage) -> Option<String> {
    if incoming.platform != "telegram" || !incoming.is_dm {
        return None;
    }
    let text = incoming.text.trim();
    if !text.starts_with("/topic") {
        return None;
    }
    let db = open_db()?;
    let chat_id = incoming.chat_id.as_str();
    let user_id = incoming.user_id.as_str();
    let parts: Vec<&str> = text.split_whitespace().collect();
    let sub = parts.get(1).copied();

    if is_root_dm(incoming) {
        return Some(handle_root_topic_command(&db, chat_id, user_id, sub));
    }
    if is_topic_dm(incoming) {
        return Some(handle_thread_topic_command(
            &db,
            incoming,
            chat_id,
            user_id,
            sub,
        ));
    }
    None
}

fn handle_root_topic_command(
    db: &StateDb,
    chat_id: &str,
    user_id: &str,
    sub: Option<&str>,
) -> String {
    match sub.map(str::trim) {
        None | Some("") => {
            let _ = db.apply_telegram_topic_migration();
            let _ = db.enable_telegram_topic_mode(chat_id, user_id, None, None);
            format!("{ONBOARDING}\n\n{}", format_unlinked_list(db, user_id))
        }
        Some("off") => {
            let _ = db.disable_telegram_topic_mode(chat_id, true);
            "Telegram multi-session topic mode disabled for this chat.".into()
        }
        Some("help") => ONBOARDING.to_string(),
        Some(other) => format!(
            "Unknown /topic subcommand '{other}'. Try /topic, /topic off, or /topic help."
        ),
    }
}

fn handle_thread_topic_command(
    db: &StateDb,
    incoming: &IncomingMessage,
    chat_id: &str,
    user_id: &str,
    sub: Option<&str>,
) -> String {
    let thread_id = incoming.message_thread_id.as_deref().unwrap_or_default();
    match sub.map(str::trim) {
        None | Some("") => {
            if let Ok(Some(binding)) = db.get_telegram_topic_binding(chat_id, thread_id) {
                format!(
                    "This topic is linked to:\nSession ID: {}\nSession key: {}\n\n\
Use /new to replace this topic with a fresh session.\n\
For parallel work, create another topic with the + button.",
                    binding.session_id, binding.session_key
                )
            } else {
                "This topic is not linked yet — your next message will create a session lane.".into()
            }
        }
        Some(session_id) => {
            let _ = db.apply_telegram_topic_migration();
            let session_key = compose_telegram_session_key(incoming)
                .unwrap_or_else(|| format!("telegram:{chat_id}:{thread_id}"));
            match db.bind_telegram_topic(
                chat_id,
                thread_id,
                user_id,
                &session_key,
                session_id,
                "restored",
            ) {
                Ok(()) => format!(
                    "Session restored: {session_id}\n\nSend a message to continue this conversation in this topic."
                ),
                Err(e) => format!("Failed to restore session: {e}"),
            }
        }
    }
}

fn format_unlinked_list(db: &StateDb, user_id: &str) -> String {
    let Ok(rows) = db.list_unlinked_telegram_sessions_for_user(user_id, 10) else {
        return String::new();
    };
    if rows.is_empty() {
        return "No unlinked previous Telegram sessions found.".into();
    }
    let mut out = String::from("Unlinked previous sessions:\n");
    for (idx, row) in rows.iter().enumerate() {
        let title = row.title.as_deref().unwrap_or("(untitled)");
        let preview = row.preview.as_deref().unwrap_or("");
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!("{}. {} — id: {} — {}\n", idx + 1, title, row.id, preview),
        );
    }
    out.push_str("\nTo restore: create a topic with +, then send /topic <id> inside it.");
    out
}

/// Lobby gate: block normal prompts in root DM when topic mode is active.
pub fn telegram_lobby_reply(incoming: &IncomingMessage) -> Option<String> {
    if !is_root_dm(incoming) {
        return None;
    }
    let text = incoming.text.trim();
    if text.starts_with('/') {
        if hermes_tools::state_db::is_telegram_lobby_system_command(text)
            || text.starts_with("/topic")
        {
            return None;
        }
        if text.eq_ignore_ascii_case("/new") {
            return Some(
                "To start a new parallel Hermes chat, create a new topic with the + button.\n\n\
Each topic is an independent Hermes session. Use /new inside a topic only if you want to \
replace that topic's current session."
                    .into(),
            );
        }
    }
    let db = open_db()?;
    if !db.is_telegram_topic_mode_enabled(&incoming.chat_id, &incoming.user_id) {
        return None;
    }
    Some(LOBBY_PROMPT.to_string())
}

/// Returns the Hermes session id bound to this Telegram topic thread, if any.
pub fn bound_session_id(incoming: &IncomingMessage) -> Option<String> {
    if !is_topic_dm(incoming) {
        return None;
    }
    let db = open_db()?;
    let chat_id = incoming.chat_id.as_str();
    let thread_id = incoming.message_thread_id.as_deref()?;
    db.get_telegram_topic_binding(chat_id, thread_id)
        .ok()
        .flatten()
        .map(|b| b.session_id)
}

/// Auto-bind a new topic lane on first message (when no binding exists).
pub fn maybe_bind_new_topic_lane(
    incoming: &IncomingMessage,
    session_key: &str,
    session_id: &str,
) {
    if !is_topic_dm(incoming) {
        return;
    }
    let Some(db) = open_db() else {
        return;
    };
    let chat_id = incoming.chat_id.as_str();
    let user_id = incoming.user_id.as_str();
    let thread_id = incoming.message_thread_id.as_deref().unwrap_or_default();
    if db
        .get_telegram_topic_binding(chat_id, thread_id)
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    let _ = db.bind_telegram_topic(
        chat_id,
        thread_id,
        user_id,
        session_key,
        session_id,
        "auto",
    );
}
