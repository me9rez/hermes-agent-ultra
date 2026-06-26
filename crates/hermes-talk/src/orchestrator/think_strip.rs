//! Strip `<think...>...</redacted_thinking>` blocks from streaming LLM text before TTS.
//!
//! Ported from `hermes-tools::tts_streaming::sanitizer::IncrementalThinkStripper` so talk
//! does not depend on the full tools crate.

use std::sync::OnceLock;

use regex::Regex;

pub(crate) const REDACTED_THINKING_OPEN_TAG: &str = concat!("<", "redacted_", "thinking>");
pub(crate) const REDACTED_THINKING_CLOSE_TAG: &str = concat!("</", "redacted_", "thinking>");
const REDACTED_THINKING_OPEN_PREFIX: &str = concat!("<", "redacted_", "thinking");
pub(crate) const THINK_CLOSE_TAG: &str = concat!("</", "think>");
const THINK_OPEN_PREFIX: &str = concat!("<", "think");

fn closed_block_re(open: &str, close: &str) -> Regex {
    Regex::new(&format!(
        r"(?is){}.*?{}",
        regex::escape(open),
        regex::escape(close),
    ))
    .unwrap()
}

fn closed_block_capture_re(open: &str, close: &str) -> Regex {
    Regex::new(&format!(
        r"(?is){}(.*?){}",
        regex::escape(open),
        regex::escape(close),
    ))
    .unwrap()
}

const CLOSE_TAGS: &[&str] = &[
    REDACTED_THINKING_CLOSE_TAG,
    "</thinking>",
    THINK_CLOSE_TAG,
    "</thought>",
    "</reasoning>",
    "</REASONING_SCRATCHPAD>",
];

fn closed_redacted_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(&format!(
            r"(?is){}\b[^>]*>.*?{}",
            regex::escape(REDACTED_THINKING_OPEN_PREFIX),
            regex::escape(REDACTED_THINKING_CLOSE_TAG),
        ))
        .unwrap()
    })
}

fn closed_think_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(&format!(
            r"(?is){}\b[^>]*>.*?{}",
            regex::escape(THINK_OPEN_PREFIX),
            regex::escape(THINK_CLOSE_TAG),
        ))
        .unwrap()
    })
}

fn closed_thinking_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<thinking>.*?</thinking>").unwrap())
}

fn closed_thought_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<thought>.*?</thought>").unwrap())
}

fn closed_reason_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<reasoning>.*?</reasoning>").unwrap())
}

fn closed_scratch_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<REASONING_SCRATCHPAD>.*?</REASONING_SCRATCHPAD>").unwrap())
}

fn unterminated_think_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?is)(?:^|\n)[ \t]*<(?:redacted_thinking|think|thinking|reasoning|thought|REASONING_SCRATCHPAD)\b[^>]*>.*$",
        )
        .unwrap()
    })
}

fn orphan_think_tags_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)</?(?:redacted_thinking|think|thinking|reasoning|thought|REASONING_SCRATCHPAD)>\s*",
        )
        .unwrap()
    })
}

fn extract_patterns() -> &'static [Regex; 5] {
    static PATTERNS: OnceLock<[Regex; 5]> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            closed_block_capture_re(REDACTED_THINKING_OPEN_TAG, REDACTED_THINKING_CLOSE_TAG),
            Regex::new(r"(?is)<thinking>(.*?)</thinking>").unwrap(),
            Regex::new(r"(?is)<thought>(.*?)</thought>").unwrap(),
            Regex::new(r"(?is)<reasoning>(.*?)</reasoning>").unwrap(),
            Regex::new(r"(?s)<REASONING_SCRATCHPAD>(.*?)</REASONING_SCRATCHPAD>").unwrap(),
        ]
    })
}

/// Remove thinking / reasoning XML blocks from a complete assistant string.
pub fn strip_think_blocks(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let mut c = content.to_string();
    c = closed_redacted_re().replace_all(&c, "").to_string();
    c = closed_think_re().replace_all(&c, "").to_string();
    c = closed_thinking_re().replace_all(&c, "").to_string();
    c = closed_reason_re().replace_all(&c, "").to_string();
    c = closed_scratch_re().replace_all(&c, "").to_string();
    c = closed_thought_re().replace_all(&c, "").to_string();
    if !stream_has_think_close_tag(&c) {
        c = unterminated_think_re().replace_all(&c, "").to_string();
    }
    c = orphan_think_tags_re().replace_all(&c, "").to_string();
    c.trim().to_string()
}

