//! Strip `<think...>...</think>` blocks from streaming LLM text before TTS.
//!
//! Ported from `hermes-tools::tts_streaming::sanitizer::IncrementalThinkStripper` so talk
//! does not depend on the full tools crate.

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
            match buf.find("<think") {
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
        match self.inside_buf.find("</think>") {
            Some(pos) => {
                let after = self.inside_buf[pos + "</think>".len()..].to_string();
                self.inside_buf.clear();
                self.inside = false;
                self.pending = after;
                let buf = std::mem::take(&mut self.pending);
                self.drain_outside(buf)
            }
            None => {
                const TRAILING: usize = "</think".len();
                if self.inside_buf.len() > TRAILING {
                    let cut = self.inside_buf.len() - TRAILING;
                    let safe_cut = (0..=cut)
                        .rev()
                        .find(|&i| self.inside_buf.is_char_boundary(i))
                        .unwrap_or(0);
                    self.inside_buf.drain(..safe_cut);
                }
                String::new()
            }
        }
    }
}

fn tail_safe_emit_boundary(buf: &str) -> usize {
    const OPEN: &str = "<think";
    let max = OPEN.len() - 1;
    for k in (1..=max).rev() {
        if buf.len() < k {
            continue;
        }
        let start = buf.len() - k;
        if !buf.is_char_boundary(start) {
            continue;
        }
        let suffix = &buf[start..];
        if OPEN.starts_with(suffix) {
            return start;
        }
    }
    buf.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_complete_think_block() {
        let mut s = IncrementalThinkStripper::new();
        let out = s.push("before <think>secret</think> after");
        assert_eq!(out, "before  after");
    }

    #[test]
    fn drops_block_with_attributes() {
        let mut s = IncrementalThinkStripper::new();
        let out = s.push("x <think zh,>y</think>z");
        assert_eq!(out, "x z");
    }

    #[test]
    fn drops_unclosed_block_on_flush() {
        let mut s = IncrementalThinkStripper::new();
        assert_eq!(s.push("safe <think>still thinking"), "safe ");
        assert_eq!(s.flush(), "");
        assert!(!s.is_inside());
    }
}
