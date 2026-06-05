//! Gateway-backed cron delivery (`CronDeliveryBackend`).

use std::sync::Arc;

use async_trait::async_trait;
use hermes_cron::CronDeliveryBackend;
use hermes_gateway::gateway::Gateway;

/// Forwards cron deliveries to [`Gateway::send_message`].
pub struct GatewayCronDeliveryBackend {
    gateway: Arc<Gateway>,
}

impl GatewayCronDeliveryBackend {
    pub fn new(gateway: Arc<Gateway>) -> Self {
        Self { gateway }
    }
}

#[async_trait]
impl CronDeliveryBackend for GatewayCronDeliveryBackend {
    async fn send(&self, platform: &str, chat_id: &str, message: &str) -> Result<(), String> {
        let result = self
            .gateway
            .send_message(platform, chat_id, message, None)
            .await
            .map_err(|e| format!("gateway send failed: {e}"));
        if platform.eq_ignore_ascii_case("whatsapp") {
            match &result {
                Ok(()) => println!(
                    "[whatsapp] Cron reminder delivered (chat={chat_id}, chars={})",
                    message.chars().count()
                ),
                Err(e) => eprintln!("[whatsapp] Cron reminder failed (chat={chat_id}): {e}"),
            }
        }
        result
    }
}
