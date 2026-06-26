//! Per-dimension SVG / HTML mini charts for DEEP SCAN cards.

use serde_json::Value;

use crate::research::report::content::{ExternalBlock, ExternalCoverage};
use crate::research::report::dim_viz::render_pe_relative_bar;
use crate::research::report::svg::render_svg_percentile;
use crate::research::report_filter::is_web_only_dim;

const CYAN: &str = "#0891b2";
const GOLD: &str = "#d97706";
const GREEN: &str = "#059669";
const RED: &str = "#dc2626";
const MUTED: &str = "#64748b";
const INK: &str = "#0f172a";

/// Render optional viz block for a dimension card (empty if no chart data).
#[must_use]
pub fn render_dim_chart(key: &str, raw_dims: &Value, external: &ExternalBlock) -> String {
    let inner = match key {
        "1_financials" => chart_financials(&dim_data(raw_dims, "1_financials")),
        "10_valuation" => chart_valuation(&dim_data(raw_dims, "10_valuation"), raw_dims),
        "2_kline" => chart_kline(&dim_data(raw_dims, "2_kline")),
        "12_capital_flow" => chart_capital_flow(&dim_data(raw_dims, "12_capital_flow")),
        "4_peers" => chart_peers(&dim_data(raw_dims, "4_peers")),
        "7_industry" => chart_industry(&dim_data(raw_dims, "7_industry"), raw_dims),
        "6_research" => chart_research(&dim_data(raw_dims, "6_research")),
        "6_fund_holders" => chart_fund_holders(&dim_data(raw_dims, "6_fund_holders")),
        "15_events" => chart_events(&dim_data(raw_dims, "15_events")),
        "16_lhb" => chart_lhb(&dim_data(raw_dims, "16_lhb")),
        "18_trap" => chart_trap(&dim_data(raw_dims, "18_trap")),
        "3_macro" if external.coverage == ExternalCoverage::WebFilled => {
            chart_macro_panel(&dim_data(raw_dims, "3_macro"), external)
        }
        "13_policy" if external.coverage == ExternalCoverage::WebFilled => {
            chart_external_bullets("政策", &external.policy_bullets)
        }
        "17_sentiment" if external.coverage == ExternalCoverage::WebFilled => {
            chart_external_bullets("舆情", &external.sentiment_bullets)
        }
        _ if is_web_only_dim(key) => chart_neutral_grid(key),
        _ => String::new(),
    };
    if inner.is_empty() {
        return String::new();
    }
    format!(r#"<div class="dim-viz">{inner}</div>"#)
}

fn dim_data(raw_dims: &Value, key: &str) -> Value {
    raw_dims
        .get(key)
        .and_then(|v| v.get("data"))
        .cloned()
        .unwrap_or(Value::Null)
}

fn chart_financials(data: &Value) -> String {
    let rev = f64_array(data, "revenue_history");
    let np = f64_array(data, "net_profit_history");
    let roe = f64_array(data, "roe_history");
    let years = year_labels(rev.len().max(np.len()));
    let mut out = String::new();
    if !rev.is_empty() && !np.is_empty() {
        out.push_str(&format!(
            r#"<div class="viz-caption">📊 营收（亿）· 金线=净利（亿）</div>{}"#,
            render_revenue_net_profit_combo(&rev, &np, &years, 320, 130)
        ));
    } else if !rev.is_empty() {
        out.push_str(&format!(
            r#"<div class="viz-caption">📊 营收（亿）</div>{}"#,
            render_bar_chart(&rev, &year_labels(rev.len()), 280, 110)
        ));
    }
    if !roe.is_empty() {
        out.push_str(&format!(
            r#"<div class="viz-caption" style="margin-top:8px">ROE 走势 %</div>{}"#,
            render_sparkline(&roe, GREEN, 280, 36)
        ));
    }
    if let Some(health) = data.get("financial_health").and_then(|v| v.as_object()) {
        if let Some(debt) = health.get("debt_ratio").and_then(|v| v.as_f64()) {
            out.push_str(&render_h_bar("资产负债率", debt, 100.0, "%", debt < 60.0));
        }
        if let Some(cr) = health.get("current_ratio").and_then(|v| v.as_f64()) {
            out.push_str(&render_h_bar("流动比率", cr, 3.0, "", cr >= 1.0));
        }
    } else if let Some(debt) = f64_val(data, "debt_ratio") {
        out.push_str(&render_h_bar("资产负债率", debt, 100.0, "%", debt < 60.0));
    }
    out
}

fn chart_valuation(data: &Value, raw_dims: &Value) -> String {
    let mut out = String::new();
    if let Some(pct) = f64_val(data, "pe_percentile") {
        out.push_str(&render_svg_percentile(pct));
    }
    if let (Some(pe), Some(ind_pe)) = (
        f64_val(data, "pe_ttm").or_else(|| {
            dim_data(raw_dims, "0_basic")
                .get("pe_ttm")
                .and_then(|v| v.as_f64())
        }),
        f64_val(data, "industry_pe").or_else(|| {
            dim_data(raw_dims, "7_industry")
                .get("industry_pe")
                .and_then(|v| v.as_f64())
        }),
    ) && pe > 0.0
        && ind_pe > 0.0
    {
        out.push_str(&render_pe_relative_bar(pe, ind_pe));
    }
    if let Some(pb_pct) = f64_val(data, "pb_percentile") {
        out.push_str(&render_h_bar("PB 分位", pb_pct, 100.0, "%", pb_pct < 50.0));
    }
    out
}

fn chart_kline(data: &Value) -> String {
    let candles = parse_ohlc_candles(data.get("recent_candles"));
    let stage = str_val(data, "stage");
    let ma_align = str_val(data, "ma_align");
    let dd = data
        .get("kline_stats")
        .and_then(|v| v.get("max_drawdown"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let rsi = f64_val(data, "rsi14");
    let mut out = String::new();
    if candles.len() >= 5 {
        out.push_str(&format!(
            r#"<div class="viz-caption">🕯️ 日 K（近 {} 根）</div>{}"#,
            candles.len(),
            render_candlestick_chart(&candles, 320, 150)
        ));
    }
    let mut kpis = vec![
        ("📈", "阶段", stage.unwrap_or_else(|| "—".into())),
        ("📐", "均线", ma_align.unwrap_or_else(|| "—".into())),
    ];
    if let Some(r) = rsi {
        kpis.push(("⚡", "RSI14", format!("{r:.1}")));
    }
    kpis.push(("📉", "最大回撤", format!("{dd:.1}%")));
    out.push_str(&render_kpi_grid(&kpis));
    if let (Some(ma5), Some(ma20), Some(ma60)) = (
        f64_val(data, "ma5"),
        f64_val(data, "ma20"),
        f64_val(data, "ma60"),
    ) {
        out.push_str(&render_ma_compare(ma5, ma20, ma60));
    }
    out
}

fn chart_capital_flow(data: &Value) -> String {
    let main = f64_val(data, "main_fund_5d_net_yi");
    let holder = f64_val(data, "holder_change_ratio");
    let mut out = String::new();
    if let Some(v) = main {
        let good = v > 0.0;
        out.push_str(&render_h_bar(
            "主力5日",
            v.abs(),
            v.abs().max(5.0),
            "亿",
            good,
        ));
        out.push_str(&format!(
            r#"<div class="viz-caption" style="color:{}">{}{:.2} 亿</div>"#,
            if good { GREEN } else { RED },
            if good { "净流入 " } else { "净流出 " },
            v
        ));
    }
    if let Some(h) = holder {
        out.push_str(&render_h_bar(
            "户数变化",
            h.abs(),
            h.abs().max(20.0),
            "%",
            h < 0.0,
        ));
    }
    if out.is_empty() {
        out = render_kpi_grid(&[("💰", "资金流", "暂无近期数据".into())]);
    }
    out
}

fn chart_peers(data: &Value) -> String {
    let table = data.get("peer_table").and_then(|v| v.as_array());
    let Some(rows) = table else {
        return String::new();
    };
    if rows.is_empty() {
        return String::new();
    }
    let mut bars = String::new();
    for row in rows.iter().take(5) {
        let name = row.get("name").and_then(|v| v.as_str()).unwrap_or("—");
        let pe = row.get("pe").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if pe <= 0.0 {
            continue;
        }
        bars.push_str(&render_h_bar(
            &truncate_label(name, 6),
            pe,
            80.0,
            "",
            pe < 30.0,
        ));
    }
    if bars.is_empty() {
        return String::new();
    }
    format!(r#"<div class="viz-caption">同业 PE 对比</div>{bars}"#)
}

fn chart_industry(data: &Value, raw_dims: &Value) -> String {
    let industry = str_val(data, "industry").unwrap_or_else(|| "—".into());
    let growth = f64_val(data, "growth");
    let ind_pe = f64_val(data, "industry_pe");
    let company_pe = f64_val(&dim_data(raw_dims, "0_basic"), "pe_ttm")
        .or_else(|| f64_val(&dim_data(raw_dims, "10_valuation"), "pe_ttm"));
    let mut items = vec![("🏭", "行业", industry)];
    if let Some(g) = growth {
        items.push(("📈", "景气增速", format!("{g:+.1}%")));
    }
    if let Some(p) = ind_pe {
        items.push(("📊", "行业PE", format!("{p:.1}")));
    }
    let mut out = render_kpi_grid(&items);
    if let (Some(c), Some(i)) = (company_pe, ind_pe)
        && c > 0.0
        && i > 0.0
    {
        out.push_str(&render_pe_relative_bar(c, i));
    }
    out
}

fn chart_research(data: &Value) -> String {
    let count = data
        .get("research_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let buy = data
        .get("rating_buy_pct")
        .and_then(|v| v.as_f64())
        .or_else(|| data.get("buy_pct").and_then(|v| v.as_f64()));
    let mut items = vec![("📑", "覆盖券商", format!("{count} 家"))];
    if let Some(p) = buy {
        items.push(("👍", "买入占比", format!("{p:.0}%")));
    }
    render_kpi_grid(&items)
}

fn chart_fund_holders(data: &Value) -> String {
    let chg = f64_val(data, "holder_change_ratio");
    let cnt = data
        .get("holder_count")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            data.get("holder_count")
                .and_then(|v| v.as_f64())
                .map(|f| f as u64)
        });
    let mut items: Vec<(&str, &str, String)> = Vec::new();
    if let Some(c) = cnt {
        items.push(("👥", "股东户数", format!("{c}")));
    }
    if let Some(h) = chg {
        items.push(("📉", "户数变化", format!("{h:+.1}%")));
    }
    if items.is_empty() {
        return String::new();
    }
    let mut out = render_kpi_grid(&items);
    if let Some(h) = chg {
        out.push_str(&render_h_bar(
            "变化幅度",
            h.abs(),
            h.abs().max(15.0),
            "%",
            h < 0.0,
        ));
    }
    out
}

fn chart_events(data: &Value) -> String {
    let ann = data
        .get("announcement_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let news = data.get("news_count").and_then(|v| v.as_u64()).unwrap_or(0);
    if ann == 0 && news == 0 {
        return String::new();
    }
    let max = ann.max(news).max(1) as f64;
    format!(
        r#"<div class="viz-caption">事件密度</div>
{ann_bar}{news_bar}"#,
        ann_bar = render_h_bar("公告", ann as f64, max, "条", true),
        news_bar = render_h_bar("新闻", news as f64, max, "条", true),
    )
}

fn chart_lhb(data: &Value) -> String {
    let count = data
        .get("lhb_count_30d")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let youzi = data
        .get("matched_youzi")
        .and_then(|v| v.as_u64())
        .or_else(|| data.get("youzi_seats").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    render_kpi_grid(&[
        ("🐉", "30日上榜", format!("{count} 次")),
        ("🎯", "游资席位", format!("{youzi} 次")),
    ])
}

fn chart_trap(data: &Value) -> String {
    let promo = data
        .get("promo_hits")
        .and_then(|v| v.as_u64())
        .or_else(|| data.get("promotion_count").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let icon = if promo == 0 { "🟢" } else { "🔴" };
    let msg = if promo == 0 {
        "未发现推广痕迹"
    } else {
        "检测到推广内容"
    };
    render_kpi_grid(&[(icon, "推广检测", msg.into())])
}

fn chart_external_bullets(title: &str, bullets: &[String]) -> String {
    if bullets.is_empty() {
        return String::new();
    }
    let items: Vec<(&str, &str, String)> = bullets
        .iter()
        .take(4)
        .enumerate()
        .map(|(i, b)| {
            let icon = match i {
                0 => "📌",
                1 => "📰",
                2 => "💬",
                _ => "•",
            };
            (icon, title, truncate_label(b, 28))
        })
        .collect();
    render_kpi_grid(&items)
}

fn chart_neutral_grid(key: &str) -> String {
    let label = match key {
        "5_chain" => "产业链待定性",
        "8_materials" => "原材料成本",
        "9_futures" => "期货关联度",
        "11_governance" => "治理结构",
        "14_moat" => "护城河",
        "19_contests" => "实盘比赛",
        _ => "待 web 补数",
    };
    render_kpi_grid(&[("ℹ️", "状态", label.into())])
}

fn render_kpi_grid(items: &[(&str, &str, String)]) -> String {
    let cells: String = items
        .iter()
        .map(|(icon, k, v)| {
            format!(
                r#"<div class="dim-kpi"><div class="k">{icon} {k}</div><div class="v">{v}</div></div>"#
            )
        })
        .collect();
    format!(r#"<div class="dim-kpis">{cells}</div>"#)
}

fn render_h_bar(label: &str, value: f64, max: f64, unit: &str, good: bool) -> String {
    let pct = (value / max.max(0.01) * 100.0).clamp(4.0, 100.0);
    let color = if good {
        GREEN
    } else if value > max * 0.7 {
        RED
    } else {
        GOLD
    };
    format!(
        r#"<div class="h-bar-row"><div class="lbl">{label}</div>
<div class="track"><div class="fill" style="width:{pct:.0}%;background:{color}"></div></div>
<div class="val">{value:.1}{unit}</div></div>"#
    )
}

fn render_ma_compare(ma5: f64, ma20: f64, ma60: f64) -> String {
    format!(
        r#"<div class="viz-caption" style="margin-top:6px">均线结构</div>
<div class="h-bar-row"><div class="lbl">MA5</div><div class="track"><div class="fill" style="width:100%;background:{CYAN}"></div></div><div class="val">{ma5:.2}</div></div>
<div class="h-bar-row"><div class="lbl">MA20</div><div class="track"><div class="fill" style="width:{w20:.0}%;background:{GOLD}"></div></div><div class="val">{ma20:.2}</div></div>
<div class="h-bar-row"><div class="lbl">MA60</div><div class="track"><div class="fill" style="width:{w60:.0}%;background:{GREEN}"></div></div><div class="val">{ma60:.2}</div></div>"#,
        w20 = (ma20 / ma5 * 100.0).clamp(20.0, 100.0),
        w60 = (ma60 / ma5 * 100.0).clamp(20.0, 100.0),
    )
}

fn render_bar_chart(values: &[f64], labels: &[String], width: u32, height: u32) -> String {
    let n = values.len();
    if n == 0 {
        return String::new();
    }
    let max = values.iter().copied().fold(0.0_f64, f64::max).max(1.0);
    let pad_l = 28u32;
    let pad_b = 22u32;
    let chart_w = width.saturating_sub(pad_l + 8);
    let chart_h = height.saturating_sub(pad_b + 8);
    let bar_w = (chart_w as f64 / n as f64 * 0.65).max(8.0);
    let gap = chart_w as f64 / n as f64;
    let mut rects = String::new();
    let mut texts = String::new();
    let mut xlabels = String::new();
    for (i, &v) in values.iter().enumerate() {
        let x = pad_l as f64 + gap * (i as f64 + 0.5) - bar_w / 2.0;
        let h = (v / max * chart_h as f64).max(2.0);
        let y = pad_b as f64 + chart_h as f64 - h;
        rects.push_str(&format!(
            r#"<rect x="{x:.1}" y="{y:.1}" width="{bar_w:.1}" height="{h:.1}" fill="{CYAN}" rx="2"/>"#
        ));
        texts.push_str(&format!(
            r#"<text x="{cx:.1}" y="{ty:.1}" text-anchor="middle" font-size="8" fill="{INK}" font-weight="700">{v:.1}</text>"#,
            cx = x + bar_w / 2.0,
            ty = y - 3.0,
        ));
        let lbl = labels.get(i).map(String::as_str).unwrap_or("");
        xlabels.push_str(&format!(
            r#"<text x="{cx:.1}" y="{ylbl}" text-anchor="middle" font-size="8" fill="{MUTED}">{lbl}</text>"#,
            cx = x + bar_w / 2.0,
            ylbl = height - 4,
        ));
    }
    format!(
        r#"<svg width="{width}" height="{height}" viewBox="0 0 {width} {height}">{rects}{texts}{xlabels}</svg>"#
    )
}

fn render_sparkline(values: &[f64], color: &str, width: u32, height: u32) -> String {
    let n = values.len();
    if n < 2 {
        return String::new();
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(0.01);
    let w = width as f64 - 8.0;
    let h = height as f64 - 8.0;
    let points: Vec<String> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = 4.0 + w * i as f64 / (n - 1) as f64;
            let y = 4.0 + h * (1.0 - (v - min) / span);
            format!("{x:.1},{y:.1}")
        })
        .collect();
    let last = values.last().copied().unwrap_or(0.0);
    let lx = 4.0 + w;
    let ly = 4.0 + h * (1.0 - (last - min) / span);
    let tx = width as f64 - 4.0;
    format!(
        r#"<svg width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<polyline points="{pts}" fill="none" stroke="{color}" stroke-width="2" stroke-linejoin="round"/>
<circle cx="{lx:.1}" cy="{ly:.1}" r="2.5" fill="{color}"/>
<text x="{tx:.1}" y="10" text-anchor="end" font-size="9" fill="{INK}" font-weight="700">{last:.1}%</text>
</svg>"#,
        pts = points.join(" "),
    )
}

fn f64_array(v: &Value, key: &str) -> Vec<f64> {
    v.get(key)
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
        .unwrap_or_default()
}

fn f64_val(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64())
}

fn str_val(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

fn year_labels(n: usize) -> Vec<String> {
    const BASE: i32 = 2026;
    (0..n)
        .map(|i| format!("{}", BASE - (n as i32 - 1 - i as i32)))
        .collect()
}

fn truncate_label(s: &str, max_chars: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max_chars {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(max_chars).collect::<String>())
    }
}

#[derive(Debug, Clone, Copy)]
struct OhlcCandle {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

fn parse_ohlc_candles(value: Option<&Value>) -> Vec<OhlcCandle> {
    let Some(arr) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|row| {
            Some(OhlcCandle {
                open: row.get("o").or_else(|| row.get("open"))?.as_f64()?,
                high: row.get("h").or_else(|| row.get("high"))?.as_f64()?,
                low: row.get("l").or_else(|| row.get("low"))?.as_f64()?,
                close: row.get("c").or_else(|| row.get("close"))?.as_f64()?,
            })
        })
        .collect()
}

fn chart_macro_panel(data: &Value, external: &ExternalBlock) -> String {
    let mut tiles = [
        (
            "📉",
            "利率",
            str_val(data, "rate_cycle").unwrap_or_else(|| "中性（货币政策）".into()),
        ),
        (
            "💱",
            "汇率",
            str_val(data, "fx_trend").unwrap_or_else(|| "中性（人民币走势）".into()),
        ),
        (
            "🌐",
            "地缘",
            str_val(data, "geo_risk").unwrap_or_else(|| "中性（地缘风险）".into()),
        ),
        (
            "📦",
            "大宗",
            str_val(data, "commodity").unwrap_or_else(|| "中性（大宗周期）".into()),
        ),
    ];
    if tiles.iter().all(|(_, _, v)| v.starts_with("中性")) {
        for (i, bullet) in external.macro_bullets.iter().take(4).enumerate() {
            tiles[i].2 = truncate_label(bullet, 36);
        }
    }
    render_macro_quad(&tiles)
}

fn render_macro_quad(tiles: &[(&str, &str, String)]) -> String {
    let cells: String = tiles
        .iter()
        .map(|(icon, label, value)| {
            format!(
                r#"<div class="macro-cell"><div class="macro-icon">{icon}</div><div class="macro-k">{label}</div><div class="macro-v">{value}</div></div>"#
            )
        })
        .collect();
    format!(r#"<div class="macro-quad">{cells}</div>"#)
}

fn render_candlestick_chart(candles: &[OhlcCandle], width: u32, height: u32) -> String {
    let n = candles.len();
    if n == 0 {
        return String::new();
    }
    let pad_l = 8u32;
    let pad_r = 8u32;
    let pad_t = 8u32;
    let pad_b = 14u32;
    let chart_w = width.saturating_sub(pad_l + pad_r);
    let chart_h = height.saturating_sub(pad_t + pad_b);
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for c in candles {
        lo = lo.min(c.low);
        hi = hi.max(c.high);
    }
    let span = (hi - lo).max(0.01);
    let gap = chart_w as f64 / n as f64;
    let body_w = (gap * 0.55).max(2.0);
    let mut parts = String::new();
    for (i, c) in candles.iter().enumerate() {
        let cx = pad_l as f64 + gap * (i as f64 + 0.5);
        let y = |price: f64| pad_t as f64 + chart_h as f64 * (1.0 - (price - lo) / span);
        let y_high = y(c.high);
        let y_low = y(c.low);
        let y_open = y(c.open);
        let y_close = y(c.close);
        let bullish = c.close >= c.open;
        let color = if bullish { RED } else { GREEN };
        parts.push_str(&format!(
            r#"<line x1="{cx:.1}" y1="{y_high:.1}" x2="{cx:.1}" y2="{y_low:.1}" stroke="{color}" stroke-width="1"/>"#
        ));
        let top = y_open.min(y_close);
        let body_h = (y_open - y_close).abs().max(1.0);
        parts.push_str(&format!(
            r#"<rect x="{x:.1}" y="{top:.1}" width="{body_w:.1}" height="{body_h:.1}" fill="{color}" stroke="{color}" stroke-width="1"/>"#,
            x = cx - body_w / 2.0,
        ));
    }
    format!(
        r#"<svg width="{width}" height="{height}" viewBox="0 0 {width} {height}">{parts}</svg>"#
    )
}

fn render_revenue_net_profit_combo(
    revenue: &[f64],
    net_profit: &[f64],
    labels: &[String],
    width: u32,
    height: u32,
) -> String {
    let n = revenue.len().min(net_profit.len());
    if n == 0 {
        return String::new();
    }
    let rev = &revenue[..n];
    let np = &net_profit[..n];
    let pad_l = 28u32;
    let pad_r = 36u32;
    let pad_b = 22u32;
    let chart_w = width.saturating_sub(pad_l + pad_r);
    let chart_h = height.saturating_sub(pad_b + 8);
    let max_rev = rev.iter().copied().fold(0.0_f64, f64::max).max(1.0);
    let max_np = np.iter().map(|v| v.abs()).fold(0.0_f64, f64::max).max(1.0);
    let gap = chart_w as f64 / n as f64;
    let bar_w = (gap * 0.55).max(6.0);
    let mut rects = String::new();
    let mut line_pts = String::new();
    let mut np_labels = String::new();
    for (i, (&r, &p)) in rev.iter().zip(np.iter()).enumerate() {
        let x = pad_l as f64 + gap * (i as f64 + 0.5) - bar_w / 2.0;
        let h = (r / max_rev * chart_h as f64).max(2.0);
        let y = pad_b as f64 + chart_h as f64 - h;
        rects.push_str(&format!(
            r#"<rect x="{x:.1}" y="{y:.1}" width="{bar_w:.1}" height="{h:.1}" fill="{CYAN}" rx="2"/>"#
        ));
        let py = pad_b as f64 + chart_h as f64 / 2.0 - (p / max_np) * (chart_h as f64 / 2.0 - 4.0);
        let cx = pad_l as f64 + gap * (i as f64 + 0.5);
        if i == 0 {
            line_pts.push_str(&format!("{cx:.1},{py:.1}"));
        } else {
            line_pts.push_str(&format!(" {cx:.1},{py:.1}"));
        }
        np_labels.push_str(&format!(
            r#"<circle cx="{cx:.1}" cy="{py:.1}" r="2.5" fill="{GOLD}"/>"#
        ));
        let lbl = labels.get(i).map(String::as_str).unwrap_or("");
        let ylbl = height - 4;
        rects.push_str(&format!(
            r#"<text x="{cx:.1}" y="{ylbl}" text-anchor="middle" font-size="8" fill="{MUTED}">{lbl}</text>"#,
            cx = cx,
        ));
    }
    let label_x = width - 4;
    format!(
        r#"<svg width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<text x="{label_x}" y="10" text-anchor="end" font-size="8" fill="{GOLD}">净利→</text>
{rects}
<polyline points="{line_pts}" fill="none" stroke="{GOLD}" stroke-width="2.5" stroke-linejoin="round"/>
{np_labels}
</svg>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn financials_combo_renders() {
        let data = json!({
            "revenue_history": [18.0, 24.0, 36.0],
            "net_profit_history": [4.0, 5.0, 3.0],
            "roe_history": [1.0, 2.0, -5.0],
            "financial_health": { "debt_ratio": 42.0, "current_ratio": 1.4 }
        });
        let html = chart_financials(&data);
        assert!(html.contains("金线=净利"));
        assert!(html.contains("<svg"));
    }

    #[test]
    fn financials_revenue_bar_renders() {
        let data = json!({
            "revenue_history": [18.0, 24.0, 36.0],
            "roe_history": [1.0, 2.0, -5.0],
            "financial_health": { "debt_ratio": 42.0, "current_ratio": 1.4 }
        });
        let html = chart_financials(&data);
        assert!(html.contains("<svg"));
        assert!(html.contains("资产负债率"));
    }

    #[test]
    fn candlestick_renders() {
        let candles = vec![
            OhlcCandle {
                open: 10.0,
                high: 11.0,
                low: 9.5,
                close: 10.5,
            };
            20
        ];
        let svg = render_candlestick_chart(&candles, 300, 120);
        assert!(svg.contains("<rect"));
        assert!(svg.contains("<line"));
    }

    #[test]
    fn macro_quad_renders() {
        let data = json!({
            "rate_cycle": "降息周期",
            "fx_trend": "人民币偏弱",
            "geo_risk": "中性",
            "commodity": "周期底部"
        });
        let html = chart_macro_panel(&data, &ExternalBlock::default());
        assert!(html.contains("macro-quad"));
        assert!(html.contains("降息周期"));
    }

    #[test]
    fn events_dual_bar_renders() {
        let data = json!({ "announcement_count": 10, "news_count": 8 });
        let html = chart_events(&data);
        assert!(html.contains("公告"));
        assert!(html.contains("新闻"));
    }

    #[test]
    fn empty_data_returns_empty_wrapper() {
        assert!(render_dim_chart("1_financials", &json!({}), &ExternalBlock::default()).is_empty());
    }
}
