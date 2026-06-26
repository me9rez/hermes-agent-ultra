/// Horizontal score bar (0–max), pure CSS + inline width.
#[must_use]
pub fn render_dim_bar(score: u8, max: u8) -> String {
    let pct = (f64::from(score) / f64::from(max) * 100.0).clamp(0.0, 100.0);
    let color = if score >= 7 {
        "#16a34a"
    } else if score <= 4 {
        "#dc2626"
    } else {
        "#ca8a04"
    };
    format!(
        r#"<span class="dim-bar" title="{score}/{max}"><span class="dim-fill" style="width:{pct:.0}%;background:{color}"></span></span>"#
    )
}

#[must_use]
pub fn score_tier_class(score: u8) -> &'static str {
    if score >= 7 {
        "high"
    } else if score <= 4 {
        "low"
    } else {
        "mid"
    }
}

#[must_use]
pub fn render_weight_stars(weight: u8) -> String {
    let filled = weight.min(5);
    let empty = 5usize.saturating_sub(filled as usize);
    format!("{}{}", "★".repeat(filled as usize), "☆".repeat(empty))
}

/// Full-width bar for DEEP SCAN cards.
#[must_use]
pub fn render_deep_scan_bar(score: u8, max: u8) -> String {
    let pct = (f64::from(score) / f64::from(max) * 100.0).clamp(0.0, 100.0);
    let tier = score_tier_class(score);
    format!(r#"<div class="dim-bar"><div class="fill {tier}" style="width:{pct:.0}%"></div></div>"#)
}

/// Company PE vs industry PE relative bar.
#[must_use]
pub fn render_pe_relative_bar(company_pe: f64, industry_pe: f64) -> String {
    if industry_pe <= 0.0 || company_pe <= 0.0 {
        return String::new();
    }
    let ratio = (company_pe / industry_pe).clamp(0.2, 2.0);
    let pct = (ratio / 2.0 * 100.0).clamp(5.0, 100.0);
    let color = if ratio < 0.9 {
        "#16a34a"
    } else if ratio > 1.1 {
        "#dc2626"
    } else {
        "#ca8a04"
    };
    let bg = "#e2e8f0";
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="40" viewBox="0 0 200 40">
<text x="100" y="12" text-anchor="middle" font-size="9" fill="{muted}">PE 相对行业 ({company_pe:.1} / {industry_pe:.1})</text>
<rect x="20" y="18" width="160" height="10" fill="{bg}" rx="3"/>
<rect x="20" y="18" width="{pct:.0}" height="10" fill="{color}" rx="3"/>
<text x="100" y="36" text-anchor="middle" font-size="9" fill="{muted}">{ratio:.2}x</text>
</svg>"#,
        muted = "#64748b",
        bg = bg,
    )
}

/// Badge for missing dimension keys or data fields.
#[must_use]
pub fn render_missing_chip(label: &str) -> String {
    format!(r#"<span class="chip chip-missing">{label}</span>"#)
}

/// Verdict badge class suffix for institutional cover.
#[must_use]
pub fn verdict_badge_class(verdict: &str) -> &'static str {
    match verdict {
        "strongly_buy" => "badge-strong-buy",
        "buy" => "badge-buy",
        "avoid" => "badge-avoid",
        "insufficient_data" => "badge-muted",
        _ => "badge-watch",
    }
}

#[must_use]
pub fn verdict_label_zh(verdict: &str) -> &'static str {
    match verdict {
        "strongly_buy" => "强烈偏多",
        "buy" => "偏多",
        "avoid" => "偏空",
        "insufficient_data" => "数据不足",
        _ => "观望",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_scan_bar_uses_tier_class() {
        let s = render_deep_scan_bar(8, 10);
        assert!(s.contains("fill high"));
        assert!(s.contains("width:80%"));
    }

    #[test]
    fn weight_stars_five_of_five() {
        assert_eq!(render_weight_stars(5), "★★★★★");
        assert_eq!(render_weight_stars(3), "★★★☆☆");
    }

    #[test]
    fn dim_bar_width_reflects_score() {
        let s = render_dim_bar(8, 10);
        assert!(s.contains("width:80%"));
        assert!(s.contains("#16a34a"));
    }

    #[test]
    fn pe_relative_bar_renders() {
        let s = render_pe_relative_bar(28.5, 22.0);
        assert!(s.contains("svg"));
        assert!(s.contains("1.30"));
    }

    #[test]
    fn missing_chip_has_class() {
        assert!(render_missing_chip("fcf_latest_yi").contains("chip-missing"));
    }
}
