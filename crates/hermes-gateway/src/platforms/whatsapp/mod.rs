//! WhatsApp adapter (native Rust client via wa-rs).
//!
//! Replaces the former Node.js Baileys bridge with an in-process wa-rs client.

mod config;
mod format;
mod policy;
mod qr_terminal;
mod rust_client;
mod session_store;
mod text_batch;

pub use config::{DEFAULT_REPLY_PREFIX, MAX_MESSAGE_LENGTH, WhatsAppConfig};
pub use format::{format_message, outgoing_chunks};
pub use policy::WhatsAppPolicy;
pub use rust_client::{WaMessage, WhatsAppRustClient};
pub use session_store::{
    clear_pairing_session, ensure_session_dir, has_legacy_baileys_session, is_paired,
    legacy_creds_path, mark_paired, session_db_path,
};
pub use text_batch::{batch_key, TextBatchState};

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::{mpsc, Mutex, Notify, RwLock};
use tracing::{info, warn};

use hermes_core::errors::GatewayError;
use hermes_core::traits::{ParseMode, PlatformAdapter};

use crate::adapter::BasePlatformAdapter;
use crate::gateway::IncomingMessage;
use crate::whatsapp_identity::canonical_whatsapp_identifier;

struct WhatsAppInner {
    config: WhatsAppConfig,
    policy: WhatsAppPolicy,
    base: BasePlatformAdapter,
    client: WhatsAppRustClient,
    inbound_tx: RwLock<Option<mpsc::Sender<IncomingMessage>>>,
    inbound_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    shutting_down: Mutex<bool>,
    fatal_code: Mutex<Option<String>>,
    fatal_message: Mutex<Option<String>>,
    text_batch: TextBatchState,
    stop: Notify,
}

pub struct WhatsAppAdapter {
    inner: Arc<WhatsAppInner>,
}

impl WhatsAppAdapter {
    pub fn new(config: WhatsAppConfig) -> Result<Self, GatewayError> {
        let base = BasePlatformAdapter::new("whatsapp").with_proxy(config.proxy.clone());
        let policy = WhatsAppPolicy::from_config(&config);
        let text_batch = TextBatchState::new(&config);
        let client = WhatsAppRustClient::new(config.clone());
        Ok(Self {
            inner: Arc::new(WhatsAppInner {
                config,
                policy,
                base,
                client,
                inbound_tx: RwLock::new(None),
                inbound_task: Mutex::new(None),
                shutting_down: Mutex::new(false),
                fatal_code: Mutex::new(None),
                fatal_message: Mutex::new(None),
                text_batch,
                stop: Notify::new(),
            }),
        })
    }

    pub fn config(&self) -> &WhatsAppConfig {
        &self.inner.config
    }

    pub fn rust_client(&self) -> &WhatsAppRustClient {
        &self.inner.client
    }

    pub async fn set_inbound_sender(&self, tx: mpsc::Sender<IncomingMessage>) {
        *self.inner.inbound_tx.write().await = Some(tx);
    }

    pub fn enforces_own_access_policy(&self) -> bool {
        self.inner.policy.enforces_own_access_policy()
    }

    async fn set_fatal(&self, code: &str, message: impl Into<String>) {
        *self.inner.fatal_code.lock().await = Some(code.to_string());
        *self.inner.fatal_message.lock().await = Some(message.into());
    }

    async fn connect(&self) -> Result<(), GatewayError> {
        if self.inner.client.is_connected().await
            && self.inner.inbound_task.lock().await.is_some()
        {
            return Ok(());
        }
        if let Some(task) = self.inner.inbound_task.lock().await.take() {
            task.abort();
        }
        self.inner.client.stop().await;

        let session_path = self.inner.config.session_path();
        if has_legacy_baileys_session(&session_path) && !is_paired(&session_path) {
            self.set_fatal(
                "whatsapp_legacy_session",
                "Legacy Baileys session detected — run `hermes whatsapp` to re-pair with the Rust client.",
            )
            .await;
            return Err(GatewayError::ConnectionFailed(
                "Legacy Baileys creds.json found; re-pair required".into(),
            ));
        }
        if !is_paired(&session_path) {
            self.set_fatal(
                "whatsapp_not_paired",
                "WhatsApp enabled but not paired — run `hermes whatsapp` to pair.",
            )
            .await;
            return Err(GatewayError::ConnectionFailed(
                "WhatsApp not paired".into(),
            ));
        }

        let (wa_tx, mut wa_rx) = mpsc::unbounded_channel();
        self.inner.client.start(wa_tx).await?;

        let inner = self.inner.clone();
        let handle = tokio::spawn(async move {
            while let Some(msg) = wa_rx.recv().await {
                if let Some(incoming) = build_incoming(&inner, &msg).await {
                    let preview: String = incoming.text.chars().take(48).collect();
                    println!(
                        "[whatsapp] Queued for agent (chat={}, preview={preview:?}, ~{:.0}s batch)",
                        incoming.chat_id, inner.config.text_batch_delay_seconds
                    );
                    dispatch_incoming(&inner, incoming).await;
                } else {
                    let preview: String = msg.body.chars().take(48).collect();
                    println!(
                        "[whatsapp] Dropped after accept (policy/filter): chat={}, preview={preview:?}",
                        msg.chat_id
                    );
                }
            }
        });
        *self.inner.inbound_task.lock().await = Some(handle);
        Ok(())
    }
}

