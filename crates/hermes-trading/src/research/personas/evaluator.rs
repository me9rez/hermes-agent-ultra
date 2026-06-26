//! Investor evaluation engine (UZI investor_evaluator.py).

use serde::{Deserialize, Serialize};

use super::investors::{INVESTORS, MarketScope, find_investor, locked_school};
use super::rules::{Rule, rules_for};
use crate::research::types::FeatureVector;

const BULLISH_THRESHOLD: f64 = 65.0;
const BEARISH_THRESHOLD: f64 = 35.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleHit {
    pub rule_id: String,
    pub name: String,
    pub weight: u8,
    pub msg: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersonaVote {
    pub id: String,
    pub vote: String,
    pub score: f64,
    pub signal: String,
    pub confidence: f64,
    pub cited_rule: Option<String>,
    pub pass_rules: Vec<RuleHit>,
    pub fail_rules: Vec<RuleHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
}

/// Evaluate one investor against features.
#[must_use]
pub fn evaluate(investor_id: &str, features: &FeatureVector) -> PersonaVote {
    if let Some(locked) = locked_school()
        && let Some(meta) = find_investor(investor_id)
        && meta.group != locked
    {
        return skip_vote(
            investor_id,
            format!("用户锁定 {locked} 派视角 · 非该派评委不参与"),
        );
    }

    let meta = match find_investor(investor_id) {
        Some(m) => m,
        None => {
            return skip_vote(investor_id, "未知评委".into());
        }
    };

    let market = features.market.as_deref().unwrap_or("A");
    if meta.market_scope == MarketScope::AShareOnly && market != "A" {
        return skip_vote(investor_id, "游资只看 A 股".into());
    }

    if is_youzi_out_of_range(investor_id, meta.name, features) {
        return skip_vote(investor_id, "市值不在游资射程".into());
    }

    let rules = rules_for(investor_id);
    if rules.is_empty() {
        return skip_vote(investor_id, "无规则".into());
    }

    let mut pass_list = Vec::new();
    let mut fail_list = Vec::new();
    let mut weight_pass = 0u32;
    let mut weight_total = 0u32;

    for rule in rules {
        weight_total += u32::from(rule.weight);
        if !rule_prerequisites_met(rule.rule_id, features) {
            fail_list.push(missing_data_hit(rule));
            continue;
        }
        if safe_check(rule, features) {
            weight_pass += u32::from(rule.weight);
            pass_list.push(hit(rule, rule.pass_msg, features));
        } else {
            fail_list.push(hit(rule, rule.fail_msg, features));
        }
    }

    let score = if weight_total > 0 {
        (f64::from(weight_pass) / f64::from(weight_total) * 100.0 * 10.0).round() / 10.0
    } else {
        50.0
    };

    let signal = if score >= BULLISH_THRESHOLD {
        "bullish"
    } else if score < BEARISH_THRESHOLD {
        "bearish"
    } else {
        "neutral"
    };

    let confidence = vote_confidence(features, rules.len(), &pass_list, &fail_list);
    let cited = pass_list
        .first()
        .or(fail_list.first())
        .map(|r| r.rule_id.clone());
    let vote = score_to_verdict(score, signal);

    PersonaVote {
        id: investor_id.to_string(),
        vote,
        score,
        signal: signal.into(),
        confidence,
        cited_rule: cited,
        pass_rules: pass_list,
        fail_rules: fail_list,
        skip_reason: None,
    }
}

/// Evaluate all registered investors, optionally filtered to a subset (lite quick-scan).
#[must_use]
pub fn evaluate_all(features: &FeatureVector) -> Vec<PersonaVote> {
    evaluate_filtered(features, None)
}

/// Evaluate investors matching `ids` when provided.
#[must_use]
pub fn evaluate_filtered(features: &FeatureVector, ids: Option<&[&str]>) -> Vec<PersonaVote> {
    match ids {
        Some(list) => list.iter().map(|id| evaluate(id, features)).collect(),
        None => INVESTORS.iter().map(|m| evaluate(m.id, features)).collect(),
    }
}

fn safe_check(rule: &Rule, features: &FeatureVector) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (rule.check)(features)))
        .unwrap_or(false)
}

/// Rules that need explicit fields must fail (not pass) when data is absent.
fn rule_prerequisites_met(rule_id: &str, features: &FeatureVector) -> bool {
    match rule_id {
        "fcf_positive" | "fcf" => {
            features.fcf_positive.is_some() || features.fcf_latest_yi.is_some()
        }
        "safety_margin_pe" | "margin_safety" => features.pe_quantile_5y.is_some(),
        _ => true,
    }
}

fn missing_data_hit(rule: &Rule) -> RuleHit {
    RuleHit {
        rule_id: rule.rule_id.to_string(),
        name: rule.name.to_string(),
        weight: rule.weight,
        msg: "数据缺失，规则不通过".into(),
    }
}

