//! The company report: every project in a table of chosen columns, saved to run again, with a chart.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::report::visible_reports;
use buildkiosk_engine::model::project::ProjectStage;
use buildkiosk_engine::model::saved_report::{DeliveryRhythm, ReportDelivery};
use buildkiosk_engine::model::saved_report::{ReportColumn, SavedReport};
use buildkiosk_engine::model::{Choice, Level, Tool, perm};
use buildkiosk_engine::report::{Cell, CompanyRow, company_report, company_totals};
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, csv_cell, empty, field};

/// What a report is run with: a saved report's definition, or the standard portfolio.
struct Def {
    columns: Vec<ReportColumn>,
    stage: Option<ProjectStage>,
    sort: Option<ReportColumn>,
    descending: bool,
}

fn definition(saved: Option<&SavedReport>) -> Def {
    match saved {
        Some(r) => Def { columns: r.columns.clone(), stage: r.stage, sort: r.sort, descending: r.descending },
        None => Def { columns: ReportColumn::STANDARD.to_vec(), stage: None, sort: None, descending: false },
    }
}

fn text_of(c: &Cell) -> String {
    match c {
        Cell::Text(t) => t.clone(),
        Cell::Count(n) => n.to_string(),
        Cell::Money(m) => m.to_string(),
        Cell::Date(d) => d.to_string(),
        Cell::Blank => String::new(),
    }
}

fn cell(c: &Cell, column: ReportColumn) -> Markup {
    match c {
        Cell::Blank => html! { td class="num" { span class="muted" { "—" } } },
        Cell::Count(n) if matches!(column, ReportColumn::Overdue | ReportColumn::LateTasks | ReportColumn::RfisOverdue) => {
            html! { td class="num" { @if *n > 0 { span class="overdue" { (n) } } @else { (n) } } }
        }
        Cell::Money(m) if column == ReportColumn::Variance => {
            html! { td class="num" { span class=(if m.is_negative() { "overdue" } else { "" }) { (m) } } }
        }
        Cell::Money(m) => html! { td class="num" { (m) } },
        Cell::Count(n) => html! { td class="num" { (n) } },
        Cell::Date(d) => html! { td class="nowrap" { (d.pretty()) } },
        Cell::Text(t) => html! { td { (t) } },
    }
}

/// A bar for each project: the chosen column as a horizontal bar chart, drawn as an SVG (no script). Negative values
/// run left of the axis in red.
fn chart(rows: &[CompanyRow], index: usize, label: &str, money: bool, kind: ChartKind) -> Markup {
    let values = chart_values(rows, index);
    match kind {
        ChartKind::Columns if !values.is_empty() => return column_chart(&values, label, money),
        ChartKind::Pie if !values.is_empty() => return pie_chart(&values, label, money),
        _ => {}
    }
    if values.is_empty() {
        return html! { p class="muted" { "Nothing to chart: no project has a figure for " (label) " that you can see." } };
    }
    let max = values.iter().map(|(_, v)| v.abs()).max().unwrap_or(1).max(1);
    let negative = values.iter().any(|(_, v)| *v < 0);
    let (width, row_h, label_w) = (640.0_f64, 28.0_f64, 170.0_f64);
    let bar_w = width - label_w - 90.0;
    // With negatives the axis sits in the middle; otherwise at the left.
    let axis_x = if negative { label_w + bar_w / 2.0 } else { label_w };
    let scale = if negative { bar_w / 2.0 } else { bar_w } / max as f64;
    let height = row_h * values.len() as f64 + 8.0;
    html! {
        figure class="chart" {
            svg viewBox=(format!("0 0 {width} {height}")) role="img" aria-label=(format!("{label} by project")) {
                title { (label) " by project" }
                @for (i, (name, v)) in values.iter().enumerate() {
                    @let y = 4.0 + row_h * i as f64;
                    @let w = (v.abs() as f64 * scale).max(if *v == 0 { 0.0 } else { 1.0 });
                    @let x = if *v < 0 { axis_x - w } else { axis_x };
                    text x=(label_w - 8.0) y=(y + 17.0) text-anchor="end" class="chart-label" { (name.chars().take(26).collect::<String>()) }
                    rect x=(format!("{x:.1}")) y=(format!("{:.1}", y + 4.0)) width=(format!("{w:.1}")) height="16" class=(if *v < 0 { "bar bad" } else { "bar" }) {}
                    text x=(format!("{:.1}", if *v < 0 { x - 6.0 } else { x + w + 6.0 })) y=(y + 17.0) text-anchor=(if *v < 0 { "end" } else { "start" }) class="chart-value" { (value_text(*v, money)) }
                }
                line x1=(axis_x) y1="0" x2=(axis_x) y2=(height) class="axis" {}
            }
            figcaption class="muted small" { (label) ", one bar per project" }
        }
    }
}

