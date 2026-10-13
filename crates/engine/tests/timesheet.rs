//! Timesheets: logging, the 24-hour day, submit, approve, send back, visibility.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::timesheet::TimeStatus;
use buildkiosk_testkit::World;
use serde_json::json;

fn status(w: &World, id: u64) -> TimeStatus {
    w.engine.state().time_entries.iter().find(|e| e.id.raw() == id).unwrap().status
}

#[test]
fn log_submit_approve_and_return() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let concrete = w.engine.state().cost_codes.iter().find(|c| c.code == "03-300").unwrap().id.raw();

    let a = w.ok(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "7.5", "cost_code": concrete, "description": "Footings"}))
        ["id"]
        .as_u64()
        .unwrap();
    let b = w.ok(sup, "timesheet.add", json!({"project": p, "date": "2026-10-13", "hours": 8}))["id"].as_u64().unwrap();
    w.err(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "17"}));
    w.err(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "0"}));
    w.err(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "abc"}));
    w.err(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "1", "person": eng.raw()}));
    w.ok(pm, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "4", "person": eng.raw()}));

    // Everyone sees their own; only admins see all.
    assert_eq!(w.ok(sup, "timesheet.list", json!({"project": p})).as_array().unwrap().len(), 2);
    assert_eq!(w.ok(pm, "timesheet.list", json!({"project": p})).as_array().unwrap().len(), 3);
    w.err(sup, "timesheet.list", json!({"project": p, "person": eng.raw()}));
    w.err(eng, "timesheet.update", json!({"project": p, "id": a, "hours": "1"}));
    w.ok(sup, "timesheet.update", json!({"project": p, "id": a, "hours": "8"}));
    w.err(sup, "timesheet.update", json!({"project": p, "id": a, "date": "2026-10-13", "hours": "17"}));

    w.err(sup, "timesheet.submit", json!({"project": p, "from": "2026-11-01"}));
    assert_eq!(w.ok(sup, "timesheet.submit", json!({"project": p, "from": "2026-10-12"}))["submitted"], 2);
    assert_eq!((status(&w, a), status(&w, b)), (TimeStatus::Submitted, TimeStatus::Submitted));
    w.err(sup, "timesheet.update", json!({"project": p, "id": a, "hours": "1"}));
    w.err(sup, "timesheet.approve", json!({"project": p, "ids": [a]}));

    w.ok(pm, "timesheet.return", json!({"project": p, "ids": [b], "note": "Wrong cost code"}));
    assert_eq!(status(&w, b), TimeStatus::Draft);
    assert_eq!(w.engine.state().time_entries.iter().find(|e| e.id.raw() == b).unwrap().returned_note, "Wrong cost code");
    w.ok(sup, "timesheet.update", json!({"project": p, "id": b, "cost_code": concrete}));
    w.ok(sup, "timesheet.submit", json!({"project": p, "from": "2026-10-12"}));
    assert_eq!(w.ok(pm, "timesheet.approve", json!({"project": p, "person": sup.raw(), "from": "2026-10-12", "to": "2026-10-18"}))["approved"], 2);
    assert_eq!(status(&w, a), TimeStatus::Approved);
    w.err(pm, "timesheet.approve", json!({"project": p, "ids": [a]}));
    w.err(pm, "timesheet.update", json!({"project": p, "id": a, "hours": "1"}));
    w.err(sup, "timesheet.delete", json!({"project": p, "id": a}));

    let other = w.project("Other");
    w.err(pm, "timesheet.approve", json!({"project": other, "ids": [a]}));
    w.assert_replay();
}

