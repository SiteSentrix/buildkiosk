//! PDF versions of the documents people print or send: each builder checks the same visibility
//! rules as the matching page and returns `(file name, PDF bytes)`.

use crate::workitems::{Filter, Group, WorkPage, filtered, sections};
use buildkiosk_core::Id;
use buildkiosk_engine::model::daily_log::{DailyLog, hours_text};
use buildkiosk_engine::model::directory::Person;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::rfi::Rfi;
use buildkiosk_engine::model::submittal::Submittal;
use buildkiosk_engine::model::{Choice, CompanyState, Level, Tool};
use buildkiosk_engine::workitem::items_for;
use buildkiosk_engine::{report, sched, view};

use crate::layout::Req;
use crate::pdf::{Align, Block, Doc, Table, render};

pub type Pdf = (String, Vec<u8>);

fn names(state: &CompanyState, ids: &[Id<Person>]) -> String {
    let s = view::people_names(state, ids);
    if s.is_empty() { "—".into() } else { s }
}

fn person(state: &CompanyState, p: Option<Id<Person>>) -> String {
    p.map(|p| view::person_name(state, p)).unwrap_or_else(|| "—".into())
}

fn dash(s: &str) -> String {
    if s.is_empty() { "—".into() } else { s.to_string() }
}

fn start(req: &Req<'_>, project: &Project, title: String, subtitle: &str) -> (Doc, String) {
    let company = req.engine.state().company().map(|c| c.name.clone()).unwrap_or_default();
    let sub = if subtitle.is_empty() { project.name.clone() } else { format!("{} · {subtitle}", project.name) };
    (Doc { title, subtitle: sub, running_head: format!("{company} · {}", project.name), blocks: Vec::new() }, company)
}

