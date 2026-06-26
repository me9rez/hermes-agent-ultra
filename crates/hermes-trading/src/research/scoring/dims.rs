//! Dimension scoring ported from UZI score_fns.py (all 19 dims).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::research::profile::{AnalysisProfile, LITE_SCORE_DIM_KEYS};
use crate::research::report::labels::dimension_display_name;
use crate::research::types::FeatureVector;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DimScore {
    pub score: u8,
    pub weight: u8,
    #[serde(default)]
    pub display_name: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons_pass: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons_fail: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoreDimensionsResult {
    pub ticker: String,
    pub fundamental_score: f64,
    pub dimensions: std::collections::BTreeMap<String, DimScore>,
}

/// Score all 19 fundamental dimensions from raw dimension data + features.
#[must_use]
pub fn score_dimensions(
    ticker: &str,
    raw_dims: &Value,
    features: &FeatureVector,
    profile: &AnalysisProfile,
) -> ScoreDimensionsResult {
    let get = |key: &str| -> Value {
        raw_dims
            .get(key)
            .and_then(|v| v.get("data"))
            .cloned()
            .unwrap_or(Value::Null)
    };

    let mut out: std::collections::BTreeMap<String, DimScore> = std::collections::BTreeMap::new();

    // 1 · financials
    let fin = get("1_financials");
    let roe = f64_val(&fin, "roe").or(features.roe_latest).unwrap_or(0.0);
    let last_roe = fin
        .get("roe_history")
        .and_then(|h| h.as_array())
        .and_then(|a| a.last())
        .and_then(|v| v.as_f64())
        .unwrap_or(roe);
    let net_margin = f64_val(&fin, "net_margin")
        .or(features.net_margin)
        .unwrap_or(0.0);
    let debt = f64_val(
        fin.get("financial_health").unwrap_or(&Value::Null),
        "debt_ratio",
    )
    .or(features.debt_ratio)
    .unwrap_or(0.0);
    let rev_hist = fin.get("revenue_history").and_then(|v| v.as_array());
    let growth = rev_hist
        .and_then(|h| {
            if h.len() >= 2 {
                let prev = h[h.len() - 2].as_f64()?;
                let last = h[h.len() - 1].as_f64()?;
                if prev != 0.0 {
                    Some((last - prev) / prev * 100.0)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .or(features.revenue_growth_latest)
        .unwrap_or(0.0);

    let mut score_1: i32 = 5;
    let mut missing_1 = Vec::new();
    if features.revenue_latest_yi.is_none() && rev_hist.is_none() {
        missing_1.push("revenue".into());
    }
    if last_roe >= 15.0 {
        score_1 += 2;
    } else if last_roe >= 10.0 {
        score_1 += 1;
    } else if last_roe < 5.0 {
        score_1 -= 2;
    }
    if net_margin >= 15.0 {
        score_1 += 1;
    }
    if growth >= 20.0 {
        score_1 += 1;
    }
    if debt >= 60.0 {
        score_1 -= 1;
    }
    score_1 = score_1.clamp(1, 10);
    out.insert(
        "1_financials".into(),
        DimScore {
            score: score_1 as u8,
            weight: 5,
            display_name: String::new(),
            label: format!("ROE {last_roe:.1}% · 营收增速 {growth:+.1}% · 负债率 {debt:.0}%"),
            missing: missing_1,
            reasons_pass: vec![],
            reasons_fail: vec![],
        },
    );

    // 2 · kline (momentum)
    let kline = get("2_kline");
    let stage = kline
        .get("stage")
        .and_then(|v| v.as_str())
        .or(features.stage.as_deref())
        .unwrap_or("")
        .to_string();
    let ma_align = kline
        .get("ma_align")
        .and_then(|v| v.as_str())
        .or(features.ma_align.as_deref())
        .unwrap_or("")
        .to_string();
    let dd = f64_val(
        kline.get("kline_stats").unwrap_or(&Value::Null),
        "max_drawdown",
    )
    .or(features.max_drawdown_1y)
    .unwrap_or(0.0);
    let mut score_2: i32 = 5;
    if stage.contains("Stage 2") {
        score_2 += 2;
    } else if stage.contains("Stage 1") {
        score_2 += 1;
    } else if stage.contains("Stage 3") || stage.contains("Stage 4") {
        score_2 -= 2;
    }
    if ma_align.contains("多头") {
        score_2 += 1;
    }
    if dd <= -30.0 {
        score_2 -= 1;
    }
    score_2 = score_2.clamp(1, 10);
    out.insert(
        "2_kline".into(),
        DimScore {
            score: score_2 as u8,
            weight: 4,
            display_name: String::new(),
            label: format!("{stage} · 均线{ma_align}"),
            missing: if stage.is_empty() {
                vec!["stage".into()]
            } else {
                vec![]
            },
            reasons_pass: vec![],
            reasons_fail: vec![],
        },
    );

    // 3-9 stubs / light logic
    out.insert(
        "3_macro".into(),
        medium_web_dim("3_macro", 3, "宏观环境中性", raw_dims, profile),
    );
    let peers_data = get("4_peers");
    let has_peers = peers_data
        .get("peer_table")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    let mut missing_4 = Vec::new();
    if !profile.is_lite() && !has_peers {
        missing_4.push("4_peers".into());
    }
    out.insert(
        "4_peers".into(),
        DimScore {
            score: if has_peers { 7 } else { 5 },
            weight: 4,
            display_name: String::new(),
            label: if has_peers {
                "同行对比".into()
            } else {
                "同行对比（缺数）".into()
            },
            missing: missing_4,
            reasons_pass: vec![],
            reasons_fail: vec![],
        },
    );
    out.insert(
        "5_chain".into(),
        medium_web_dim("5_chain", 4, "产业链", raw_dims, profile),
    );
    let research = get("6_research");
    let research_count = u64_count(&research, "research_count");
    let mut score_6: i32 = 5;
    if research_count >= 8 {
        score_6 += 2;
    } else if research_count >= 3 {
        score_6 += 1;
    } else if research_count == 0 {
        score_6 -= 1;
    }
    let research_label = if research.is_null() || research_count == 0 {
        "券商研报数据缺失".into()
    } else {
        format!("券商研报 {research_count} 篇")
    };
    out.insert(
        "6_research".into(),
        DimScore {
            score: score_6.clamp(1, 10) as u8,
            weight: 3,
            display_name: String::new(),
            label: research_label,
            missing: research_missing_fields(&research),
            reasons_pass: vec![],
            reasons_fail: vec![],
        },
    );
    let industry_dim = get("7_industry");
    let ind_growth = f64_val(&industry_dim, "growth").unwrap_or(0.0);
    let ind_pe = f64_val(&industry_dim, "industry_pe")
        .or(features.industry_pe)
        .unwrap_or(0.0);
    let ind_name = industry_dim
        .get("industry")
        .and_then(|v| v.as_str())
        .or(features.industry.as_deref())
        .unwrap_or("—");
    let mut score_7: i32 = 5;
    if ind_growth >= 15.0 {
        score_7 += 2;
    } else if ind_growth >= 5.0 {
        score_7 += 1;
    } else if ind_growth < 0.0 {
        score_7 -= 2;
    }
    if ind_pe > 0.0 && features.pe.is_some_and(|pe| pe < ind_pe) {
        score_7 += 1;
    }
    out.insert(
        "7_industry".into(),
        DimScore {
            score: score_7.clamp(1, 10) as u8,
            weight: 4,
            display_name: String::new(),
            label: format!("{ind_name} · 增速 {ind_growth:+.1}% · 行业PE {ind_pe:.1}"),
            missing: if ind_name == "—" {
                vec!["industry".into()]
            } else {
                vec![]
            },
            reasons_pass: vec![],
            reasons_fail: vec![],
        },
    );
    out.insert(
        "8_materials".into(),
        medium_web_dim("8_materials", 3, "原材料成本关注中", raw_dims, profile),
    );
    out.insert(
        "9_futures".into(),
        medium_web_dim("9_futures", 2, "无强关联期货品种", raw_dims, profile),
    );

    // 10 · valuation
    let val = get("10_valuation");
    let pe_percentile = val
        .get("pe_percentile")
        .and_then(|v| v.as_f64())
        .map(|v| v.round() as i32)
        .or_else(|| {
            val.get("pe_quantile")
                .and_then(|v| v.as_str())
                .and_then(parse_pe_quantile)
        })
        .or(features.pe_quantile_5y.map(|v| v as i32));
    let score_10 = match pe_percentile {
        Some(q) if q < 30 => 9,
        Some(q) if q < 50 => 7,
        Some(q) if q < 70 => 5,
        Some(q) if q < 85 => 3,
        Some(_) => 2,
        None => 5,
    };
    let pe_q = pe_percentile.unwrap_or(-1);
    out.insert(
        "10_valuation".into(),
        DimScore {
            score: score_10,
            weight: 5,
            display_name: String::new(),
            label: if pe_percentile.is_some() {
                format!(
                    "PE {} · 5 年 {pe_q} 分位",
                    val.get("pe_ttm")
                        .or_else(|| val.get("pe"))
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "—".into())
                )
            } else {
                format!(
                    "PE {}",
                    val.get("pe_ttm")
                        .or_else(|| val.get("pe"))
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "—".into())
                )
            },
            missing: vec![],
            reasons_pass: if pe_percentile.is_some_and(|q| q < 50) {
                vec!["PE 在 5 年中位数以下".into()]
            } else {
                vec![]
            },
            reasons_fail: if pe_percentile.is_some_and(|q| q >= 75) {
                vec!["PE 已在 5 年高位区".into()]
            } else {
                vec![]
            },
        },
    );

    // 11-19
    out.insert(
        "11_governance".into(),
        medium_web_dim("11_governance", 3, "治理结构", raw_dims, profile),
    );
    let cf = get("12_capital_flow");
    let main_5d = f64_val(&cf, "main_fund_5d_net_yi");
    let fh = get("6_fund_holders");
    let holder_chg = f64_val(&cf, "holder_change_ratio").or(f64_val(&fh, "holder_change_ratio"));
    let mut missing_12 = Vec::new();
    if main_5d.is_none() {
        missing_12.push("main_fund_flow".into());
    }
    if holder_chg.is_none() {
        missing_12.push("holder_change_ratio".into());
    }
    if !profile.is_lite() && main_5d.is_none() && holder_chg.is_none() {
        missing_12.push("12_capital_flow".into());
    }
    let main_5d = main_5d.unwrap_or(0.0);
    let holder_chg = holder_chg.unwrap_or(0.0);
    let mut score_12: i32 = 5;
    if main_5d > 0.0 {
        score_12 += 2;
    } else if main_5d < 0.0 {
        score_12 -= 1;
    }
    if holder_chg < -5.0 {
        score_12 += 1;
    } else if holder_chg > 10.0 {
        score_12 -= 1;
    }
    out.insert(
        "12_capital_flow".into(),
        DimScore {
            score: score_12.clamp(1, 10) as u8,
            weight: 4,
            display_name: String::new(),
            label: format!("主力 5日 {main_5d:.2} 亿 · 户数变化 {holder_chg:+.1}%"),
            missing: missing_12,
            reasons_pass: vec![],
            reasons_fail: vec![],
        },
    );
    out.insert(
        "13_policy".into(),
        medium_web_dim("13_policy", 3, "政策环境中性", raw_dims, profile),
    );
    out.insert(
        "14_moat".into(),
        medium_web_dim("14_moat", 3, "护城河需定性评估", raw_dims, profile),
    );
    let events = get("15_events");
    let ann_count = u64_count(&events, "announcement_count");
    let news_count = u64_count(&events, "news_count");
    let total_events = ann_count + news_count;
    let mut score_15: i32 = 5;
    if total_events >= 8 {
        score_15 += 2;
    } else if total_events >= 3 {
        score_15 += 1;
    } else if total_events == 0 {
        score_15 -= 1;
    }
    let events_label = if events.is_null() || total_events == 0 {
        "事件/公告数据缺失".into()
    } else {
        format!("公告 {ann_count} 条 · 新闻 {news_count} 条")
    };
    let events_missing = events_missing_fields(&events);
    out.insert(
        "15_events".into(),
        DimScore {
            score: score_15.clamp(1, 10) as u8,
            weight: 4,
            display_name: String::new(),
            label: events_label,
            missing: events_missing,
            reasons_pass: vec![],
            reasons_fail: vec![],
        },
    );
    let lhb = get("16_lhb");
    let lhb_count = u64_count(&lhb, "lhb_count_30d") as i32;
    let youzi_count = lhb
        .get("matched_youzi")
        .and_then(|v| v.as_array())
        .map_or(0, |a| a.len()) as i32;
    let mut score_16: i32 = 5 + (lhb_count / 2).min(3);
    if lhb_count == 0 {
        score_16 -= 1;
    }
    let lhb_label = if lhb.is_null() || lhb_count == 0 {
        "暂无近期龙虎榜".into()
    } else if youzi_count > 0 {
        format!("近 30 天上榜 {lhb_count} 次 · 游资 {youzi_count} 条")
    } else {
        format!("近 30 天上榜 {lhb_count} 次")
    };
    out.insert(
        "16_lhb".into(),
        DimScore {
            score: score_16.clamp(1, 10) as u8,
            weight: 4,
            display_name: String::new(),
            label: lhb_label,
            missing: vec![],
            reasons_pass: vec![],
            reasons_fail: vec![],
        },
    );
    out.insert(
        "17_sentiment".into(),
        medium_web_dim("17_sentiment", 3, "舆情", raw_dims, profile),
    );
    let trap_keywords = ["涨停", "牛股", "翻倍", "龙头", "妖股"];
    let mut trap_score: i32 = 9;
    let mut trap_label = "🟢 未发现推广痕迹".to_string();
    if event_titles_contain_trap_keyword(&events, &trap_keywords) {
        trap_score = 3;
        trap_label = "🔴 舆情含推广措辞".into();
    }
    out.insert(
        "18_trap".into(),
        DimScore {
            score: trap_score.clamp(1, 10) as u8,
            weight: 5,
            display_name: String::new(),
            label: trap_label,
            missing: if events.is_null() {
                vec!["15_events".into()]
            } else {
                vec![]
            },
            reasons_pass: vec![],
            reasons_fail: vec![],
        },
    );
    out.insert(
        "19_contests".into(),
        medium_web_dim("19_contests", 4, "实盘比赛", raw_dims, profile),
    );

    if profile.is_lite() {
        let allowed: std::collections::HashSet<&str> =
            LITE_SCORE_DIM_KEYS.iter().copied().collect();
        out.retain(|k, _| allowed.contains(k.as_str()));
    }

    let dim_keys: Vec<String> = out.keys().cloned().collect();
    for key in dim_keys {
        if let Some(d) = out.get_mut(&key) {
            d.display_name = dimension_display_name(&key);
        }
    }

    let total_weighted: f64 = out.values().map(|d| f64::from(d.score * d.weight)).sum();
    let total_weight: f64 = out.values().map(|d| f64::from(d.weight)).sum();
    let fundamental = if total_weight > 0.0 {
        (total_weighted / total_weight * 10.0 * 10.0).round() / 100.0
    } else {
        0.0
    };

    ScoreDimensionsResult {
        ticker: ticker.to_string(),
        fundamental_score: fundamental,
        dimensions: out,
    }
}

fn neutral_dim(score: u8, weight: u8, label: &str) -> DimScore {
    DimScore {
        score,
        weight,
        display_name: String::new(),
        label: label.into(),
        missing: vec![],
        reasons_pass: vec![],
        reasons_fail: vec![],
    }
}

fn medium_web_dim(
    dim_key: &str,
    weight: u8,
    label: &str,
    raw_dims: &Value,
    profile: &AnalysisProfile,
) -> DimScore {
    if profile.is_lite() {
        return neutral_dim(6, weight, label);
    }
    if dim_collected_ok(raw_dims, dim_key) {
        return neutral_dim(6, weight, label);
    }
    DimScore {
        score: 5,
        weight,
        display_name: String::new(),
        label: label.to_string(),
        missing: vec![],
        reasons_pass: vec![],
        reasons_fail: vec![],
    }
}

fn dim_collected_ok(raw_dims: &Value, key: &str) -> bool {
    let Some(entry) = raw_dims.get(key) else {
        return false;
    };
    let quality = entry.get("quality").and_then(|v| v.as_str());
    if matches!(quality, Some("missing") | Some("error") | Some("skipped")) {
        return false;
    }
    let data = entry.get("data").unwrap_or(&Value::Null);
    !dim_data_empty(data)
}

fn dim_data_empty(data: &Value) -> bool {
    data.is_null() || data.as_object().is_some_and(|o| o.is_empty())
}

fn f64_val(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64())
}

fn u64_count(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

fn events_missing_fields(events: &Value) -> Vec<String> {
    if events.is_null() {
        return vec!["15_events".into()];
    }
    let mut missing = Vec::new();
    let ann_count = u64_count(events, "announcement_count");
    let news_count = u64_count(events, "news_count");
    if ann_count == 0 || events.get("announcement_error").is_some() {
        missing.push("announcements".into());
    }
    if news_count == 0 || events.get("news_error").is_some() {
        missing.push("news".into());
    }
    missing
}

fn research_missing_fields(research: &Value) -> Vec<String> {
    if research.is_null() {
        return vec!["6_research".into()];
    }
    let count = u64_count(research, "research_count");
    if count == 0 || research.get("research_error").is_some() {
        vec!["research_reports".into()]
    } else {
        vec![]
    }
}

fn event_titles_contain_trap_keyword(data: &Value, keywords: &[&str]) -> bool {
    for field in ["news", "announcements"] {
        let Some(items) = data.get(field).and_then(|v| v.as_array()) else {
            continue;
        };
        for item in items {
            if let Some(title) = item.get("title").and_then(|v| v.as_str())
                && keywords.iter().any(|k| title.contains(k))
            {
                return true;
            }
        }
    }
    false
}

fn parse_pe_quantile(s: &str) -> Option<i32> {
    s.chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::profile::AnalysisProfile;
    use crate::research::types::FeatureVector;
    use serde_json::json;

    fn score_events(raw: Value) -> (DimScore, DimScore) {
        let raw_dims = json!({ "15_events": { "data": raw } });
        let scored = score_dimensions(
            "600519.SH",
            &raw_dims,
            &FeatureVector::default(),
            &AnalysisProfile::medium(),
        );
        let events = scored.dimensions.get("15_events").unwrap().clone();
        let trap = scored.dimensions.get("18_trap").unwrap().clone();
        (events, trap)
    }

    fn score_research(raw: Value) -> DimScore {
        let raw_dims = json!({ "6_research": { "data": raw } });
        let scored = score_dimensions(
            "600519.SH",
            &raw_dims,
            &FeatureVector::default(),
            &AnalysisProfile::medium(),
        );
        scored.dimensions.get("6_research").unwrap().clone()
    }

    fn score_lhb(raw: Value) -> DimScore {
        let raw_dims = json!({ "16_lhb": { "data": raw } });
        let scored = score_dimensions(
            "600519.SH",
            &raw_dims,
            &FeatureVector::default(),
            &AnalysisProfile::medium(),
        );
        scored.dimensions.get("16_lhb").unwrap().clone()
    }

    #[test]
    fn events_dim_scores_from_counts() {
        let (events, trap) = score_events(json!({
            "announcement_count": 5,
            "news_count": 4,
            "announcements": [{"title": "年度报告"}],
            "news": [{"title": "行业观察"}]
        }));
        assert_eq!(events.score, 7);
        assert!(events.label.contains("公告 5 条"));
        assert!(events.missing.is_empty());
        assert_eq!(trap.score, 9);
    }

    #[test]
    fn events_missing_when_no_data() {
        let (events, trap) = score_events(json!({}));
        assert_eq!(events.score, 4);
        assert!(events.missing.iter().any(|m| m == "announcements"));
        assert!(trap.missing.is_empty());
    }

    #[test]
    fn trap_missing_when_events_dim_absent() {
        let raw_dims = json!({ "0_basic": { "data": { "price": 1.0 } } });
        let scored = score_dimensions(
            "600519.SH",
            &raw_dims,
            &FeatureVector::default(),
            &AnalysisProfile::medium(),
        );
        assert_eq!(
            scored.dimensions.get("18_trap").unwrap().missing,
            vec!["15_events"]
        );
    }

    #[test]
    fn trap_reads_announcement_titles() {
        let (_, trap) = score_events(json!({
            "announcement_count": 1,
            "announcements": [{"title": "妖股龙头翻倍攻略"}],
            "news_count": 0,
            "news": []
        }));
        assert_eq!(trap.score, 3);
        assert!(trap.label.contains("推广"));
    }

    #[test]
    fn events_missing_news_when_news_error_present() {
        let (events, _) = score_events(json!({
            "announcement_count": 3,
            "announcements": [{"title": "年报"}],
            "news_error": "upstream timeout"
        }));
        assert_eq!(events.score, 6);
        assert!(events.missing.iter().any(|m| m == "news"));
        assert!(!events.missing.iter().any(|m| m == "announcements"));
    }

    #[test]
    fn research_dim_scores_from_count() {
        let research = score_research(json!({
            "research_count": 10,
            "research_reports": [{"title": "买入", "org": "中信"}]
        }));
        assert_eq!(research.score, 7);
        assert!(research.label.contains("10 篇"));
        assert!(research.missing.is_empty());
    }

    #[test]
    fn research_missing_when_empty_or_error() {
        let empty = score_research(json!({
            "research_reports": [],
            "research_count": 0
        }));
        assert_eq!(empty.score, 4);
        assert!(empty.missing.iter().any(|m| m == "research_reports"));

        let errored = score_research(json!({
            "research_reports": [],
            "research_count": 0,
            "research_error": "upstream timeout"
        }));
        assert!(errored.missing.iter().any(|m| m == "research_reports"));
    }

    #[test]
    fn research_missing_when_dim_absent() {
        let raw_dims = json!({ "0_basic": { "data": { "price": 1.0 } } });
        let scored = score_dimensions(
            "600519.SH",
            &raw_dims,
            &FeatureVector::default(),
            &AnalysisProfile::medium(),
        );
        let research = scored.dimensions.get("6_research").expect("6_research");
        assert!(research.missing.iter().any(|m| m == "6_research"));
        assert!(research.label.contains("缺失"));
    }

    #[test]
    fn lhb_dim_scores_from_count_and_youzi() {
        let lhb = score_lhb(json!({
            "lhb_count_30d": 4,
            "matched_youzi": ["日涨幅偏离值达7%"],
            "lhb_records": 4
        }));
        assert_eq!(lhb.score, 7);
        assert!(lhb.label.contains("上榜 4 次"));
        assert!(lhb.label.contains("游资"));
        assert!(lhb.missing.is_empty());
    }

    #[test]
    fn lhb_missing_when_empty_or_error() {
        let empty = score_lhb(json!({
            "lhb_count_30d": 0,
            "matched_youzi": [],
            "lhb_records": 0
        }));
        assert_eq!(empty.score, 4);
        assert!(empty.missing.is_empty());

        let errored = score_lhb(json!({
            "lhb_count_30d": 0,
            "matched_youzi": [],
            "lhb_error": "upstream timeout"
        }));
        assert!(errored.missing.is_empty());
    }
}