#[test]
fn pay_rates_overtime_crews_and_posting_labour() {
    use buildkiosk_engine::cmd::timesheet::{labour, report};
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let codes: Vec<_> = w.engine.state().cost_codes.iter().map(|c| c.id.raw()).collect();
    let (a, b) = (codes[0], codes[1]);

    // Rates and rules are for admins; a rate on the same day replaces the old one.
    w.err(sup, "timesheet.set_rate", json!({"project": p, "person": sup.raw(), "hourly": "40"}));
    w.err(pm, "timesheet.set_rate", json!({"project": p, "person": sup.raw(), "hourly": "0"}));
    w.err(pm, "timesheet.set_rate", json!({"project": p, "person": sup.raw(), "hourly": "20000"}));
    w.ok(pm, "timesheet.set_rate", json!({"project": p, "person": sup.raw(), "hourly": "35", "from": "2026-10-01"}));
    w.ok(pm, "timesheet.set_rate", json!({"project": p, "person": sup.raw(), "hourly": "40", "from": "2026-10-01"}));
    assert_eq!(w.ok(pm, "timesheet.rates", json!({"project": p})).as_array().unwrap().len(), 1);
    w.err(sup, "timesheet.rates", json!({"project": p}));
    w.err(sup, "timesheet.set_rules", json!({"project": p, "daily_hours": 8}));
    w.err(pm, "timesheet.set_rules", json!({"project": p, "daily_hours": 30}));
    w.err(pm, "timesheet.set_rules", json!({"project": p, "multiplier": "50"}));

    // Monday 10 hours, Tuesday to Friday 8 each, Saturday 4 (2026-10-12 is a Monday).
    let days = [
        ("2026-10-12", "10", a),
        ("2026-10-13", "8", a),
        ("2026-10-14", "8", a),
        ("2026-10-15", "8", 0),
        ("2026-10-16", "8", 0),
        ("2026-10-17", "4", 0),
    ];
    for (d, h, c) in days {
        let mut params = json!({"project": p, "date": d, "hours": h});
        if c != 0 {
            params["cost_code"] = json!(c);
        }
        w.ok(sup, "timesheet.add", params);
    }
    let plain: i64 = labour(w.engine.state(), p).iter().map(|l| l.cost.cents()).sum();
    assert_eq!(plain, 40 * 100 * 46); // no rules: 46 hours at 40

    w.ok(pm, "timesheet.set_rules", json!({"project": p, "daily_hours": 8, "weekly_hours": 40, "multiplier": "150"}));
    let lines = labour(w.engine.state(), p);
    let (reg, ot): (u32, u32) = lines.iter().fold((0, 0), |s, l| (s.0 + l.regular, s.1 + l.overtime));
    assert_eq!((reg, ot), (400, 60)); // Monday's 2 and Saturday's 4 are overtime
    assert_eq!(lines.iter().map(|l| l.cost.cents()).sum::<i64>(), 40 * 100 * 40 + 60 * 100 * 6);
    assert_eq!((lines[0].regular, lines[0].overtime, lines[5].regular, lines[5].overtime), (80, 20, 0, 40));

    // The report counts only approved hours unless asked for all.
    let r = w.ok(pm, "timesheet.cost_report", json!({"project": p}));
    assert_eq!(r["total"]["overtime_tenths"], 0);
    w.ok(sup, "timesheet.submit", json!({"project": p, "from": "2026-10-12"}));
    w.err(pm, "timesheet.post_costs", json!({"project": p, "from": "2026-10-12"}));
    w.ok(pm, "timesheet.approve", json!({"project": p, "person": sup.raw(), "from": "2026-10-12"}));
    let rep = report(w.engine.state(), p, None, None, true);
    assert_eq!((rep.total.regular, rep.total.overtime, rep.by_code.len(), rep.by_person.len(), rep.no_rate.len()), (400, 60, 2, 1, 0));
    let r = w.ok(pm, "timesheet.cost_report", json!({"project": p, "from": "2026-10-12", "to": "2026-10-13", "include": "all"}));
    assert_eq!(r["total"]["regular_tenths"], 160);
    w.err(pm, "timesheet.cost_report", json!({"project": p, "include": "maybe"}));
    w.err(sup, "timesheet.cost_report", json!({"project": p}));

    // Posting: hours without a code need a default; the result is labour direct costs, once.
    w.err(pm, "timesheet.post_costs", json!({"project": p, "from": "2026-10-12"}));
    w.err(sup, "timesheet.post_costs", json!({"project": p, "from": "2026-10-12", "default_cost_code": b}));
    let posted = w.ok(pm, "timesheet.post_costs", json!({"project": p, "from": "2026-10-12", "default_cost_code": b}));
    assert_eq!(posted["posted"], 6);
    let costs: Vec<_> = w.engine.state().direct_costs.iter().collect();
    assert_eq!(costs.len(), 2);
    assert_eq!(costs.iter().map(|c| c.amount.cents()).sum::<i64>(), 40 * 100 * 40 + 60 * 100 * 6);
    assert!(costs.iter().all(|c| c.kind == buildkiosk_model::project::CostType::Labor));
    w.err(pm, "timesheet.post_costs", json!({"project": p, "from": "2026-10-12", "default_cost_code": b}));
    w.err(pm, "timesheet.update", json!({"project": p, "id": w.engine.state().time_entries.iter().next().unwrap().id.raw(), "hours": "1"}));

    // A crew: one call, one entry each; the next week's hours need a rate before they post.
    w.err(sup, "timesheet.add", json!({"project": p, "date": "2026-10-19", "hours": "4", "people": [sup.raw(), eng.raw()]}));
    let crew = w.ok(pm, "timesheet.add", json!({"project": p, "date": "2026-10-19", "hours": "4", "people": [sup.raw(), eng.raw()], "cost_code": a}));
    assert_eq!(crew["ids"].as_array().unwrap().len(), 2);
    w.err(pm, "timesheet.add", json!({"project": p, "date": "2026-10-19", "hours": "21", "people": [sup.raw(), eng.raw()]}));
    for who in [sup, eng] {
        w.ok(who, "timesheet.submit", json!({"project": p, "from": "2026-10-19"}));
    }
    // The worker's sign-off is recorded when they submit.
    assert!(w.engine.state().time_entries.iter().filter(|e| e.date.to_string() == "2026-10-19").all(|e| e.submitted.is_some()));
    w.ok(pm, "timesheet.approve", json!({"project": p, "from": "2026-10-19"}));
    let err = w.err(pm, "timesheet.post_costs", json!({"project": p, "from": "2026-10-19"}));
    assert!(err.to_string().contains("Eli"), "{err}");
    assert_eq!(report(w.engine.state(), p, buildkiosk_core::Date::parse("2026-10-19"), None, true).no_rate.len(), 1);
    w.ok(pm, "timesheet.set_rate", json!({"project": p, "person": eng.raw(), "hourly": "30.50", "from": "2026-10-01"}));
    w.ok(pm, "timesheet.post_costs", json!({"project": p, "from": "2026-10-19"}));
    assert_eq!(w.engine.state().direct_costs.iter().count(), 3);
    // Removing a rate leaves earlier postings alone.
    let rid = w.engine.state().pay_rates.iter().find(|r| r.deleted.is_none() && r.person == eng).unwrap().id.raw();
    w.ok(pm, "timesheet.delete_rate", json!({"project": p, "id": rid}));
    w.err(pm, "timesheet.delete_rate", json!({"project": p, "id": rid}));
    w.assert_replay();
}

