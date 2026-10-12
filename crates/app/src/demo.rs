//! The demo company: entirely fictional names, for trying the app.

use buildkiosk_core::Id;
use buildkiosk_engine::Actor;
use buildkiosk_store::{Credentials, DataDir, Tenant};
use serde_json::{Value, json};

use crate::now;

/// Every demo user signs in with this password.
const PASSWORD: &str = "demo-password-1";

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
    }
    t.store.save_credentials(&creds).map_err(|e| e.to_string())?;
    println!("Demo company created in {dir}.");
    println!("Start it with:  buildkiosk serve --data {dir}");
    println!("Sign in with ada@example.com (administrator) or priya.raman@example.com (project manager); password: {PASSWORD}");
    Ok(())
}
