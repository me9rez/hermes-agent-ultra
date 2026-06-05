#![cfg(feature = "whatsapp")]

//! WhatsApp message formatting tests.

use hermes_gateway::platforms::whatsapp::{
    format_message, outgoing_chunks, WhatsAppConfig, DEFAULT_REPLY_PREFIX, MAX_MESSAGE_LENGTH,
};

#[test]
fn bold_double_asterisk() {
    assert_eq!(format_message("**hi**"), "*hi*");
}

#[test]
fn code_fence_preserved() {
    let input = "before\n```rust\nlet x = 1;\n```\nafter";
    let out = format_message(input);
    assert!(out.contains("```rust"));
    assert!(out.contains("let x = 1;"));
}

#[test]
fn multi_chunk() {
    let cfg = WhatsAppConfig::default();
    let long = "x".repeat(MAX_MESSAGE_LENGTH + 100);
    let chunks = outgoing_chunks(&cfg, &long, true);
    assert!(chunks.len() > 1);
}

#[test]
fn effective_prefix_empty_disables() {
    let mut cfg = WhatsAppConfig::default();
    cfg.reply_prefix = Some(String::new());
    assert!(cfg.effective_reply_prefix().is_empty());
}

#[test]
fn default_prefix_when_unset() {
    let cfg = WhatsAppConfig::default();
    assert!(cfg.effective_reply_prefix().contains("Hermes"));
    assert!(DEFAULT_REPLY_PREFIX.contains("Hermes"));
}
