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

const SHA: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn vendor(w: &mut World, name: &str) -> u64 {
    let admin = w.admin();
    w.ok(admin, "vendor.create", json!({"name": name, "kind": "subcontractor"}))["id"].as_u64().unwrap()
}

#[test]
fn questionnaires_are_answered_and_scored() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let admin = w.admin();
    let v = vendor(&mut w, "Keystone Concrete");
    let form_text = "# Safety\nDo you have a written safety program? | 20\nWhat is your EMR? | 10\n# Financial\nLatest audited statements on file | 30\nBonding capacity\n";
    w.err(pm, "prequal.form_save", json!({"name": "Subcontractors", "text": form_text}));
    w.err(admin, "prequal.form_save", json!({"name": "x", "text": "no section first | 5"}));
    w.err(admin, "prequal.form_save", json!({"name": "x", "text": "# Empty section"}));
    w.err(admin, "prequal.form_save", json!({"name": "x", "text": "# S\nQ | 0"}));
    w.err(admin, "prequal.form_save", json!({"name": "x", "text": "# S\nQ | lots"}));
    w.err(admin, "prequal.form_save", json!({"name": "x"}));
    let f = w.ok(admin, "prequal.form_save", json!({"name": "Subcontractors", "text": form_text}))["id"].as_u64().unwrap();
    w.err(admin, "prequal.form_save", json!({"name": "subcontractors", "text": form_text}));
    let forms = w.ok(admin, "prequal.forms", json!({}));
    assert_eq!(forms[0]["sections"].as_array().unwrap().len(), 2);
    assert_eq!(w.engine.state().prequal_forms.iter().next().unwrap().possible(), 70);
    // Sections can also come as data.
    let g = w.ok(
        admin,
        "prequal.form_save",
        json!({"name": "Suppliers", "sections": [{"title": "Delivery", "questions": [{"text": "Lead times", "points": 5}]}]}),
    )["id"]
        .as_u64()
        .unwrap();

    // The company has to have a record first; its answers are written by the team.
    w.err(admin, "prequal.answer", json!({"vendor": v, "form": f, "replies": []}));
    w.ok(admin, "prequal.save", json!({"vendor": v}));
    w.err(admin, "prequal.answer", json!({"vendor": v, "replies": [{"section": 0, "question": 0, "text": "x"}]}));
    w.err(admin, "prequal.answer", json!({"vendor": v, "form": f, "replies": [{"section": 5, "question": 0, "text": "x"}]}));
    w.err(admin, "prequal.answer", json!({"vendor": v, "form": 999}));
    w.ok(admin, "prequal.answer", json!({"vendor": v, "form": f, "replies": [{"section": 0, "question": 0, "text": "Yes, reviewed yearly"}, {"section": 0, "question": 1, "text": "0.85"}, {"section": 1, "question": 0, "text": "FY25 attached"}]}));
    let rec = |w: &World| w.engine.state().prequalifications.iter().next().unwrap().clone();
    assert_eq!((rec(&w).answers.len(), rec(&w).form.map(|x| x.raw())), (3, Some(f)));

    // Only an admin scores; points can't exceed the question's worth; the score is the share earned.
    w.err(pm, "prequal.grade", json!({"vendor": v, "points": []}));
    w.err(admin, "prequal.grade", json!({"vendor": v, "points": [{"section": 0, "question": 0, "points": 21}]}));
    w.err(admin, "prequal.grade", json!({"vendor": v, "points": [{"section": 0, "question": 0, "points": "lots"}]}));
    let r = w.ok(admin, "prequal.grade", json!({"vendor": v, "points": [{"section": 0, "question": 0, "points": 20}, {"section": 0, "question": 1, "points": 5}, {"section": 1, "question": 0, "points": 15}]}));
    assert_eq!((r["earned"].as_u64(), r["possible"].as_u64(), r["score"].as_u64()), (Some(40), Some(70), Some(57)));
    assert_eq!(rec(&w).score, Some(57));
    let form = w.engine.state().prequal_forms.get(buildkiosk_core::Id::new(f)).unwrap().clone();
    assert_eq!((rec(&w).points(&form, Some(0)), rec(&w).points(&form, Some(1))), ((25, 30), (15, 40)));
    // A question graded but never answered still counts; clearing points lowers the score.
    w.ok(
        admin,
        "prequal.grade",
        json!({"vendor": v, "points": [{"section": 1, "question": 1, "points": 10}, {"section": 0, "question": 1, "points": ""}]}),
    );
    assert_eq!(rec(&w).score, Some(64), "20 + 15 + 10 = 45 of 70");
    // Switching questionnaire starts over; a questionnaire in use can't be deleted.
    w.err(admin, "prequal.form_delete", json!({"id": f}));
    w.ok(admin, "prequal.answer", json!({"vendor": v, "form": g}));
    assert!(rec(&w).answers.is_empty());
    w.ok(admin, "prequal.form_delete", json!({"id": f}));
    w.err(admin, "prequal.form_delete", json!({"id": f}));
    w.assert_replay();
}

