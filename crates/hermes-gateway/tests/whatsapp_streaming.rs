#![cfg(feature = "whatsapp")]

//! WhatsApp streaming / edit_message tests (unit-level, no live WhatsApp).

use hermes_gateway::platforms::whatsapp::{outgoing_chunks, WhatsAppConfig};

#[test]
fn edit_chunks_long_progress_text() {
    let cfg = WhatsAppConfig::default();
    let text = "x".repeat(5000);
    let chunks = outgoing_chunks(&cfg, &text, true);
    assert!(chunks.len() > 1);
    assert!(chunks[0].len() <= cfg.outgoing_chunk_limit());
}
