use super::*;

fn default_model_cost_per_million(model: &str) -> Option<(f64, f64)> {
    let m = model.to_lowercase();
    if m.contains("gpt-4o-mini") || m.contains("4.1-mini") || m.contains("haiku") {
        return Some((0.15, 0.60));
    }
    if m.contains("gpt-4o") || m.contains("4.1") || m.contains("sonnet") {
        return Some((2.5, 10.0));
    }
    if m.contains("o3") {
        return Some((10.0, 40.0));
    }
    None
}

pub(crate) fn extract_objective_state_marker(text: &str) -> String {
    for line in text.lines() {
        let lowered = line.trim().to_ascii_lowercase();
        if let Some(rest) = lowered.split("objective_state=").nth(1) {
            let token = rest
                .trim_start()
                .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                .next()
                .unwrap_or("")
                .trim_matches(|c: char| c == ')' || c == '.');
            if !token.is_empty() {
                return token.to_string();
            }
        }
        if let Some(rest) = lowered.split("objective_state:").nth(1) {
            let token = rest
                .trim_start()
                .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                .next()
                .unwrap_or("")
                .trim_matches(|c: char| c == ')' || c == '.');
            if !token.is_empty() {
                return token.to_string();
            }
        }
    }
    "unspecified".to_string()
}

pub(crate) fn extract_marker_values(text: &str, marker: &str, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(idx) = line.find(marker) else {
            continue;
        };
        let rest = &line[idx + marker.len()..];
        let value = rest
            .split(|c: char| c.is_whitespace() || c == ')' || c == ',' || c == ';' || c == '|')
            .next()
            .unwrap_or("")
            .trim();
        if value.is_empty() {
            continue;
        }
        let normalized = value.trim_matches(|c: char| c == '"' || c == '\'' || c == '`');
        if normalized.is_empty() || out.iter().any(|v| v == normalized) {
            continue;
        }
        out.push(normalized.to_string());
        if out.len() >= limit {
            break;
        }
    }
    out
}

pub(crate) fn estimate_usage_cost_usd(
    usage: &UsageStats,
    model: &str,
    config: &AgentConfig,
) -> Option<f64> {
    if let Some(v) = usage.estimated_cost {
        return Some(v.max(0.0));
    }
    let canonical = usage_stats_to_canonical(usage);
    let provider = config.provider.as_deref();
    let cost =
        hermes_intelligence::usage_pricing::calculate_cost(model, &canonical, provider, None);
    if let Some(amount) = cost.amount_usd {
        return Some(amount.max(0.0));
    }
    let (in_pm, out_pm) = match (
        config.prompt_cost_per_million_usd,
        config.completion_cost_per_million_usd,
    ) {
        (Some(i), Some(o)) => (i, o),
        _ => default_model_cost_per_million(model)?,
    };
    let prompt_cost = (usage.prompt_tokens as f64 / 1_000_000.0) * in_pm;
    let completion_cost = (usage.completion_tokens as f64 / 1_000_000.0) * out_pm;
    Some(prompt_cost + completion_cost)
}

fn usage_stats_to_canonical(
    usage: &UsageStats,
) -> hermes_intelligence::usage_pricing::CanonicalUsage {
    let input = if usage.input_tokens > 0 {
        usage.input_tokens
    } else {
        usage
            .prompt_tokens
            .saturating_sub(usage.cache_read_tokens + usage.cache_write_tokens)
    };
    let output = if usage.output_tokens > 0 {
        usage.output_tokens
    } else {
        usage.completion_tokens
    };
    hermes_intelligence::usage_pricing::CanonicalUsage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: usage.cache_read_tokens,
        cache_write_tokens: usage.cache_write_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        request_count: 1,
    }
}

/// Merge two UsageStats, summing token counts and keeping the latest cost estimate.
pub(crate) fn merge_usage(existing: Option<UsageStats>, new: &UsageStats) -> UsageStats {
    match existing {
        Some(prev) => UsageStats {
            prompt_tokens: prev.prompt_tokens + new.prompt_tokens,
            completion_tokens: prev.completion_tokens + new.completion_tokens,
            total_tokens: prev.total_tokens + new.total_tokens,
            input_tokens: prev.input_tokens + new.input_tokens,
            output_tokens: prev.output_tokens + new.output_tokens,
            cache_read_tokens: prev.cache_read_tokens + new.cache_read_tokens,
            cache_write_tokens: prev.cache_write_tokens + new.cache_write_tokens,
            reasoning_tokens: prev.reasoning_tokens + new.reasoning_tokens,
            estimated_cost: match (prev.estimated_cost, new.estimated_cost) {
                (Some(a), Some(b)) => Some(a + b),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            },
        },
        None => new.clone(),
    }
}