#[test]
fn certificates_attach_to_the_record() {
    let mut w = World::new();
    let admin = w.admin();
    let v = vendor(&mut w, "Keystone Concrete");
    w.err(admin, "prequal.attach", json!({"vendor": v, "sha256": SHA, "name": "coi.pdf", "size": 10}));
    w.ok(admin, "prequal.save", json!({"vendor": v}));
    w.err(admin, "prequal.attach", json!({"vendor": v, "sha256": "short", "name": "coi.pdf", "size": 10}));
    w.ok(admin, "prequal.attach", json!({"vendor": v, "sha256": SHA, "name": "coi.pdf", "size": 10, "mime": "application/pdf"}));
    w.ok(admin, "prequal.attach", json!({"vendor": v, "sha256": SHA, "name": "coi.pdf", "size": 10}));
    assert_eq!(w.engine.state().prequalifications.iter().next().unwrap().attachments.len(), 1, "the same file is kept once");
    w.err(admin, "prequal.detach", json!({"vendor": v, "sha256": "d".repeat(64)}));
    w.ok(admin, "prequal.detach", json!({"vendor": v, "sha256": SHA}));
    assert!(w.engine.state().prequalifications.iter().next().unwrap().attachments.is_empty());
    w.assert_replay();
}

#[test]
fn the_admins_are_warned_before_things_lapse_and_only_once() {
    let mut w = World::new();
    let admin = w.admin();
    let me = w.ok(admin, "me.get", json!({}))["person"]["id"].as_u64().unwrap();
    let me = buildkiosk_core::Id::<buildkiosk_model::directory::Person>::new(me);
    let unread = |w: &mut World| w.ok(me, "notification.list", json!({"unread": true})).as_array().unwrap().len();
    let v = vendor(&mut w, "Keystone Concrete");
    // Today is 12 October 2026: approval in 50 days, insurance in 7, licence lapsed, the next far away.
    w.ok(
        admin,
        "prequal.save",
        json!({"vendor": v, "valid_until": "2026-12-01", "insurance_expires": "2026-10-19", "license_expires": "2026-10-01"}),
    );
    w.ok(admin, "prequal.review", json!({"vendor": v, "decision": "approved", "valid_until": "2026-12-01"}));
    let far = vendor(&mut w, "Far Away Steel");
    w.ok(admin, "prequal.save", json!({"vendor": far, "valid_until": "2028-01-01", "insurance_expires": "2028-01-01"}));
    w.ok(admin, "prequal.review", json!({"vendor": far, "decision": "approved", "valid_until": "2028-01-01"}));
    let pending = vendor(&mut w, "Not Approved Yet");
    w.ok(admin, "prequal.save", json!({"vendor": pending, "insurance_expires": "2026-10-13"}));
    let before = unread(&mut w);

    w.err(me, "prequal.remind", json!({}));
    let r = w.ok(buildkiosk_engine::Actor::System, "prequal.remind", json!({}));
    assert_eq!(r["reminded"], 3, "approval, insurance and licence of the one company; not the approved-later or unapproved ones");
    assert_eq!(unread(&mut w), before + 3);
    assert_eq!(w.ok(buildkiosk_engine::Actor::System, "prequal.remind", json!({}))["reminded"], 0, "nothing is repeated");
    // Time passes: the approval comes within 30 days, then the insurance lapses.
    w.advance_days(25);
    assert_eq!(w.ok(buildkiosk_engine::Actor::System, "prequal.remind", json!({}))["reminded"], 2, "approval at 30 days, insurance lapsed");
    assert_eq!(w.ok(buildkiosk_engine::Actor::System, "prequal.remind", json!({}))["reminded"], 0);
    // A renewed date starts afresh.
    w.ok(admin, "prequal.save", json!({"vendor": v, "insurance_expires": "2026-12-20"}));
    assert_eq!(w.ok(buildkiosk_engine::Actor::System, "prequal.remind", json!({}))["reminded"], 1, "the new insurance date is within 60 days");
    let text = w.ok(me, "notification.list", json!({"unread": true}))[0]["text"].as_str().unwrap().to_string();
    assert!(text.contains("Keystone Concrete") || text.contains("insurance") || text.contains("approval") || text.contains("license"), "{text}");
    w.assert_replay();
}

