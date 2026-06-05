//! Native WhatsApp client via wa-rs (replaces Node Baileys bridge).

use std::collections::HashSet;
use std::io::Cursor;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Mutex, Notify, RwLock};
use tracing::{debug, info, warn};
use wa_rs::bot::Bot;
use wa_rs::store::SqliteStore;
use wa_rs::types::events::Event;
use wa_rs::types::message::MessageInfo;
use wa_rs::Jid;
use wa_rs_core::download::MediaType;
use wa_rs_core::proto_helpers::MessageExt;
use wa_rs_proto::whatsapp as wa;
use wa_rs_tokio_transport::TokioWebSocketTransportFactory;
use wa_rs_ureq_http::UreqHttpClient;

use hermes_core::errors::GatewayError;

use crate::whatsapp_identity::expand_whatsapp_aliases;

use super::config::WhatsAppConfig;
use super::policy::WhatsAppPolicy;
use super::session_store::{ensure_session_dir, mark_paired, session_db_path};

/// Normalized inbound message (same shape as the former Baileys bridge queue).
#[derive(Debug, Clone)]
pub struct WaMessage {
    pub message_id: String,
    pub chat_id: String,
    pub sender_id: String,
    pub is_group: bool,
    pub body: String,
    pub has_media: bool,
    pub media_type: String,
    pub media_urls: Vec<String>,
    pub mentioned_ids: Vec<String>,
    pub quoted_participant: Option<String>,
    pub bot_ids: Vec<String>,
}

struct ClientState {
    client: Option<Arc<wa_rs::Client>>,
    bot_ids: Vec<String>,
    bot_pn: Option<String>,
    bot_lid: Option<String>,
    recently_sent: HashSet<String>,
    connected: bool,
}

pub struct WhatsAppRustClient {
    config: WhatsAppConfig,
    state: Arc<RwLock<ClientState>>,
    run_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    stop: Arc<Notify>,
    pair_done: Arc<Notify>,
    pairing_qr_shown: Arc<AtomicBool>,
    pairing_crypto_done: Arc<AtomicBool>,
}

impl WhatsAppRustClient {
    pub fn new(config: WhatsAppConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(ClientState {
                client: None,
                bot_ids: Vec::new(),
                bot_pn: None,
                bot_lid: None,
                recently_sent: HashSet::new(),
                connected: false,
            })),
            run_handle: Mutex::new(None),
            stop: Arc::new(Notify::new()),
            pair_done: Arc::new(Notify::new()),
            pairing_qr_shown: Arc::new(AtomicBool::new(false)),
            pairing_crypto_done: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn client(&self) -> Result<Arc<wa_rs::Client>, GatewayError> {
        self.state
            .read()
            .await
            .client
            .clone()
            .ok_or_else(|| GatewayError::SendFailed("WhatsApp client not connected".into()))
    }

    pub async fn is_connected(&self) -> bool {
        self.state.read().await.connected
    }

    /// Self-chat inbound often uses `@lid`; outbound to PN is more reliable in wa-rs.
    pub async fn resolve_outbound_chat_id(&self, inbound_chat_id: &str) -> String {
        if !inbound_chat_id.contains("@lid") {
            return inbound_chat_id.to_string();
        }
        let st = self.state.read().await;
        if let Some(pn) = st.bot_pn.as_ref() {
            return format!("{pn}@s.whatsapp.net");
        }
        inbound_chat_id.to_string()
    }

