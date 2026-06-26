/// Whether assistant `content` tokens may be streamed to TTS (full accumulated buffer).
pub fn assistant_content_tts_allowed(buf: &str, actionable_tool_deltas: bool) -> bool {
    if actionable_tool_deltas {
        return false;
    }
    let stripped = super::think_strip::strip_think_blocks(buf);
    hermes_core::speakable_tts_prefix_end(&stripped) == stripped.len()
}

/// Safe prefix of one gate-emitted speakable delta for incremental streaming TTS.
pub fn speakable_stream_delta(chunk: &str) -> &str {
    let end = hermes_core::speakable_tts_prefix_end(chunk);
    &chunk[..end]
}

/// Append one speakable stream delta to the TTS buffer when tool calls are not active.
pub fn append_speakable_stream_delta(
    tts_buf: &mut String,
    speakable: &str,
    actionable_tool_deltas: bool,
) -> bool {
    if actionable_tool_deltas {
        return false;
    }
    let safe = speakable_stream_delta(speakable);
    if safe.is_empty() {
        return false;
    }
    tts_buf.push_str(safe);
    true
}

pub fn has_actionable_tool_deltas(
    map: &std::collections::HashMap<u32, crate::llm::AccumulatedToolCall>,
) -> bool {
    map.values().any(|acc| !acc.name.trim().is_empty())
}

#[cfg(not(all(feature = "rockchip", not(feature = "sherpa-asr-tts"))))]
fn speakable_char_len(buf: &str) -> usize {
    let end = hermes_core::speakable_tts_prefix_end(buf);
    buf[..end].chars().count()
}

/// First speakable chunk for low-latency TTS (before full sentence).
pub fn take_early_chunk(buf: &mut String, min_chars: usize) -> Option<String> {
    #[cfg(all(feature = "rockchip", not(feature = "sherpa-asr-tts")))]
    {
        let count = buf.chars().count();
        if count < min_chars {
            return None;
        }
        let s: String = buf.chars().take(min_chars).collect();
        let rest: String = buf.chars().skip(min_chars).collect();
        if s.trim().is_empty() {
            return None;
        }
        *buf = rest;
        return Some(s);
    }
    #[cfg(not(all(feature = "rockchip", not(feature = "sherpa-asr-tts"))))]
    {
        let safe_chars = speakable_char_len(buf);
        if safe_chars < min_chars {
            return None;
        }
        let chunk: String = buf.chars().take(min_chars).collect();
        if chunk.trim().is_empty() {
            return None;
        }
        *buf = buf.chars().skip(min_chars).collect();
        Some(chunk)
    }
}

/// Extract a speakable sentence from the LLM buffer if ready.
pub fn take_sentence(buf: &mut String, min_len: usize) -> Option<String> {
    #[cfg(all(feature = "rockchip", not(feature = "sherpa-asr-tts")))]
    {
        return take_sentence_inner(buf, min_len);
    }
    #[cfg(not(all(feature = "rockchip", not(feature = "sherpa-asr-tts"))))]
    {
        let safe_chars = speakable_char_len(buf);
        if safe_chars == 0 {
            return None;
        }
        let mut safe: String = buf.chars().take(safe_chars).collect();
        let sentence = take_sentence_inner(&mut safe, min_len)?;
        let spoken = safe_chars - safe.chars().count();
        *buf = buf.chars().skip(spoken).collect();
        Some(sentence)
    }
}

pub fn flush_remainder(buf: &mut String) -> Option<String> {
    #[cfg(all(feature = "rockchip", not(feature = "sherpa-asr-tts")))]
    {
        return flush_remainder_inner(buf);
    }
    #[cfg(not(all(feature = "rockchip", not(feature = "sherpa-asr-tts"))))]
    {
        let safe_chars = speakable_char_len(buf);
        if safe_chars == 0 {
            return None;
        }
        let mut safe: String = buf.chars().take(safe_chars).collect();
        let out = flush_remainder_inner(&mut safe)?;
        let spoken = safe_chars - safe.chars().count();
        *buf = buf.chars().skip(spoken).collect();
        Some(out)
    }
}