fn value_text(v: i128, money: bool) -> String {
    if money { buildkiosk_core::Money::from_cents(i64::try_from(v).unwrap_or(0)).to_string() } else { v.to_string() }
}

/// How the chosen column is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChartKind {
    /// A horizontal bar for each project.
    Bars,
    /// A vertical column for each project.
    Columns,
    /// Each project's share of the total.
    Pie,
}

impl ChartKind {
    pub const ALL: [ChartKind; 3] = [ChartKind::Bars, ChartKind::Columns, ChartKind::Pie];

    pub fn key(self) -> &'static str {
        match self {
            ChartKind::Bars => "bars",
            ChartKind::Columns => "columns",
            ChartKind::Pie => "pie",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ChartKind::Bars => "Bars",
            ChartKind::Columns => "Columns",
            ChartKind::Pie => "Pie",
        }
    }

    pub fn parse(s: &str) -> Option<ChartKind> {
        ChartKind::ALL.into_iter().find(|k| k.key() == s.trim())
    }
}

/// The values to chart: each project with a figure for the column.
fn chart_values(rows: &[CompanyRow], index: usize) -> Vec<(String, i128)> {
    rows.iter()
        .filter_map(|r| match r.cells.get(index)? {
            Cell::Count(n) => Some((r.name.clone(), *n as i128)),
            Cell::Money(m) => Some((r.name.clone(), i128::from(m.cents()))),
            _ => None,
        })
        .collect()
}

/// A vertical column for each project, drawn as an SVG. Negative values hang below the axis in red.
fn column_chart(values: &[(String, i128)], label: &str, money: bool) -> Markup {
    let max = values.iter().map(|(_, v)| v.abs()).max().unwrap_or(1).max(1);
    let negative = values.iter().any(|(_, v)| *v < 0);
    let (slot, plot_h, top, label_h) = (64.0_f64, 150.0_f64, 22.0_f64, 56.0_f64);
    let width = (slot * values.len() as f64 + 16.0).max(320.0);
    let axis_y = if negative { top + plot_h / 2.0 } else { top + plot_h };
    let scale = if negative { plot_h / 2.0 } else { plot_h } / max as f64;
    let height = top + plot_h + label_h;
    html! {
        figure class="chart" {
            svg viewBox=(format!("0 0 {width} {height}")) role="img" aria-label=(format!("{label} by project")) {
                title { (label) " by project" }
                @for (i, (name, v)) in values.iter().enumerate() {
                    @let x = 8.0 + slot * i as f64;
                    @let h = (v.abs() as f64 * scale).max(if *v == 0 { 0.0 } else { 1.0 });
                    @let y = if *v < 0 { axis_y } else { axis_y - h };
                    rect x=(format!("{:.1}", x + 10.0)) y=(format!("{y:.1}")) width=(format!("{:.1}", slot - 20.0)) height=(format!("{h:.1}")) class=(if *v < 0 { "bar bad" } else { "bar" }) {}
                    text x=(format!("{:.1}", x + slot / 2.0)) y=(format!("{:.1}", if *v < 0 { y + h + 12.0 } else { y - 4.0 })) text-anchor="middle" class="chart-value" { (value_text(*v, money)) }
                    text x=(format!("{:.1}", x + slot / 2.0)) y=(format!("{:.1}", top + plot_h + 16.0)) text-anchor="middle" class="chart-label" { (name.chars().take(9).collect::<String>()) }
                }
                line x1="4" y1=(axis_y) x2=(width - 4.0) y2=(axis_y) class="axis" {}
            }
            figcaption class="muted small" { (label) ", one column per project" }
        }
    }
}