fn file_name(stem: &str) -> String {
    let s: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    format!("{}.pdf", s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-"))
}

fn table(headers: &[&str], widths: &[f32], right: &[usize], rows: Vec<Vec<String>>) -> Block {
    Block::Table(Table {
        headers: headers.iter().map(|h| h.to_string()).collect(),
        widths: widths.to_vec(),
        aligns: (0..headers.len()).map(|i| if right.contains(&i) { Align::Right } else { Align::Left }).collect(),
        rows,
        footer: None,
    })
}

fn project_of<'a>(req: &'a Req<'_>, id: Id<Project>, tool: Tool) -> Option<&'a Project> {
    let state = req.engine.state();
    let p = state.projects.get(id).filter(|p| p.deleted.is_none())?;
    (view::level(state, req.actor(), id, tool) >= Level::Read).then_some(p)
}

pub fn rfi(req: &Req<'_>, id: Id<Project>, rid: Id<Rfi>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Rfis)?;
    let r = state.rfis.get(rid).filter(|r| r.project == id && r.deleted.is_none() && view::can_see_rfi(state, req.actor(), r))?;
    let (mut doc, _) = start(req, p, format!("{}: {}", r.label(), r.subject), &format!("Status: {}", r.status.label()));
    doc.blocks.push(Block::Facts(vec![
        ("Manager".into(), person(state, r.manager)),
        ("Assigned to".into(), names(state, &r.assignees)),
        ("Copied".into(), names(state, &r.distribution)),
        ("Due".into(), r.due.map(|d| d.pretty()).unwrap_or_else(|| "—".into())),
        ("Spec section".into(), dash(&r.spec_section)),
        ("Drawing".into(), dash(&r.drawing_number)),
        ("Location".into(), dash(&r.location)),
        ("Cost impact".into(), r.cost_impact.label().to_string() + &r.cost_amount.map(|m| format!(" ({m})")).unwrap_or_default()),
        ("Schedule impact".into(), r.schedule_impact.label().to_string() + &r.schedule_days.map(|d| format!(" ({d} days)")).unwrap_or_default()),
    ]));
    doc.blocks.push(Block::Heading("Question".into()));
    doc.blocks.push(Block::Paragraph(r.question.clone()));
    doc.blocks.push(Block::Heading("Responses".into()));
    if r.responses.is_empty() {
        doc.blocks.push(Block::Paragraph("No responses yet.".into()));
    }
    for x in &r.responses {
        doc.blocks.push(Block::Facts(vec![(
            if x.official { "Official response".into() } else { "Response".into() },
            format!("{}, {}", person(state, x.by), req.when(x.at)),
        )]));
        doc.blocks.push(Block::Paragraph(x.body.clone()));
    }
    Some((file_name(&r.label()), render(&doc)))
}

pub fn submittal(req: &Req<'_>, id: Id<Project>, sid: Id<Submittal>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Submittals)?;
    let s = state.submittals.get(sid).filter(|s| s.project == id && s.deleted.is_none() && view::can_see_submittal(state, req.actor(), s))?;
    let (mut doc, _) = start(req, p, format!("{}: {}", s.label(), s.title), &format!("Status: {}", s.status.label()));
    doc.blocks.push(Block::Facts(vec![
        ("Spec section".into(), dash(&s.spec_section)),
        ("Type".into(), dash(&s.kind)),
        ("Manager".into(), person(state, s.manager)),
        ("Submitted by".into(), person(state, s.submitter)),
        ("Reviewers".into(), names(state, &s.reviewers)),
        ("Due".into(), s.due.map(|d| d.pretty()).unwrap_or_else(|| "—".into())),
        ("Final decision".into(), s.final_decision.map(|d| d.label().to_string()).unwrap_or_else(|| "—".into())),
    ]));
    doc.blocks.push(Block::Heading("Reviews".into()));
    let rows = s
        .reviews
        .iter()
        .map(|r| vec![person(state, r.by), format!("{}", r.revision), r.decision.label().to_string(), r.comment.clone(), req.when(r.at)])
        .collect();
    doc.blocks.push(table(&["Reviewer", "Rev.", "Decision", "Comment", "When"], &[2.0, 0.6, 1.6, 4.0, 2.0], &[], rows));
    Some((file_name(&s.label()), render(&doc)))
}

pub fn daily_log(req: &Req<'_>, id: Id<Project>, lid: Id<DailyLog>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::DailyLog)?;
    let l = state.daily_logs.get(lid).filter(|l| l.project == id && view::can_see_daily_log(state, req.actor(), l))?;
    let (mut doc, _) = start(req, p, format!("Daily log: {}", l.date.pretty()), &format!("Status: {}", l.status.label()));
    let w = &l.weather;
    doc.blocks.push(Block::Facts(vec![
        (
            "Weather".into(),
            format!(
                "{}{}{}",
                w.sky.label(),
                w.high.map(|h| format!(", high {h}")).unwrap_or_default(),
                w.low.map(|x| format!(", low {x}")).unwrap_or_default()
            ),
        ),
        ("Weather notes".into(), dash(&w.notes)),
    ]));
    for (h, t) in [("Work performed", &l.work_performed), ("Delays", &l.delays), ("Safety", &l.safety), ("Notes", &l.notes)] {
        if !t.is_empty() {
            doc.blocks.push(Block::Heading(h.into()));
            doc.blocks.push(Block::Paragraph(t.clone()));
        }
    }
    if !l.manpower.is_empty() {
        let (workers, tenths) = l.headcount();
        doc.blocks.push(Block::Heading(format!("Manpower: {workers} workers, {} hours", hours_text(tenths))));
        let rows = l
            .manpower
            .iter()
            .map(|m| vec![m.company.clone(), m.trade.clone(), m.workers.to_string(), hours_text(u64::from(m.hours_tenths)), m.notes.clone()])
            .collect();
        doc.blocks.push(table(&["Company", "Trade", "Workers", "Hours each", "Notes"], &[3.0, 2.0, 1.0, 1.2, 3.0], &[2, 3], rows));
    }
    if !l.equipment.is_empty() {
        doc.blocks.push(Block::Heading("Equipment".into()));
        let rows = l.equipment.iter().map(|e| vec![e.name.clone(), hours_text(u64::from(e.hours_tenths)), e.notes.clone()]).collect();
        doc.blocks.push(table(&["Equipment", "Hours", "Notes"], &[4.0, 1.0, 4.0], &[1], rows));
    }
    if !l.deliveries.is_empty() {
        doc.blocks.push(Block::Heading("Deliveries".into()));
        let rows = l.deliveries.iter().map(|d| vec![d.description.clone(), d.company.clone(), d.notes.clone()]).collect();
        doc.blocks.push(table(&["Delivery", "From", "Notes"], &[4.0, 2.5, 3.0], &[], rows));
    }
    if !l.visitors.is_empty() {
        doc.blocks.push(Block::Heading("Visitors".into()));
        let rows = l.visitors.iter().map(|v| vec![v.name.clone(), v.company.clone(), v.purpose.clone()]).collect();
        doc.blocks.push(table(&["Visitor", "Company", "Purpose"], &[3.0, 3.0, 4.0], &[], rows));
    }
    Some((file_name(&format!("daily log {}", l.date)), render(&doc)))
}

pub fn meeting(req: &Req<'_>, id: Id<Project>, mid: Id<buildkiosk_engine::model::meeting::Meeting>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Meetings)?;
    let m = state.meetings.get(mid).filter(|m| m.project == id && m.deleted.is_none())?;
    let (mut doc, _) = start(
        req,
        p,
        format!("{}: {}", m.label(), m.title),
        &format!("{}{}", m.date.pretty(), if m.time.is_empty() { String::new() } else { format!(" · {}", m.time) }),
    );
    doc.blocks.push(Block::Facts(vec![
        ("Type".into(), dash(&m.category)),
        ("Location".into(), dash(&m.location)),
        ("Minutes".into(), m.status.label().to_string()),
    ]));
    doc.blocks.push(Block::Heading("Attendees".into()));
    let rows = m
        .attendees
        .iter()
        .map(|a| vec![view::person_name(state, a.person), if a.present { "Present".into() } else { "Not recorded".into() }])
        .collect();
    doc.blocks.push(table(&["Name", "Attendance"], &[4.0, 2.0], &[], rows));
    if !m.agenda.is_empty() {
        doc.blocks.push(Block::Heading("Agenda and notes".into()));
        for (i, a) in m.agenda.iter().enumerate() {
            doc.blocks.push(Block::Facts(vec![(format!("{}.", i + 1), a.title.clone())]));
            if !a.notes.is_empty() {
                doc.blocks.push(Block::Paragraph(a.notes.clone()));
            }
        }
    }
    if !m.minutes.is_empty() {
        doc.blocks.push(Block::Heading("Other minutes".into()));
        doc.blocks.push(Block::Paragraph(m.minutes.clone()));
    }
    if !m.actions.is_empty() {
        doc.blocks.push(Block::Heading("Action items".into()));
        let rows = m
            .actions
            .iter()
            .map(|a| {
                vec![
                    a.description.clone(),
                    person(state, a.assignee),
                    a.due.map(|d| d.to_string()).unwrap_or_default(),
                    if a.done { "Done".into() } else { "Open".into() },
                ]
            })
            .collect();
        doc.blocks.push(table(&["Action", "Who", "Due", "Status"], &[5.0, 2.5, 1.6, 1.2], &[], rows));
    }
    Some((file_name(&m.label()), render(&doc)))
}

pub fn transmittal(req: &Req<'_>, id: Id<Project>, tid: Id<buildkiosk_engine::model::transmittal::Transmittal>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Transmittals)?;
    let t = state
        .transmittals
        .get(tid)
        .filter(|t| t.project == id && t.deleted.is_none() && buildkiosk_engine::cmd::transmittal::can_see(state, req.actor(), t))?;
    let (mut doc, _) = start(req, p, format!("{}: {}", t.label(), t.subject), t.purpose.label());
    doc.blocks.push(Block::Facts(vec![
        ("From".into(), person(state, t.sent.and_then(|s| s.by).or(t.created.by))),
        ("To".into(), names(state, &t.to)),
        ("Copied".into(), names(state, &t.cc)),
        ("Sent".into(), t.sent.map(|s| req.when(s.at)).unwrap_or_else(|| "Not sent yet".into())),
        ("Response by".into(), t.due.map(|d| d.pretty()).unwrap_or_else(|| "—".into())),
    ]));
    if !t.message.is_empty() {
        doc.blocks.push(Block::Paragraph(t.message.clone()));
    }
    doc.blocks.push(Block::Heading("Items sent".into()));
    let mut rows: Vec<Vec<String>> = t.items.iter().map(|i| vec![i.description.clone(), i.revision.clone(), i.copies.to_string()]).collect();
    rows.extend(t.attachments.iter().map(|f| vec![format!("File: {}", f.name), String::new(), "1".into()]));
    doc.blocks.push(table(&["Description", "Version", "Copies"], &[5.2, 1.4, 1.0], &[2], rows));
    // The receipt block: who has acknowledged, and a line to sign and return for those who haven't.
    if t.status == buildkiosk_engine::model::transmittal::TransmittalStatus::Sent {
        doc.blocks.push(Block::Heading("Receipt".into()));
        let rows =
            t.to.iter()
                .map(|p| (*p, "To"))
                .chain(t.cc.iter().map(|p| (*p, "Copy")))
                .map(|(who, role)| {
                    let got = t.receipts.iter().find(|r| r.person == who);
                    vec![
                        view::person_name(state, who),
                        role.to_string(),
                        got.map(|r| format!("Received {}", req.when(r.stamp.at))).unwrap_or_else(|| "______________  Date: ________".into()),
                    ]
                })
                .collect();
        doc.blocks.push(table(&["Recipient", "", "Received (sign and return)"], &[3.5, 1.0, 4.0], &[], rows));
    }
    Some((file_name(&t.label()), render(&doc)))
}

pub fn correspondence(req: &Req<'_>, id: Id<Project>, cid: Id<buildkiosk_engine::model::correspondence::Correspondence>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Correspondence)?;
    let c = state
        .correspondence
        .get(cid)
        .filter(|c| c.project == id && c.deleted.is_none() && buildkiosk_engine::cmd::correspondence::can_see(state, req.actor(), c))?;
    let (mut doc, _) = start(req, p, format!("{}: {}", c.label(), c.subject), &c.date.pretty());
    doc.blocks.push(Block::Facts(vec![
        ("Type".into(), dash(&c.kind)),
        ("From".into(), person(state, c.created.by)),
        ("To".into(), names(state, &c.to)),
        ("Copied".into(), names(state, &c.cc)),
        ("Reply wanted by".into(), c.reply_by.map(|d| d.pretty()).unwrap_or_else(|| "—".into())),
    ]));
    doc.blocks.push(Block::Paragraph(c.body.clone()));
    if !c.replies.is_empty() {
        doc.blocks.push(Block::Heading("Replies".into()));
        for r in &c.replies {
            doc.blocks.push(Block::Facts(vec![("Reply".into(), format!("{}, {}", person(state, r.by), req.when(r.at)))]));
            doc.blocks.push(Block::Paragraph(r.body.clone()));
        }
    }
    Some((file_name(&c.label()), render(&doc)))
}

/// The item set out as a letter: date, addressees, a subject line, the text, enclosures and the author's name.
pub fn correspondence_letter(req: &Req<'_>, id: Id<Project>, cid: Id<buildkiosk_engine::model::correspondence::Correspondence>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Correspondence)?;
    let c = state
        .correspondence
        .get(cid)
        .filter(|c| c.project == id && c.deleted.is_none() && buildkiosk_engine::cmd::correspondence::can_see(state, req.actor(), c))?;
    let kind = if c.kind.is_empty() { "Letter".to_string() } else { c.kind.clone() };
    let (mut doc, _) = start(req, p, format!("{kind} {}", c.label()), &c.date.pretty());
    let mut facts = vec![("Date".to_string(), c.date.pretty()), ("To".to_string(), names(state, &c.to))];
    if !c.cc.is_empty() {
        facts.push(("Copy".into(), names(state, &c.cc)));
    }
    if let Some(s) = &c.incoming {
        facts.insert(0, ("From".into(), if s.email.is_empty() { s.name.clone() } else { format!("{} <{}>", s.name, s.email) }));
    }
    facts.push(("Re".into(), c.subject.clone()));
    let rfis: Vec<String> = c
        .rfis
        .iter()
        .filter_map(|r| state.rfis.get(*r))
        .filter(|r| r.project == id && view::can_see_rfi(state, req.actor(), r))
        .map(|r| format!("RFI #{} {}", r.number, r.subject))
        .collect();
    let subs: Vec<String> = c
        .submittals
        .iter()
        .filter_map(|x| state.submittals.get(*x))
        .filter(|x| x.project == id && view::can_see_submittal(state, req.actor(), x))
        .map(|x| format!("Submittal #{} {}", x.number, x.title))
        .collect();
    let refers: Vec<String> = rfis.into_iter().chain(subs).collect();
    if !refers.is_empty() {
        facts.push(("Refers to".into(), refers.join("; ")));
    }
    if let Some(d) = c.reply_by {
        facts.push(("Reply requested by".into(), d.pretty()));
    }
    doc.blocks.push(Block::Facts(facts));
    doc.blocks.push(Block::Space);
    for para in c.body.split("\n\n") {
        if !para.trim().is_empty() {
            doc.blocks.push(Block::Paragraph(para.trim().to_string()));
        }
    }
    doc.blocks.push(Block::Space);
    if c.incoming.is_none() {
        doc.blocks.push(Block::Paragraph("Sincerely,".into()));
        doc.blocks.push(Block::Space);
        doc.blocks.push(Block::Paragraph(person(state, c.created.by)));
    }
    if !c.attachments.is_empty() {
        doc.blocks.push(Block::Space);
        doc.blocks.push(Block::Paragraph(format!("Enclosures: {}", c.attachments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", "))));
    }
    Some((file_name(&format!("{} letter", c.label())), render(&doc)))
}

