//! The stylesheet, built from the theme tokens. Served at `/static/app.css`; no inline styles
//! anywhere (the Content-Security-Policy allows only this file).

use std::sync::OnceLock;

use crate::theme::{DARK, LIGHT};

/// The full stylesheet.
pub fn stylesheet() -> &'static str {
    static CSS: OnceLock<String> = OnceLock::new();
    CSS.get_or_init(|| {
        format!(
            ":root{{{light}color-scheme:light dark;}}@media (prefers-color-scheme:dark){{:root{{{dark}}}}}{RULES}",
            light = LIGHT.css_vars(),
            dark = DARK.css_vars()
        )
    })
}

/// A short hash of the stylesheet, for cache-busting URLs.
pub fn version() -> String {
    // FNV-1a: tiny, deterministic, good enough to tell versions apart.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in stylesheet().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:08x}", h & 0xffff_ffff)
}

const RULES: &str = r#"
*,*::before,*::after{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--bg);color:var(--text);font:15px/1.5 system-ui,-apple-system,"Segoe UI",Roboto,"Helvetica Neue",Arial,sans-serif}
a{color:var(--primary)}
h1,h2,h3{line-height:1.25;margin:0 0 .5em}
h1{font-size:22px;font-weight:650}
h2{font-size:17px;font-weight:650}
h3{font-size:15px;font-weight:650}
p{margin:0 0 .75em}
code,.mono{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:13px}
.skip{position:absolute;left:-999px;top:0;background:var(--surface);padding:8px 12px;z-index:100}
.skip:focus{left:8px;top:8px}
.icon{width:18px;height:18px;flex:none;stroke:currentColor;fill:none;stroke-width:1.8;stroke-linecap:round;stroke-linejoin:round}
.icon.lg{width:28px;height:28px}

