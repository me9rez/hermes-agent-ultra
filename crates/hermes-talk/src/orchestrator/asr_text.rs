//! ASR transcript helpers for streaming partial / full hypothesis merge.

/// Min suffix/prefix overlap (chars) to treat a candidate as incremental; below → segment reset.
const SEGMENT_RESET_MAX_OVERLAP: usize = 2;

/// Trim whitespace before LLM trigger.
pub fn normalize_asr_transcript(s: &str) -> String {
    s.trim().to_string()
}

/// True when `prefix` matches the leading chars of `s`.
pub fn is_char_prefix(prefix: &str, s: &str) -> bool {
    let mut p = prefix.chars();
    let mut sc = s.chars();
    loop {
        match p.next() {
            None => return true,
            Some(pc) => match sc.next() {
                None => return false,
                Some(c) if c == pc => {}
                Some(_) => return false,
            },
        }
    }
}

/// Longest suffix/prefix overlap between two strings (char count).
pub fn suffix_prefix_overlap_chars(existing: &str, new: &str) -> usize {
    if existing.is_empty() || new.is_empty() {
        return 0;
    }
    let existing_chars: Vec<char> = existing.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();
    let max_overlap = existing_chars.len().min(new_chars.len());
    for overlap in (1..=max_overlap).rev() {
        if existing_chars[existing_chars.len() - overlap..] == new_chars[..overlap] {
            return overlap;
        }
    }
    0
}

/// Return the suffix of `new` after the longest suffix/prefix overlap with `existing`.
pub fn strip_overlap_prefix<'a>(existing: &str, new: &'a str) -> &'a str {
    if existing.is_empty() || new.is_empty() {
        return new;
    }
    let overlap = suffix_prefix_overlap_chars(existing, new);
    if overlap == 0 {
        return new;
    }
    let new_chars: Vec<char> = new.chars().collect();
    let byte_start: usize = new_chars[..overlap].iter().map(|c| c.len_utf8()).sum();
    &new[byte_start..]
}

/// Merge streaming partial + optional SDK full hypothesis into `assembled`.
pub fn merge_hypothesis(assembled: &mut String, piece: &str, full: Option<&str>) {
    let piece = piece.trim();
    let full = full.map(str::trim).filter(|s| !s.is_empty());

    if let Some(full) = full {
        let before = assembled.clone();
        apply_candidate(assembled, full);
        if *assembled != before || *assembled == full {
            return;
        }
    }

    if piece.is_empty() {
        return;
    }
    apply_candidate(assembled, piece);
}

fn apply_candidate(assembled: &mut String, candidate: &str) {
    if candidate.is_empty() {
        return;
    }
    if assembled.is_empty() {
        *assembled = candidate.to_string();
        return;
    }
    if assembled == candidate {
        return;
    }
    // SDK rollback: shorter prefix of current hypothesis → ignore.
    if is_char_prefix(candidate, assembled) && candidate.chars().count() < assembled.chars().count()
    {
        return;
    }
    // Full hypothesis extension.
    if is_char_prefix(assembled, candidate) {
        *assembled = candidate.to_string();
        return;
    }

    let overlap = suffix_prefix_overlap_chars(assembled, candidate);
    if overlap < SEGMENT_RESET_MAX_OVERLAP {
        if should_append_disconnected_tail(assembled, candidate) {
            if !assembled.ends_with(candidate) {
                assembled.push_str(candidate);
            }
            return;
        }
        *assembled = candidate.to_string();
        return;
    }

    // Incremental delta with overlap.
    let delta = strip_overlap_prefix(assembled, candidate);
    if !delta.is_empty() {
        assembled.push_str(delta);
    }
}

/// True when the transcript lacks sentence-ending punctuation (likely mid-utterance).
pub fn utterance_likely_incomplete(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && !matches!(t.chars().last(), Some('。' | '？' | '！' | '.' | '?' | '!'))
}

/// Pick best transcript at utterance flush from partial assembly and SDK final.
pub fn resolve_utterance_text(assembled: &str, asr_final: Option<&str>) -> Option<String> {
    resolve_utterance_text_with_best(assembled, asr_final, None, None)
}

/// Pick best transcript at flush: longest complete sentence among all hypotheses.
pub fn resolve_utterance_text_with_best(
    assembled: &str,
    asr_final: Option<&str>,
    best_full: Option<&str>,
    peak_partial: Option<&str>,
) -> Option<String> {
    let candidates = [
        assembled.trim(),
        best_full.unwrap_or("").trim(),
        asr_final.unwrap_or("").trim(),
        peak_partial.unwrap_or("").trim(),
    ];
    select_best_transcript(&candidates)
}

/// True when `a` is a better flush candidate than `b`.
fn prefer_transcript(a: &str, b: &str) -> bool {
    if b.is_empty() {
        return true;
    }
    if a.is_empty() {
        return false;
    }
    let a_complete = !utterance_likely_incomplete(a);
    let b_complete = !utterance_likely_incomplete(b);
    if a_complete != b_complete {
        return a_complete;
    }
    let ac = a.chars().count();
    let bc = b.chars().count();
    if ac != bc {
        return ac > bc;
    }
    is_char_prefix(b, a) && !is_char_prefix(a, b)
}

fn select_best_transcript(candidates: &[&str]) -> Option<String> {
    let mut best = "";
    for c in candidates {
        let t = c.trim();
        if t.is_empty() {
            continue;
        }
        if prefer_transcript(t, best) {
            best = t;
        }
    }
    if best.is_empty() {
        None
    } else {
        Some(best.to_string())
    }
}