pub fn estimate(req: &Req<'_>, id: Id<Project>, eid: Id<buildkiosk_engine::model::estimate::Estimate>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Estimating)?;
    let e = state.estimates.get(eid).filter(|e| e.project == id && e.deleted.is_none())?;
    let (mut doc, _) = start(
        req,
        p,
        format!("{}: {}", e.label(), e.name),
        if e.status == buildkiosk_engine::model::estimate::EstimateStatus::Final { "Issued" } else { "Draft" },
    );
    if !e.notes.is_empty() {
        doc.blocks.push(Block::Paragraph(e.notes.clone()));
    }
    let code = |c| state.cost_codes.get(c).map(|c| format!("{} {}", c.code, c.description)).unwrap_or_default();
    let mut rows: Vec<Vec<String>> = e
        .lines
        .iter()
        .map(|l| {
            let q = if l.quantity_milli % 1000 == 0 {
                (l.quantity_milli / 1000).to_string()
            } else {
                format!("{}.{}", l.quantity_milli / 1000, format!("{:03}", l.quantity_milli % 1000).trim_end_matches('0'))
            };
            vec![
                code(l.cost_code),
                l.description.clone(),
                q,
                l.unit.clone(),
                l.unit_cost.to_string(),
                l.total().map(|t| t.to_string()).unwrap_or_default(),
            ]
        })
        .collect();
    for a in e.alternates.iter().filter(|a| a.accepted) {
        rows.push(vec![code(a.cost_code), format!("Alternate: {}", a.name), String::new(), String::new(), String::new(), a.amount.to_string()]);
    }
    if let Some(t) = e.totals() {
        rows.push(vec![String::new(), "Subtotal".into(), String::new(), String::new(), String::new(), t.subtotal.to_string()]);
        for (m, a) in e.markups.iter().zip(&t.markups) {
            rows.push(vec![String::new(), format!("{} ({})", m.name, m.percent), String::new(), String::new(), String::new(), a.to_string()]);
        }
        rows.push(vec![String::new(), "Total".into(), String::new(), String::new(), String::new(), t.total.to_string()]);
    }
    doc.blocks.push(table(&["Cost code", "Description", "Qty", "Unit", "Unit cost", "Amount"], &[3.0, 3.5, 1.0, 0.8, 1.5, 1.7], &[2, 4, 5], rows));
    let open: Vec<_> = e.alternates.iter().filter(|a| !a.accepted).collect();
    if !open.is_empty() {
        doc.blocks.push(Block::Heading("Alternates not included".into()));
        let alt_rows = open.iter().map(|a| vec![a.name.clone(), a.description.clone(), a.amount.to_string()]).collect();
        doc.blocks.push(table(&["Alternate", "Description", "Add / deduct"], &[3.0, 4.5, 1.8], &[2], alt_rows));
    }
    Some((file_name(&e.label()), render(&doc)))
}