fn take_sentence_inner(buf: &mut String, min_len: usize) -> Option<String> {
    let delimiters = ['。', '！', '？', '\n', '.', '!', '?'];
    let split_at = buf
        .char_indices()
        .find_map(|(i, ch)| delimiters.contains(&ch).then_some(i + ch.len_utf8()));
    if let Some(end) = split_at {
        let sentence: String = buf.drain(..end).collect();
        let trimmed = sentence.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    if buf.chars().count() >= min_len {
        let s = buf.trim().to_string();
        if !s.is_empty() {
            buf.clear();
            return Some(s);
        }
    }
    None
}

fn flush_remainder_inner(buf: &mut String) -> Option<String> {
    let s = buf.trim().to_string();
    buf.clear();
    if s.is_empty() { None } else { Some(s) }
}

/// TTS preprocessing before synthesis.
pub fn normalize_tts_text(text: &str) -> String {
    #[cfg(all(feature = "rockchip", not(feature = "sherpa-asr-tts")))]
    {
        let text = super::normalizer::normalize_chinese_numbers(text);
        return super::normalizer::normalize_quotes(&text);
    }
    #[cfg(not(all(feature = "rockchip", not(feature = "sherpa-asr-tts"))))]
    {
        super::normalizer::preprocess_tts_text(text)
    }
}

/// Whether ASR final is compatible with an earlier speculative partial.
pub fn texts_compatible(partial: &str, final_text: &str) -> bool {
    fn norm(s: &str) -> String {
        s.chars()
            .filter(|c| {
                !c.is_whitespace() && !['，', '。', '？', '！', '.', ',', '?', '!'].contains(c)
            })
            .collect()
    }
    let a = norm(partial);
    let b = norm(final_text);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || a.starts_with(&b) || b.starts_with(&a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(all(feature = "rockchip", not(feature = "sherpa-asr-tts"))))]
    fn take_sentence_withholds_powershell_markup() {
        let mut buf = "现在是下午三点。<powershell>powershell".to_string();
        let s = take_sentence(&mut buf, 4).expect("sentence");
        assert_eq!(s, "现在是下午三点。");
        assert!(buf.contains("powershell"));
    }

    #[test]
    #[cfg(not(all(feature = "rockchip", not(feature = "sherpa-asr-tts"))))]
    fn take_early_chunk_withholds_tool_markup() {
        let mut buf = "好的，我来查一下。<seed:tool_call>".to_string();
        assert!(take_early_chunk(&mut buf, 3).is_some());
        assert!(buf.contains("<seed:tool_call>"));
    }

    #[test]
    #[cfg(not(all(feature = "rockchip", not(feature = "sherpa-asr-tts"))))]
    fn take_sentence_withholds_tool_markup() {
        let mut buf = "现在是下午三点。<function=execute_command>".to_string();
        let s = take_sentence(&mut buf, 4).expect("sentence");
        assert_eq!(s, "现在是下午三点。");
        assert!(buf.starts_with("<function="));
    }

    #[test]
    #[cfg(not(all(feature = "rockchip", not(feature = "sherpa-asr-tts"))))]
    fn flush_remainder_skips_tool_markup() {
        let mut buf = "完成。<seed:tool_call></seed:tool_call>".to_string();
        assert_eq!(flush_remainder(&mut buf).as_deref(), Some("完成。"));
        assert!(buf.contains("seed:tool_call"));
    }

    #[test]
    fn assistant_content_tts_allowed_with_thinking_block_in_buf() {
        use crate::orchestrator::think_strip::{
            REDACTED_THINKING_CLOSE_TAG, REDACTED_THINKING_OPEN_TAG,
        };
        let input = format!(
            "{OPEN}plan{CLOSE}tail",
            OPEN = REDACTED_THINKING_OPEN_TAG,
            CLOSE = REDACTED_THINKING_CLOSE_TAG,
        );
        assert!(assistant_content_tts_allowed(&input, false));
    }

    #[test]
    fn speakable_stream_delta_withholds_tool_suffix() {
        let chunk = "你好呀。<seed:tool_call>";
        assert_eq!(speakable_stream_delta(chunk), "你好呀。");
    }

    #[test]
    fn append_speakable_stream_delta_respects_actionable_tools() {
        let mut buf = String::new();
        assert!(!append_speakable_stream_delta(&mut buf, "你好", true));
        assert!(append_speakable_stream_delta(&mut buf, "你好", false));
        assert_eq!(buf, "你好");
    }

    #[test]
    fn assistant_content_tts_blocked_when_tool_deltas_actionable() {
        assert!(!assistant_content_tts_allowed("好的，我来查。", true));
        assert!(assistant_content_tts_allowed("好的，我来查。", false));
    }

    #[test]
    fn has_actionable_tool_deltas_requires_non_empty_name() {
        use crate::llm::AccumulatedToolCall;
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert(
            0,
            AccumulatedToolCall {
                index: 0,
                id: String::new(),
                name: String::new(),
                arguments: "{".to_string(),
            },
        );
        assert!(!has_actionable_tool_deltas(&map));

        map.get_mut(&0).unwrap().name = "execute".to_string();
        assert!(has_actionable_tool_deltas(&map));
    }
}
