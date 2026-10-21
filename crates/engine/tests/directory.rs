//! The directory, permission templates, projects and membership.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::company::template_named;
use buildkiosk_engine::{Actor, EngineError};
use buildkiosk_model::{Level, Tool, perm};
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn setup_creates_admin_templates_and_cost_codes() {
    let w = World::new();
    let s = w.engine.state();
    assert_eq!(s.company().unwrap().slug, "summit-ridge-builders");
    assert!(s.people.get(w.admin).unwrap().company_admin);
    assert_eq!(s.templates.len(), 6);
    assert!(s.cost_codes.len() > 10);
    let mut w = w;
    let e = w.err(Actor::System, "company.setup", json!({"name": "Again", "admin_first_name": "A", "admin_last_name": "B", "admin_email": "a@b.co"}));
    assert!(matches!(e, EngineError::Conflict(_)));
    // People can't run system commands.
    let admin = w.admin();
    assert!(matches!(w.err(admin, "company.setup", json!({})), EngineError::Forbidden(_)));
}

#[test]
fn emails_are_unique_and_users_need_one() {
    let mut w = World::new();
    let admin = w.admin();
    w.ok(admin, "person.create", json!({"first_name": "Ann", "last_name": "Lee", "email": "ann@example.com"}));
    let e = w.err(admin, "person.create", json!({"first_name": "Ann", "last_name": "Two", "email": "ANN@example.com"}));
    assert!(e.to_string().contains("already"), "{e}");
    let e = w.err(admin, "person.create", json!({"first_name": "No", "last_name": "Mail", "is_user": true}));
    assert!(e.to_string().contains("email"), "{e}");
    let e = w.err(admin, "person.create", json!({"first_name": "", "last_name": ""}));
    assert!(matches!(e, EngineError::BadParams { .. }), "{e}");
}

#[test]
fn the_last_company_admin_is_protected() {
    let mut w = World::new();
    let admin = w.admin;
    let e = w.err(admin, "person.update", json!({"id": admin, "company_admin": false}));
    assert!(matches!(e, EngineError::Conflict(_)), "{e}");
    let e = w.err(admin, "person.update", json!({"id": admin, "active": false}));
    assert!(matches!(e, EngineError::Conflict(_)), "{e}");
    let second = w.user("Bea", "Boss");
    let third = w.user("Cy", "Clerk");
    w.ok(admin, "person.update", json!({"id": second, "company_admin": true}));
    w.ok(admin, "person.update", json!({"id": admin, "company_admin": false}));
    // Ada is no longer a company admin: she has no Directory access left and can't make admins.
    let e = w.err(admin, "person.update", json!({"id": third, "company_admin": true}));
    assert!(matches!(e, EngineError::Forbidden(_)), "{e}");
    // Bea, the remaining admin, can.
    w.ok(Actor::Person(second), "person.update", json!({"id": third, "company_admin": true}));
    w.assert_replay();
}

#[test]
fn only_company_admins_grant_company_admin() {
    let mut w = World::new();
    let admin = w.admin();
    let dir_admin_template =
        w.ok(admin, "template.create", json!({"name": "Directory admin", "levels": {"directory": "admin"}}))["id"].as_u64().unwrap();
    let clerk = w.user("Cy", "Clerk");
    w.ok(admin, "person.update", json!({"id": clerk, "company_template": dir_admin_template}));
    assert_eq!(perm::company_level(w.engine.state(), clerk, Tool::Directory), Level::Admin);
    let other = w.user("Dee", "Doer");
    let e = w.err(clerk, "person.update", json!({"id": other, "company_admin": true}));
    assert!(matches!(e, EngineError::Forbidden(_)), "{e}");
    // But a Directory admin can give sign-in access.
    let contact = w.ok(clerk, "person.create", json!({"first_name": "Ed", "last_name": "Ext", "email": "ed@sub.example"}))["id"].as_u64().unwrap();
    w.ok(clerk, "person.update", json!({"id": contact, "is_user": true}));
}

