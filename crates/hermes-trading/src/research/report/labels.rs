//! Chinese display names and stable ordering for 19 scoring dimensions.

/// Canonical dimension key order (1 → 19).
pub const DIM_ORDER: &[&str] = &[
    "1_financials",
    "2_kline",
    "3_macro",
    "4_peers",
    "5_chain",
    "6_research",
    "7_industry",
    "8_materials",
    "9_futures",
    "10_valuation",
    "11_governance",
    "12_capital_flow",
    "13_policy",
    "14_moat",
    "15_events",
    "16_lhb",
    "17_sentiment",
    "18_trap",
    "19_contests",
];

#[must_use]
pub fn dimension_display_name(key: &str) -> String {
    match key {
        "1_financials" => "财务面".into(),
        "2_kline" => "技术面 (K线)".into(),
        "3_macro" => "宏观环境".into(),
        "4_peers" => "同行对比".into(),
        "5_chain" => "产业链".into(),
        "6_research" => "券商研报".into(),
        "7_industry" => "行业景气".into(),
        "8_materials" => "原材料成本".into(),
        "9_futures" => "期货关联".into(),
        "10_valuation" => "估值 (PE/PB)".into(),
        "11_governance" => "治理结构".into(),
        "12_capital_flow" => "资金流向".into(),
        "13_policy" => "政策环境".into(),
        "14_moat" => "护城河".into(),
        "15_events" => "事件驱动".into(),
        "16_lhb" => "龙虎榜".into(),
        "17_sentiment" => "舆情".into(),
        "18_trap" => "推广陷阱".into(),
        "19_contests" => "实盘比赛".into(),
        _ => key.to_string(),
    }
}

/// Dimension groups for HTML section layout.
#[must_use]
pub fn dimension_group_title(group: &str) -> String {
    match group {
        "fundamentals" => "基本面".into(),
        "market" => "市场与板块".into(),
        "external" => "外部与定性".into(),
        _ => group.into(),
    }
}

/// DEEP SCAN category rows (aligned with FloatFu standalone report).
pub const SCAN_CATEGORIES: &[(&str, &str, &[&str])] = &[
    (
        "fundamentals",
        "💰 财务面 · FUNDAMENTALS",
        &["1_financials", "10_valuation"],
    ),
    (
        "market",
        "📈 行情面 · MARKET ACTION",
        &["2_kline", "12_capital_flow", "16_lhb"],
    ),
    (
        "industry",
        "🏭 行业面 · INDUSTRY CHAIN",
        &[
            "4_peers",
            "7_industry",
            "5_chain",
            "8_materials",
            "9_futures",
        ],
    ),
    (
        "company",
        "🏢 公司面 · COMPANY",
        &["11_governance", "14_moat", "6_research", "6_fund_holders"],
    ),
    (
        "environment",
        "🌍 环境面 · ENVIRONMENT",
        &["3_macro", "13_policy"],
    ),
    (
        "safety",
        "🛡️ 安全面 · SAFETY & SENTIMENT",
        &["15_events", "17_sentiment", "18_trap", "19_contests"],
    ),
];

#[must_use]
pub fn dimension_english_name(key: &str) -> &'static str {
    match key {
        "1_financials" => "Financials",
        "2_kline" => "Technical",
        "3_macro" => "Macro",
        "4_peers" => "Peers",
        "5_chain" => "Chain",
        "6_research" => "Sell-side",
        "6_fund_holders" => "Holders",
        "7_industry" => "Industry",
        "8_materials" => "Materials",
        "9_futures" => "Futures",
        "10_valuation" => "Valuation",
        "11_governance" => "Governance",
        "12_capital_flow" => "Capital Flow",
        "13_policy" => "Policy",
        "14_moat" => "Moat",
        "15_events" => "Events",
        "16_lhb" => "Dragon Tiger",
        "17_sentiment" => "Sentiment",
        "18_trap" => "Promotion Trap",
        "19_contests" => "Contests",
        _ => "Dimension",
    }
}

/// Stable DIM index for card header (matches key prefix).
#[must_use]
pub fn dimension_dim_index(key: &str) -> String {
    key.split('_')
        .next()
        .and_then(|n| n.parse::<u32>().ok())
        .map(|n| format!("{n:02}"))
        .unwrap_or_else(|| "00".into())
}

#[must_use]
pub fn dimensions_in_group(group: &str) -> &'static [&'static str] {
    match group {
        "fundamentals" => &[
            "1_financials",
            "10_valuation",
            "6_research",
            "6_fund_holders",
        ],
        "market" => &[
            "2_kline",
            "4_peers",
            "7_industry",
            "12_capital_flow",
            "16_lhb",
        ],
        "external" => &[
            "3_macro",
            "5_chain",
            "8_materials",
            "9_futures",
            "11_governance",
            "13_policy",
            "14_moat",
            "15_events",
            "17_sentiment",
            "18_trap",
            "19_contests",
        ],
        _ => &[],
    }
}