/// Each project's share of the column's total as a pie drawn in SVG, with a key. Only for figures that are not negative.
fn pie_chart(values: &[(String, i128)], label: &str, money: bool) -> Markup {
    let total: i128 = values.iter().map(|(_, v)| *v).sum();
    if values.iter().any(|(_, v)| *v < 0) || total <= 0 {
        return html! { p class="muted" { "A pie needs figures that are not negative and add up to more than nothing. Use bars or columns for " (label) "." } };
    }
    // Slices are told apart by a lightness step on the accent colour and by the key beside them, not by colour alone.
    let (cx, cy, r) = (110.0_f64, 110.0_f64, 100.0_f64);
    let mut start = -std::f64::consts::FRAC_PI_2;
    let slices: Vec<(usize, f64, f64, f64)> = values
        .iter()
        .enumerate()
        .filter(|(_, (_, v))| *v > 0)
        .map(|(i, (_, v))| {
            let share = *v as f64 / total as f64;
            let end = start + share * std::f64::consts::TAU;
            let out = (i, start, end, share);
            start = end;
            out
        })
        .collect();
    let point = |a: f64| (cx + r * a.cos(), cy + r * a.sin());
    html! {
        figure class="chart pie" {
            svg viewBox="0 0 220 220" role="img" aria-label=(format!("{label} by project, as shares of the total")) {
                title { (label) " by project, as shares of the total" }
                @for (i, a0, a1, share) in &slices {
                    @let (x0, y0) = point(*a0);
                    @let (x1, y1) = point(*a1);
                    @if *share >= 0.9999 {
                        circle cx=(cx) cy=(cy) r=(r) class=(format!("slice s{}", i % 6)) {}
                    } @else {
                        path d=(format!("M{cx:.1} {cy:.1} L{x0:.2} {y0:.2} A{r:.1} {r:.1} 0 {} 1 {x1:.2} {y1:.2} Z", u8::from(a1 - a0 > std::f64::consts::PI))) class=(format!("slice s{}", i % 6)) {}
                    }
                }
            }
            ol class="key" {
                @for (i, (name, v)) in values.iter().enumerate().filter(|(_, (_, v))| *v > 0) {
                    li { span class=(format!("swatch s{}", i % 6)) {} " " (name) ": " (value_text(*v, money)) " (" (format!("{:.0}", *v as f64 / total as f64 * 100.0)) "%)" }
                }
            }
            figcaption class="muted small" { (label) ", each project's share of the total" }
        }
    }
}