/// Speakable tail after the first recognized thinking close tag, else strip fallback.
pub fn speakable_after_think_close(content: &str) -> String {
    if let Some((pos, tag)) = find_close_tag(content) {
        let after = content[pos + tag.len()..].trim();
        if !after.is_empty() {
            return after.to_string();
        }
    }
    strip_think_blocks(content)
}

/// Collect inner text from closed thinking blocks (for reasoning logs when the model
/// embeds CoT in `content` instead of `reasoning_content`).
pub fn extract_inline_thinking(content: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for re in extract_patterns() {
        for cap in re.captures_iter(content) {
            if let Some(m) = cap.get(1) {
                let s = m.as_str().trim();
                if !s.is_empty() {
                    parts.push(s.to_string());
                }
            }
        }
    }
    // Unterminated opening tag at end of stream (common with local rkllm).
    if let Some(caps) = Regex::new(
        r"(?is)<(?:redacted_thinking|think|thinking|reasoning|thought|REASONING_SCRATCHPAD)\b[^>]*>(.*)$",
    )
    .unwrap()
    .captures(content)
    {
        if let Some(m) = caps.get(1) {
            let s = m.as_str().trim();
            if !s.is_empty() && !parts.iter().any(|p| p.contains(s) || s.contains(p)) {
                parts.push(s.to_string());
            }
        }
    }
    parts.join("\n")
}

const OPEN_PREFIXES: &[&str] = &[
    REDACTED_THINKING_OPEN_PREFIX,
    "<REASONING_SCRATCHPAD",
    "<thinking",
    "<reasoning",
    "<thought",
    THINK_OPEN_PREFIX,
];