.topbar{position:sticky;top:0;z-index:20;height:52px;display:flex;align-items:center;gap:14px;padding:0 16px;background:var(--topbar);color:var(--topbar-ink)}
.topbar a{color:inherit;text-decoration:none}
.brand{display:flex;align-items:center;gap:9px;font-weight:700;font-size:16px;letter-spacing:.2px;white-space:nowrap}
.brand .icon{width:22px;height:22px;stroke-width:2}
.topbar .company{opacity:.75;font-size:14px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.topbar .spacer{flex:1}
.topbar .who{display:flex;align-items:center;gap:10px;font-size:14px}
.topbar .avatar{width:30px;height:30px;border-radius:50%;background:var(--primary);color:var(--primary-ink);display:grid;place-items:center;font-size:12px;font-weight:700}
.topbar form{margin:0}
.topbar button{background:transparent;border:1px solid rgba(255,255,255,.22);color:inherit;border-radius:8px;padding:5px 10px;font:inherit;font-size:13px;cursor:pointer;display:inline-flex;gap:6px;align-items:center}
.topbar button:hover{background:rgba(255,255,255,.08)}
.menu-button{display:none;cursor:pointer;padding:6px;border-radius:8px}
.nav-toggle{position:absolute;opacity:0;pointer-events:none}

.shell{display:grid;grid-template-columns:248px minmax(0,1fr);min-height:calc(100vh - 52px)}
.sidebar{background:var(--surface);border-right:1px solid var(--border);padding:10px 8px 24px;position:sticky;top:52px;height:calc(100vh - 52px);overflow-y:auto}
.sidebar .context{padding:10px 10px 6px;font-weight:650;font-size:14px;line-height:1.3}
.sidebar .context small{display:block;color:var(--muted);font-weight:500;font-size:12px;margin-top:2px}
.nav-group{margin:14px 0 4px;padding:0 10px;font-size:11px;font-weight:650;text-transform:uppercase;letter-spacing:.06em;color:var(--muted)}
.nav a{display:flex;align-items:center;gap:10px;padding:6px 10px;border-radius:8px;color:var(--text);text-decoration:none;font-size:14px}
.nav a:hover{background:var(--surface-2)}
.nav a.active{background:var(--primary-soft);color:var(--primary);font-weight:650}
.nav .icon{color:var(--muted)}
.nav a.active .icon{color:var(--primary)}
.main{padding:22px 28px 72px;min-width:0}
.content{max-width:1240px}

.crumbs{font-size:13px;color:var(--muted);margin-bottom:6px;display:flex;flex-wrap:wrap;gap:6px;align-items:center}
.crumbs a{color:inherit;text-decoration:none}
.crumbs a:hover{text-decoration:underline}
.page-head{display:flex;align-items:flex-start;justify-content:space-between;gap:16px;margin-bottom:18px;flex-wrap:wrap}
.page-head h1{margin:0;display:flex;align-items:center;gap:10px;flex-wrap:wrap}
.page-head .sub{color:var(--muted);font-size:14px;margin-top:4px}
.actions{display:flex;gap:8px;flex-wrap:wrap;align-items:center}
.actions form{margin:0}

.btn{display:inline-flex;align-items:center;gap:7px;padding:8px 14px;border-radius:8px;border:1px solid var(--border);background:var(--surface);color:var(--text);font:inherit;font-size:14px;font-weight:600;cursor:pointer;text-decoration:none;line-height:1.2;white-space:nowrap}
.btn:hover{background:var(--surface-2)}
.btn.primary{background:var(--primary);border-color:var(--primary);color:var(--primary-ink)}
.btn.primary:hover{filter:brightness(1.07)}
.btn.danger{color:var(--danger)}
.btn.danger:hover{background:var(--danger-soft)}
.btn.small{padding:5px 10px;font-size:13px}
.btn.link{border:0;background:none;padding:0;color:var(--primary);font-weight:600}
.btn .icon{width:16px;height:16px}

.card{background:var(--surface);border:1px solid var(--border);border-radius:12px;padding:18px 20px;margin-bottom:16px}
.card > h2:first-child,.card > h3:first-child{margin-top:0}
.card-head{display:flex;justify-content:space-between;align-items:center;gap:12px;margin-bottom:12px}
.card-head h2{margin:0}

.flash{padding:11px 16px;border-radius:10px;margin-bottom:16px;font-size:14px;border:1px solid transparent}
.flash.ok{background:var(--ok-soft);color:var(--ok)}
.flash.error{background:var(--danger-soft);color:var(--danger)}
.flash.info{background:var(--primary-soft);color:var(--primary)}

.table-wrap{background:var(--surface);border:1px solid var(--border);border-radius:12px;overflow-x:auto}
table.data{width:100%;border-collapse:collapse;font-size:14px}
table.data th{text-align:left;font-weight:650;font-size:12px;text-transform:uppercase;letter-spacing:.04em;color:var(--muted);padding:10px 14px;border-bottom:1px solid var(--border);background:var(--surface-2);white-space:nowrap}
.viewer-bar{display:flex;flex-wrap:wrap;gap:6px 10px;align-items:center;margin-bottom:10px}
.chip{display:inline-block;padding:3px 10px;border:1px solid var(--border);border-radius:999px;font-size:13px;background:var(--surface);color:var(--text);text-decoration:none}
.chip.active{background:var(--primary);border-color:var(--primary);color:var(--primary-ink)}
.sheet-scroll{overflow:auto;max-width:100%;border:1px solid var(--border);border-radius:8px;background:#fff;margin-bottom:14px}
.sheet-view{position:relative;display:inline-block}
.sheet-view img,.sheet-view input[type=image],.sheet-scroll>img{display:block;max-width:none;cursor:crosshair}
.sheet-scroll>img{cursor:default}
.sheet-overlay{position:absolute;left:0;top:0;pointer-events:none}
.classic-only{display:contents}
.wasm-viewer{display:none}
.wasm-viewer-on .wasm-viewer{display:block}
.wasm-viewer-on .classic-only{display:none}
.wasm-viewer [hidden]{display:none}
button.chip{font:inherit;font-size:13px;cursor:pointer}
.wasm-canvas{display:block;width:100%;height:72vh;min-height:320px;border:1px solid var(--border);border-radius:8px;margin-bottom:14px;cursor:grab;touch-action:none;background:#dfe3e8}
.wasm-canvas.marking{cursor:crosshair}
.swatch{display:inline-block;width:12px;height:12px;border-radius:3px;vertical-align:-1px}
.swatch.red{background:#dc1e1e}.swatch.blue{background:#1e5adc}.swatch.gray{background:#bebebe}
.notice:empty{display:none}
.notice{padding:8px 12px;border-radius:8px;background:var(--surface-2);border:1px solid var(--border)}
table.data tr.subtotal td{font-weight:600;border-top:1px solid var(--border)}
table.data tr.group-row th{text-transform:none;font-size:14px;letter-spacing:0;color:var(--text);background:var(--surface-2);border-top:1px solid var(--border)}
table.data td{padding:10px 14px;border-bottom:1px solid var(--border);vertical-align:top}
table.data tr:last-child td{border-bottom:0}
table.data tbody tr:hover td{background:var(--surface-2)}
table.data .num{text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}
table.data .nowrap{white-space:nowrap}
table.data tfoot td{font-weight:650;background:var(--surface-2)}
a.row-link{color:var(--text);font-weight:600;text-decoration:none}
a.row-link:hover{color:var(--primary);text-decoration:underline}

.pill{display:inline-flex;align-items:center;gap:6px;padding:2px 10px;border-radius:999px;font-size:12px;font-weight:650;white-space:nowrap;background:var(--surface-2);color:var(--muted)}
.pill::before{content:"";width:6px;height:6px;border-radius:50%;background:currentColor}
.pill.blue{background:var(--primary-soft);color:var(--primary)}
.pill.green{background:var(--ok-soft);color:var(--ok)}
.pill.red{background:var(--danger-soft);color:var(--danger)}
.pill.amber{background:var(--warn-soft);color:var(--warn)}
.pill.violet{background:var(--violet-soft);color:var(--violet)}
.tag{display:inline-block;padding:1px 8px;border-radius:6px;background:var(--surface-2);color:var(--muted);font-size:12px;font-weight:600}

.filters{display:flex;flex-wrap:wrap;gap:8px;align-items:center;margin-bottom:14px}
.filters input[type=search]{width:260px;max-width:100%}
.filters select{width:auto}
.filters label.check{display:inline-flex;gap:6px;align-items:center;font-size:14px;margin:0 4px}
.tabs{display:flex;gap:2px;border-bottom:1px solid var(--border);margin-bottom:14px;overflow-x:auto}
.tabs a{padding:8px 12px;color:var(--muted);text-decoration:none;border-bottom:2px solid transparent;font-weight:600;font-size:14px;white-space:nowrap;margin-bottom:-1px}
.tabs a:hover{color:var(--text)}
.tabs a.active{color:var(--primary);border-bottom-color:var(--primary)}
.tabs .count{display:inline-block;min-width:20px;padding:0 6px;margin-left:6px;border-radius:999px;background:var(--surface-2);font-size:12px;text-align:center}

.form-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:16px 20px}
.form-grid .full{grid-column:1/-1}
.field label,.field .label{display:block;font-size:13px;font-weight:650;margin-bottom:6px}
.field .hint{font-size:12px;color:var(--muted);margin-top:4px}
.field .req{color:var(--danger)}
input[type=text],input[type=email],input[type=password],input[type=date],input[type=time],input[type=number],input[type=search],input[type=url],input[type=tel],select,textarea{width:100%;padding:9px 11px;border:1px solid var(--border);border-radius:8px;background:var(--surface);color:var(--text);font:inherit;font-size:14px}
textarea{min-height:120px;resize:vertical}
textarea.short{min-height:72px}
input:focus,select:focus,textarea:focus,.btn:focus-visible,a:focus-visible,summary:focus-visible{outline:2px solid var(--focus);outline-offset:1px}
label.check{display:flex;align-items:center;gap:8px;font-weight:500;font-size:14px}
.form-actions{display:flex;gap:10px;justify-content:flex-end;margin-top:20px;flex-wrap:wrap}
fieldset{border:0;padding:0;margin:0}

details.multi summary{list-style:none;cursor:pointer;padding:9px 11px;border:1px solid var(--border);border-radius:8px;background:var(--surface);font-size:14px}
details.multi summary::-webkit-details-marker{display:none}
details.multi summary::after{content:"▾";float:right;color:var(--muted)}
details.multi .options{max-height:240px;overflow:auto;border:1px solid var(--border);border-radius:8px;margin-top:4px;padding:6px;background:var(--surface)}
details.multi .options label{display:flex;gap:8px;align-items:center;padding:4px 6px;font-weight:400;margin:0;font-size:14px;border-radius:6px}
details.multi .options label:hover{background:var(--surface-2)}
details.disclosure > summary{cursor:pointer;font-weight:600;color:var(--primary)}

.detail{display:grid;grid-template-columns:minmax(0,1fr) 320px;gap:20px;align-items:start}
dl.facts{display:grid;grid-template-columns:minmax(110px,40%) 1fr;gap:8px 12px;margin:0;font-size:14px}
dl.facts dt{color:var(--muted)}
dl.facts dd{margin:0;overflow-wrap:anywhere}
.prose{white-space:pre-wrap;overflow-wrap:anywhere}

.stat-row{display:grid;grid-template-columns:repeat(auto-fill,minmax(170px,1fr));gap:12px;margin-bottom:18px}
.stat{background:var(--surface);border:1px solid var(--border);border-radius:12px;padding:14px 16px;text-decoration:none;color:inherit;display:block}
a.stat:hover{border-color:var(--primary)}
.stat .label{font-size:12px;color:var(--muted);text-transform:uppercase;letter-spacing:.04em;font-weight:650}
.stat .value{font-size:26px;font-weight:700;margin-top:2px;font-variant-numeric:tabular-nums}
.stat.alert .value{color:var(--danger)}
.stat.good .value{color:var(--ok)}

.project-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(290px,1fr));gap:14px}
.project-card{display:block;text-decoration:none;color:inherit;background:var(--surface);border:1px solid var(--border);border-radius:12px;padding:16px 18px}
.project-card:hover{border-color:var(--primary)}
.project-card .num{color:var(--muted);font-size:12px;font-weight:650;letter-spacing:.03em}
.project-card h3{margin:2px 0 6px;font-size:16px}
.project-card .meta{display:flex;gap:10px;flex-wrap:wrap;align-items:center;color:var(--muted);font-size:13px}
.project-card .counts{display:flex;gap:16px;margin-top:12px;font-size:13px;color:var(--muted)}
.project-card .counts strong{color:var(--text);font-size:15px;margin-right:4px}

.response{border:1px solid var(--border);border-radius:10px;padding:12px 14px;margin-bottom:10px;background:var(--surface)}
.response.official{border-color:var(--ok);box-shadow:inset 4px 0 0 var(--ok)}
.response .meta{display:flex;justify-content:space-between;gap:10px;flex-wrap:wrap;font-size:13px;color:var(--muted);margin-bottom:6px}
.timeline{list-style:none;margin:0;padding:0}
.timeline li{padding:8px 0 10px 18px;border-left:2px solid var(--border);position:relative;font-size:14px}
.timeline li::before{content:"";position:absolute;left:-6px;top:13px;width:10px;height:10px;border-radius:50%;background:var(--surface);border:2px solid var(--primary)}
.timeline .when{color:var(--muted);font-size:12px}
.timeline .changes{margin:4px 0 0;padding:0;list-style:none;color:var(--muted);font-size:13px}

.empty{text-align:center;padding:44px 16px;color:var(--muted)}
.empty .icon{width:36px;height:36px;color:var(--muted)}
.empty h3{color:var(--text);margin:10px 0 4px}
.overdue{color:var(--danger);font-weight:650}
.muted{color:var(--muted)}
.small{font-size:13px}
.right{text-align:right}
.stack > * + *{margin-top:12px}
.row{display:flex;gap:10px;align-items:center;flex-wrap:wrap}
.grid-2{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:16px}
.matrix td,.matrix th{padding:6px 10px}
.matrix select{width:auto;padding:4px 8px}

.gantt{min-width:980px}
.gantt th.timeline,.gantt td.timeline{width:55%;padding:0}
.gantt .track,.gantt .months{min-width:480px}
.gantt .months,.gantt .track{display:block;width:100%;min-width:480px}
.gantt .months{height:20px}
.gantt .months text{font-size:11px;font-weight:600;fill:var(--muted)}
.gantt .months .tick{stroke:var(--border)}
.gantt .track{height:30px}
.gantt tr.summary td{background:var(--surface-2);font-weight:650}
.gantt .bar{fill:var(--primary-soft);stroke:var(--primary)}
.gantt .bar.critical{fill:var(--danger-soft);stroke:var(--danger)}
.gantt .bar.summary{fill:var(--text);stroke:var(--text)}
td.over{background:var(--danger-soft);color:var(--danger);font-weight:700}
.gantt .bar[data-task],.gantt .done[data-task],.gantt .diamond[data-task]{cursor:grab}
.gantt .handle{fill:transparent;cursor:ew-resize}
.gantt #gantt-move,#gantt-move[hidden]{display:none}
.gantt .dayoff{fill:var(--surface-2);opacity:.9}
.gantt .baseline{fill:none;stroke:var(--muted);stroke-width:1.5}
.gantt .done{fill:var(--primary)}
.gantt .done.critical{fill:var(--danger)}
.gantt .diamond{fill:var(--text)}
.gantt .diamond.critical{fill:var(--danger)}
.gantt .today{stroke:var(--accent);stroke-width:2;stroke-dasharray:4 3;vector-effect:non-scaling-stroke}
.gantt td.d1{padding-left:32px}.gantt td.d2{padding-left:50px}.gantt td.d3{padding-left:68px}.gantt td.d4{padding-left:86px}.gantt td.d5{padding-left:104px}.gantt td.d6{padding-left:122px}.gantt td.d7{padding-left:140px}.gantt td.d8{padding-left:158px}
.sheet{max-width:900px;margin:24px auto;padding:0 16px}
.print-body{background:#fff;color:#000}
.print-table th,.print-table td{padding:6px 8px}
.signatures{display:flex;gap:32px;margin-top:56px}
.signatures>div{flex:1;font-size:13px}
.signatures .line{border-bottom:1px solid currentColor;height:28px;margin-bottom:4px}
.gallery{display:grid;grid-template-columns:repeat(auto-fill,minmax(220px,1fr));gap:14px;margin-bottom:18px}
.gallery .shot{display:flex;flex-direction:column;gap:4px;text-decoration:none;color:inherit;background:var(--surface);border:1px solid var(--border);border-radius:12px;padding:8px}
.gallery .shot img{width:100%;height:165px;object-fit:cover;border-radius:8px;background:var(--surface-2)}
.gallery .shot .cap{font-weight:650;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.photo-large{max-width:100%;height:auto;border-radius:8px}
.nav-section{margin-top:6px}
.nav-section>summary{cursor:pointer;list-style:none;padding:8px 12px;font-size:11px;font-weight:700;letter-spacing:.06em;text-transform:uppercase;color:var(--muted)}
.nav-section>summary::-webkit-details-marker{display:none}
.nav-section>summary::after{content:"+";float:right;font-weight:400}
.nav-section[open]>summary::after{content:"–"}
.auth-wrap{min-height:100vh;display:grid;place-items:center;padding:24px}
.auth-card{width:100%;max-width:420px;background:var(--surface);border:1px solid var(--border);border-radius:14px;padding:28px}
.auth-card .brand{color:var(--text);margin-bottom:18px}
.auth-card .brand .icon{color:var(--primary)}
.auth-card .stack label{display:block}

@media (max-width:1000px){.detail{grid-template-columns:minmax(0,1fr)}}
@media (max-width:900px){
.shell{grid-template-columns:minmax(0,1fr)}
.sidebar{display:none;position:static;height:auto;border-right:0;border-bottom:1px solid var(--border)}
.nav-toggle:checked ~ .shell .sidebar{display:block}
.menu-button{display:inline-flex}
.main{padding:16px 16px 64px}
.topbar .company{display:none}
}
@media (max-width:700px){.form-grid,.grid-2{grid-template-columns:minmax(0,1fr)}.filters input[type=search]{width:100%}.topbar .who .name{display:none}}
@media print{.topbar,.sidebar,.actions,.filters,.no-print{display:none!important}.shell{display:block}.main{padding:0}body{background:#fff}}
"#;