#[test]
fn a_company_can_require_prequalification_for_commitments_and_awards() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let admin = w.admin();
    let ok = vendor(&mut w, "Approved Concrete");
    let lapsed = vendor(&mut w, "Lapsed Steel");
    let unknown = vendor(&mut w, "Unknown Roofing");
    w.ok(admin, "prequal.save", json!({"vendor": ok, "insurance_expires": "2027-12-01"}));
    w.ok(admin, "prequal.review", json!({"vendor": ok, "decision": "approved", "valid_until": "2027-12-01"}));
    w.ok(admin, "prequal.save", json!({"vendor": lapsed, "insurance_expires": "2026-10-20"}));
    w.ok(admin, "prequal.review", json!({"vendor": lapsed, "decision": "approved", "valid_until": "2027-12-01"}));
    let code = w.engine.state().cost_codes.iter().find(|c| c.code == "03-300").unwrap().id.raw();
    let commit = |w: &mut World, v: u64| -> u64 {
        w.ok(pm, "commitment.create", json!({"project": p, "title": "Work", "vendor": v, "lines": [{"cost_code": code, "amount": "1000"}]}))["id"]
            .as_u64()
            .unwrap()
    };
    let (a, b, c) = (commit(&mut w, ok), commit(&mut w, lapsed), commit(&mut w, unknown));
    // Off by default: anyone can be committed to.
    w.ok(pm, "commitment.approve", json!({"project": p, "id": c}));
    w.err(pm, "prequal.require", json!({"required": true}));
    w.ok(admin, "prequal.require", json!({"required": true}));
    let d = commit(&mut w, unknown);
    assert!(w.err(pm, "commitment.approve", json!({"project": p, "id": d})).to_string().contains("not prequalified"));
    w.ok(pm, "commitment.approve", json!({"project": p, "id": a}));
    // A lapse is refused once it has happened.
    w.ok(pm, "commitment.approve", json!({"project": p, "id": b}));
    let e = commit(&mut w, lapsed);
    w.advance_days(10);
    assert!(w.err(pm, "commitment.approve", json!({"project": p, "id": e})).to_string().contains("lapsed"));
    // Bids can't be awarded to an unprequalified company either.
    let pkg =
        w.ok(pm, "bid_package.create", json!({"project": p, "title": "Roofing", "cost_code": code, "invite": [unknown, ok]}))["id"].as_u64().unwrap();
    w.ok(pm, "bid_package.open", json!({"project": p, "id": pkg}));
    w.ok(pm, "bid_package.record_bid", json!({"project": p, "id": pkg, "vendor": unknown, "amount": "900"}));
    w.ok(pm, "bid_package.record_bid", json!({"project": p, "id": pkg, "vendor": ok, "amount": "950"}));
    w.err(pm, "bid_package.award", json!({"project": p, "id": pkg, "vendor": unknown}));
    w.ok(pm, "bid_package.award", json!({"project": p, "id": pkg, "vendor": ok}));
    w.ok(admin, "prequal.require", json!({"required": false}));
    w.assert_replay();
}
