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
"#
}
