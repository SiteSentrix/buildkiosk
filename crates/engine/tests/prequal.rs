//! Prequalification: records, review, expiry standing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Date;
use buildkiosk_model::prequal::Standing;
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn save_review_and_lapse() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let admin = w.admin();
    let v = w.ok(admin, "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();

    w.ok(admin, "prequal.save", json!({"vendor": v, "insurance_expires": "2027-03-01", "license_expires": "2027-06-30", "emr": "0.85", "bonding_capacity": "2,500,000", "notes": "Good references"}));
    w.ok(admin, "prequal.save", json!({"vendor": v, "notes": "Good references; visited the shop"}));
    assert_eq!(w.engine.state().prequalifications.iter().count(), 1, "one live record per company");
    let r = w.engine.state().prequalifications.iter().next().unwrap().clone();
    assert_eq!((r.emr_hundredths, r.notes.as_str()), (Some(85), "Good references; visited the shop"));
    for bad in ["abc", "10.5", "0.855", "-1"] {
        w.err(admin, "prequal.save", json!({"vendor": v, "emr": bad}));
    }
    w.err(admin, "prequal.save", json!({"vendor": 9999}));
    w.ok(admin, "prequal.save", json!({"vendor": v, "emr": ""}));

    w.err(admin, "prequal.review", json!({"vendor": v, "decision": "maybe"}));
    w.ok(admin, "prequal.submit", json!({"vendor": v}));
    w.err(admin, "prequal.submit", json!({"vendor": v}));
    w.err(admin, "prequal.review", json!({"vendor": v, "decision": "approved", "valid_until": "2020-01-01"}));
    w.ok(admin, "prequal.review", json!({"vendor": v, "decision": "approved", "score": 88, "valid_until": "2027-10-01"}));
    let r = w.engine.state().prequalifications.iter().next().unwrap().clone();
    assert_eq!(r.score, Some(88));
    let d = |s: &str| Date::parse(s).unwrap();
    assert_eq!(r.standing(d("2026-10-13")), Standing::Current);
    assert_eq!(r.standing(d("2027-01-15")), Standing::ExpiringSoon, "insurance lapses within 60 days");
    assert_eq!(r.standing(d("2027-03-02")), Standing::Lapsed);
    w.err(admin, "prequal.review", json!({"vendor": v, "decision": "approved", "score": 101}));

    assert_eq!(w.ok(admin, "prequal.list", json!({"status": "approved", "q": "keystone"})).as_array().unwrap().len(), 1);
    assert_eq!(w.ok(admin, "prequal.get", json!({"vendor": v}))["company"], "Keystone Concrete");

    // People who are not company staff with prequalification access can't see it.
    w.err(pm, "prequal.list", json!({}));
    w.err(pm, "prequal.save", json!({"vendor": v}));
    w.ok(admin, "prequal.reset", json!({"vendor": v}));
    w.ok(admin, "prequal.delete", json!({"vendor": v}));
    w.err(admin, "prequal.get", json!({"vendor": v}));
    w.assert_replay();
}