pub fn page(req: &Req<'_>, report: Option<Id<SavedReport>>, chart_column: Option<ReportColumn>, kind: ChartKind) -> Markup {
    let state = req.engine.state();
    if perm::company_level(state, req.person, Tool::CompanyReports) == Level::None && state.people.get(req.person).is_none() {
        return crate::pages::not_found(req);
    }
    let saved = visible_reports(state, req.actor());
    let current = report.and_then(|id| saved.iter().copied().find(|r| r.id == id));
    let def = definition(current);
    let rows = company_report(state, req.actor(), req.today(), &def.columns, def.stage, def.sort, def.descending);
    let totals = company_totals(&rows, &def.columns);
    let title = current.map_or("Portfolio".to_string(), |r| r.name.clone());
    let query = current.map(|r| format!("?report={}", r.id)).unwrap_or_default();
    let numeric: Vec<(usize, ReportColumn)> = def.columns.iter().copied().enumerate().filter(|(_, c)| c.adds_up()).collect();
    let chosen = chart_column.and_then(|c| numeric.iter().copied().find(|(_, n)| *n == c)).or_else(|| numeric.first().copied());
    let owner = current.is_some_and(|r| r.owner == req.person) || perm::company_level(state, req.person, Tool::CompanyAdmin) == Level::Admin;
    let here = "/reports";
    let body = html! {
        div class="page-head" {
            div { h1 { (title) } div class="sub" { "Every project you can open, with the columns chosen" @if let Some(s) = def.stage { " · " (s.label()) " projects only" } } }
            div class="actions" {
                a class="btn" href="/injury-log" { "Injury log" }
                a class="btn" href=(format!("{here}.pdf{query}")) { "PDF" }
                a class="btn" href=(format!("{here}.csv{query}")) { "Export CSV" }
            }
        }
        div class="tabs" {
            a href=(here) class=(if current.is_none() { "active" } else { "" }) { "Portfolio" }
            @for r in &saved { a href=(format!("{here}?report={}", r.id)) class=(if current.is_some_and(|c| c.id == r.id) { "active" } else { "" }) { (r.name) } }
        }
        @if rows.is_empty() {
            div class="card" { (empty(tool_icon(Tool::CompanyReports), "No projects", "Nothing matches, or you can't open any project yet.")) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "Project" } @for c in &def.columns { th class=(if c.adds_up() { "num" } else { "" }) { (c.label()) } } } }
                tbody { @for r in &rows { tr {
                    td { a href=(format!("/projects/{}", r.project)) { (r.name) } }
                    @for (c, v) in def.columns.iter().zip(&r.cells) { (cell(v, *c)) }
                } } }
                tfoot { tr { td { "Total" } @for (c, v) in def.columns.iter().zip(&totals) { (cell(v, *c)) } } }
            } }
            @if let Some((i, c)) = chosen {
                div class="card" {
                    div class="card-head" {
                        h2 { "Chart" }
                        form method="get" action=(here) class="filters" {
                            @if let Some(r) = current { input type="hidden" name="report" value=(r.id); }
                            select name="chart" aria-label="Column to chart" { @for (_, n) in &numeric { option value=(n.key()) selected[*n == c] { (n.label()) } } }
                            select name="kind" aria-label="Kind of chart" { @for k in ChartKind::ALL { option value=(k.key()) selected[k == kind] { (k.label()) } } }
                            button type="submit" class="btn small" { "Chart it" }
                        }
                    }
                    (chart(&rows, i, c.label(), c.is_money(), kind))
                }
            }
        }
        div class="card" {
            h2 { (if current.is_some() && owner { "Change this report" } else { "Save a report" }) }
            p class="muted small" { "Choose the columns, and which projects, and how to sort. A saved report keeps its choices and can be shared with everyone in the company." }
            (command_form(req, "company_report.save", &format!("{here}?report={{id}}"), here, html! {
                @if let (Some(r), true) = (current, owner) { input type="hidden" name="id" value=(r.id); }
                div class="form-grid" {
                    (field("Name", "cr_name", true, false, html! { input type="text" id="cr_name" name="name" required value=(current.filter(|_| owner).map(|r| r.name.as_str()).unwrap_or_default()); }))
                    (field("Only projects in", "cr_stage", false, false, html! { select id="cr_stage" name="stage" { option value="" { "Any stage" } @for s in ProjectStage::ALL { option value=(s.key()) selected[def.stage == Some(*s)] { (s.label()) } } } }))
                    (field("Sort by", "cr_sort", false, false, html! { select id="cr_sort" name="sort" { option value="" { "Project name" } @for c in ReportColumn::ALL { option value=(c.key()) selected[def.sort == Some(*c)] { (c.label()) } } } }))
                    (field("Order", "cr_desc", false, false, html! { select id="cr_desc" name="descending" { option value="false" selected[!def.descending] { "Lowest first" } option value="true" selected[def.descending] { "Highest first" } } }))
                    (field("Shared", "cr_shared", false, false, html! { select id="cr_shared" name="shared" { option value="false" selected[!current.is_some_and(|r| r.shared)] { "Only me (and company admins)" } option value="true" selected[current.is_some_and(|r| r.shared)] { "Everyone in the company" } } }))
                }
                fieldset { legend { "Columns" } div class="options" { @for c in ReportColumn::ALL { label { input type="checkbox" name="columns[]" value=(c.key()) checked[def.columns.contains(c)]; (c.label()) } } } }
                div class="form-actions" { button type="submit" class="btn primary" { (if current.is_some() && owner { "Save changes" } else { "Save as a new report" }) } }
            }))
            @if let (Some(r), true) = (current, owner) { (button_form(req, "company_report.delete", here, &[("id", r.id.to_string())], "Delete this report", "small danger")) }
        }

        @if let (Some(r), true) = (current, owner) {
            (delivery_card(req, r, here))
        }
    };
    layout::app(req, &title, Nav::CompanyReports, &[("Projects", Some("/".into())), ("Reports", None)], body)
}