fn vote_confidence(
    features: &FeatureVector,
    rule_count: usize,
    pass_list: &[RuleHit],
    fail_list: &[RuleHit],
) -> f64 {
    let mut confidence = (50.0 + rule_count as f64 * 8.0).min(100.0);
    if features.pe_quantile_5y.is_none() {
        confidence = (confidence - 15.0).max(0.0);
    }
    if features.fcf_positive.is_none() && features.fcf_latest_yi.is_none() {
        confidence = (confidence - 15.0).max(0.0);
    }
    if pass_list.is_empty() && fail_list.iter().any(|r| r.msg.contains("数据缺失")) {
        confidence = (confidence - 10.0).max(0.0);
    }
    confidence
}

fn hit(rule: &Rule, template: &str, features: &FeatureVector) -> RuleHit {
    RuleHit {
        rule_id: rule.rule_id.to_string(),
        name: rule.name.to_string(),
        weight: rule.weight,
        msg: format_msg(template, features),
    }
}

fn format_msg(template: &str, features: &FeatureVector) -> String {
    if template.is_empty() {
        return String::new();
    }
    let map = features.as_format_map();
    let mut out = template.to_string();
    for (k, v) in map {
        out = out.replace(&format!("{{{k}}}"), &v);
    }
    out
}

fn score_to_verdict(score: f64, signal: &str) -> String {
    match signal {
        "bullish" if score >= 80.0 => "强烈买入".into(),
        "bullish" => "买入".into(),
        "bearish" if score <= 20.0 => "回避".into(),
        "bearish" => "观望".into(),
        _ if score >= 50.0 => "关注".into(),
        _ => "观望".into(),
    }
}

fn skip_vote(id: &str, reason: String) -> PersonaVote {
    PersonaVote {
        id: id.to_string(),
        vote: "不适合".into(),
        score: -1.0,
        signal: "skip".into(),
        confidence: 0.0,
        cited_rule: None,
        pass_rules: vec![],
        fail_rules: vec![],
        skip_reason: Some(reason),
    }
}

/// ponytail: simplified youzi range — mega-cap >5000亿 skip unless LHB match.
fn is_youzi_out_of_range(investor_id: &str, nickname: &str, features: &FeatureVector) -> bool {
    let _ = (investor_id, nickname);
    let mc = features.market_cap_yi.unwrap_or(0.0);
    if mc <= 5000.0 {
        return false;
    }
    if features
        .matched_youzi
        .iter()
        .any(|n| n.contains(nickname) || nickname.contains(n.as_str()))
    {
        return false;
    }
    find_investor(investor_id).is_some_and(|m| m.group == 'F')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::types::FeatureVector;

    #[test]
    fn buffett_bullish_on_quality() {
        let f = FeatureVector {
            market: Some("A".into()),
            roe_5y_above_15: Some(5.0),
            roe_5y_min: Some(16.0),
            net_margin: Some(18.0),
            debt_ratio: Some(35.0),
            fcf_positive: Some(true),
            moat_total: Some(30.0),
            pe_quantile_5y: Some(40.0),
            consecutive_dividend_years: Some(8.0),
            ..Default::default()
        };
        let v = evaluate("buffett", &f);
        assert_eq!(v.signal, "bullish");
        assert!(v.score >= 65.0);
    }

    #[test]
    fn buffett_not_bullish_when_fcf_and_pe_quantile_missing() {
        let f = FeatureVector {
            market: Some("A".into()),
            symbol: "600519.SH".into(),
            price: Some(1680.0),
            pe: Some(28.5),
            roe_latest: Some(32.0),
            net_margin: Some(52.0),
            debt_ratio: Some(18.0),
            ..Default::default()
        };
        let v = evaluate("buffett", &f);
        assert_ne!(v.signal, "bullish");
        assert!(v.score < 65.0);
        assert!(
            v.fail_rules
                .iter()
                .any(|r| r.msg.contains("数据缺失") || r.rule_id == "fcf_positive"),
            "expected FCF rule failure, got {:?}",
            v.fail_rules
        );
        assert!(
            v.fail_rules.iter().any(|r| r.rule_id == "safety_margin_pe"),
            "expected PE quantile rule failure"
        );
        assert!(v.confidence < 80.0);
    }

    #[test]
    fn klarman_fails_margin_safety_without_pe_quantile() {
        let f = FeatureVector {
            market: Some("A".into()),
            fcf_positive: Some(true),
            debt_ratio: Some(30.0),
            ..Default::default()
        };
        let v = evaluate("klarman", &f);
        assert!(
            v.fail_rules
                .iter()
                .any(|r| r.rule_id == "margin_safety" && r.msg.contains("数据缺失")),
            "klarman margin_safety should fail closed on missing pe_quantile"
        );
    }

    #[test]
    fn fisher_growth_passes_with_revenue_growth() {
        let f = FeatureVector {
            market: Some("A".into()),
            revenue_growth_latest: Some(20.0),
            roe_latest: Some(22.0),
            net_margin: Some(18.0),
            ..Default::default()
        };
        let v = evaluate("fisher", &f);
        assert!(v.score >= 65.0, "fisher score {}", v.score);
    }

    #[test]
    fn soros_bearish_without_trend_data() {
        let f = FeatureVector {
            market: Some("A".into()),
            price: Some(10.0),
            ..Default::default()
        };
        let v = evaluate("soros", &f);
        assert_ne!(v.signal, "bullish");
    }
}