pub fn budget(req: &Req<'_>, id: Id<Project>, chosen: Option<Id<buildkiosk_engine::model::finance::BudgetView>>) -> Option<Pdf> {
    use buildkiosk_engine::model::finance::BudgetGroup;
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Budget)?;
    let rows = view::budget_rows(state, id).ok()?;
    let t = view::budget_totals(&rows).ok()?;
    let (saved, columns, group) = crate::finance::budget_layout(state, id, chosen);
    let sub = match saved {
        Some(v) => format!("{} · as of {}", v.name, req.today().pretty()),
        None => format!("As of {}", req.today().pretty()),
    };
    let (mut doc, _) = start(req, p, "Budget".into(), &sub);
    let code_of = |r: &view::BudgetRow| state.cost_codes.get(r.cost_code).map(|c| c.code.clone()).unwrap_or_default();
    let mut body: Vec<Vec<String>> = Vec::new();
    let mut at = 0;
    while let Some(first) = rows.get(at) {
        let key = (group == BudgetGroup::Division).then(|| view::code_division(&code_of(first)).to_string());
        let end = rows
            .iter()
            .skip(at)
            .position(|r| (group == BudgetGroup::Division).then(|| view::code_division(&code_of(r)).to_string()) != key)
            .map_or(rows.len(), |n| at + n);
        let list: Vec<&view::BudgetRow> = rows.iter().skip(at).take(end - at).collect();
        if let Some(k) = &key {
            let mut head = vec![format!("Division {k}")];
            head.extend(columns.iter().map(|_| String::new()));
            body.push(head);
        }
        for r in &list {
            let c = state.cost_codes.get(r.cost_code);
            let mut line = vec![format!("{} {}", c.map(|c| c.code.as_str()).unwrap_or("?"), c.map(|c| c.description.as_str()).unwrap_or(""))];
            line.extend(columns.iter().map(|col| crate::finance::budget_text(r, *col)));
            body.push(line);
        }
        if key.is_some() {
            let owned: Vec<view::BudgetRow> = list.iter().map(|r| (*r).clone()).collect();
            if let Ok(st) = view::budget_totals(&owned) {
                let mut line = vec!["Subtotal".to_string()];
                line.extend(columns.iter().map(|col| crate::finance::total_text(&st, *col)));
                body.push(line);
            }
        }
        at = end;
    }
    let mut total = vec!["Total".to_string()];
    total.extend(columns.iter().map(|col| crate::finance::total_text(&t, *col)));
    body.push(total);
    let mut heads: Vec<&str> = vec!["Cost code"];
    heads.extend(columns.iter().map(|c| c.label()));
    let mut widths: Vec<f32> = vec![3.4];
    widths.extend(columns.iter().map(|_| 1.6));
    let right: Vec<usize> = (1..=columns.len()).collect();
    doc.blocks.push(table(&heads, &widths, &right, body));
    doc.blocks.push(Block::Paragraph("Cost to date is committed cost plus direct costs. Projected is cost to date plus any forecast to complete; where there is none it follows the project's forecasting method (by default the revised budget is assumed to be spent). A negative over / under is a projected overrun.".into()));
    Some(("budget.pdf".into(), render(&doc)))
}

pub fn drawings(req: &Req<'_>, id: Id<Project>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Drawings)?;
    let (mut doc, _) = start(req, p, "Drawing log".into(), &format!("As of {}", req.today().pretty()));
    let mut list: Vec<_> = state.drawings.iter().filter(|d| d.project == id && d.deleted.is_none()).collect();
    list.sort_by_key(|d| d.number.to_lowercase());
    let rows = list
        .iter()
        .map(|d| {
            let c = d.current();
            vec![
                d.number.clone(),
                d.title.clone(),
                d.discipline.label().to_string(),
                d.area.clone(),
                c.map(|c| c.label.clone()).unwrap_or_default(),
                c.and_then(|c| c.date).map(|x| x.to_string()).unwrap_or_default(),
            ]
        })
        .collect();
    doc.blocks.push(table(&["Sheet", "Title", "Discipline", "Area", "Rev.", "Date"], &[1.3, 4.0, 1.8, 1.8, 0.8, 1.5], &[], rows));
    Some(("drawing-log.pdf".into(), render(&doc)))
}