fn should_append_disconnected_tail(assembled: &str, candidate: &str) -> bool {
    let asm_len = assembled.chars().count();
    let cand_len = candidate.chars().count();
    if cand_len == 0 || asm_len == 0 {
        return false;
    }
    if cand_len * 2 < asm_len {
        return true;
    }
    if asm_len >= 6
        && cand_len <= 8
        && candidate
            .chars()
            .next()
            .is_some_and(|c| matches!(c, '是' | '有' | '能' | '会' | '吗'))
    {
        return true;
    }
    if asm_len >= 2
        && cand_len <= 4
        && candidate
            .chars()
            .next()
            .is_some_and(|c| matches!(c, '了' | '吗' | '呢' | '啊' | '吧' | '嘛'))
    {
        return true;
    }
    false
}

/// Keep the longest non-empty transcript among `candidates`.
pub fn bump_longest_transcript(into: &mut String, candidates: &[&str]) {
    for c in candidates {
        let t = c.trim();
        if !t.is_empty() && t.chars().count() > into.chars().count() {
            *into = t.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims() {
        assert_eq!(normalize_asr_transcript("  你好  "), "你好");
    }

    #[test]
    fn strip_overlap_prefix_cases() {
        assert_eq!(strip_overlap_prefix("", "查一下"), "查一下");
        assert_eq!(strip_overlap_prefix("帮我", "查一下"), "查一下");
        assert_eq!(strip_overlap_prefix("帮我查一下", "查一下明天的"), "明天的");
        assert_eq!(
            strip_overlap_prefix("帮我查一下明天的", "查一下明天的天气。"),
            "天气。"
        );
        assert_eq!(strip_overlap_prefix("帮我", "帮我查一下"), "查一下");
    }

    #[test]
    fn merge_hypothesis_incremental_weather() {
        let mut a = String::new();
        merge_hypothesis(&mut a, "帮", None);
        merge_hypothesis(&mut a, "我查一下", Some("帮我查一下"));
        merge_hypothesis(&mut a, "查一下明天的", Some("帮我查一下明天的"));
        merge_hypothesis(&mut a, "查一下明天的天气。", Some("帮我查一下明天的天气。"));
        assert_eq!(a, "帮我查一下明天的天气。");
    }

    #[test]
    fn merge_hypothesis_ignores_rollback() {
        let mut a = String::new();
        merge_hypothesis(&mut a, "现", None);
        merge_hypothesis(&mut a, "现在几点", Some("现在几点"));
        merge_hypothesis(&mut a, "现在", Some("现在"));
        merge_hypothesis(&mut a, "了。", None);
        assert_eq!(a, "现在几点了。");
    }

    #[test]
    fn merge_segment_reset_not_duplicate_concat() {
        let mut a = String::new();
        merge_hypothesis(&mut a, "你刚才", None);
        merge_hypothesis(&mut a, "你刚才说", Some("你刚才说"));
        merge_hypothesis(&mut a, "你刚才", Some("你刚才"));
        merge_hypothesis(&mut a, "的关", None);
        merge_hypothesis(&mut a, "的关于马", None);
        merge_hypothesis(&mut a, "的关于蚂蚁的那个笑话", None);
        merge_hypothesis(&mut a, "是啥意思？", None);
        assert!(!a.contains("关于马的关于蚂蚁"));
        assert!(a.ends_with("是啥意思？"));
    }

    #[test]
    fn resolve_prefers_longer_assembled_when_final_is_shorter() {
        assert_eq!(
            resolve_utterance_text(
                "两分钟后提醒我和两分钟后提醒我喝水。",
                Some("两分钟后提醒我喝水"),
            ),
            Some("两分钟后提醒我和两分钟后提醒我喝水。".to_string())
        );
    }

    #[test]
    fn resolve_prefers_assembled_when_it_contains_final() {
        assert_eq!(
            resolve_utterance_text("现在几点了。", Some("几点了。")),
            Some("现在几点了。".to_string())
        );
    }

    #[test]
    fn resolve_uses_final_when_assembled_empty() {
        assert_eq!(
            resolve_utterance_text("", Some("几点了。")),
            Some("几点了。".to_string())
        );
    }

    #[test]
    fn resolve_prefers_longer_complete_over_truncated_final() {
        let peak = "轻松愉快的话题吧，除了小松鼠。";
        let final_t = "聊点轻松愉快的话题吧。除了小松";
        assert_eq!(
            resolve_utterance_text_with_best(final_t, Some(final_t), Some(peak), Some(peak)),
            Some(peak.to_string())
        );
    }

    #[test]
    fn resolve_prefers_longer_peak_over_final() {
        let long = "既然你这么开心，那我们就接着聊点轻松愉快的话题吧，除了小松鼠。";
        let final_t = "聊点轻松愉快的话题吧。除了小松";
        assert_eq!(
            resolve_utterance_text_with_best(final_t, Some(final_t), Some(long), Some(long),),
            Some(long.to_string())
        );
    }

    #[test]
    fn resolve_ignores_short_tail_final_on_long_assembled() {
        let assembled = "你刚才说的关于蚂蚁的那个笑话是啥意思？";
        assert_eq!(
            resolve_utterance_text_with_best(assembled, Some("是啥意思？"), None, None),
            Some(assembled.to_string())
        );
    }

    #[test]
    fn resolve_prefers_best_full_over_shorter_assembled() {
        let best = "你刚才说的关于蚂蚁的那个笑话是啥意思？";
        let assembled = "聊点轻松愉快的话题吧。除了小松";
        assert_eq!(
            resolve_utterance_text_with_best(assembled, Some(assembled), Some(best), Some(best)),
            Some(best.to_string())
        );
    }

    #[test]
    fn utterance_incomplete_without_sentence_end() {
        assert!(utterance_likely_incomplete("帮我查一下明天的"));
        assert!(!utterance_likely_incomplete("帮我查一下明天的天气。"));
    }
}