#[test]
fn saved_crews_log_hours_together() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.err(sup, "crew.save", json!({"project": p, "name": "Concrete", "members": [sup.raw()]}));
    w.err(pm, "crew.save", json!({"project": p, "name": "Empty", "members": []}));
    w.err(pm, "crew.save", json!({"project": p, "name": "Ghosts", "members": [9999]}));
    let c =
        w.ok(pm, "crew.save", json!({"project": p, "name": "Concrete crew", "members": [sup.raw(), eng.raw(), sup.raw()]}))["id"].as_u64().unwrap();
    w.err(pm, "crew.save", json!({"project": p, "name": "concrete CREW", "members": [eng.raw()]}));
    assert_eq!(w.engine.state().crews.iter().next().unwrap().members.len(), 2);
    assert_eq!(w.ok(pm, "crew.list", json!({"project": p})).as_array().unwrap().len(), 1);
    w.err(sup, "crew.list", json!({"project": p}));

    // One call logs the crew; a crew can be joined by extra people; only admins do it.
    w.err(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": 8, "crew": c}));
    let r = w.ok(pm, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": 8, "crew": c, "people": [pm.raw(), sup.raw()]}));
    assert_eq!(r["ids"].as_array().unwrap().len(), 3, "Sue is in the crew and listed again: one entry each");
    w.err(pm, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": 17, "crew": c}));
    w.err(pm, "timesheet.add", json!({"project": p, "date": "2026-10-13", "hours": 8, "crew": 9999}));
    w.ok(pm, "crew.save", json!({"project": p, "id": c, "name": "Concrete crew", "members": [eng.raw()]}));
    w.ok(pm, "crew.delete", json!({"project": p, "id": c}));
    w.err(pm, "crew.delete", json!({"project": p, "id": c}));
    w.err(pm, "timesheet.add", json!({"project": p, "date": "2026-10-13", "hours": 8, "crew": c}));
    w.assert_replay();
}