pub fn schedule(req: &Req<'_>, id: Id<Project>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Schedule)?;
    let (mut doc, _) = start(req, p, "Schedule".into(), &format!("As of {}", req.today().pretty()));
    let rows = sched::rows(state, id, req.today())
        .into_iter()
        .map(|r| {
            vec![
                r.number.to_string(),
                format!("{}{}", "  ".repeat(r.depth), r.name),
                r.start.to_string(),
                r.finish.to_string(),
                if r.milestone { "◆".into() } else { r.duration.to_string() },
                format!("{}%", r.percent),
                r.float.map(|f| f.to_string()).unwrap_or_default(),
                if r.critical {
                    "Critical".into()
                } else if r.late {
                    "Late".into()
                } else {
                    String::new()
                },
            ]
        })
        .collect();
    doc.blocks.push(table(
        &["#", "Task", "Start", "Finish", "Days", "Done", "Float", ""],
        &[0.6, 4.2, 1.5, 1.5, 0.8, 0.9, 0.8, 1.1],
        &[0, 4, 5, 6],
        rows,
    ));
    doc.blocks.push(Block::Paragraph(
        "Days are working days of the project calendar. Float is the working days a task can slip without moving the last finish.".into(),
    ));
    Some(("schedule.pdf".into(), render(&doc)))
}

/// The work under way or starting in each of the next `weeks` weeks from `from`.
pub fn lookahead(req: &Req<'_>, id: Id<Project>, weeks: i64, from: Option<buildkiosk_core::Date>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Schedule)?;
    let from = from.unwrap_or_else(|| req.today());
    let weeks = weeks.clamp(1, 12);
    let (mut doc, _) = start(req, p, "Look ahead".into(), &format!("{weeks} weeks from {}", from.pretty()));
    let all = sched::rows(state, id, req.today());
    for i in 0..weeks {
        let (Some(a), Some(b)) = (from.add_days(i * 7), from.add_days(i * 7 + 6)) else { break };
        doc.blocks.push(Block::Heading(format!("Week {} · {} to {}", i + 1, a.pretty(), b.pretty())));
        let rows: Vec<Vec<String>> = all
            .iter()
            .filter(|r| !r.summary && r.start <= b && r.finish >= a && (r.percent < 100 || r.milestone))
            .map(|r| {
                let company = state
                    .schedule_tasks
                    .get(r.id)
                    .and_then(|t| t.company)
                    .and_then(|c| state.vendors.get(c))
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                vec![
                    r.number.to_string(),
                    r.name.clone(),
                    r.start.to_string(),
                    r.finish.to_string(),
                    format!("{}%", r.percent),
                    company,
                    if r.critical {
                        "Critical".into()
                    } else if r.late {
                        "Late".into()
                    } else {
                        String::new()
                    },
                ]
            })
            .collect();
        if rows.is_empty() {
            doc.blocks.push(Block::Paragraph("Nothing scheduled.".into()));
        } else {
            doc.blocks.push(table(&["#", "Task", "Start", "Finish", "Done", "Company", ""], &[0.6, 4.0, 1.5, 1.5, 0.9, 2.4, 1.1], &[0, 4], rows));
        }
        doc.blocks.push(Block::Space);
    }
    Some(("look-ahead.pdf".into(), render(&doc)))
}

pub fn project_report(req: &Req<'_>, id: Id<Project>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Reports)?;
    let (mut doc, _) = start(req, p, "Project report".into(), &format!("As of {}", req.today().pretty()));
    let rows = report::project_summary(state, req.actor(), id, req.today())
        .into_iter()
        .map(|r| vec![r.tool.to_string(), r.total.to_string(), r.open.to_string(), r.overdue.to_string()])
        .collect();
    doc.blocks.push(table(&["Tool", "Total", "Open", "Past due"], &[5.0, 1.2, 1.2, 1.2], &[1, 2, 3], rows));
    if let Some(me) = req.actor().person() {
        let todo = report::waiting_on(state, req.actor(), id, me);
        if !todo.is_empty() {
            doc.blocks.push(Block::Heading("Waiting on you".into()));
            let rows = todo.into_iter().map(|t| vec![t.label, t.title, t.due.map(|d| d.to_string()).unwrap_or_default()]).collect();
            doc.blocks.push(table(&["Item", "What", "Due"], &[2.0, 6.0, 1.5], &[], rows));
        }
    }
    Some(("project-report.pdf".into(), render(&doc)))
}

/// A punch list, observations, tasks or incidents list as a document for a site walk. Takes the
/// same filter and grouping as the page; each section starts a new heading.
pub fn work_list<T: WorkPage>(req: &Req<'_>, id: Id<Project>, title: &str, filter: &Filter) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, T::TOOL)?;
    let (mut doc, _) = start(req, p, title.into(), &format!("As of {}", req.today().pretty()));
    let all = items_for::<T>(state, req.actor(), id);
    let shown = filtered::<T>(req, &all, filter);
    for (heading, items) in sections::<T>(req, &shown, filter.group) {
        if filter.group != Group::None {
            doc.blocks.push(Block::Heading(format!("{heading} ({})", items.len())));
        }
        let rows = items
            .iter()
            .map(|i| {
                vec![
                    i.number.to_string(),
                    i.title.clone(),
                    i.location.clone(),
                    i.assignees.iter().map(|a| view::person_name(state, *a)).collect::<Vec<_>>().join(", "),
                    i.due.map(|d| d.to_string()).unwrap_or_default(),
                    i.status.label().to_string(),
                ]
            })
            .collect();
        doc.blocks.push(table(&["#", "Item", "Location", "Assigned to", "Due", "Status"], &[0.6, 4.0, 2.0, 2.2, 1.4, 1.5], &[0], rows));
    }
    Some((file_name(title), render(&doc)))
}

