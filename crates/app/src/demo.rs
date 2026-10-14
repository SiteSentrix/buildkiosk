//! The demo company: entirely fictional names, for trying the app.

use buildkiosk_core::Id;
use buildkiosk_engine::Actor;
use buildkiosk_store::{Credentials, DataDir, Tenant};
use serde_json::{Value, json};

use crate::now;

/// Every demo user signs in with this password.
pub const PASSWORD: &str = "demo-password-1";

fn exec(t: &mut Tenant, actor: Actor, cmd: &str, params: Value) -> Result<Value, String> {
    t.engine.execute(&mut t.store, actor, now(), cmd, &params).map_err(|e| format!("{cmd}: {e}"))
}

fn id_of(v: &Value, key: &str) -> Result<u64, String> {
    v.get(key).and_then(Value::as_u64).ok_or_else(|| format!("missing `{key}` in {v}"))
}

pub fn create(dir: &str) -> Result<(), String> {
    let data = DataDir::open(dir).map_err(|e| e.to_string())?;
    if !data.companies().map_err(|e| e.to_string())?.is_empty() {
        return Err(format!("{dir} already has a company; use an empty folder for the demo"));
    }
    let mut t = data.create("summit-ridge").map_err(|e| e.to_string())?;
    let out = exec(
        &mut t,
        Actor::System,
        "company.setup",
        json!({"name": "Summit Ridge Builders", "slug": "summit-ridge", "admin_first_name": "Ada", "admin_last_name": "Admin", "admin_email": "ada@example.com"}),
    )?;
    let admin = Actor::Person(Id::new(id_of(&out, "admin")?));
    let mut creds = Credentials::default();
    creds.set_password(id_of(&out, "admin")?, PASSWORD, now()).map_err(|e| e.to_string())?;

    let templates = exec(&mut t, admin, "template.list", json!({}))?;
    let template = |name: &str| -> Result<u64, String> {
        templates
            .as_array()
            .and_then(|l| l.iter().find(|x| x.get("name").and_then(Value::as_str) == Some(name)))
            .and_then(|x| x.get("id").and_then(Value::as_u64))
            .ok_or_else(|| format!("no template {name}"))
    };

    let vendors = [("Harbor Design Studio", "design"), ("Northline Electric", "subcontractor"), ("Keystone Properties", "owner")];
    let mut vendor_ids = Vec::new();
    for (name, kind) in vendors {
        let v = exec(&mut t, admin, "vendor.create", json!({"name": name, "kind": kind}))?;
        vendor_ids.push(id_of(&v, "id")?);
    }
    let vendor = |i: usize| vendor_ids.get(i).copied().unwrap_or(0);

    let team = [
        ("Priya", "Raman", "priya.raman@example.com", "Project manager", None),
        ("Marcus", "Lee", "marcus.lee@example.com", "Superintendent", None),
        ("Elena", "Cruz", "elena.cruz@example.com", "Project engineer", None),
        ("Tom", "Okafor", "tom.okafor@example.com", "Subcontractor", Some(1)),
        ("Hana", "Berg", "hana.berg@example.com", "Owner / design team", Some(0)),
    ];
    let mut people = Vec::new();
    for (first, last, email, tpl, v) in team {
        let mut params = json!({"first_name": first, "last_name": last, "email": email, "is_user": true});
        if let (Some(i), Some(map)) = (v, params.as_object_mut()) {
            map.insert("vendor".into(), json!(vendor(i)));
        }
        let p = exec(&mut t, admin, "person.create", params)?;
        let pid = id_of(&p, "id")?;
        creds.set_password(pid, PASSWORD, now()).map_err(|e| e.to_string())?;
        people.push((pid, template(tpl)?, tpl));
    }
    let person = |i: usize| people.get(i).map(|p| p.0).unwrap_or(0);

    let projects = [("2026-014", "Riverside Medical Office", "Millbrook", "WA"), ("2026-021", "Cedar Street Lofts", "Millbrook", "WA")];
    for (number, name, city, region) in projects {
        let pr = exec(
            &mut t,
            admin,
            "project.create",
            json!({"number": number, "name": name, "city": city, "region": region, "stage": "construction", "owner": vendor(2), "description": "Fictional sample project."}),
        )?;
        let pid = id_of(&pr, "project").or_else(|_| id_of(&pr, "id"))?;
        for (p, tpl, label) in &people {
            let role = match *label {
                "Project manager" => "Project manager",
                "Superintendent" => "Superintendent",
                "Project engineer" => "Project engineer",
                "Subcontractor" => "Electrical",
                _ => "Architect",
            };
            exec(&mut t, admin, "member.add", json!({"project": pid, "person": p, "template": tpl, "role": role}))?;
        }
        let pm = Actor::Person(Id::new(person(0)));
        let rfis = [
            ("Clarify ceiling height at corridor 2", "The reflected ceiling plan shows 9'-0\" but the section shows 8'-6\". Which governs?", true),
            (
                "Panel schedule conflicts with one-line diagram",
                "Panel LP-2 shows 100A on the schedule and 125A on the one-line. Please confirm.",
                true,
            ),
            ("Exterior sealant colour", "The finish schedule lists two sealant colours for the east elevation. Please confirm.", false),
        ];
        for (i, (subject, question, open)) in rfis.iter().enumerate() {
            let mut params = json!({"project": pid, "subject": subject, "question": question, "manager": person(0), "assignees": [person(4)], "spec_section": "07 90 00"});
            if *open && let Some(map) = params.as_object_mut() {
                map.insert("open".into(), json!(true));
            }
            let r = exec(&mut t, pm, "rfi.create", params)?;
            if i == 0 {
                let rid = id_of(&r, "id")?;
                exec(
                    &mut t,
                    Actor::Person(Id::new(person(4))),
                    "rfi.respond",
                    json!({"project": pid, "id": rid, "body": "The section governs: 8'-6\" above finished floor. The RCP will be revised."}),
                )?;
            }
        }
        exec(
            &mut t,
            pm,
            "spec.import",
            json!({"project": pid, "text": "05 12 00, Structural steel framing\n08 41 13, Aluminum storefront\n23 74 00, Packaged rooftop units"}),
        )?;
        let folder = exec(&mut t, pm, "folder.create", json!({"project": pid, "name": "Meeting minutes"}))?;
        exec(&mut t, pm, "folder.create", json!({"project": pid, "name": "Bids", "private": true, "access": [person(2)]}))?;
        let note = b"Kickoff meeting\n\nAttendees: owner, architect, general contractor.\nAction: confirm the long-lead steel order.\n";
        let (sha256, size) = t.store.blobs.put(note).map_err(|e| e.to_string())?;
        exec(
            &mut t,
            pm,
            "document.add",
            json!({"project": pid, "folder": id_of(&folder, "id")?, "title": "Kickoff meeting notes", "sha256": sha256, "name": "kickoff-notes.txt", "size": size, "mime": "text/plain"}),
        )?;
        exec(
            &mut t,
            pm,
            "drawing.import",
            json!({"project": pid, "text": "S-201, Foundation plan, structural, Level 0\nM-301, Rooftop unit plan, mechanical, Roof\nE-401, Power plan, electrical, Level 1"}),
        )?;
        let sheet = b"Sheet A-101: first floor plan (demo placeholder, not a real drawing)\n";
        let (sha256, size) = t.store.blobs.put(sheet).map_err(|e| e.to_string())?;
        let plan = exec(
            &mut t,
            pm,
            "drawing.add",
            json!({"project": pid, "number": "A-101", "title": "First floor plan", "discipline": "architectural", "area": "Level 1", "revision": "0", "date": "2026-08-03", "set": "Permit set", "sha256": sha256, "name": "A-101.txt", "size": size, "mime": "text/plain"}),
        )?;
        exec(
            &mut t,
            pm,
            "drawing.add_revision",
            json!({"project": pid, "id": id_of(&plan, "id")?, "revision": "A", "date": "2026-09-14", "set": "Bulletin 1", "note": "Door 104 moved to the east wall"}),
        )?;
        let meeting = exec(
            &mut t,
            pm,
            "meeting.create",
            json!({"project": pid, "title": "Weekly coordination", "date": now().date_at(0).to_string(), "category": "Coordination", "time": "9:00 AM", "location": "Site trailer",
                   "invitees": [person(1), person(2), person(4)],
                   "agenda": [{"title": "Three-week look-ahead", "notes": "Steel arrives the week after next."}, {"title": "Open RFIs", "notes": "Door hardware answer needed."}],
                   "actions": [{"description": "Confirm the steel delivery date", "assignee": person(2)}, {"description": "Answer the door hardware RFI", "assignee": person(4)}]}),
        )?;
        exec(&mut t, pm, "meeting.issue", json!({"project": pid, "id": id_of(&meeting, "id")?}))?;
        let transmittal = exec(
            &mut t,
            pm,
            "transmittal.create",
            json!({"project": pid, "subject": "Bulletin 1 drawings", "purpose": "for_review", "to": [person(4)], "items": [{"description": "Revised sheet A-101", "copies": 2}]}),
        )?;
        exec(&mut t, pm, "transmittal.send", json!({"project": pid, "id": id_of(&transmittal, "id")?}))?;
        // A small schedule starting on the Monday of the current week.
        let today = now().date_at(0);
        let monday = today.add_days(-i64::from(today.weekday() as u8)).unwrap_or(today);
        let site = exec(&mut t, pm, "schedule.add", json!({"project": pid, "name": "Site work", "start": monday.to_string(), "days": 1}))?;
        let site = id_of(&site, "id")?;
        for (name, days, preds, parent, pct) in [
            ("Clear and grub", 3, "", Some(site), 100),
            ("Excavate and grade", 4, "2", Some(site), 60),
            ("Foundations", 8, "3", None, 0),
            ("Structural steel", 10, "4", None, 0),
            ("Rooftop units set", 2, "5", None, 0),
            ("Interior rough-in", 12, "5", None, 0),
            ("Permit close-out paperwork", 3, "", None, 0),
        ] {
            let start = monday.to_string();
            exec(
                &mut t,
                pm,
                "schedule.add",
                json!({"project": pid, "name": name, "start": start, "days": days, "predecessors": preds, "parent": parent, "percent": pct}),
            )?;
        }
        exec(
            &mut t,
            pm,
            "schedule.add",
            json!({"project": pid, "name": "Substantial completion", "start": monday.to_string(), "milestone": true, "predecessors": "6, 7"}),
        )?;
        exec(&mut t, pm, "schedule.recalculate", json!({"project": pid}))?;
        exec(&mut t, pm, "schedule.baseline", json!({"project": pid}))?;
        let eng = Actor::Person(Id::new(person(2)));
        let log = exec(
            &mut t,
            eng,
            "daily_log.create",
            json!({"project": pid, "sky": "partly_cloudy", "high": 61, "low": 44, "work_performed": "Framed second-floor walls; started rough electrical in corridor 2.",
                   "manpower": [{"company": "Northline Electric", "trade": "Electrical", "workers": 4, "hours": "8"}, {"company": "Summit Ridge Builders", "trade": "Carpentry", "workers": 6, "hours": "7.5"}],
                   "equipment": [{"name": "Telehandler", "hours": "4"}], "deliveries": [{"description": "Lumber package", "company": "Cedar Supply"}],
                   "visitors": [{"name": "County inspector", "purpose": "Framing inspection"}]}),
        )?;
        exec(&mut t, eng, "daily_log.submit", json!({"project": pid, "id": id_of(&log, "id")?}))?;
        let items = [
            ("Touch up paint at door 114", "Level 1 corridor", "Paint", "high"),
            ("Replace cracked ceiling tile", "Room 208", "Ceilings", "medium"),
            ("Adjust door closer, east entry", "Main lobby", "Doors", "low"),
        ];
        for (n, (title, location, kind, priority)) in items.iter().enumerate() {
            let item = exec(
                &mut t,
                pm,
                "punch.create",
                json!({"project": pid, "title": title, "location": location, "kind": kind, "priority": priority, "assignees": [person(3)], "due": "2026-10-30"}),
            )?;
            if n == 0 {
                exec(
                    &mut t,
                    Actor::Person(Id::new(person(3))),
                    "punch.complete",
                    json!({"project": pid, "id": id_of(&item, "id")?, "comment": "Two coats applied."}),
                )?;
            }
        }
        let observations = [
            ("Open floor edge at level 2 stair", "Level 2 stair hall", "Safety", "high", "Install guardrail and post a warning sign."),
            ("Concrete cylinders stored in direct sun", "Laydown yard", "Quality", "medium", "Move to the shaded curing box."),
        ];
        for (title, location, kind, priority, action) in observations {
            exec(
                &mut t,
                pm,
                "observation.create",
                json!({"project": pid, "title": title, "location": location, "kind": kind, "priority": priority, "corrective_action": action, "assignees": [person(1)]}),
            )?;
        }
        exec(&mut t, pm, "inspection_template.install_starters", json!({"project": pid}))?;
        let templates = exec(&mut t, pm, "inspection_template.list", json!({"project": pid}))?;
        let framing = templates
            .as_array()
            .and_then(|l| l.iter().find(|x| x.get("name").and_then(Value::as_str) == Some("Framing walk-through")))
            .and_then(|x| x.get("id").and_then(Value::as_u64));
        if let Some(template) = framing {
            let insp = exec(
                &mut t,
                pm,
                "inspection.create",
                json!({"project": pid, "template": template, "location": "Level 2", "inspector": person(1), "scheduled": "2026-10-16"}),
            )?;
            let answers: Vec<Value> = (1..=4).map(|n| json!({"line": n, "answer": if n == 3 { "fail" } else { "pass" }, "note": if n == 3 { "No backing at the sink wall." } else { "" }})).collect();
            exec(
                &mut t,
                Actor::Person(Id::new(person(1))),
                "inspection.answer",
                json!({"project": pid, "id": id_of(&insp, "id")?, "answers": answers}),
            )?;
        }
        let codes = exec(&mut t, pm, "cost_code.list", json!({"project": pid}))?;
        let code = |wanted: &str| {
            codes
                .as_array()
                .and_then(|l| l.iter().find(|c| c.get("code").and_then(Value::as_str) == Some(wanted)))
                .and_then(|c| c.get("id").and_then(Value::as_u64))
                .unwrap_or(0)
        };
        for (c, amount) in [("03-300", "250000"), ("05-100", "420000"), ("06-100", "135000"), ("09-200", "96000")] {
            exec(&mut t, pm, "budget.set_line", json!({"project": pid, "cost_code": code(c), "original": amount}))?;
        }
        let steel = exec(
            &mut t,
            pm,
            "commitment.create",
            json!({"project": pid, "title": "Structural steel supply and erection", "kind": "subcontract", "vendor": vendor(1), "lines": [
                {"cost_code": code("05-100"), "description": "Fabricate and erect", "amount": "384500"}]}),
        )?;
        exec(&mut t, pm, "commitment.approve", json!({"project": pid, "id": id_of(&steel, "id")?}))?;
        exec(
            &mut t,
            pm,
            "commitment.create",
            json!({"project": pid, "title": "Slab concrete", "kind": "purchase_order", "lines": [{"cost_code": code("03-300"), "description": "Ready-mix, 220 cy", "amount": "41250.75"}]}),
        )?;
        for (c, amount, what) in [
            ("03-300", "1850.00", "Pump truck, footings"),
            ("06-100", "12400.40", "Framing lumber package"),
            ("09-200", "3150.00", "Drywall delivery"),
        ] {
            exec(
                &mut t,
                pm,
                "direct_cost.create",
                json!({"project": pid, "cost_code": code(c), "amount": amount, "description": what, "kind": "material"}),
            )?;
        }
        let subs = [("Rooftop unit shop drawings", "23 74 00", "Shop drawing"), ("Storefront glazing product data", "08 41 13", "Product data")];
        for (title, spec, kind) in subs {
            let sub = exec(
                &mut t,
                pm,
                "submittal.create",
                json!({"project": pid, "title": title, "spec_section": spec, "kind": kind, "reviewers": [person(4), person(2)], "ordered": title.starts_with("Storefront"), "open": true}),
            )?;
            if title.starts_with("Rooftop") {
                exec(
                    &mut t,
                    Actor::Person(Id::new(person(4))),
                    "submittal.review",
                    json!({"project": pid, "id": id_of(&sub, "id")?, "decision": "approved_as_noted", "comment": "Provide curb detail at the east unit."}),
                )?;
            }
        }
    }
    t.store.save_credentials(&creds).map_err(|e| e.to_string())?;
    println!("Demo company created in {dir}.");
    println!("Start it with:  buildkiosk serve --data {dir}");
    println!("Sign in with ada@example.com (administrator) or priya.raman@example.com (project manager); password: {PASSWORD}");
    Ok(())
}
