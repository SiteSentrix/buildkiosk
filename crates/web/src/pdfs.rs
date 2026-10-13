//! PDF versions of the documents people print or send: each builder checks the same visibility
//! rules as the matching page and returns `(file name, PDF bytes)`.

use buildkiosk_core::Id;
use buildkiosk_engine::model::daily_log::{DailyLog, hours_text};
use buildkiosk_engine::model::directory::Person;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::rfi::Rfi;
use buildkiosk_engine::model::submittal::Submittal;
use buildkiosk_engine::model::{Choice, CompanyState, Level, Tool};
use buildkiosk_engine::workitem::WorkItem;
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
    let mut rows: Vec<Vec<String>> = t.items.iter().map(|i| vec![i.description.clone(), i.copies.to_string()]).collect();
    rows.extend(t.attachments.iter().map(|f| vec![format!("File: {}", f.name), "1".into()]));
    doc.blocks.push(table(&["Description", "Copies"], &[6.0, 1.0], &[1], rows));
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
    if let Some(t) = e.totals() {
        rows.push(vec![String::new(), "Subtotal".into(), String::new(), String::new(), String::new(), t.subtotal.to_string()]);
        for (m, a) in e.markups.iter().zip(&t.markups) {
            rows.push(vec![String::new(), format!("{} ({})", m.name, m.percent), String::new(), String::new(), String::new(), a.to_string()]);
        }
        rows.push(vec![String::new(), "Total".into(), String::new(), String::new(), String::new(), t.total.to_string()]);
    }
    doc.blocks.push(table(&["Cost code", "Description", "Qty", "Unit", "Unit cost", "Amount"], &[3.0, 3.5, 1.0, 0.8, 1.5, 1.7], &[2, 4, 5], rows));
    Some((file_name(&e.label()), render(&doc)))
}

pub fn budget(req: &Req<'_>, id: Id<Project>) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, Tool::Budget)?;
    let rows = view::budget_rows(state, id).ok()?;
    let t = view::budget_totals(&rows).ok()?;
    let (mut doc, _) = start(req, p, "Budget".into(), &format!("As of {}", req.today().pretty()));
    let line = |label: String, r: (String, String, String, String, String, String)| vec![label, r.0, r.1, r.2, r.3, r.4, r.5];
    let mut body: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let c = state.cost_codes.get(r.cost_code);
            line(
                format!("{} {}", c.map(|c| c.code.as_str()).unwrap_or("?"), c.map(|c| c.description.as_str()).unwrap_or("")),
                (
                    r.original.to_string(),
                    r.changes.to_string(),
                    r.revised().to_string(),
                    r.cost().to_string(),
                    r.projected().to_string(),
                    r.variance().to_string(),
                ),
            )
        })
        .collect();
    body.push(line(
        "Total".into(),
        (
            t.original.to_string(),
            t.changes.to_string(),
            t.revised().to_string(),
            t.cost().to_string(),
            t.projected.to_string(),
            t.variance().to_string(),
        ),
    ));
    doc.blocks.push(table(
        &["Cost code", "Original", "Changes", "Revised", "Cost to date", "Projected", "Over / under"],
        &[3.4, 1.6, 1.3, 1.6, 1.6, 1.6, 1.5],
        &[1, 2, 3, 4, 5, 6],
        body,
    ));
    doc.blocks.push(Block::Paragraph("Cost to date is committed cost plus direct costs. Projected is cost to date plus any forecast to complete, or the revised budget when there is no forecast. A negative over / under is a projected overrun.".into()));
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
        "Days are working days (Monday to Friday). Float is the working days a task can slip without moving the last finish.".into(),
    ));
    Some(("schedule.pdf".into(), render(&doc)))
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

/// A punch list, observations, tasks or incidents list as a document for a site walk.
pub fn work_list<T: WorkItem>(req: &Req<'_>, id: Id<Project>, title: &str) -> Option<Pdf> {
    let state = req.engine.state();
    let p = project_of(req, id, T::TOOL)?;
    let (mut doc, _) = start(req, p, title.into(), &format!("As of {}", req.today().pretty()));
    let mut items: Vec<&T> = T::table(state).iter().filter(|i| i.project() == id && i.deleted.is_none()).collect();
    items.sort_by_key(|i| i.number);
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
        .fields
        .iter()
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