pub fn form_record(req: &Req<'_>, id: Id<Project>, rid: Id<buildkiosk_engine::model::form::FormRecord>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Forms)?;
    let r = state.form_records.get(rid).filter(|r| r.project == id && r.deleted.is_none())?;
    let (mut doc, _) =
        start(req, p, format!("{}{}", r.label(), if r.title.is_empty() { String::new() } else { format!(": {}", r.title) }), r.status.label());
    doc.blocks.push(Block::Facts(vec![("Started".into(), format!("{} by {}", req.when(r.created.at), person(state, r.created.by)))]));
    let facts = r
        .shown_fields()
        .into_iter()
        .map(|f| {
            let a = r.answers.get(&f.key).cloned().unwrap_or_default();
            let a = match a.as_str() {
                "yes" => "Yes".to_string(),
                "no" => "No".to_string(),
                "" => "—".to_string(),
                other => other.to_string(),
            };
            (f.label.clone(), a)
        })
        .collect();
    doc.blocks.push(Block::Facts(facts));
    if !r.signers.is_empty() {
        doc.blocks.push(Block::Heading("Signatures".into()));
        let rows = r
            .signers
            .iter()
            .map(|role| match r.signatures.iter().find(|x| x.role.eq_ignore_ascii_case(role)) {
                Some(x) => vec![role.clone(), x.name.clone(), req.when(x.stamp.at)],
                None => vec![role.clone(), "Not signed".into(), String::new()],
            })
            .collect();
        doc.blocks.push(table(&["Role", "Signed by", "When"], &[3.0, 4.0, 4.0], &[], rows));
    }
    Some((file_name(&r.label()), render(&doc)))
}

pub fn action_plan(req: &Req<'_>, id: Id<Project>, aid: Id<buildkiosk_engine::model::action_plan::ActionPlan>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::ActionPlans)?;
    let a = state.action_plans.get(aid).filter(|a| a.project == id && a.deleted.is_none())?;
    let (done, all) = a.progress();
    let (mut doc, _) = start(req, p, format!("{}: {}", a.label(), a.title), &format!("{} · {done} of {all} items done", a.status.label()));
    if !a.description.is_empty() {
        doc.blocks.push(Block::Paragraph(a.description.clone()));
    }
    for s in &a.sections {
        doc.blocks.push(Block::Heading(s.title.clone()));
        let rows = s
            .items
            .iter()
            .map(|i| {
                vec![
                    if i.done { "[x]".into() } else { "[ ]".into() },
                    i.text.clone(),
                    person(state, i.assignee),
                    i.due.map(|d| d.to_string()).unwrap_or_default(),
                    i.note.clone(),
                ]
            })
            .collect();
        doc.blocks.push(table(&["", "Item", "Owner", "Due", "Note"], &[0.6, 4.5, 2.0, 1.5, 3.0], &[], rows));
        for i in s.items.iter().filter(|i| !i.comments.is_empty()) {
            for c in &i.comments {
                doc.blocks.push(Block::Paragraph(format!(
                    "{} ({}, {}): {}",
                    i.text,
                    person(state, c.stamp.by),
                    c.stamp.at.date_at(state.utc_offset()),
                    c.text
                )));
            }
        }
    }
    if !a.approvers.is_empty() {
        doc.blocks.push(Block::Heading("Approval".into()));
        let rows = a
            .approvers
            .iter()
            .map(|who| match a.approvals.iter().find(|x| x.person == *who) {
                Some(x) => vec![
                    person(state, Some(*who)),
                    if x.approved { "Approved".into() } else { "Declined".into() },
                    x.stamp.at.date_at(state.utc_offset()).to_string(),
                    x.note.clone(),
                ],
                None => vec![person(state, Some(*who)), "Waiting".into(), String::new(), String::new()],
            })
            .collect();
        doc.blocks.push(table(&["Approver", "Answer", "Date", "Note"], &[3.0, 2.0, 2.0, 4.0], &[], rows));
    }
    if let Some(c) = a.completed {
        doc.blocks.push(Block::Paragraph(format!("Signed off by {} on {}.", person(state, c.by), c.at.date_at(state.utc_offset()))));
    }
    Some((file_name(&a.label()), render(&doc)))
}

pub fn bid_package(req: &Req<'_>, id: Id<Project>, bid: Id<buildkiosk_engine::model::bidding::BidPackage>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Bidding)?;
    let b = state.bid_packages.get(bid).filter(|b| b.project == id && b.deleted.is_none())?;
    let (mut doc, _) = start(req, p, format!("{}: {}", b.label(), b.title), "Bid leveling");
    if !b.scope.is_empty() {
        doc.blocks.push(Block::Heading("Scope".into()));
        doc.blocks.push(Block::Paragraph(b.scope.clone()));
    }
    let low = b.standing().first().map(|x| x.amount);
    let rows = b
        .standing()
        .iter()
        .map(|x| {
            vec![
                view::vendor_name(state, x.vendor),
                x.amount.to_string(),
                match low {
                    Some(l) if l != x.amount => x.amount.checked_sub(l).map(|d| d.to_string()).unwrap_or_default(),
                    _ => "Low".into(),
                },
                x.received.to_string(),
                x.notes.clone(),
            ]
        })
        .collect();
    doc.blocks.push(table(&["Company", "Bid", "Over low", "Received", "Notes"], &[3.0, 1.6, 1.4, 1.4, 4.0], &[1, 2], rows));
    if let Some(v) = b.awarded_to {
        doc.blocks.push(Block::Paragraph(format!("Awarded to {}.", view::vendor_name(state, v))));
    }
    Some((file_name(&b.label()), render(&doc)))
}