#[test]
fn project_creator_runs_the_project_and_gets_cost_codes() {
    let mut w = World::new();
    let admin = w.admin();
    let t = w.ok(admin, "template.create", json!({"name": "Project starter", "levels": {"portfolio": "standard"}}))["id"].as_u64().unwrap();
    let pm = w.user("Paula", "Manager");
    w.ok(admin, "person.update", json!({"id": pm, "company_template": t}));
    let p = Id::new(
        w.ok(pm, "project.create", json!({"name": "Clinic", "number": "2026-014", "start": "2026-11-02", "completion": "2027-09-30"}))["id"]
            .as_u64()
            .unwrap(),
    );
    let s = w.engine.state();
    assert_eq!(perm::project_level(s, pm, p, Tool::Budget), Level::Admin);
    let company_codes = s.cost_codes.iter().filter(|c| c.project.is_none()).count();
    assert_eq!(s.cost_codes.iter().filter(|c| c.project == Some(p)).count(), company_codes);
    let e = w.err(pm, "project.create", json!({"name": "Dup", "number": "2026-014"}));
    assert!(e.to_string().contains("in use"), "{e}");
    let e = w.err(pm, "project.create", json!({"name": "Backwards", "start": "2027-01-01", "completion": "2026-01-01"}));
    assert!(e.to_string().contains("before"), "{e}");
    w.assert_replay();
}

#[test]
fn member_overrides_beat_templates() {
    let mut w = World::new();
    let p = w.project("Clinic");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let s = w.engine.state();
    assert_eq!(perm::project_level(s, sub, p, Tool::Rfis), Level::Standard);
    assert_eq!(perm::project_level(s, sub, p, Tool::Budget), Level::None);
    let admin = w.admin();
    w.ok(admin, "member.update", json!({"project": p, "person": sub, "overrides": {"budget": "read", "rfis": "admin"}}));
    let s = w.engine.state();
    assert_eq!(perm::project_level(s, sub, p, Tool::Budget), Level::Read);
    assert_eq!(perm::project_level(s, sub, p, Tool::Rfis), Level::Admin);
    w.ok(admin, "member.update", json!({"project": p, "person": sub, "overrides": {"rfis": "inherit"}}));
    assert_eq!(perm::project_level(w.engine.state(), sub, p, Tool::Rfis), Level::Standard);
    w.ok(admin, "member.remove", json!({"project": p, "person": sub}));
    assert_eq!(perm::project_level(w.engine.state(), sub, p, Tool::ProjectHome), Level::None);
    let e = w.err(admin, "member.update", json!({"project": p, "person": sub, "role": "x"}));
    assert!(matches!(e, EngineError::NotFound(_)));
    // Re-adding works after removal.
    let t = template_named(w.engine.state(), "Read only").unwrap();
    w.ok(admin, "member.add", json!({"project": p, "person": sub, "template": t}));
    w.assert_replay();
}

#[test]
fn templates_in_use_cannot_be_deleted() {
    let mut w = World::new();
    let p = w.project("Clinic");
    w.member(p, "Sam", "Sub", "Subcontractor");
    let t = template_named(w.engine.state(), "Subcontractor").unwrap();
    let admin = w.admin();
    let e = w.err(admin, "template.delete", json!({"id": t}));
    assert!(matches!(e, EngineError::Conflict(_)), "{e}");
    let e = w.err(admin, "template.update", json!({"id": t, "levels": {"rfis": "superuser"}}));
    assert!(matches!(e, EngineError::BadParams { .. }), "{e}");
}

#[test]
fn cost_code_import_skips_duplicates_and_comments() {
    let mut w = World::new();
    let p = w.project("Clinic");
    let admin = w.admin();
    let out =
        w.ok(admin, "cost_code.import", json!({"project": p, "text": "# my list\n90-100\tCustom one\n90-200,Custom two\n\n01-100\tAlready there"}));
    assert_eq!(out["added"], 2);
    let out = w.ok(admin, "company.cost_code.import", json!({"text": "A-1,Alpha", "replace": true}));
    assert_eq!(out["added"], 1);
    assert_eq!(w.engine.state().cost_codes.iter().filter(|c| c.project.is_none() && c.deleted.is_none()).count(), 1);
}

