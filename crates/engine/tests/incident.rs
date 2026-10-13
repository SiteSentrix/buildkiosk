//! Incidents: the shared workflow plus severity, recordable and the investigation fields.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::incident::Severity;
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn report_investigate_and_hide_from_trades() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");

    let id = w.ok(
        sup,
        "incident.create",
        json!({"project": p, "title": "Worker slipped on ramp", "kind": "Fall", "severity": "first_aid", "recordable": "true",
               "occurred": "2026-10-12", "people_involved": "J. Smith (laborer); witness R. Jones", "assignees": [pm.raw()]}),
    )["id"]
        .as_u64()
        .unwrap();
    let i = w.engine.state().incidents.iter().next().unwrap().clone();
    assert_eq!((i.severity, i.recordable, i.kind.as_str()), (Severity::FirstAid, true, "Fall"));
    w.err(sup, "incident.create", json!({"project": p, "title": "x", "severity": "bad"}));

    // Subcontractors have no access to incidents at all.
    w.err(sub, "incident.list", json!({"project": p}));
    w.err(sub, "incident.get", json!({"project": p, "id": id}));
    w.err(sub, "incident.create", json!({"project": p, "title": "x"}));

    w.ok(pm, "incident.update", json!({"project": p, "id": id, "root_cause": "Wet ramp, no grip strips", "corrective_action": "Strips installed"}));
    w.ok(pm, "incident.complete", json!({"project": p, "id": id, "comment": "Investigation done."}));
    w.ok(sup, "incident.close", json!({"project": p, "id": id}));
    assert_eq!(w.engine.state().incidents.iter().next().unwrap().root_cause, "Wet ramp, no grip strips");
    assert_eq!(w.ok(pm, "incident.list", json!({"project": p})).as_array().unwrap().len(), 1);
    w.assert_replay();
}