pub fn submittal_log(req: &Req<'_>, id: Id<Project>) -> Option<Pdf> {
    let p = project_of(req, id, Tool::Submittals)?;
    let log = crate::submittals::log_data(req, id)?;
    let (mut doc, _) = start(req, p, "Submittal log".into(), &format!("As of {}", req.today().pretty()));
    let avg = log.average_days.map(|d| d.to_string()).unwrap_or_else(|| "—".into());
    doc.blocks.push(Block::Facts(vec![
        ("Submittals".into(), log.total.to_string()),
        ("Drafts".into(), log.draft.to_string()),
        ("In review".into(), log.in_review.to_string()),
        ("Overdue".into(), log.overdue.to_string()),
        ("Closed".into(), log.closed.to_string()),
        ("Average days in review".into(), avg),
    ]));
    let rows = log
        .rows
        .iter()
        .map(|r| {
            vec![
                dash(&r.spec),
                r.number.clone(),
                r.title.clone(),
                r.status.label().to_string(),
                dash(&r.decision),
                dash(&r.ball),
                dash(&r.due),
                r.days.map(|d| d.to_string()).unwrap_or_else(|| "—".into()),
            ]
        })
        .collect();
    doc.blocks.push(table(
        &["Spec", "#", "Title", "Status", "Decision", "Ball in court", "Due", "Days"],
        &[1.3, 0.7, 3.2, 1.2, 1.6, 2.0, 1.4, 0.7],
        &[7],
        rows,
    ));
    Some(("submittal-log.pdf".into(), render(&doc)))
}

pub fn daily_log_week(req: &Req<'_>, id: Id<Project>, any_day: buildkiosk_core::Date) -> Option<Pdf> {
    let p = project_of(req, id, Tool::DailyLog)?;
    let w = crate::daily_logs::week_data(req, id, any_day)?;
    let end = w.start.add_days(6).unwrap_or(w.start);
    let (mut doc, _) = start(req, p, "Daily log: weekly summary".into(), &format!("{} to {}", w.start.pretty(), end.pretty()));
    doc.blocks.push(Block::Facts(vec![
        ("Days logged".into(), format!("{} of 7", w.logged)),
        ("Worker-days".into(), w.worker_days.to_string()),
        ("Worker-hours".into(), hours_text(w.tenths)),
    ]));
    let rows = w
        .days
        .iter()
        .map(|d| {
            vec![
                format!("{} {}", d.date.weekday().short(), d.date.pretty()),
                if d.log.is_some() { dash(&d.weather) } else { "No log".into() },
                if d.log.is_some() { d.workers.to_string() } else { String::new() },
                if d.log.is_some() { hours_text(d.tenths) } else { String::new() },
                d.work.clone(),
                d.delays.clone(),
                d.safety.clone(),
            ]
        })
        .collect();
    doc.blocks.push(table(
        &["Day", "Weather", "Workers", "Hours", "Work performed", "Delays", "Safety"],
        &[1.9, 1.6, 0.9, 0.9, 3.4, 2.0, 1.8],
        &[2, 3],
        rows,
    ));
    Some((file_name(&format!("daily-log-week-{}", w.start)), render(&doc)))
}

/// An inspection's checklist to print: answered lines show the result, open ones leave boxes to tick.
pub fn inspection(req: &Req<'_>, id: Id<Project>, iid: Id<buildkiosk_engine::model::inspection::Inspection>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Inspections)?;
    let i = state.inspections.get(iid).filter(|i| i.project == id && view::can_see_inspection(state, req.actor(), i))?;
    let (mut doc, _) = start(req, p, format!("{}: {}", i.label(), i.title), &format!("Status: {}", i.status.label()));
    doc.blocks.push(Block::Facts(vec![
        ("Checklist".into(), dash(&i.template_name)),
        ("Location".into(), dash(&i.location)),
        ("Inspector".into(), person(state, i.inspector)),
        ("Manager".into(), person(state, i.manager)),
        ("Scheduled".into(), i.scheduled.map(|d| d.pretty()).unwrap_or_else(|| "—".into())),
        ("Answered".into(), format!("{} of {}, {} failed", i.answered(), i.items.len(), i.failed())),
        (
            "Signed".into(),
            i.signed
                .as_ref()
                .map(|s| format!("{}, {}", s.name, req.when(s.at)))
                .unwrap_or_else(|| if i.require_signature { "Signature required: ____________________".into() } else { "—".into() }),
        ),
    ]));
    let mut section: Option<&str> = None;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let flush = |doc: &mut Doc, section: Option<&str>, rows: &mut Vec<Vec<String>>| {
        if rows.is_empty() {
            return;
        }
        if let Some(s) = section.filter(|s| !s.is_empty()) {
            doc.blocks.push(Block::Heading(s.to_string()));
        }
        doc.blocks.push(table(&["#", "Item", "Result", "Note"], &[0.6, 4.6, 2.4, 3.0], &[0], std::mem::take(rows)));
    };
    for (n, item) in i.items.iter().enumerate() {
        if section != Some(item.section.as_str()) {
            flush(&mut doc, section, &mut rows);
            section = Some(item.section.as_str());
        }
        let result = match item.answer {
            Some(a) => a.label().to_string(),
            None => "[  ] Pass  [  ] Fail  [  ] N/A".to_string(),
        };
        rows.push(vec![(n + 1).to_string(), item.text.clone(), result, item.note.clone()]);
    }
    flush(&mut doc, section, &mut rows);
    Some((file_name(&i.label()), render(&doc)))
}