    async fn build_bot(
        &self,
        inbound_tx: Option<mpsc::UnboundedSender<WaMessage>>,
        pair_only: bool,
    ) -> Result<Bot, GatewayError> {
        let session_path = self.config.session_path();
        ensure_session_dir(&session_path).map_err(|e| {
            GatewayError::ConnectionFailed(format!("create session dir: {e}"))
        })?;
        let db_path = session_db_path(&session_path);
        let db_path_str = db_path.to_string_lossy().into_owned();

        let backend = Arc::new(
            SqliteStore::new(&db_path_str)
                .await
                .map_err(|e| GatewayError::ConnectionFailed(format!("sqlite store: {e}")))?,
        );

        let config = self.config.clone();
        let session_path_clone = session_path.clone();
        let pair_done = Arc::clone(&self.pair_done);
        let pairing_qr_shown = Arc::clone(&self.pairing_qr_shown);
        let pairing_crypto_done = Arc::clone(&self.pairing_crypto_done);
        let stop = Arc::clone(&self.stop);
        let state = self.state.clone();

        let builder = Bot::builder()
            .with_backend(backend)
            .with_transport_factory(TokioWebSocketTransportFactory::new())
            .with_http_client(UreqHttpClient::new())
            .skip_history_sync()
            .on_event(move |event, client| {
                let inbound_tx = inbound_tx.clone();
                let config = config.clone();
                let session_path = session_path_clone.clone();
                let pair_done = pair_done.clone();
                let pairing_qr_shown = pairing_qr_shown.clone();
                let pairing_crypto_done = pairing_crypto_done.clone();
                let stop = stop.clone();
                let state = state.clone();
                async move {
                    handle_event(
                        event,
                        client,
                        inbound_tx,
                        &config,
                        &session_path,
                        pair_only,
                        pair_done,
                        pairing_qr_shown,
                        pairing_crypto_done,
                        stop,
                        state,
                    )
                    .await;
                }
            });

        builder
            .build()
            .await
            .map_err(|e| GatewayError::ConnectionFailed(format!("build bot: {e}")))
    }

    pub async fn start(
        &self,
        inbound_tx: mpsc::UnboundedSender<WaMessage>,
    ) -> Result<(), GatewayError> {
        let mut bot = self.build_bot(Some(inbound_tx), false).await?;
        let client = bot.client();
        {
            let mut st = self.state.write().await;
            st.client = Some(client);
        }
        let handle = bot
            .run()
            .await
            .map_err(|e| GatewayError::ConnectionFailed(format!("run bot: {e}")))?;
        *self.run_handle.lock().await = Some(handle);
        Ok(())
    }

    /// Run QR pairing until success or failure. Used by CLI wizard.
    pub async fn run_pairing(&self) -> Result<(), GatewayError> {
        self.pairing_qr_shown.store(false, Ordering::SeqCst);
        self.pairing_crypto_done.store(false, Ordering::SeqCst);
        let mut bot = self.build_bot(None, true).await?;
        println!("Connecting to WhatsApp for QR pairing...");
        let handle = bot
            .run()
            .await
            .map_err(|e| GatewayError::ConnectionFailed(format!("run pairing bot: {e}")))?;

        let result = tokio::select! {
            _ = self.pair_done.notified() => Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(300)) => {
                Err(GatewayError::ConnectionFailed("pairing timed out after 5 minutes".into()))
            }
        };

        // Let wa-rs finish post-pair reconnect + SQLite flush before closing the DB.
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Some(client) = self.state.read().await.client.clone() {
            client.disconnect().await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        handle.abort();
        result
    }

    pub async fn stop(&self) {
        self.stop.notify_one();
        if let Some(client) = self.state.read().await.client.clone() {
            client.disconnect().await;
        }
        if let Some(handle) = self.run_handle.lock().await.take() {
            handle.abort();
        }
        let mut st = self.state.write().await;
        st.client = None;
        st.connected = false;
    }

