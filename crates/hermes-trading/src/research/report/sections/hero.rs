//! Hero + document shell.

use crate::research::report::identity::ReportIdentity;
use crate::research::report::sections::util::escape_html;
use crate::research::report::styles::institutional_styles;
use crate::research::synthesis::SynthesisReport;

use super::super::dim_viz::{verdict_badge_class, verdict_label_zh};

#[must_use]
pub fn render_shell_start(identity: &ReportIdentity, syn: &SynthesisReport) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{}</title>
<style>{css}</style>
</head>
<body><div class="wrap">
{hero}
"#,
        escape_html(&identity.html_document_title()),
        css = institutional_styles(),
        hero = render_hero(identity, syn),
    )
}

#[must_use]
pub fn render_warn_banner(confidence: f64) -> String {
    format!(
        r#"<div class="banner"><strong>数据置信度 {:.0}%</strong> — 基于公开行情与财报数据；政策/舆情需 web 补数时见对应章节。</div>"#,
        confidence * 100.0,
    )
}

fn render_hero(identity: &ReportIdentity, syn: &SynthesisReport) -> String {
    let badge_class = verdict_badge_class(&syn.verdict);
    let badge_label = verdict_label_zh(&syn.verdict);
    let mut chips = String::new();
    if let Some(price) = identity.price {
        chips.push_str(&chip(&format!("现价 ¥{price:.2}")));
    }
    if let Some(cap) = identity.market_cap_yi {
        chips.push_str(&chip(&format!("市值 {cap:.0} 亿")));
    }
    if let Some(pe) = identity.pe {
        chips.push_str(&chip(&format!("PE {pe:.1}")));
    }
    if let Some(pb) = identity.pb {
        chips.push_str(&chip(&format!("PB {pb:.2}")));
    }
    if let Some(score) = identity.fundamental_score {
        chips.push_str(&chip(&format!("Alpha {:.1}/100", score)));
    }
    if let Some(ind) = &identity.industry {
        chips.push_str(&chip(ind));
    }

    format!(
        r#"<section class="hero">
<h1>{title}</h1>
<p><span class="badge {badge_class}">{badge_label}</span>
<span class="sub">{headline}</span></p>
<div class="hero-chips">{chips}</div>
</section>"#,
        title = escape_html(&identity.title_prefix()),
        headline = escape_html(&syn.headline),
        badge_class = badge_class,
        badge_label = badge_label,
    )
}

fn chip(label: &str) -> String {
    format!(r#"<span class="chip-data">{}</span>"#, escape_html(label))
}