/// One incident as a report to print and sign.
pub fn incident(req: &Req<'_>, id: Id<Project>, iid: Id<buildkiosk_engine::model::incident::Incident>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Incidents)?;
    let i = state
        .incidents
        .get(iid)
        .filter(|i| i.project == id && buildkiosk_engine::workitem::can_see(state, req.actor(), *i) && i.deleted.is_none())?;
    let (mut doc, _) = start(req, p, format!("{}: {}", i.label(), i.title), &format!("Status: {}", i.status.label()));
    doc.blocks.push(Block::Facts(vec![
        ("Occurred".into(), i.occurred.map(|d| d.pretty()).unwrap_or_else(|| "—".into())),
        ("Severity".into(), i.severity.label().to_string()),
        ("Recordable".into(), if i.recordable { "Yes" } else { "No" }.to_string()),
        ("Type".into(), dash(&i.kind)),
        ("Location".into(), dash(&i.location)),
        ("Reported by".into(), person(state, i.created.by)),
        ("Manager".into(), person(state, i.manager)),
        ("Assigned to".into(), names(state, &i.assignees)),
    ]));
    for (heading, body) in [
        ("What happened", &i.description),
        ("People involved and witnesses", &i.people_involved),
        ("Root cause", &i.root_cause),
        ("Corrective action", &i.corrective_action),
    ] {
        doc.blocks.push(Block::Heading(heading.into()));
        doc.blocks.push(Block::Paragraph(if body.is_empty() { "—".into() } else { body.clone() }));
    }
    if !i.comments.is_empty() {
        doc.blocks.push(Block::Heading("Comments".into()));
        for c in &i.comments {
            doc.blocks.push(Block::Facts(vec![("Comment".into(), format!("{}, {}", person(state, c.by), req.when(c.at)))]));
            doc.blocks.push(Block::Paragraph(c.body.clone()));
        }
    }
    doc.blocks.push(Block::Heading("Signatures".into()));
    doc.blocks.push(Block::Facts(vec![
        ("Reported by".into(), "______________________________   Date: ____________".into()),
        ("Reviewed by".into(), "______________________________   Date: ____________".into()),
    ]));
    Some((file_name(&i.label()), render(&doc)))
}

/// The injury log of a year, to post or file.
pub fn injury_log(req: &Req<'_>, id: Id<Project>, year: Option<i32>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Incidents)?;
    let year = year.unwrap_or_else(|| req.today().ymd().0);
    let list = buildkiosk_engine::cmd::incident::incidents_in_year(state, req.actor(), id, year);
    let recordable: Vec<_> = list.iter().filter(|i| i.recordable).collect();
    let lost_time = recordable.iter().filter(|i| i.severity.is_lost_time()).count() as u64;
    let (sheets, other) = buildkiosk_engine::cmd::incident::hours_worked(state, id, year);
    let tenths = sheets.saturating_add(other);
    let rate = |cases: u64| buildkiosk_engine::cmd::incident::rate(cases, tenths).unwrap_or_else(|| "—".into());
    let (mut doc, _) = start(req, p, format!("Injury log, {year}"), &format!("As of {}", req.today().pretty()));
    doc.blocks.push(Block::Facts(vec![
        ("Incidents".into(), list.len().to_string()),
        ("Recordable cases".into(), recordable.len().to_string()),
        ("Lost-time cases".into(), lost_time.to_string()),
        (
            "Hours worked".into(),
            format!("{}.{} ({}.{} on timesheets, {}.{} other)", tenths / 10, tenths % 10, sheets / 10, sheets % 10, other / 10, other % 10),
        ),
        ("Recordable rate".into(), format!("{} per 200,000 hours", rate(recordable.len() as u64))),
        ("Lost-time rate".into(), format!("{} per 200,000 hours", rate(lost_time))),
    ]));
    doc.blocks.push(Block::Heading("Recordable cases".into()));
    let rows: Vec<Vec<String>> = recordable
        .iter()
        .map(|i| {
            vec![
                i.number.to_string(),
                i.occurred.map(|d| d.to_string()).unwrap_or_default(),
                i.title.clone(),
                i.people_involved.clone(),
                i.severity.label().to_string(),
            ]
        })
        .collect();
    doc.blocks.push(table(&["#", "Date", "Case", "Person", "Severity"], &[0.6, 1.6, 4.0, 3.0, 1.8], &[0], rows));
    doc.blocks.push(Block::Paragraph(
        "Rates are cases times 200,000 divided by hours worked. Only incidents the person printing this can see are included.".into(),
    ));
    Some((file_name(&format!("injury log {year}")), render(&doc)))
}

/// A sheet to pass round the room: who was invited, with a line to sign, and blank lines for the rest.
pub fn meeting_signin(req: &Req<'_>, id: Id<Project>, mid: Id<buildkiosk_engine::model::meeting::Meeting>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Meetings)?;
    let m = state.meetings.get(mid).filter(|m| m.project == id && m.deleted.is_none())?;
    let (mut doc, _) = start(
        req,
        p,
        format!("Sign-in: {}", m.title),
        &format!(
            "{}{}{}",
            m.date.pretty(),
            if m.time.is_empty() { String::new() } else { format!(" · {}", m.time) },
            if m.location.is_empty() { String::new() } else { format!(" · {}", m.location) }
        ),
    );
    let mut rows: Vec<Vec<String>> = m
        .attendees
        .iter()
        .map(|a| {
            let who = state.people.get(a.person);
            let company = who.and_then(|w| w.vendor).map(|v| view::vendor_name(state, v)).unwrap_or_default();
            vec![view::person_name(state, a.person), company, String::new()]
        })
        .collect();
    // Room for people who weren't invited.
    for _ in 0..8 {
        rows.push(vec![String::new(); 3]);
    }
    doc.blocks.push(table(&["Name", "Company", "Signature"], &[3.5, 3.0, 4.0], &[], rows));
    Some((file_name(&format!("sign-in {}", m.label())), render(&doc)))
}

/// The company report (a saved one, or the standard portfolio) as a document.
pub fn company_report(req: &Req<'_>, report: Option<Id<buildkiosk_engine::model::saved_report::SavedReport>>) -> Option<Pdf> {
    let crate::company_reports::TableText { title, headers, mut rows, totals, right } = crate::company_reports::table_text(req, report)?;
    let company = req.engine.state().company().map(|c| c.name.clone()).unwrap_or_default();
    let mut doc = Doc { title: title.clone(), subtitle: format!("As of {}", req.today().pretty()), running_head: company, blocks: Vec::new() };
    rows.push(totals);
    let n = headers.len().max(2);
    let first = 3.4_f32;
    let rest = (10.0 - first) / (n - 1) as f32;
    let widths: Vec<f32> = std::iter::once(first).chain(std::iter::repeat_n(rest, n - 1)).collect();
    let header_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
    let right_cols: Vec<usize> = right.iter().enumerate().filter(|(_, r)| **r).map(|(i, _)| i).collect();
    doc.blocks.push(table(&header_refs, &widths, &right_cols, rows));
    doc.blocks.push(Block::Paragraph("Only the projects you can open, and only what you can see of each, are included.".into()));
    Some((file_name(&title), render(&doc)))
}