    pub async fn send_text(
        &self,
        chat_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<Option<String>, GatewayError> {
        let client = self.client().await?;
        let jid = parse_jid(chat_id)?;
        let mut message = text_message(text);
        if let Some(reply_id) = reply_to {
            message = reply_message(text, reply_id, chat_id);
        }
        let msg_id = client
            .send_message(jid, message)
            .await
            .map_err(|e| GatewayError::SendFailed(format!("send text: {e}")))?;
        self.track_sent_id(&msg_id).await;
        Ok(Some(msg_id))
    }

    pub async fn edit_message(
        &self,
        chat_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<(), GatewayError> {
        let client = self.client().await?;
        let jid = parse_jid(chat_id)?;
        client
            .edit_message(jid, message_id, text_message(text))
            .await
            .map_err(|e| GatewayError::SendFailed(format!("edit message: {e}")))?;
        Ok(())
    }

    pub async fn send_media_file(
        &self,
        chat_id: &str,
        file_path: &str,
        media_type: &str,
        caption: Option<&str>,
        file_name: Option<&str>,
    ) -> Result<(), GatewayError> {
        let client = self.client().await?;
        let jid = parse_jid(chat_id)?;
        let data = std::fs::read(file_path).map_err(|e| {
            GatewayError::SendFailed(format!("read media file {}: {e}", file_path))
        })?;
        let path = Path::new(file_path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mime = mime_for_ext(&ext, media_type);
        let wa_media = match media_type {
            "image" => MediaType::Image,
            "video" => MediaType::Video,
            "audio" => MediaType::Audio,
            _ => MediaType::Document,
        };
        let upload = client
            .upload(data, wa_media)
            .await
            .map_err(|e| GatewayError::SendFailed(format!("upload media: {e}")))?;
        let message = build_media_message(media_type, &mime, &upload, caption, file_name);
        let msg_id = client
            .send_message(jid, message)
            .await
            .map_err(|e| GatewayError::SendFailed(format!("send media: {e}")))?;
        self.track_sent_id(&msg_id).await;
        Ok(())
    }

    pub async fn send_typing(&self, chat_id: &str) -> Result<(), GatewayError> {
        let client = self.client().await?;
        let jid = parse_jid(chat_id)?;
        client
            .chatstate()
            .send_composing(&jid)
            .await
            .map_err(|e| GatewayError::SendFailed(format!("typing: {e}")))?;
        Ok(())
    }

    pub async fn stop_typing(&self, chat_id: &str) -> Result<(), GatewayError> {
        let client = self.client().await?;
        let jid = parse_jid(chat_id)?;
        client
            .chatstate()
            .send_paused(&jid)
            .await
            .map_err(|e| GatewayError::SendFailed(format!("stop typing: {e}")))?;
        Ok(())
    }

    async fn track_sent_id(&self, id: &str) {
        let mut st = self.state.write().await;
        st.recently_sent.insert(id.to_string());
        if st.recently_sent.len() > 256 {
            st.recently_sent.clear();
        }
    }
}

async fn handle_event(
    event: Event,
    client: Arc<wa_rs::Client>,
    inbound_tx: Option<mpsc::UnboundedSender<WaMessage>>,
    config: &WhatsAppConfig,
    session_path: &Path,
    pair_only: bool,
    pair_done: Arc<Notify>,
    pairing_qr_shown: Arc<AtomicBool>,
    pairing_crypto_done: Arc<AtomicBool>,
    _stop: Arc<Notify>,
    state: Arc<RwLock<ClientState>>,
) {
    match event {
        Event::PairingQrCode { code, timeout } => {
            pairing_qr_shown.store(true, Ordering::SeqCst);
            info!("WhatsApp QR code (valid ~{}s)", timeout.as_secs());
            super::qr_terminal::print_pairing_qr(&code, timeout);
        }
        Event::PairSuccess(success) => {
            info!("WhatsApp pair-success received");
            pairing_crypto_done.store(true, Ordering::SeqCst);
            if pair_only {
                println!(
                    "\nQR accepted — completing login on your phone (keep WhatsApp open)..."
                );
            }
            let pn = success.id.to_string();
            let lid = success.lid.to_string();
            let bot_ids = vec![pn.clone(), lid.clone()];
            {
                let mut st = state.write().await;
                st.bot_ids = bot_ids.clone();
                st.bot_pn = Some(jid_user_part(&pn));
                st.bot_lid = Some(jid_user_part(&lid));
                st.client = Some(client.clone());
            }
            // wa-rs expects a disconnect + reconnect after pair-success; finish on Connected.
            if pair_only {
                let pair_done = pair_done.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    pair_done.notify_one();
                });
            }
        }
        Event::Connected(_) => {
            info!("WhatsApp connected");
            {
                let mut st = state.write().await;
                st.connected = true;
                st.client = Some(client.clone());
            }
            if let Some(pn) = client.get_pn().await {
                let pn_str = pn.to_string();
                let mut st = state.write().await;
                if st.bot_pn.is_none() {
                    st.bot_pn = Some(jid_user_part(&pn_str));
                }
                if !st.bot_ids.iter().any(|id| id == &pn_str) {
                    st.bot_ids.push(pn_str);
                }
            }
            if let Some(lid) = client.get_lid().await {
                let lid_str = lid.to_string();
                let mut st = state.write().await;
                if st.bot_lid.is_none() {
                    st.bot_lid = Some(jid_user_part(&lid_str));
                }
                if !st.bot_ids.iter().any(|id| id == &lid_str) {
                    st.bot_ids.push(lid_str);
                }
            }
            if pair_only && pairing_crypto_done.load(Ordering::SeqCst) {
                if let Err(e) = mark_paired(session_path) {
                    warn!("[whatsapp] Failed to write paired marker: {e}");
                }
                println!("\nWhatsApp linked successfully!");
                pair_done.notify_one();
            } else if !pair_only {
                let mode = config.whatsapp_mode();
                let st = state.read().await;
                let pn = st.bot_pn.as_deref().unwrap_or("unknown");
                let lid = st.bot_lid.as_deref().unwrap_or("unknown");
                if mode == "self-chat" {
                    println!(
                        "WhatsApp connected (mode=self-chat). Send in “Message yourself” on this phone; bot PN={pn}, LID={lid}."
                    );
                } else {
                    println!("WhatsApp connected (mode={mode}). bot PN={pn}, LID={lid}.");
                }
            }
        }
        Event::Disconnected(_) => {
            warn!("[whatsapp] Disconnected from WhatsApp");
            state.write().await.connected = false;
            if pair_only
                && !pairing_qr_shown.load(Ordering::SeqCst)
                && !pairing_crypto_done.load(Ordering::SeqCst)
            {
                eprintln!(
                    "\nWhatsApp disconnected before showing a QR code. Check network and try again."
                );
                pair_done.notify_one();
            } else if !pair_only {
                eprintln!(
                    "WhatsApp disconnected — inbound messages will not be processed until reconnected."
                );
            }
        }
        Event::LoggedOut(_) => {
            warn!("[whatsapp] Logged out from WhatsApp");
            state.write().await.connected = false;
            if pair_only && !pairing_crypto_done.load(Ordering::SeqCst) {
                eprintln!("\nWhatsApp session was logged out. Retry QR pairing.");
                pair_done.notify_one();
            } else if !pair_only {
                eprintln!(
                    "WhatsApp logged out — run `hermes gateway setup` (WhatsApp) to re-pair."
                );
            }
        }
        Event::PairError(err) => {
            warn!("[whatsapp] Pairing error: {}", err.error);
            if pair_only {
                eprintln!("\nWhatsApp pairing error: {}", err.error);
            }
            pair_done.notify_one();
        }
        Event::Message(msg, info) => {
            if pair_only {
                return;
            }
            let Some(tx) = inbound_tx else {
                return;
            };
            let st = state.read().await;
            if !st.connected && !client.is_logged_in() {
                return;
            }
            let bot_ids = st.bot_ids.clone();
            let recently_sent = st.recently_sent.clone();
            let bot_pn = st.bot_pn.clone();
            let bot_lid = st.bot_lid.clone();
            drop(st);

            if !should_accept_incoming(
                config,
                &info,
                &msg,
                &bot_pn,
                &bot_lid,
                &bot_ids,
                session_path,
                &recently_sent,
            ) {
                let body = extract_body(&msg);
                let preview: String = body.chars().take(48).collect();
                println!(
                    "[whatsapp] Ignored inbound (chat={}, from_me={}, preview={preview:?})",
                    info.source.chat, info.source.is_from_me
                );
                debug!(
                    chat = %info.source.chat,
                    from_me = info.source.is_from_me,
                    mode = %config.whatsapp_mode(),
                    bot_pn = ?bot_pn,
                    bot_lid = ?bot_lid,
                    "[whatsapp] Ignoring inbound message (self-chat filter or echo)"
                );
                return;
            }

            let chat = info.source.chat.to_string();
            let body_preview: String = extract_body(&msg).chars().take(48).collect();
            match convert_incoming_message(&client, session_path, msg, info, bot_ids).await {
                Ok(Some(wa_msg)) => {
                    if tx.send(wa_msg).is_err() {
                        eprintln!(
                            "[whatsapp] Inbound queue closed (adapter stopped?); chat={chat}"
                        );
                    }
                }
                Ok(None) => {
                    println!(
                        "[whatsapp] Dropped after filter (empty/edit), chat={chat}, preview={body_preview:?}"
                    );
                }
                Err(e) => {
                    eprintln!("[whatsapp] Failed to convert incoming message: {e}");
                    warn!("[whatsapp] Failed to convert incoming message: {e}");
                }
            }
        }
        _ => {}
    }
}

/// True when the chat JID is the owner's self-chat (PN, LID, or alias mapping).
fn chat_matches_self(
    chat_id: &str,
    bot_pn: &Option<String>,
    bot_lid: &Option<String>,
    bot_ids: &[String],
    session_path: &Path,
) -> bool {
    let chat_user = jid_user_part(chat_id);
    if chat_user.is_empty() {
        return false;
    }
    if bot_pn.as_ref().is_some_and(|pn| chat_user == *pn) {
        return true;
    }
    if bot_lid.as_ref().is_some_and(|lid| chat_user == *lid) {
        return true;
    }
    for id in bot_ids {
        if jid_user_part(id) == chat_user {
            return true;
        }
    }
    let aliases = expand_whatsapp_aliases(&chat_user, Some(session_path));
    if bot_pn.as_ref().is_some_and(|pn| aliases.contains(pn)) {
        return true;
    }
    bot_lid.as_ref().is_some_and(|lid| aliases.contains(lid))
}

fn should_accept_incoming(
    config: &WhatsAppConfig,
    info: &MessageInfo,
    msg: &wa::Message,
    bot_pn: &Option<String>,
    bot_lid: &Option<String>,
    bot_ids: &[String],
    session_path: &Path,
    recently_sent: &HashSet<String>,
) -> bool {
    let chat_id = info.source.chat.to_string();
    if WhatsAppPolicy::is_broadcast_chat(&chat_id) {
        return false;
    }

    let is_group = info.source.is_group;
    let from_me = info.source.is_from_me;
    let body = extract_body(msg);

    if config.whatsapp_mode() == "self-chat" {
        if is_group {
            return false;
        }
        if !chat_matches_self(&chat_id, bot_pn, bot_lid, bot_ids, session_path) {
            return false;
        }
        // Bot echoes and linked-device sends we originated (from_me) — skip.
        if from_me {
            let prefix = config.effective_reply_prefix();
            if !prefix.is_empty() && body.starts_with(&prefix) {
                return false;
            }
            if recently_sent.contains(&info.id) {
                return false;
            }
        }
        // Phone → "Message yourself" often arrives as from_me=false on the Web session.
        return !body.trim().is_empty() || message_has_media(msg);
    }

    if from_me {
        if is_group {
            return false;
        }
        if config.whatsapp_mode() == "bot" {
            return false;
        }
    }

    !body.trim().is_empty() || message_has_media(msg)
}

async fn convert_incoming_message(
    client: &Arc<wa_rs::Client>,
    session_path: &Path,
    msg: Box<wa::Message>,
    info: MessageInfo,
    bot_ids: Vec<String>,
) -> Result<Option<WaMessage>, GatewayError> {
    if info.source.is_from_me && info.edit != Default::default() {
        return Ok(None);
    }

    let chat_id = info.source.chat.to_string();
    let sender_id = info.source.sender.to_string();
    let is_group = info.source.is_group;
    let mut body = extract_body(&msg);
    let (has_media, media_type, media_urls) =
        extract_media(client, session_path, &msg).await.unwrap_or_default();

    if has_media && body.trim().is_empty() {
        body = format!("[{media_type} received]");
    }
    if body.trim().is_empty() && !has_media {
        return Ok(None);
    }

    let (mentioned_ids, quoted_participant) = extract_context(&msg);

    Ok(Some(WaMessage {
        message_id: info.id.clone(),
        chat_id,
        sender_id,
        is_group,
        body,
        has_media,
        media_type,
        media_urls,
        mentioned_ids,
        quoted_participant,
        bot_ids,
    }))
}

fn extract_body(msg: &wa::Message) -> String {
    msg.text_content()
        .or_else(|| msg.get_caption())
        .unwrap_or("")
        .to_string()
}

fn message_has_media(msg: &wa::Message) -> bool {
    let base = msg.get_base_message();
    base.image_message.is_some()
        || base.video_message.is_some()
        || base.audio_message.is_some()
        || base.document_message.is_some()
        || base.sticker_message.is_some()
}

async fn extract_media(
    client: &Arc<wa_rs::Client>,
    session_path: &Path,
    msg: &wa::Message,
) -> Result<(bool, String, Vec<String>), GatewayError> {
    let base = msg.get_base_message();
    let cache_root = session_path.parent().unwrap_or(session_path);

    macro_rules! download {
        ($field:ident, $type_name:expr, $media:expr, $subdir:expr, $prefix:expr, $ext:expr) => {
            if let Some(media) = &base.$field {
                let mut buf = Cursor::new(Vec::new());
                client
                    .download_to_file(media.as_ref(), &mut buf)
                    .await
                    .map_err(|e| GatewayError::ConnectionFailed(format!("download media: {e}")))?;
                let dir = cache_root.join($subdir);
                std::fs::create_dir_all(&dir).map_err(|e| {
                    GatewayError::ConnectionFailed(format!("create media cache: {e}"))
                })?;
                let name = format!("{}_{}{}", $prefix, random_hex(), $ext);
                let path = dir.join(name);
                std::fs::write(&path, buf.into_inner()).map_err(|e| {
                    GatewayError::ConnectionFailed(format!("write media cache: {e}"))
                })?;
                return Ok((true, $type_name.to_string(), vec![path.to_string_lossy().into_owned()]));
            }
        };
    }

    download!(
        image_message,
        "image",
        MediaType::Image,
        "image_cache",
        "img",
        ".jpg"
    );
    download!(
        video_message,
        "video",
        MediaType::Video,
        "document_cache",
        "vid",
        ".mp4"
    );
    download!(
        audio_message,
        "audio",
        MediaType::Audio,
        "audio_cache",
        "aud",
        ".ogg"
    );
    if let Some(doc) = &base.document_message {
        let mut buf = Cursor::new(Vec::new());
        client
            .download_to_file(doc.as_ref(), &mut buf)
            .await
            .map_err(|e| GatewayError::ConnectionFailed(format!("download document: {e}")))?;
        let dir = cache_root.join("document_cache");
        std::fs::create_dir_all(&dir).map_err(|e| {
            GatewayError::ConnectionFailed(format!("create document cache: {e}"))
        })?;
        let safe_name = doc
            .file_name
            .as_deref()
            .unwrap_or("document")
            .chars()
            .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'))
            .collect::<String>();
        let path = dir.join(format!("doc_{}_{safe_name}", random_hex()));
        std::fs::write(&path, buf.into_inner()).map_err(|e| {
            GatewayError::ConnectionFailed(format!("write document cache: {e}"))
        })?;
        return Ok((
            true,
            "document".to_string(),
            vec![path.to_string_lossy().into_owned()],
        ));
    }

    Ok((false, String::new(), Vec::new()))
}

fn extract_context(msg: &wa::Message) -> (Vec<String>, Option<String>) {
    let base = msg.get_base_message();
    let ctx = base
        .extended_text_message
        .as_ref()
        .and_then(|m| m.context_info.as_deref())
        .or_else(|| {
            base.image_message
                .as_ref()
                .and_then(|m| m.context_info.as_deref())
        })
        .or_else(|| {
            base.video_message
                .as_ref()
                .and_then(|m| m.context_info.as_deref())
        })
        .or_else(|| {
            base.document_message
                .as_ref()
                .and_then(|m| m.context_info.as_deref())
        });

    let Some(ctx) = ctx else {
        return (Vec::new(), None);
    };
    let mentioned = ctx
        .mentioned_jid
        .iter()
        .filter(|j| !j.is_empty())
        .cloned()
        .collect();
    let quoted = ctx.participant.clone().filter(|p| !p.is_empty());
    (mentioned, quoted)
}

fn text_message(text: &str) -> wa::Message {
    if text.contains('\n') || text.contains('*') || text.contains('_') {
        wa::Message {
            extended_text_message: Some(Box::new(wa::message::ExtendedTextMessage {
                text: Some(text.to_string()),
                ..Default::default()
            })),
            ..Default::default()
        }
    } else {
        wa::Message {
            conversation: Some(text.to_string()),
            ..Default::default()
        }
    }
}

fn reply_message(text: &str, reply_id: &str, chat_id: &str) -> wa::Message {
    wa::Message {
        extended_text_message: Some(Box::new(wa::message::ExtendedTextMessage {
            text: Some(text.to_string()),
            context_info: Some(Box::new(wa::ContextInfo {
                stanza_id: Some(reply_id.to_string()),
                remote_jid: Some(chat_id.to_string()),
                ..Default::default()
            })),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn build_media_message(
    media_type: &str,
    mime: &str,
    upload: &wa_rs::upload::UploadResponse,
    caption: Option<&str>,
    file_name: Option<&str>,
) -> wa::Message {
    match media_type {
        "image" => wa::Message {
            image_message: Some(Box::new(wa::message::ImageMessage {
                mimetype: Some(mime.to_string()),
                caption: caption.map(str::to_string),
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key.clone()),
                file_enc_sha256: Some(upload.file_enc_sha256.clone()),
                file_sha256: Some(upload.file_sha256.clone()),
                file_length: Some(upload.file_length),
                ..Default::default()
            })),
            ..Default::default()
        },
        "video" => wa::Message {
            video_message: Some(Box::new(wa::message::VideoMessage {
                mimetype: Some(mime.to_string()),
                caption: caption.map(str::to_string),
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key.clone()),
                file_enc_sha256: Some(upload.file_enc_sha256.clone()),
                file_sha256: Some(upload.file_sha256.clone()),
                file_length: Some(upload.file_length),
                ..Default::default()
            })),
            ..Default::default()
        },
        "audio" => wa::Message {
            audio_message: Some(Box::new(wa::message::AudioMessage {
                mimetype: Some(mime.to_string()),
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key.clone()),
                file_enc_sha256: Some(upload.file_enc_sha256.clone()),
                file_sha256: Some(upload.file_sha256.clone()),
                file_length: Some(upload.file_length),
                ..Default::default()
            })),
            ..Default::default()
        },
        _ => wa::Message {
            document_message: Some(Box::new(wa::message::DocumentMessage {
                mimetype: Some(mime.to_string()),
                file_name: file_name.map(str::to_string),
                caption: caption.map(str::to_string),
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key.clone()),
                file_enc_sha256: Some(upload.file_enc_sha256.clone()),
                file_sha256: Some(upload.file_sha256.clone()),
                file_length: Some(upload.file_length),
                ..Default::default()
            })),
            ..Default::default()
        },
    }
}

fn parse_jid(chat_id: &str) -> Result<Jid, GatewayError> {
    Jid::from_str(chat_id)
        .map_err(|e| GatewayError::SendFailed(format!("invalid chat id {chat_id}: {e}")))
}

fn jid_user_part(jid: &str) -> String {
    jid.split('@').next().unwrap_or(jid).split(':').next().unwrap_or(jid).to_string()
}

fn random_hex() -> String {
    uuid::Uuid::new_v4().to_string().replace('-', "")
}

fn mime_for_ext(ext: &str, media_type: &str) -> String {
    match ext {
        "jpg" | "jpeg" => "image/jpeg".into(),
        "png" => "image/png".into(),
        "webp" => "image/webp".into(),
        "gif" => "image/gif".into(),
        "mp4" => "video/mp4".into(),
        "mov" => "video/quicktime".into(),
        "pdf" => "application/pdf".into(),
        "ogg" => "audio/ogg".into(),
        "mp3" => "audio/mpeg".into(),
        "m4a" => "audio/mp4".into(),
        _ => match media_type {
            "image" => "image/jpeg".into(),
            "video" => "video/mp4".into(),
            "audio" => "audio/ogg".into(),
            _ => "application/octet-stream".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jid_user_part_strips_server() {
        assert_eq!(jid_user_part("15551234567@s.whatsapp.net"), "15551234567");
        assert_eq!(jid_user_part("123:10@lid"), "123");
    }

    #[test]
    fn chat_matches_self_pn_or_lid() {
        let pn = Some("8619996253338".into());
        let lid = Some("248662248677608".into());
        let ids = vec![
            "8619996253338@s.whatsapp.net".into(),
            "248662248677608@lid".into(),
        ];
        let session = std::path::Path::new(".");
        assert!(chat_matches_self(
            "8619996253338@s.whatsapp.net",
            &pn,
            &lid,
            &ids,
            session
        ));
        assert!(chat_matches_self("248662248677608@lid", &pn, &lid, &ids, session));
        assert!(!chat_matches_self(
            "15551234567@s.whatsapp.net",
            &pn,
            &lid,
            &ids,
            session
        ));
    }

    #[test]
    fn text_message_uses_conversation_for_simple_text() {
        let msg = text_message("hello");
        assert_eq!(msg.conversation.as_deref(), Some("hello"));
    }
}
