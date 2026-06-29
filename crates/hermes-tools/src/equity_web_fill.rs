//! Deterministic web_search gap-fill for equity research (`analyze_stock`).

use std::time::Instant;

use futures::future::join_all;
use hermes_trading::research::analyze::{AnalyzeStockResult, apply_external_context};
use hermes_trading::research::profile::AnalysisProfile;
use hermes_trading::research::report::{ExternalContextOverlay, has_unfilled_web_dims};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::tools::web::WebSearchBackend;

const MAX_BULLETS_PER_SLOT: usize = 3;
const MAX_BULLET_CHARS: usize = 160;
const SEARCH_NUM_RESULTS: usize = 5;

/// Outcome of automatic web gap-fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFillReport {
    pub queries_run: u32,
    pub filled: bool,
    pub skipped: bool,
}

impl WebFillReport {
    fn skipped() -> Self {
        Self {
            queries_run: 0,
            filled: false,
            skipped: true,
        }
    }

    fn already_filled() -> Self {
        Self {
            queries_run: 0,
            filled: true,
            skipped: false,
        }
    }
}

/// Env kill-switch for tests (`HERMES_EQUITY_WEB_FILL=0`).
#[must_use]
pub fn web_fill_enabled() -> bool {
    match std::env::var("HERMES_EQUITY_WEB_FILL").as_deref() {
        Ok("0") | Ok("false") | Ok("off") | Ok("disabled") => false,
        _ => true,
    }
}

/// Run templated parallel `web_search` queries and merge into `result`.
pub async fn fill_web_dims_if_needed(
    backend: &dyn WebSearchBackend,
    result: &mut AnalyzeStockResult,
    profile: &AnalysisProfile,
) -> WebFillReport {
    if !web_fill_enabled() {
        debug!("equity web fill disabled via HERMES_EQUITY_WEB_FILL");
        return WebFillReport::skipped();
    }
    if !profile.allow_web_supplement || !has_unfilled_web_dims(result, profile) {
        return WebFillReport::already_filled();
    }

    let ctx = extract_stock_context(result);
    let plans = build_query_plans(&ctx, profile);
    if plans.is_empty() {
        return WebFillReport::already_filled();
    }

    let started = Instant::now();
    let query_count = plans.len() as u32;
    info!(
        symbol = %result.symbol,
        depth = %result.depth,
        queries = query_count,
        "equity web fill: starting parallel web_search"
    );

    let tasks = plans.iter().map(|plan| {
        let query = plan.query.clone();
        async move {
            let json = backend
                .search(&query, SEARCH_NUM_RESULTS, None)
                .await
                .unwrap_or_else(|e| {
                    warn!(query = %query, error = %e, "equity web fill search failed");
                    String::new()
                });
            (plan.slot, parse_search_hits(&json))
        }
    });
    let batches = join_all(tasks).await;
    let mut overlay = ExternalContextOverlay::default();
    for (slot, hits) in batches {
        let filtered = filter_hits_for_slot(&hits, slot, &ctx);
        if hits.len() > filtered.len() {
            debug!(
                slot = ?slot,
                kept = filtered.len(),
                dropped = hits.len() - filtered.len(),
                "equity web fill: filtered irrelevant search hits"
            );
        }
        apply_hits_to_overlay(&mut overlay, slot, &filtered);
    }
    sanitize_overlay_bullets(&mut overlay, &ctx);

    let filled = overlay_has_content(&overlay);
    if filled {
        apply_external_context(result, &overlay);
        info!(
            symbol = %result.symbol,
            elapsed_ms = started.elapsed().as_millis(),
            sources = overlay.sources.len(),
            "equity web fill: merged external_context"
        );
    } else {
        warn!(
            symbol = %result.symbol,
            elapsed_ms = started.elapsed().as_millis(),
            queries = query_count,
            "equity web fill: searches returned no usable snippets"
        );
    }

    WebFillReport {
        queries_run: query_count,
        filled,
        skipped: false,
    }
}

/// Last-chance web fill before HTML delivery (sync; safe from slash / stale cache).
#[cfg(all(feature = "trading-research", feature = "web"))]
pub fn ensure_web_dims_filled(result: &mut AnalyzeStockResult) {
    let profile = AnalysisProfile::from_depth_str(&result.depth);
    if !profile.allow_web_supplement || !has_unfilled_web_dims(result, &profile) {
        return;
    }
    let backend = crate::backends::web::search_backend_from_env_or_fallback();
    let report = block_on_async(fill_web_dims_if_needed(backend.as_ref(), result, &profile));
    if report.queries_run > 0 {
        info!(
            symbol = %result.symbol,
            filled = report.filled,
            queries = report.queries_run,
            "equity web fill: delivery-time gap-fill finished"
        );
    }
}