/// The report as CSV: the project and the chosen columns, then the totals.
pub fn csv(req: &Req<'_>, report: Option<Id<SavedReport>>) -> Option<String> {
    let state = req.engine.state();
    let saved = visible_reports(state, req.actor());
    let current = match report {
        Some(id) => Some(saved.iter().copied().find(|r| r.id == id)?),
        None => None,
    };
    let def = definition(current);
    let rows = company_report(state, req.actor(), req.today(), &def.columns, def.stage, def.sort, def.descending);
    let totals = company_totals(&rows, &def.columns);
    let mut out = format!("Project,{}\r\n", def.columns.iter().map(|c| csv_cell(c.label())).collect::<Vec<_>>().join(","));
    for r in &rows {
        out.push_str(&csv_cell(&r.name));
        for v in &r.cells {
            out.push(',');
            out.push_str(&csv_cell(&match v {
                Cell::Money(m) => m.plain(),
                other => text_of(other),
            }));
        }
        out.push_str("\r\n");
    }
    out.push_str("Total");
    for v in &totals {
        out.push(',');
        out.push_str(&csv_cell(&match v {
            Cell::Money(m) => m.plain(),
            other => text_of(other),
        }));
    }
    out.push_str("\r\n");
    Some(out)
}

/// A report as text, for the PDF.
pub struct TableText {
    pub title: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub totals: Vec<String>,
    /// Which columns are numbers (aligned right).
    pub right: Vec<bool>,
}

pub fn table_text(req: &Req<'_>, report: Option<Id<SavedReport>>) -> Option<TableText> {
    let state = req.engine.state();
    let saved = visible_reports(state, req.actor());
    let current = match report {
        Some(id) => Some(saved.iter().copied().find(|r| r.id == id)?),
        None => None,
    };
    let def = definition(current);
    let rows = company_report(state, req.actor(), req.today(), &def.columns, def.stage, def.sort, def.descending);
    let totals = company_totals(&rows, &def.columns);
    let dash = |c: &Cell| if matches!(c, Cell::Blank) { "—".to_string() } else { text_of(c) };
    let mut headers = vec!["Project".to_string()];
    headers.extend(def.columns.iter().map(|c| c.label().to_string()));
    let body = rows.iter().map(|r| std::iter::once(r.name.clone()).chain(r.cells.iter().map(dash)).collect()).collect();
    let total = std::iter::once("Total".to_string()).chain(totals.iter().map(dash)).collect();
    let right = std::iter::once(false).chain(def.columns.iter().map(|c| c.adds_up())).collect();
    Some(TableText { title: current.map_or("Portfolio".to_string(), |r| r.name.clone()), headers, rows: body, totals: total, right })
}

