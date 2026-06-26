//! Shared CSS for institutional HTML report.

#[must_use]
pub fn institutional_styles() -> &'static str {
    r#"
:root { --ink:#1e293b; --muted:#64748b; --line:#e2e8f0; --bg:#f8fafc; }
* { box-sizing:border-box; }
body { font-family:"Segoe UI",system-ui,sans-serif; margin:0; color:var(--ink); background:var(--bg); }
.wrap { max-width:920px; margin:0 auto; padding:1.5rem; }
.hero { background:#fff; border:1px solid var(--line); border-radius:12px; padding:1.25rem 1.5rem; margin-bottom:1rem; }
.hero h1 { margin:0 0 .5rem; font-size:1.35rem; }
.hero-chips { display:flex; flex-wrap:wrap; gap:.4rem; margin:.75rem 0 0; }
.chip-data { font-size:.78rem; padding:.2rem .55rem; border-radius:999px; background:var(--bg); border:1px solid var(--line); color:var(--ink); }
.badge { display:inline-block; padding:.2rem .65rem; border-radius:999px; font-size:.85rem; font-weight:600; }
.badge-strong-buy { background:#dcfce7; color:#166534; }
.badge-buy { background:#dbeafe; color:#1d4ed8; }
.badge-watch { background:#fef9c3; color:#854d0e; }
.badge-avoid { background:#fee2e2; color:#991b1b; }
.badge-muted { background:#f1f5f9; color:#475569; }
.sub { color:var(--muted); font-size:.95rem; margin:.35rem 0 0; }
.banner { background:#fff7ed; border:1px solid #fed7aa; color:#9a3412; padding:.75rem 1rem; border-radius:8px; margin-bottom:1rem; }
.card { background:#fff; border:1px solid var(--line); border-radius:10px; padding:1rem 1.25rem; margin-bottom:1rem; }
.card h2 { margin:0 0 .75rem; font-size:1.05rem; }
.card h3 { margin:1rem 0 .5rem; font-size:.95rem; color:var(--muted); }
.metrics { display:grid; grid-template-columns:repeat(auto-fill,minmax(160px,1fr)); gap:.75rem; }
.metric { background:var(--bg); border-radius:8px; padding:.65rem .75rem; }
.metric .k { font-size:.78rem; color:var(--muted); }
.metric .v { font-size:1rem; font-weight:600; margin-top:.15rem; }
table { width:100%; border-collapse:collapse; font-size:.9rem; }
th,td { border-bottom:1px solid var(--line); padding:.45rem .35rem; text-align:left; vertical-align:middle; }
th { color:var(--muted); font-weight:600; }
.dim-bar { display:inline-block; width:72px; height:8px; background:#e2e8f0; border-radius:4px; overflow:hidden; vertical-align:middle; }
.dim-fill { display:block; height:100%; border-radius:4px; }
.chips { display:flex; flex-wrap:wrap; gap:.35rem; }
.chip-missing { background:#fef2f2; color:#991b1b; border:1px solid #fecaca; font-size:.78rem; padding:.15rem .5rem; border-radius:999px; }
.bullets { margin:.25rem 0 0 1.1rem; padding:0; line-height:1.55; }
.muted-note { color:var(--muted); font-size:.9rem; font-style:italic; }
.gauges { margin:.75rem 0; display:flex; flex-wrap:wrap; gap:.75rem; }
ul.risk { margin:.25rem 0 0 1.1rem; padding:0; }
.narrative { line-height:1.6; white-space:pre-wrap; }
details.panel-details { margin-top:.75rem; }
details.panel-details summary { cursor:pointer; color:var(--muted); font-size:.9rem; }
.heat-low { background:#dcfce7; }
.heat-mid { background:#fef9c3; }
.heat-high { background:#fee2e2; }
.section-head { margin:1.5rem 0 1rem; }
.section-tag { font-family:Consolas,"Courier New",monospace; font-size:.72rem; letter-spacing:.14em; color:#0891b2; font-weight:700; }
.section-title { margin:.35rem 0 0; font-size:1.35rem; font-weight:800; letter-spacing:-.02em; }
.section-line { height:3px; width:64px; background:linear-gradient(90deg,#0891b2,#d97706); border-radius:2px; margin-top:.65rem; }
.scan-summary { color:var(--muted); font-size:.9rem; margin:0 0 1rem; }
.deep-scan { display:flex; flex-direction:column; gap:1.25rem; }
.cat-label { font-family:Consolas,"Courier New",monospace; font-size:.72rem; color:#0891b2; letter-spacing:.12em; padding:.45rem .85rem; background:#ecfeff; border-left:4px solid #0891b2; border-radius:0 8px 8px 0; font-weight:700; align-self:flex-start; }
.dim-row { display:grid; grid-template-columns:repeat(auto-fill,minmax(260px,1fr)); gap:.85rem; }
.dim-card { background:#fff; border:1px solid var(--line); border-radius:12px; padding:1rem 1.1rem .9rem; box-shadow:0 1px 2px rgba(15,23,42,.04); transition:border-color .15s,box-shadow .15s; }
.dim-card:hover { border-color:#0891b2; box-shadow:0 4px 12px rgba(15,23,42,.06); }
.dim-head { display:flex; align-items:flex-start; justify-content:space-between; gap:.75rem; margin-bottom:.5rem; }
.dim-num { font-family:Consolas,"Courier New",monospace; font-size:.62rem; color:var(--muted); letter-spacing:.1em; margin-bottom:.15rem; }
.dim-title { font-weight:800; font-size:1rem; color:var(--ink); }
.dim-en { font-family:Consolas,"Courier New",monospace; font-size:.62rem; color:var(--muted); letter-spacing:.08em; margin-top:.1rem; }
.dim-score { text-align:right; flex-shrink:0; }
.dim-score .num { font-weight:900; font-size:1.75rem; line-height:1; letter-spacing:-.03em; }
.dim-score .num.high { color:#059669; }
.dim-score .num.mid { color:#d97706; }
.dim-score .num.low { color:#dc2626; }
.dim-card .dim-bar { height:6px; background:#f1f5f9; border-radius:3px; overflow:hidden; margin-bottom:.65rem; }
.dim-card .dim-bar .fill { height:100%; border-radius:3px; }
.dim-card .dim-bar .fill.high { background:#059669; }
.dim-card .dim-bar .fill.mid { background:#d97706; }
.dim-card .dim-bar .fill.low { background:#dc2626; }
.dim-viz { margin:.65rem 0; padding:.75rem; background:#f8fafc; border:1px solid var(--line); border-radius:8px; }
.dim-viz svg { display:block; max-width:100%; height:auto; }
.dim-kpis { display:grid; grid-template-columns:repeat(2,1fr); gap:.35rem; margin-bottom:.35rem; }
.dim-kpi { padding:.45rem .55rem; background:#fff; border:1px solid var(--line); border-radius:6px; text-align:center; }
.dim-kpi .k { font-size:.58rem; color:var(--muted); letter-spacing:.08em; }
.dim-kpi .v { font-size:.78rem; font-weight:700; margin-top:.1rem; color:var(--ink); }
.dim-kpi .v.up { color:#059669; }
.dim-kpi .v.down { color:#dc2626; }
.h-bar-row { display:flex; align-items:center; gap:.5rem; margin:.25rem 0; font-size:.72rem; }
.h-bar-row .lbl { width:4.5rem; color:var(--muted); flex-shrink:0; }
.h-bar-row .track { flex:1; height:8px; background:#f1f5f9; border-radius:4px; overflow:hidden; }
.h-bar-row .track .fill { height:100%; border-radius:4px; }
.h-bar-row .val { min-width:2.5rem; text-align:right; font-weight:700; font-size:.72rem; }
.viz-caption { font-size:.62rem; color:var(--muted); margin-bottom:.25rem; }
.dim-label { font-size:.88rem; color:var(--ink); font-weight:600; line-height:1.45; margin-bottom:.5rem; }
.dim-pass-fail { font-size:.78rem; color:var(--muted); line-height:1.55; margin-top:.35rem; }
.dim-pass-fail .pass { color:#059669; }
.dim-pass-fail .fail { color:#dc2626; }
.dim-pass-fail ul { margin:.2rem 0 0; padding-left:1rem; }
.dim-source { margin-top:.5rem; padding-top:.45rem; border-top:1px dashed var(--line); font-size:.72rem; color:var(--muted); }
.dim-raw { margin-top:.45rem; }
.dim-raw summary { cursor:pointer; font-family:Consolas,"Courier New",monospace; font-size:.62rem; color:#0891b2; letter-spacing:.08em; }
.dim-raw pre { font-size:.68rem; max-height:200px; overflow:auto; margin:.35rem 0 0; padding:.5rem; background:#f8fafc; border:1px solid var(--line); border-radius:6px; white-space:pre-wrap; word-break:break-all; }
.scan-footnote { color:var(--muted); font-size:.82rem; margin:.75rem 0 0; font-style:italic; }
.badge-live { display:inline-block; padding:.1rem .4rem; border-radius:999px; background:#dcfce7; color:#166534; font-weight:600; }
.badge-web { display:inline-block; padding:.1rem .4rem; border-radius:999px; background:#dbeafe; color:#1d4ed8; font-weight:600; }
.macro-quad { display:grid; grid-template-columns:repeat(2,1fr); gap:.45rem; }
.macro-cell { padding:.55rem .45rem; background:#fff; border:1px solid var(--line); border-radius:8px; text-align:center; }
.macro-icon { font-size:1.1rem; margin-bottom:.15rem; }
.macro-k { font-size:.58rem; color:var(--muted); letter-spacing:.08em; }
.macro-v { font-size:.72rem; font-weight:600; color:var(--ink); margin-top:.15rem; line-height:1.35; }
.dcf-block { background:#fff; border:1px solid var(--line); border-radius:12px; padding:1.1rem 1.25rem; box-shadow:0 1px 2px rgba(15,23,42,.04); }
.dcf-head { display:flex; justify-content:space-between; align-items:baseline; border-bottom:2px solid #0891b2; padding-bottom:.55rem; margin-bottom:.9rem; }
.dcf-badge { background:#0891b2; color:#fff; padding:.2rem .55rem; border-radius:4px; font-size:.68rem; font-weight:700; letter-spacing:.08em; }
.dcf-subtitle { margin-left:.65rem; font-size:.85rem; color:var(--muted); }
.dcf-summary { display:grid; grid-template-columns:repeat(4,1fr); gap:1rem; margin-bottom:.9rem; }
.dcf-kpi .k { font-size:.68rem; color:var(--muted); }
.dcf-kpi .v { font-size:1.35rem; font-weight:800; margin-top:.1rem; }
.dcf-kpi .v.sm-pos { color:#059669; }
.dcf-kpi .v.sm-mid { color:#d97706; }
.dcf-kpi .v.sm-neg { color:#dc2626; }
.dcf-kpi .hint { font-size:.62rem; color:var(--muted); margin-top:.15rem; line-height:1.35; }
.dcf-methodology { margin-bottom:.75rem; }
.dcf-methodology summary { cursor:pointer; color:#0369a1; font-weight:600; font-size:.82rem; }
.dcf-methodology ol { margin:.5rem 0 0 1.25rem; color:#374151; font-size:.82rem; line-height:1.65; }
.dcf-sens-title { font-size:.75rem; color:var(--muted); margin-bottom:.35rem; }
table.sens-heatmap { width:auto; font-size:.82rem; margin:.5rem 0; }
table.sens-heatmap th, table.sens-heatmap td { border:1px solid var(--line); padding:.35rem .5rem; text-align:center; }
table.sens-heatmap th { background:#f3f4f6; font-size:.72rem; }
table.sens-heatmap td { font-weight:700; }
.sens-deep-under { background:#065f46; color:#fff; }
.sens-under { background:#10b981; color:#fff; }
.sens-fair { background:#e5e7eb; color:#111; }
.sens-over { background:#f97316; color:#fff; }
.sens-deep-over { background:#b91c1c; color:#fff; }
.dashboard-bento { display:grid; grid-template-columns:repeat(4,1fr); gap:.75rem; margin-bottom:1rem; }
.core-conclusion { grid-column:1/-1; background:linear-gradient(135deg,#ecfeff,#fff); border:1px solid var(--line); border-radius:12px; padding:1rem 1.15rem; }
.core-conclusion .label { font-family:Consolas,"Courier New",monospace; font-size:.62rem; color:#0891b2; letter-spacing:.12em; margin-bottom:.35rem; }
.core-conclusion .text { font-size:.95rem; line-height:1.55; white-space:pre-wrap; font-weight:600; }
.data-cell { background:#fff; border:1px solid var(--line); border-radius:10px; padding:.75rem .85rem; min-height:4.5rem; }
.data-cell.span-2 { grid-column:span 2; }
.data-cell .icon { font-size:1.1rem; margin-bottom:.2rem; }
.data-cell .key { font-family:Consolas,"Courier New",monospace; font-size:.58rem; color:var(--muted); letter-spacing:.1em; }
.data-cell .value { font-size:.82rem; font-weight:600; margin-top:.25rem; line-height:1.4; }
.core-metrics { grid-column:1/-1; display:grid; grid-template-columns:repeat(auto-fill,minmax(120px,1fr)); gap:.5rem; margin-top:.15rem; }
.core-metric { background:var(--bg); border:1px solid var(--line); border-radius:8px; padding:.45rem .55rem; text-align:center; }
.core-metric .k { font-size:.62rem; color:var(--muted); }
.core-metric .v { font-size:.78rem; font-weight:700; margin-top:.1rem; }
"#
}