async fn build_incoming(inner: &WhatsAppInner, msg: &WaMessage) -> Option<IncomingMessage> {
    let data = json!({
        "chatId": msg.chat_id,
        "senderId": msg.sender_id,
        "body": msg.body,
        "isGroup": msg.is_group,
        "hasMedia": msg.has_media,
        "mediaType": msg.media_type,
        "mediaUrls": msg.media_urls,
        "mentionedIds": msg.mentioned_ids,
        "quotedParticipant": msg.quoted_participant,
        "botIds": msg.bot_ids,
    });
    if !inner.policy.should_process_message(&data) {
        return None;
    }

    let session_root = inner.config.session_path();
    let user_id = if msg.is_group {
        canonical_whatsapp_identifier(&msg.sender_id, Some(&session_root))
    } else {
        canonical_whatsapp_identifier(&msg.chat_id, Some(&session_root))
    };
    let mut text = msg.body.clone();
    if msg.is_group {
        text = inner.policy.clean_bot_mention_text(&text, &data);
    }

    if msg.has_media && text.trim().is_empty() {
        text = format!("[{} received]", msg.media_type);
    }

    let mut incoming = IncomingMessage::new(
        "whatsapp",
        msg.chat_id.clone(),
        if user_id.is_empty() {
            msg.sender_id.clone()
        } else {
            user_id
        },
        text,
        !msg.is_group,
    );
    incoming.message_id = Some(msg.message_id.clone());
    incoming.media_urls = msg.media_urls.clone();
    if msg.has_media {
        incoming.media_types = vec![msg.media_type.clone()];
    }
    Some(incoming)
}

async fn dispatch_incoming(inner: &Arc<WhatsAppInner>, incoming: IncomingMessage) {
    let tx = inner.inbound_tx.read().await.clone();
    let Some(tx) = tx else {
        let preview: String = incoming.text.chars().take(48).collect();
        eprintln!(
            "[whatsapp] Cannot hand off (gateway inbound channel not wired yet), preview={preview:?}"
        );
        return;
    };
    let key = batch_key(&incoming);
    let tx_clone = tx.clone();
    inner
        .text_batch
        .enqueue(key, incoming, move |merged| {
            let tx = tx_clone.clone();
            async move {
                let preview: String = merged.text.chars().take(48).collect();
                if let Err(e) = tx.send(merged).await {
                    eprintln!(
                        "[whatsapp] Failed to hand off to gateway router (preview={preview:?}): {e}"
                    );
                } else {
                    println!(
                        "[whatsapp] Handed off to gateway router (preview={preview:?})"
                    );
                }
            }
        })
        .await;
}

impl WhatsAppAdapter {
    async fn send_whatsapp_text(
        &self,
        chat_id: &str,
        text: &str,
        _parse_mode: Option<ParseMode>,
        reply_to_message_id: Option<&str>,
        include_reply_prefix: bool,
    ) -> Result<Option<String>, GatewayError> {
        if text.trim().is_empty() {
            eprintln!("[whatsapp] Skipping send: reply text is empty (chat={chat_id})");
            return Ok(None);
        }
        let outbound_chat = self.inner.client.resolve_outbound_chat_id(chat_id).await;
        if self.inner.config.whatsapp_mode() == "self-chat" {
            // Cron `deliver=origin` / WHATSAPP_HOME_CHANNEL fallback for gateway jobs.
            // SAFETY: single-threaded gateway process; updates home channel after each outbound send.
            unsafe {
                std::env::set_var("WHATSAPP_HOME_CHANNEL", &outbound_chat);
            }
        }
        if outbound_chat != chat_id {
            println!(
                "[whatsapp] Outbound chat resolved {chat_id} → {outbound_chat}"
            );
        }
        let reply_to = if self.inner.config.whatsapp_mode() == "self-chat" {
            None
        } else {
            reply_to_message_id
        };
        let chunks = outgoing_chunks(&self.inner.config, text, include_reply_prefix);
        let mut last_id = None;
        for (idx, chunk) in chunks.iter().enumerate() {
            let reply = if idx == 0 { reply_to } else { None };
            let result = self
                .inner
                .client
                .send_text(&outbound_chat, chunk, reply)
                .await
                .map_err(|e| {
                    eprintln!("[whatsapp] Send failed (chat={outbound_chat}): {e}");
                    e
                })?;
            last_id = result;
            if chunks.len() > 1 && idx + 1 < chunks.len() {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
        }
        println!(
            "[whatsapp] Sent reply (chat={outbound_chat}, chars={}, msg_id={last_id:?})",
            text.chars().count()
        );
        Ok(last_id)
    }
}

#[async_trait]
impl PlatformAdapter for WhatsAppAdapter {
    async fn start(&self) -> Result<(), GatewayError> {
        info!("WhatsApp Rust adapter starting");
        let mode = self.inner.config.whatsapp_mode();
        println!(
            "WhatsApp: connecting (mode={mode}, session={})…",
            self.inner.config.session_path().display()
        );
        self.connect().await?;
        self.inner.base.mark_running();
        Ok(())
    }