/// The email for one person's copy of a saved report: the subject, a plain-text table with the totals and a link when the
/// server's address is known, and the CSV to attach. `None` when the person can't run the report. The figures are what
/// that person can see (`req` is theirs).
pub fn delivery_email(req: &Req<'_>, report: Id<SavedReport>, base_url: &str) -> Option<(String, String, String)> {
    let table = table_text(req, Some(report))?;
    let csv = csv(req, Some(report))?;
    let mut widths: Vec<usize> = table.headers.iter().map(|h| h.chars().count()).collect();
    for row in table.rows.iter().chain(std::iter::once(&table.totals)) {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let line = |cells: &[String]| -> String {
        let parts: Vec<String> = cells
            .iter()
            .zip(&widths)
            .enumerate()
            .map(|(i, (c, w))| {
                let pad = w.saturating_sub(c.chars().count());
                if table.right.get(i).copied().unwrap_or(false) { format!("{}{c}", " ".repeat(pad)) } else { format!("{c}{}", " ".repeat(pad)) }
            })
            .collect();
        parts.join("  ").trim_end().to_string()
    };
    let mut body = format!("{} · {}\n\n{}\n", table.title, req.today().pretty(), line(&table.headers));
    for row in &table.rows {
        body.push_str(&line(row));
        body.push('\n');
    }
    body.push_str(&line(&table.totals));
    body.push_str("\n\nThe same figures are attached as a CSV file.");
    if !base_url.is_empty() {
        body.push_str(&format!("\nOpen the report: {base_url}/reports?report={report}"));
    }
    body.push_str("\n\nYou get this because the report is emailed to you on a schedule. Each figure is only what you can see of a project.\n");
    let subject: String = format!("{} · {}", table.title, req.today().pretty()).chars().filter(|c| !c.is_control()).take(120).collect();
    Some((subject, body, csv))
}

/// The "Email this report" card of a saved report: its schedule today and the form to change it.
fn delivery_card(req: &Req<'_>, report: &SavedReport, here: &str) -> Markup {
    let state = req.engine.state();
    let back = format!("{here}?report={}", report.id);
    let people: Vec<_> = state
        .people
        .iter()
        .filter(|p| p.is_user && p.active && p.email.is_some())
        .filter(|p| report.shared || p.id == report.owner || perm::company_level(state, p.id, Tool::CompanyAdmin) == Level::Admin)
        .collect();
    let now: Option<&ReportDelivery> = report.delivery.as_ref();
    const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    html! {
        div class="card" {
            h2 { "Email this report" }
            @if let Some(d) = now {
                p { @match d.rhythm {
                    DeliveryRhythm::Daily => { "Emailed every day" }
                    DeliveryRhythm::Weekly => { "Emailed every " (DAYS.get(usize::from(d.weekday)).copied().unwrap_or("Monday")) }
                    DeliveryRhythm::Monthly => { "Emailed on day " (d.day) " of every month" }
                } " to " (d.recipients.len()) " " @if d.recipients.len() == 1 { "person" } @else { "people" }
                    @if let Some(l) = d.last_sent { ". Last sent " (l.pretty()) } "." }
            } @else {
                p class="muted" { "Not emailed. Pick a rhythm and who gets it." }
            }
            p class="muted small" { "Each person gets the report as they themselves would see it, as a table in the email and a CSV file, from about six in the morning. Outgoing email has to be set up first. The first email is the next time the rhythm comes round." }
            (command_form(req, "company_report.schedule", &back, &back, html! {
                input type="hidden" name="id" value=(report.id);
                div class="form-grid" {
                    (field("How often", "cr_rhythm", true, false, html! { select id="cr_rhythm" name="rhythm" {
                        option value="off" selected[now.is_none()] { "Not emailed" }
                        @for r in DeliveryRhythm::ALL { option value=(r.key()) selected[now.is_some_and(|d| d.rhythm == *r)] { (r.label()) } }
                    } }))
                    (field("Weekday (weekly)", "cr_weekday", false, false, html! { select id="cr_weekday" name="weekday" {
                        @for (i, d) in DAYS.iter().enumerate() { option value=(d.to_lowercase()) selected[now.map_or(i == 0, |x| usize::from(x.weekday) == i)] { (d) } }
                    } }))
                    (field("Day of the month (monthly)", "cr_day", false, false, html! { input type="number" id="cr_day" name="day" min="1" max="28" value=(now.map_or(1, |d| d.day)); }))
                }
                fieldset { legend { "Who gets it" } div class="options" {
                    @for p in &people { label { input type="checkbox" name="people[]" value=(p.id) checked[now.is_some_and(|d| d.recipients.contains(&p.id))]; (p.name()) } }
                } }
                div class="form-actions" { button type="submit" class="btn primary" { "Save" } }
            }))
        }
    }
}