#[test]
fn disabled_people_lose_access() {
    let mut w = World::new();
    let p = w.project("Clinic");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let admin = w.admin();
    w.ok(admin, "person.update", json!({"id": sub, "active": false}));
    let e = w.err(sub, "rfi.list", json!({"project": p}));
    assert!(matches!(e, EngineError::Forbidden(_)), "{e}");
    let e = w.err(sub, "me.get", json!({}));
    assert!(matches!(e, EngineError::Forbidden(_)), "{e}");
}

#[test]
fn duplicate_companies_merge() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let admin = w.admin();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let keep = id(w.ok(admin, "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"})));
    let dup =
        id(w.ok(admin, "vendor.create", json!({"name": "Keystone Concrete Inc", "kind": "subcontractor", "phone": "555-0111", "trade": "Concrete"})));
    let person = id(w.ok(admin, "person.create", json!({"first_name": "Cy", "last_name": "Crew", "vendor": dup})));
    let commitment = id(w.ok(admin, "commitment.create", json!({"project": p, "title": "Slab", "vendor": dup})));
    w.ok(admin, "prequal.save", json!({"vendor": dup, "notes": "from the duplicate"}));
    let pkg = id(w.ok(admin, "bid_package.create", json!({"project": p, "title": "Concrete", "invite": [keep, dup]})));
    w.ok(admin, "bid_package.open", json!({"project": p, "id": pkg}));
    w.ok(admin, "bid_package.record_bid", json!({"project": p, "id": pkg, "vendor": dup, "amount": "100"}));

    w.err(pm, "vendor.merge", json!({"from": dup, "into": keep}));
    w.err(admin, "vendor.merge", json!({"from": dup, "into": dup}));
    w.err(admin, "vendor.merge", json!({"from": dup, "into": 9999}));
    let out = w.ok(admin, "vendor.merge", json!({"from": dup, "into": keep}));
    assert!(out["records_moved"].as_u64().unwrap() >= 3);
    w.err(admin, "vendor.merge", json!({"from": dup, "into": keep}));

    let s = w.engine.state();
    assert_eq!(s.people.get(buildkiosk_core::Id::new(person)).unwrap().vendor.map(|v| v.raw()), Some(keep));
    assert_eq!(s.commitments.get(buildkiosk_core::Id::new(commitment)).unwrap().vendor.map(|v| v.raw()), Some(keep));
    let k = s.vendors.get(buildkiosk_core::Id::new(keep)).unwrap();
    assert_eq!((k.phone.as_str(), k.trade.as_str()), ("555-0111", "Concrete"), "blanks are filled from the duplicate");
    assert!(s.vendors.get(buildkiosk_core::Id::new(dup)).unwrap().deleted.is_some());
    let b = s.bid_packages.iter().next().unwrap();
    assert_eq!((b.invited.len(), b.bids.len(), b.bids[0].vendor.raw()), (1, 1, keep));
    assert_eq!(s.prequalifications.iter().filter(|q| q.deleted.is_none() && q.vendor.raw() == keep).count(), 1);
    w.assert_replay();
}

#[test]
fn a_person_keeps_their_own_language() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let lang = |w: &World, who| w.engine.state().people.get(who).unwrap().language.clone();
    w.ok(pm, "person.set_language", json!({"language": "ES"}));
    assert_eq!((lang(&w, pm).as_str(), lang(&w, eng).as_str()), ("es", ""), "it changes only the caller's own setting");
    w.err(pm, "person.set_language", json!({"language": "tlh"}));
    assert_eq!(lang(&w, pm), "es");
    w.ok(pm, "person.set_language", json!({"language": ""}));
    assert_eq!(lang(&w, pm), "");
    // The system has no language of its own.
    w.err(Actor::System, "person.set_language", json!({"language": "fr"}));
    w.assert_replay();
}