#[cfg(all(feature = "trading-research", feature = "web"))]
fn block_on_async<T>(future: impl std::future::Future<Output = T>) -> T {
    // Never call `Handle::block_on` on a Tokio worker without `block_in_place` — it
    // deadlocks the runtime and leaves slash delivery hanging indefinitely.
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(future))
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("equity web fill runtime")
            .block_on(future)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebFillSlot {
    MacroPolicy,
    Sentiment,
    Chain,
    Materials,
    MoatGovernance,
    FuturesContests,
    LiteGovernance,
}

struct QueryPlan {
    slot: WebFillSlot,
    query: String,
}

struct StockWebContext {
    symbol_code: String,
    company: String,
    industry: String,
}

struct SearchHit {
    title: String,
    url: String,
    snippet: String,
}

impl Clone for SearchHit {
    fn clone(&self) -> Self {
        Self {
            title: self.title.clone(),
            url: self.url.clone(),
            snippet: self.snippet.clone(),
        }
    }
}

fn extract_stock_context(result: &AnalyzeStockResult) -> StockWebContext {
    let basic = result.raw_dims.get("0_basic").and_then(|v| v.get("data"));
    let company = basic
        .and_then(|d| d.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| result.symbol.clone());
    let industry = basic
        .and_then(|d| d.get("industry"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            result
                .raw_dims
                .get("7_industry")
                .and_then(|v| v.get("data"))
                .and_then(|d| d.get("industry"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "A股".into());
    let symbol_code = result
        .symbol
        .split('.')
        .next()
        .unwrap_or(result.symbol.as_str())
        .to_string();
    StockWebContext {
        symbol_code,
        company,
        industry,
    }
}

fn build_query_plans(ctx: &StockWebContext, profile: &AnalysisProfile) -> Vec<QueryPlan> {
    if profile.is_lite() {
        let mut plans = Vec::new();
        if profile.should_run_fetcher(hermes_trading::research::fetchers::dim_keys::GOVERNANCE) {
            plans.push(QueryPlan {
                slot: WebFillSlot::LiteGovernance,
                query: format!("{} 公司治理 董事会 股权结构", ctx.company),
            });
        }
        return plans;
    }

    vec![
        QueryPlan {
            slot: WebFillSlot::MacroPolicy,
            query: format!(
                "{} {} 宏观经济 货币政策 行业监管政策 A股",
                ctx.company, ctx.industry
            ),
        },
        QueryPlan {
            slot: WebFillSlot::Sentiment,
            query: format!(
                "{} {} 投资者舆情 研报 市场关注度",
                ctx.symbol_code, ctx.company
            ),
        },
        QueryPlan {
            slot: WebFillSlot::Chain,
            query: format!(
                "{} {} {} 产业链 上下游 供应商",
                ctx.symbol_code, ctx.company, ctx.industry
            ),
        },
        QueryPlan {
            slot: WebFillSlot::Materials,
            query: format!(
                "{} {} {} 原材料 成本 采购价格",
                ctx.symbol_code, ctx.company, ctx.industry
            ),
        },
        QueryPlan {
            slot: WebFillSlot::MoatGovernance,
            query: format!(
                "{} {} 竞争优势 护城河 公司治理",
                ctx.symbol_code, ctx.company
            ),
        },
        QueryPlan {
            slot: WebFillSlot::FuturesContests,
            query: format!("{} {} 期货 套保 行业竞争", ctx.symbol_code, ctx.industry),
        },
    ]
}

fn parse_search_hits(json: &str) -> Vec<SearchHit> {
    if json.trim().is_empty() {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let rows = search_result_rows(&value);
    rows.iter().filter_map(|row| row_to_hit(row)).collect()
}

fn search_result_rows(value: &Value) -> Vec<&Value> {
    if let Some(rows) = value.get("results").and_then(Value::as_array) {
        return rows.iter().collect();
    }
    if let Some(rows) = value
        .get("data")
        .and_then(|d| d.get("web"))
        .and_then(Value::as_array)
    {
        return rows.iter().collect();
    }
    Vec::new()
}

fn row_to_hit(row: &Value) -> Option<SearchHit> {
    let title = row
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let url = row
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let snippet = row
        .get("snippet")
        .or_else(|| row.get("description"))
        .or_else(|| row.get("body"))
        .or_else(|| row.get("text"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| title.clone());
    Some(SearchHit {
        title,
        url,
        snippet,
    })
}

fn apply_hits_to_overlay(
    overlay: &mut ExternalContextOverlay,
    slot: WebFillSlot,
    hits: &[SearchHit],
) {
    if hits.is_empty() {
        return;
    }
    for hit in hits {
        if !hit.url.is_empty() {
            let source = if hit.title.is_empty() {
                hit.url.clone()
            } else {
                format!("{} — {}", hit.title, hit.url)
            };
            if !overlay.sources.iter().any(|s| s == &source) {
                overlay.sources.push(source);
            }
        }
    }

    let bullets = hits_to_bullets(hits, MAX_BULLETS_PER_SLOT);
    match slot {
        WebFillSlot::MacroPolicy => {
            overlay.macro_bullets = bullets.iter().take(2).cloned().collect();
            overlay.policy_bullets = bullets.iter().skip(2).take(2).cloned().collect();
            if overlay.macro_bullets.is_empty() && !bullets.is_empty() {
                overlay.macro_bullets.push(bullets[0].clone());
            }
            if overlay.policy_bullets.is_empty() && bullets.len() > 1 {
                overlay.policy_bullets.push(bullets[1].clone());
            }
            overlay.rate_cycle = overlay.macro_bullets.first().cloned();
            overlay.fx_trend = overlay.macro_bullets.get(1).cloned();
            overlay.geo_risk = overlay.policy_bullets.first().cloned();
            overlay.commodity = overlay.policy_bullets.get(1).cloned();
        }
        WebFillSlot::Sentiment => overlay.sentiment_bullets = bullets,
        WebFillSlot::Chain => overlay.chain_bullets = bullets,
        WebFillSlot::Materials => overlay.materials_bullets = bullets,
        WebFillSlot::MoatGovernance => {
            overlay.moat_bullets = bullets.iter().take(2).cloned().collect();
            overlay.governance_bullets = bullets.iter().skip(2).take(2).cloned().collect();
            if overlay.moat_bullets.is_empty() && !bullets.is_empty() {
                overlay.moat_bullets.push(bullets[0].clone());
            }
            if overlay.governance_bullets.is_empty() && bullets.len() > 1 {
                overlay.governance_bullets.push(bullets[1].clone());
            }
        }
        WebFillSlot::FuturesContests => {
            overlay.futures_bullets = bullets.iter().take(2).cloned().collect();
            overlay.contests_bullets = bullets.iter().skip(2).take(2).cloned().collect();
            if overlay.futures_bullets.is_empty() && !bullets.is_empty() {
                overlay.futures_bullets.push(bullets[0].clone());
            }
            if overlay.contests_bullets.is_empty() && bullets.len() > 1 {
                overlay.contests_bullets.push(bullets[1].clone());
            }
        }
        WebFillSlot::LiteGovernance => overlay.governance_bullets = bullets,
    }
}

fn hits_to_bullets(hits: &[SearchHit], max: usize) -> Vec<String> {
    hits.iter()
        .take(max)
        .map(|hit| {
            let text = if hit.snippet.len() >= 12 {
                hit.snippet.clone()
            } else {
                hit.title.clone()
            };
            truncate_bullet(&text)
        })
        .filter(|s| !s.is_empty() && !is_junk_bullet(s))
        .collect()
}

fn is_junk_bullet(text: &str) -> bool {
    DICT_JUNK_MARKERS.iter().any(|m| text.contains(m))
}

fn sanitize_overlay_bullets(overlay: &mut ExternalContextOverlay, ctx: &StockWebContext) {
    for bullets in [
        &mut overlay.macro_bullets,
        &mut overlay.policy_bullets,
        &mut overlay.sentiment_bullets,
        &mut overlay.chain_bullets,
        &mut overlay.materials_bullets,
        &mut overlay.futures_bullets,
        &mut overlay.governance_bullets,
        &mut overlay.moat_bullets,
        &mut overlay.contests_bullets,
        &mut overlay.trap_bullets,
    ] {
        bullets.retain(|b| !is_junk_bullet(b));
    }
    if overlay.materials_bullets.is_empty() {
        overlay.materials_bullets.push(format!(
            "{}（{}）原材料/大宗商品成本需结合行业价格跟踪",
            ctx.company, ctx.industry
        ));
    }
}

fn truncate_bullet(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= MAX_BULLET_CHARS {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(MAX_BULLET_CHARS).collect::<String>())
    }
}

const DICT_JUNK_MARKERS: &[&str] = &[
    "说文解字",
    "康熙字典",
    "国语辞典",
    "国语辞典",
    "汉典",
    "新华字典",
    "基本解释",
    "详细解释",
    "字源字形",
    "音韵方言",
    "zdic.net",
    "baike.com/wiki/豫",
    "汉语字典",
    "康熙大字典",
];

fn filter_hits_for_slot(
    hits: &[SearchHit],
    slot: WebFillSlot,
    ctx: &StockWebContext,
) -> Vec<SearchHit> {
    hits.iter()
        .filter(|hit| is_relevant_for_slot(hit, slot, ctx))
        .cloned()
        .collect()
}

fn is_relevant_for_slot(hit: &SearchHit, slot: WebFillSlot, ctx: &StockWebContext) -> bool {
    if is_junk_equity_hit(hit) {
        return false;
    }
    match slot {
        WebFillSlot::Materials => is_relevant_materials_hit(hit, ctx),
        WebFillSlot::Chain => is_relevant_chain_hit(hit, ctx),
        WebFillSlot::MacroPolicy | WebFillSlot::Sentiment | WebFillSlot::MoatGovernance => {
            mentions_company_or_code(hit, ctx) || !looks_like_single_char_lookup(hit)
        }
        WebFillSlot::FuturesContests | WebFillSlot::LiteGovernance => true,
    }
}

fn is_junk_equity_hit(hit: &SearchHit) -> bool {
    let blob = format!("{} {} {}", hit.title, hit.snippet, hit.url);
    if DICT_JUNK_MARKERS.iter().any(|m| blob.contains(m)) {
        return true;
    }
    looks_like_single_char_lookup(hit)
}

fn looks_like_single_char_lookup(hit: &SearchHit) -> bool {
    let title = hit.title.trim();
    if title.chars().count() == 1 {
        let blob = format!("{} {}", hit.snippet, hit.url);
        return blob.contains("解释")
            || blob.contains("字典")
            || blob.contains("辞典")
            || blob.contains("zdic")
            || blob.contains("汉典");
    }
    false
}

fn is_relevant_materials_hit(hit: &SearchHit, ctx: &StockWebContext) -> bool {
    let blob = format!("{} {}", hit.title, hit.snippet);
    const KW: &[&str] = &[
        "原材料",
        "成本",
        "采购",
        "价格",
        "大宗",
        "矿石",
        "冶炼",
        "加工费",
        "铅",
        "锌",
        "铜",
        "铝",
        "金",
        "银",
        "浆",
        "化工",
        "能源",
    ];
    if KW.iter().any(|k| blob.contains(k)) {
        return mentions_company_or_code(hit, ctx) || blob.contains(&ctx.industry);
    }
    false
}

fn is_relevant_chain_hit(hit: &SearchHit, ctx: &StockWebContext) -> bool {
    let blob = format!("{} {}", hit.title, hit.snippet);
    const KW: &[&str] = &["产业链", "上下游", "供应", "客户", "龙头", "配套", "产能"];
    if KW.iter().any(|k| blob.contains(k)) {
        return mentions_company_or_code(hit, ctx) || blob.contains(&ctx.industry);
    }
    false
}

fn mentions_company_or_code(hit: &SearchHit, ctx: &StockWebContext) -> bool {
    let blob = format!("{} {} {}", hit.title, hit.snippet, hit.url);
    if blob.contains(&ctx.symbol_code) {
        return true;
    }
    let company = ctx.company.trim();
    if company.chars().count() >= 3 && blob.contains(company) {
        return true;
    }
    // Avoid matching single-char company prefixes (e.g. 豫) without full name.
    false
}

fn overlay_has_content(overlay: &ExternalContextOverlay) -> bool {
    !overlay.macro_bullets.is_empty()
        || !overlay.policy_bullets.is_empty()
        || !overlay.sentiment_bullets.is_empty()
        || !overlay.chain_bullets.is_empty()
        || !overlay.materials_bullets.is_empty()
        || !overlay.futures_bullets.is_empty()
        || !overlay.governance_bullets.is_empty()
        || !overlay.moat_bullets.is_empty()
        || !overlay.contests_bullets.is_empty()
        || !overlay.trap_bullets.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use hermes_core::ToolError;
    use hermes_trading::research::report::content::ExternalCoverage;
    use hermes_trading::research::types::DataConfidence;
    use std::sync::{Arc, Mutex};

    struct MockSearchBackend {
        responses: Arc<Mutex<Vec<String>>>,
        canned: String,
    }

    impl MockSearchBackend {
        fn new(canned: &str) -> Self {
            Self {
                responses: Arc::new(Mutex::new(Vec::new())),
                canned: canned.into(),
            }
        }
    }

    #[async_trait]
    impl WebSearchBackend for MockSearchBackend {
        async fn search(
            &self,
            query: &str,
            _num_results: usize,
            _category: Option<&str>,
        ) -> Result<String, ToolError> {
            self.responses.lock().unwrap().push(query.to_string());
            Ok(self.canned.clone())
        }
    }

    fn medium_stub_result() -> AnalyzeStockResult {
        AnalyzeStockResult {
            symbol: "600519.SH".into(),
            depth: "medium".into(),
            dcf: serde_json::json!({}),
            comps: serde_json::json!({}),
            three_statement: serde_json::json!({}),
            lbo: serde_json::json!({}),
            scores: serde_json::json!({
                "ticker": "600519.SH",
                "fundamental_score": 60.0,
                "dimensions": {
                    "14_moat": { "score": 5, "weight": 3, "display_name": "", "label": "护城河", "missing": [], "reasons_pass": [], "reasons_fail": [] }
                }
            }),
            personas: serde_json::json!({}),
            data_confidence: DataConfidence {
                score: 0.75,
                present: vec![],
                missing: vec![],
            },
            missing_dims: vec![],
            dim_summary: vec![],
            used_fallback: vec![],
            summary_markdown: String::new(),
            synthesis: hermes_trading::research::synthesis::SynthesisReport {
                headline: String::new(),
                verdict: String::new(),
                confidence_tier: String::new(),
                key_metrics: vec![],
                risks: vec![],
                missing_highlights: vec![],
                panel_summary: hermes_trading::research::synthesis::PanelSummary {
                    consensus: 0.0,
                    vote_buy: 0,
                    vote_avoid: 0,
                    investor_count: 0,
                },
                dcf_one_liner: String::new(),
            },
            content: hermes_trading::research::report::ReportContent::default(),
            raw_dims: serde_json::json!({
                "0_basic": { "data": { "name": "贵州茅台", "industry": "白酒" } }
            }),
        }
    }

    #[test]
    fn parse_search_hits_reads_results_array() {
        let json = r#"{"results":[{"title":"宏观平稳","url":"https://example.com/a","snippet":"利率中枢下移"}]}"#;
        let hits = parse_search_hits(json);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].snippet, "利率中枢下移");
    }

    #[test]
    fn parse_search_hits_reads_ddgs_data_web_envelope() {
        let json = r#"{"success":true,"data":{"web":[{"title":"宏观","url":"https://a.com","description":"流动性合理充裕"}]}}"#;
        let hits = parse_search_hits(json);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].snippet, "流动性合理充裕");
    }

    #[test]
    fn filters_yu_dictionary_junk_for_materials() {
        let ctx = StockWebContext {
            symbol_code: "600531".into(),
            company: "豫光金铅".into(),
            industry: "铅锌".into(),
        };
        let hit = SearchHit {
            title: "豫".into(),
            url: "https://www.zdic.net/hans/%E8%B1%AB".into(),
            snippet: "异体 基本解释 详细解释 康熙字典 说文解字".into(),
        };
        assert!(is_junk_equity_hit(&hit));
        assert!(!is_relevant_materials_hit(&hit, &ctx));
        let kept = filter_hits_for_slot(&[hit], WebFillSlot::Materials, &ctx);
        assert!(kept.is_empty());
    }

    #[tokio::test]
    #[ignore = "live network — web fill for 600531 materials quality"]
    async fn live_web_fill_600531_materials_not_dictionary() {
        use crate::analyze_stock_cache;
        use crate::tools::trading_analyze_stock::AnalyzeStockHandler;
        use hermes_core::ToolHandler;
        use serde_json::json;

        analyze_stock_cache::clear_for_tests();
        AnalyzeStockHandler::new()
            .execute(json!({
                "symbol": "600531.SH",
                "depth": "medium",
                "use_providers": true
            }))
            .await
            .expect("analyze_stock 600531");
        let result = analyze_stock_cache::get("600531.SH", "medium").expect("cache");
        let materials = result
            .raw_dims
            .get("8_materials")
            .and_then(|v| v.get("data"))
            .and_then(|d| d.get("bullets"))
            .and_then(|b| b.as_array())
            .cloned()
            .unwrap_or_default();
        for bullet in &materials {
            let s = bullet.as_str().unwrap_or("");
            assert!(
                !s.contains("说文解字") && !s.contains("康熙字典") && !s.contains("基本解释"),
                "materials must not be dictionary junk: {s}"
            );
        }
    }

    #[tokio::test]
    #[ignore = "live network — web fill for 600530"]
    async fn live_web_fill_600530_no_stubs() {
        use crate::analyze_stock_cache;
        use crate::tools::trading_analyze_stock::AnalyzeStockHandler;
        use hermes_core::ToolHandler;
        use hermes_trading::research::report::content::ExternalCoverage;
        use serde_json::json;

        analyze_stock_cache::clear_for_tests();
        let handler = AnalyzeStockHandler::new();
        handler
            .execute(json!({
                "symbol": "600530.SH",
                "depth": "medium",
                "use_providers": true
            }))
            .await
            .expect("analyze_stock 600530");
        let result = analyze_stock_cache::get("600530.SH", "medium").expect("cache");
        eprintln!(
            "600530 coverage={:?} macro={} chain={} materials={} futures={}",
            result.content.external.coverage,
            result.content.external.macro_bullets.len(),
            result.content.external.chain_bullets.len(),
            result.content.external.materials_bullets.len(),
            result.content.external.futures_bullets.len(),
        );
        assert_eq!(
            result.content.external.coverage,
            ExternalCoverage::WebFilled,
            "macro={:?} policy={:?} chain={:?}",
            result.content.external.macro_bullets,
            result.content.external.policy_bullets,
            result.content.external.chain_bullets,
        );
        let html = hermes_trading::research::report::render_institutional_html(&result, None);
        let stubs = html.matches("待 web 补数").count();
        eprintln!("600530 stub_markers={stubs}");
        assert_eq!(stubs, 0, "600530 HTML must not contain web stub markers");
    }

    #[tokio::test]
    #[ignore = "live network — web fill for 600529"]
    async fn live_web_fill_600529_no_stubs() {
        use crate::analyze_stock_cache;
        use crate::tools::trading_analyze_stock::AnalyzeStockHandler;
        use hermes_core::ToolHandler;
        use hermes_trading::research::report::content::ExternalCoverage;
        use serde_json::json;

        analyze_stock_cache::clear_for_tests();
        let handler = AnalyzeStockHandler::new();
        handler
            .execute(json!({
                "symbol": "600529.SH",
                "depth": "medium",
                "use_providers": true
            }))
            .await
            .expect("analyze_stock 600529");
        let result = analyze_stock_cache::get("600529.SH", "medium").expect("cache");
        assert_eq!(
            result.content.external.coverage,
            ExternalCoverage::WebFilled,
            "expected web fill merge; macro={:?} policy={:?}",
            result.content.external.macro_bullets,
            result.content.external.policy_bullets,
        );
        let html = hermes_trading::research::report::render_institutional_html(&result, None);
        assert!(
            !html.contains("待 web 补数"),
            "600529 HTML must not contain web stub markers"
        );
    }

    #[tokio::test]
    async fn fill_web_dims_merges_overlay_for_medium() {
        let canned = serde_json::json!({
            "results": [
                {"title":"宏观","url":"https://a","snippet":"宏观流动性合理充裕"},
                {"title":"政策","url":"https://b","snippet":"消费行业政策稳定"},
                {"title":"舆情","url":"https://c","snippet":"机构关注度较高"}
            ]
        })
        .to_string();
        let backend = MockSearchBackend::new(&canned);
        let mut result = medium_stub_result();
        let profile = AnalysisProfile::medium();
        let report = fill_web_dims_if_needed(&backend, &mut result, &profile).await;
        assert!(report.filled);
        assert!(report.queries_run >= 5);
        assert_eq!(
            result.content.external.coverage,
            ExternalCoverage::WebFilled
        );
        assert!(!result.content.external.macro_bullets.is_empty());
        assert!(!result.content.external.sentiment_bullets.is_empty());
    }
}