    async fn stop(&self) -> Result<(), GatewayError> {
        info!("WhatsApp adapter stopping");
        *self.inner.shutting_down.lock().await = true;
        self.inner.stop.notify_one();
        if let Some(task) = self.inner.inbound_task.lock().await.take() {
            task.abort();
        }
        self.inner.client.stop().await;
        self.inner.base.mark_stopped();
        Ok(())
    }

    async fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        _parse_mode: Option<ParseMode>,
    ) -> Result<(), GatewayError> {
        let _ = self
            .send_message_with_id(chat_id, text, _parse_mode)
            .await?;
        Ok(())
    }

    async fn send_message_with_id(
        &self,
        chat_id: &str,
        text: &str,
        parse_mode: Option<ParseMode>,
    ) -> Result<Option<String>, GatewayError> {
        self.send_whatsapp_text(chat_id, text, parse_mode, None, false)
            .await
    }

    async fn send_message_in_thread(
        &self,
        chat_id: &str,
        text: &str,
        parse_mode: Option<ParseMode>,
        reply_to_message_id: Option<&str>,
        message_thread_id: Option<&str>,
    ) -> Result<Option<String>, GatewayError> {
        let _ = message_thread_id;
        self.send_whatsapp_text(chat_id, text, parse_mode, reply_to_message_id, true)
            .await
    }

    async fn send_message_replying(
        &self,
        chat_id: &str,
        text: &str,
        parse_mode: Option<ParseMode>,
        reply_to_message_id: Option<&str>,
    ) -> Result<Option<String>, GatewayError> {
        self.send_whatsapp_text(chat_id, text, parse_mode, reply_to_message_id, false)
            .await
    }

    async fn edit_message(
        &self,
        chat_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<(), GatewayError> {
        let chunks = outgoing_chunks(&self.inner.config, text, false);
        self.inner
            .client
            .edit_message(chat_id, message_id, &chunks[0])
            .await?;
        if chunks.len() > 1 {
            for chunk in chunks.iter().skip(1) {
                self.inner.client.send_text(chat_id, chunk, None).await?;
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
        }
        Ok(())
    }

    async fn send_file(
        &self,
        chat_id: &str,
        file_path: &str,
        caption: Option<&str>,
    ) -> Result<(), GatewayError> {
        use crate::platforms::helpers::media_category;

        let path = std::path::Path::new(file_path);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let media_type = match media_category(ext) {
            "image" => "image",
            "video" => "video",
            "audio" => "audio",
            _ => "document",
        };
        let file_name = path.file_name().and_then(|n| n.to_str());
        self.inner
            .client
            .send_media_file(chat_id, file_path, media_type, caption, file_name)
            .await
    }

    async fn send_image_url(
        &self,
        chat_id: &str,
        image_url: &str,
        caption: Option<&str>,
    ) -> Result<(), GatewayError> {
        let cache = crate::media::MediaCache::with_defaults()?;
        let local = cache.cache_image(image_url, "img.jpg").await?;
        self.send_file(chat_id, local.to_string_lossy().as_ref(), caption)
            .await
    }

    async fn trigger_typing(&self, chat_id: &str) -> Result<(), GatewayError> {
        let chat_id = self.inner.client.resolve_outbound_chat_id(chat_id).await;
        if let Err(e) = self.inner.client.send_typing(&chat_id).await {
            warn!("[whatsapp] typing failed: {e}");
        }
        Ok(())
    }

    async fn stop_typing(&self, chat_id: &str) -> Result<(), GatewayError> {
        let chat_id = self.inner.client.resolve_outbound_chat_id(chat_id).await;
        if let Err(e) = self.inner.client.stop_typing(&chat_id).await {
            warn!("[whatsapp] stop typing failed: {e}");
        }
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.inner.base.is_running()
    }

    fn platform_name(&self) -> &str {
        "whatsapp"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_message_length_is_4096() {
        assert_eq!(config::MAX_MESSAGE_LENGTH, 4096);
    }
}