fn find_close_tag(buf: &str) -> Option<(usize, &'static str)> {
    let mut best: Option<(usize, &'static str)> = None;
    for tag in CLOSE_TAGS {
        let mut start = 0;
        while let Some(rel) = buf[start..].find(tag) {
            let pos = start + rel;
            let dominated = CLOSE_TAGS
                .iter()
                .any(|other| other.len() > tag.len() && buf[pos..].starts_with(other));
            if !dominated {
                match best {
                    None => best = Some((pos, *tag)),
                    Some((best_pos, best_tag)) => {
                        if pos < best_pos || (pos == best_pos && tag.len() > best_tag.len()) {
                            best = Some((pos, *tag));
                        }
                    }
                }
            }
            start = pos + 1;
        }
    }
    best
}

fn find_think_open(buf: &str) -> Option<usize> {
    OPEN_PREFIXES
        .iter()
        .filter_map(|prefix| buf.find(prefix))
        .min()
}

fn hold_back_prefix_boundary(buf: &str, prefix: &str) -> usize {
    let max = prefix.len().saturating_sub(1);
    for k in (1..=max).rev() {
        if buf.len() < k {
            continue;
        }
        let start = buf.len() - k;
        if !buf.is_char_boundary(start) {
            continue;
        }
        if prefix.starts_with(&buf[start..]) {
            return start;
        }
    }
    buf.len()
}

fn max_partial_tag_hold() -> usize {
    OPEN_PREFIXES
        .iter()
        .map(|p| p.len())
        .chain(CLOSE_TAGS.iter().map(|t| t.len()))
        .max()
        .unwrap_or(0)
        .saturating_sub(1)
}

/// Stateful filter that removes model thinking blocks from a streaming text source.
#[derive(Debug, Default)]
pub struct IncrementalThinkStripper {
    pending: String,
    inside: bool,
    inside_buf: String,
}

impl IncrementalThinkStripper {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume the next delta and return text safe to append to a TTS buffer.
    pub fn push(&mut self, delta: &str) -> String {
        if self.inside {
            self.inside_buf.push_str(delta);
            self.drain_inside()
        } else {
            let combined = std::mem::take(&mut self.pending) + delta;
            self.drain_outside(combined)
        }
    }

    /// Mark end-of-stream; drop any partial opening tag or unclosed think block.
    pub fn flush(&mut self) -> String {
        self.inside_buf.clear();
        self.inside = false;
        let leftover = std::mem::take(&mut self.pending);
        if leftover.starts_with('<') {
            String::new()
        } else {
            leftover
        }
    }

    #[cfg(test)]
    pub fn is_inside(&self) -> bool {
        self.inside
    }

    fn drain_outside(&mut self, mut buf: String) -> String {
        let mut out = String::new();
        loop {
            match find_think_open(&buf) {
                Some(pos) => {
                    out.push_str(&buf[..pos]);
                    let rest = &buf[pos..];
                    if let Some(gt) = rest.find('>') {
                        self.inside = true;
                        self.inside_buf = rest[gt + 1..].to_string();
                        let drained = self.drain_inside();
                        out.push_str(&drained);
                        if !self.inside {
                            buf = std::mem::take(&mut self.pending);
                            continue;
                        }
                        break;
                    } else {
                        self.pending = rest.to_string();
                        break;
                    }
                }
                None => {
                    let safe_emit_end = tail_safe_emit_boundary(&buf);
                    out.push_str(&buf[..safe_emit_end]);
                    self.pending = buf[safe_emit_end..].to_string();
                    break;
                }
            }
        }
        out
    }

    fn drain_inside(&mut self) -> String {
        if let Some((pos, tag)) = find_close_tag(&self.inside_buf) {
            let after = self.inside_buf[pos + tag.len()..].to_string();
            self.inside_buf.clear();
            self.inside = false;
            self.pending = after;
            let buf = std::mem::take(&mut self.pending);
            return self.drain_outside(buf);
        }
        let trailing = max_partial_tag_hold();
        if self.inside_buf.len() > trailing {
            let cut = self.inside_buf.len() - trailing;
            let safe_cut = (0..=cut)
                .rev()
                .find(|&i| self.inside_buf.is_char_boundary(i))
                .unwrap_or(0);
            self.inside_buf.drain(..safe_cut);
        }
        String::new()
    }
}

fn tail_safe_emit_boundary(buf: &str) -> usize {
    let mut safe = buf.len();
    for prefix in OPEN_PREFIXES {
        safe = safe.min(hold_back_prefix_boundary(buf, prefix));
    }
    for tag in CLOSE_TAGS {
        safe = safe.min(hold_back_prefix_boundary(buf, tag));
    }
    safe
}

/// Gate merged `reasoning_content` + `content` stream deltas before TTS.
///
/// When thinking is enabled: suppress until `</think>` (or sibling close tags),
/// then pass through all subsequent deltas without further think filtering.
#[derive(Debug)]
pub struct StreamingThinkTtsGate {
    thinking_enabled: bool,
    waiting_close: bool,
    pending: String,
}

impl StreamingThinkTtsGate {
    pub fn new(thinking_enabled: bool) -> Self {
        Self {
            thinking_enabled,
            waiting_close: thinking_enabled,
            pending: String::new(),
        }
    }

    /// Push one merged-stream delta (`reasoning_content` and `content` in arrival order).
    pub fn push(&mut self, delta: &str) -> String {
        if delta.is_empty() {
            return String::new();
        }
        if !self.thinking_enabled {
            return delta.to_string();
        }
        if !self.waiting_close {
            return delta.to_string();
        }
        self.pending.push_str(delta);
        if let Some((pos, tag)) = find_close_tag(&self.pending) {
            let after = self.pending[pos + tag.len()..].to_string();
            self.pending.clear();
            self.waiting_close = false;
            return after;
        }
        String::new()
    }

    /// Flush speakable tail at end of LLM stream.
    pub fn flush(&mut self) -> String {
        if !self.thinking_enabled || !self.waiting_close {
            return String::new();
        }
        if let Some((pos, tag)) = find_close_tag(&self.pending) {
            let after = self.pending[pos + tag.len()..].to_string();
            self.pending.clear();
            self.waiting_close = false;
            return after;
        }
        let speakable = speakable_after_think_close(&self.pending);
        self.pending.clear();
        self.waiting_close = false;
        speakable
    }

    #[cfg(test)]
    pub fn is_speaking(&self) -> bool {
        self.thinking_enabled && !self.waiting_close
    }

    /// Snapshot for TTS skip diagnostics.
    pub fn diagnostics(&self) -> TtsGateDiagnostics {
        TtsGateDiagnostics {
            thinking_enabled: self.thinking_enabled,
            waiting_close: self.waiting_close,
            pending_chars: self.pending.chars().count(),
        }
    }
}

/// Gate state for logging why TTS did not run.
#[derive(Debug, Clone, Copy)]
pub struct TtsGateDiagnostics {
    pub thinking_enabled: bool,
    pub waiting_close: bool,
    pub pending_chars: usize,
}

/// Whether `merged` contains a recognized thinking close tag.
pub fn stream_has_think_close_tag(merged: &str) -> bool {
    find_close_tag(merged).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_complete_think_block() {
        let mut s = IncrementalThinkStripper::new();
        let out = s.push("before <redacted_thinking>secret</redacted_thinking> after");
        assert_eq!(out, "before  after");
    }

    #[test]
    fn drops_block_with_attributes() {
        let mut s = IncrementalThinkStripper::new();
        let out = s.push("x <think zh,>y</redacted_thinking>z");
        assert_eq!(out, "x z");
    }

    #[test]
    fn drops_unclosed_block_on_flush() {
        let mut s = IncrementalThinkStripper::new();
        assert_eq!(s.push("safe <redacted_thinking>still thinking"), "safe ");
        assert_eq!(s.flush(), "");
        assert!(!s.is_inside());
    }

    #[test]
    fn handles_thinking_close_tag() {
        let mut s = IncrementalThinkStripper::new();
        assert_eq!(
            s.push("pre <redacted_thinking>hidden</redacted_thi"),
            "pre "
        );
        assert_eq!(s.push("nking>post"), "post");
    }

    #[test]
    fn strip_think_blocks_removes_unterminated() {
        let input = "<redacted_thinking>secret</redacted_thinking>\n你好\n<thinking>tail";
        let out = strip_think_blocks(input);
        assert!(out.contains("你好"));
        assert!(!out.contains("secret"));
        assert!(!out.contains("tail"));
    }

    #[test]
    fn extract_inline_thinking_from_content_field() {
        let input = "<redacted_thinking>用户想查天气</redacted_thinking>\n明天可能下雨。";
        let thinking = extract_inline_thinking(input);
        assert!(thinking.contains("用户想查天气"));
        let speakable = strip_think_blocks(input);
        assert!(speakable.contains("明天可能下雨"));
        assert!(!speakable.contains("用户想查天气"));
    }

    #[test]
    fn board_log_unclosed_redacted_thinking_only() {
        let input = "<redacted_thinking>\n用户想知道明天的天气。\n我需要获取明天的天气信息。\n\
            但是我没有直接获取天气的工具，我需要调用 hermes 来帮我查询。\n\
            用户语气亲切，要求回答纯口语化，符合人设“小白”。\n\
            首先确认当前时间，以便准确描述“明天”。\n\
            然后用 hermes 查询天气。\n\
            最后根据查询结果，以口语化的方式回答用户。";
        let thinking = extract_inline_thinking(input);
        assert!(thinking.contains("用户想知道明天的天气"));
        assert!(strip_think_blocks(input).trim().is_empty());
    }

    #[test]
    fn board_log_closed_redacted_then_reply() {
        let input = "<redacted_thinking>\n用户提到“试了一下，点了你的那个”\n</redacted_thinking>\n\n\
            你指的是哪个\nzh,你指的是哪个";
        let thinking = extract_inline_thinking(input);
        assert!(thinking.contains("试了一下"));
        let speakable = strip_think_blocks(input);
        assert!(speakable.contains("你指的是哪个"));
        assert!(!speakable.contains("redacted_thinking"));
        assert!(!speakable.contains("用户提到"));
    }

    #[test]
    fn close_tag_gate_suppresses_until_redacted_close() {
        let mut gate = StreamingThinkTtsGate::new(true);
        assert_eq!(gate.push("Let's stick to"), "");
        assert_eq!(gate.push(" the simpler"), "");
        assert_eq!(gate.push("</redacted_thinking>"), "");
        assert_eq!(gate.push("好呀，你想"), "好呀，你想");
    }

    #[test]
    fn find_close_tag_on_split_buffer() {
        let mut pending = String::from("thinking</redacted_thi");
        pending.push_str("nking>");
        assert_eq!(pending.len(), 28);
        assert_eq!(&pending[8..], "</redacted_thinking>");
        for tag in CLOSE_TAGS {
            if let Some(p) = pending.find(tag) {
                assert_eq!(
                    Some((p, *tag)),
                    find_close_tag(&pending),
                    "first tag {tag} at {p}"
                );
                return;
            }
        }
        panic!("no close tag in {pending:?}");
    }

    #[test]
    fn close_tag_gate_handles_split_close_tag() {
        let mut gate = StreamingThinkTtsGate::new(true);
        assert_eq!(gate.push("thinking</redacted_thi"), "");
        assert_eq!(gate.push("nking>"), "");
        assert_eq!(gate.push("你好"), "你好");
    }

    #[test]
    fn unified_stream_reasoning_after_content_think_block() {
        let think_block = format!(
            "{}\n{}\n\n",
            REDACTED_THINKING_OPEN_TAG, REDACTED_THINKING_CLOSE_TAG,
        );
        let answer = "现在是2026年6月26日下午2点37分14秒。";
        let mut gate = StreamingThinkTtsGate::new(true);
        assert_eq!(gate.push(&think_block), "\n\n");
        assert!(gate.is_speaking());
        assert_eq!(gate.push(answer), answer);
    }

    #[test]
    fn rkllm_think_tags_then_time_reply() {
        let open = concat!("<", "think>");
        let close = concat!("</", "think>");
        let input = format!(
            "{open}\n{close}\n\n现在的时间是2026年06月26日15时03分19秒，你问这个是想安排什么活动吗。"
        );
        assert!(stream_has_think_close_tag(&input));
        let speakable = speakable_after_think_close(&input);
        assert!(speakable.contains("现在的时间是"), "got {speakable:?}");
        let mut gate = StreamingThinkTtsGate::new(true);
        let out = gate.push(&input);
        assert!(
            out.contains("现在的时间是"),
            "gate should emit after think close, got {out:?}"
        );
    }

    #[test]
    fn find_close_tag_prefers_thinking_over_think_prefix() {
        let buf = concat!("</", "thinking>", " after");
        let (pos, tag) = find_close_tag(buf).expect("close tag");
        assert_eq!(pos, 0);
        assert_eq!(tag, "</thinking>");
    }

    #[test]
    fn speaking_phase_does_not_strip_think_markup() {
        let input = format!(
            "{}\n{}\n\n",
            REDACTED_THINKING_OPEN_TAG, REDACTED_THINKING_CLOSE_TAG,
        );
        let mut gate = StreamingThinkTtsGate::new(true);
        assert_eq!(gate.push(&input), "\n\n");
        let out = gate.push("<thinking>保留</thinking>你好");
        assert!(
            out.contains("<thinking>"),
            "speaking phase passes through: {out:?}"
        );
        assert!(out.contains("你好"));
    }

    #[test]
    fn close_tag_gate_flush_emits_story_after_closed_block() {
        let input = format!(
            "{}\n用户想要听故事。\n{}\n\n有一只小刺猬特别想和兔子拥抱。",
            REDACTED_THINKING_OPEN_TAG, REDACTED_THINKING_CLOSE_TAG,
        );
        let mut gate = StreamingThinkTtsGate::new(true);
        let mut spoken = gate.push(&input);
        spoken.push_str(&gate.flush());
        assert!(
            spoken.contains("有一只小刺猬"),
            "expected story, got {spoken:?}"
        );
    }

    #[test]
    fn close_tag_gate_single_push_emits_story_after_closed_block() {
        let input = format!(
            "{}\nplanning...\n{}\n\n有一只小刺猬特别想和兔子拥抱。",
            REDACTED_THINKING_OPEN_TAG, REDACTED_THINKING_CLOSE_TAG,
        );
        let mut gate = StreamingThinkTtsGate::new(true);
        let out = gate.push(&input);
        let tail = gate.flush();
        let spoken = format!("{out}{tail}");
        assert!(
            spoken.contains("有一只小刺猬"),
            "expected story, got {spoken:?}"
        );
        assert!(!spoken.contains("planning"));
    }

    #[test]
    fn passthrough_when_thinking_disabled() {
        let mut gate = StreamingThinkTtsGate::new(false);
        assert_eq!(
            gate.push("before <think>x</think> after"),
            "before <think>x</think> after"
        );
    }

    #[test]
    fn strip_think_blocks_device_xie_ni_reply() {
        let input = format!(
            "{}\nplan...\nThinking budget exhausted, please proceed to the final answer.{}\n\n\
             哎呀，是不是打错字啦。你想说的是什么呀。",
            REDACTED_THINKING_OPEN_TAG, REDACTED_THINKING_CLOSE_TAG,
        );
        let speakable = strip_think_blocks(&input);
        assert!(speakable.contains("哎呀"));
        assert!(!speakable.contains("plan"));
    }

    #[test]
    fn device_xie_ni_reply_streams_to_tts_buf() {
        let input = format!(
            "{}\nplan...\nThinking budget exhausted, please proceed to the final answer.{}\n\n\
             哎呀，是不是打错字啦。你想说的是什么呀。",
            REDACTED_THINKING_OPEN_TAG, REDACTED_THINKING_CLOSE_TAG,
        );
        let mut gate = StreamingThinkTtsGate::new(true);
        let mut tts_buf = String::new();
        let speakable = gate.push(&input);
        crate::orchestrator::append_speakable_stream_delta(&mut tts_buf, &speakable, false);
        crate::orchestrator::append_speakable_stream_delta(&mut tts_buf, &gate.flush(), false);
        assert!(
            tts_buf.contains("哎呀"),
            "expected speakable reply in tts_buf, got {tts_buf:?}"
        );
    }

    #[test]
    fn empty_redacted_block_then_time_streams_to_tts_buf() {
        let input = format!(
            "{}\n{}\n\n现在是2026年6月26日14点26分32秒。",
            REDACTED_THINKING_OPEN_TAG, REDACTED_THINKING_CLOSE_TAG,
        );
        let mut gate = StreamingThinkTtsGate::new(true);
        let mut tts_buf = String::new();
        let speakable = gate.push(&input);
        crate::orchestrator::append_speakable_stream_delta(&mut tts_buf, &speakable, false);
        crate::orchestrator::append_speakable_stream_delta(&mut tts_buf, &gate.flush(), false);
        assert!(
            tts_buf.contains("现在是2026年"),
            "expected time reply in tts_buf, got {tts_buf:?}"
        );
        assert_eq!(
            strip_think_blocks(&input),
            "现在是2026年6月26日14点26分32秒。"
        );
    }

    #[test]
    fn gate_streams_story_chunk_by_chunk() {
        let open = REDACTED_THINKING_OPEN_TAG;
        let close = REDACTED_THINKING_CLOSE_TAG;
        let chunks = [
            format!("{open}\nplan"),
            format!("\n{close}\n\n有"),
            "一只小刺猬".to_string(),
            "特别想和兔子拥抱。".to_string(),
        ];
        let mut gate = StreamingThinkTtsGate::new(true);
        let mut tts_buf = String::new();
        for chunk in chunks {
            let speakable = gate.push(&chunk);
            crate::orchestrator::append_speakable_stream_delta(&mut tts_buf, &speakable, false);
        }
        let tail = gate.flush();
        crate::orchestrator::append_speakable_stream_delta(&mut tts_buf, &tail, false);
        assert!(tts_buf.contains("有一只小刺猬"));
        assert!(!tts_buf.contains("plan"));
    }

    #[test]
    fn stripper_drops_redacted_block_streaming() {
        let mut s = IncrementalThinkStripper::new();
        assert_eq!(s.push("x <redacted_thinking>"), "x ");
        assert_eq!(s.push("secret</redacted_thinking>z"), "z");
    }
}
