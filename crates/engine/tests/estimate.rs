//! Estimating: priced lines, markups, issue, put into the budget.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Money;
use buildkiosk_engine::cmd::estimate::parse_quantity;
use buildkiosk_testkit::World;
use serde_json::json;

fn code(w: &World, c: &str) -> u64 {
    w.engine.state().cost_codes.iter().find(|x| x.code == c).unwrap().id.raw()
}

#[test]
fn quantities() {
    assert_eq!(parse_quantity("2.5"), Some(2500));
    assert_eq!(parse_quantity("1,250"), Some(1_250_000));
    assert_eq!(parse_quantity("0.001"), Some(1));
    assert_eq!(parse_quantity(".5"), Some(500));
    assert_eq!(parse_quantity("1.2345"), None);
    assert_eq!(parse_quantity("abc"), None);
    assert_eq!(parse_quantity(""), None);
}

#[test]
fn price_issue_and_apply() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"estimating": "standard"}}));
    let (concrete, steel, general) = (code(&w, "03-300"), code(&w, "05-100"), code(&w, "01-100"));

    let id = w.ok(
        eng,
        "estimate.create",
        json!({"project": p, "name": "Schematic estimate",
               "lines": [
                   {"cost_code": concrete, "description": "Slab on grade", "quantity": "1,250.5", "unit": "sf", "unit_cost": "8.25"},
                   {"cost_code": concrete, "description": "Footings", "quantity": "40", "unit": "cy", "unit_cost": "310"},
                   {"cost_code": steel, "description": "Structural steel", "quantity": "", "unit": "ls", "unit_cost": "84,000"},
                   {"cost_code": null, "description": "", "quantity": "", "unit_cost": ""}
               ],
               "markups": [{"name": "Overhead", "percent": "5"}, {"name": "Profit", "percent": "8.5"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    let got = w.ok(eng, "estimate.get", json!({"project": p, "id": id}));
    // 1250.5 × 8.25 = 10,316.625 → 10,316.63; 40 × 310 = 12,400; steel 84,000.
    assert_eq!(got["totals"]["subtotal"], "106716.63");
    assert_eq!(got["totals"]["markups"][0], "5335.83");
    assert_eq!(got["totals"]["markups"][1], "9070.91");
    assert_eq!(got["totals"]["total"], "121123.37");

    w.err(eng, "estimate.create", json!({"project": p, "name": "Bad", "lines": [{"cost_code": concrete, "quantity": "x", "unit_cost": "1"}]}));
    w.err(eng, "estimate.create", json!({"project": p, "name": "Bad", "lines": [{"cost_code": concrete, "unit_cost": ""}]}));
    w.err(eng, "estimate.create", json!({"project": p, "name": "Bad", "lines": [{"cost_code": 9999, "unit_cost": "1"}]}));
    w.err(
        eng,
        "estimate.create",
        json!({"project": p, "name": "Big", "lines": [{"cost_code": concrete, "quantity": "999999999", "unit_cost": "9,000,000,000,000"}]}),
    );
    w.err(eng, "estimate.apply_to_budget", json!({"project": p, "id": id}));
    w.err(eng, "estimate.finalize", json!({"project": p, "id": w.engine.state().estimates.iter().next().unwrap().id.raw() + 9}));
    w.ok(eng, "estimate.finalize", json!({"project": p, "id": id}));
    w.err(eng, "estimate.update", json!({"project": p, "id": id, "name": "x"}));
    w.err(eng, "estimate.apply_to_budget", json!({"project": p, "id": id}));

    // Putting it in the budget: per code, markups on a chosen code, refused when locked.
    w.ok(pm, "estimate.apply_to_budget", json!({"project": p, "id": id, "markup_cost_code": general}));
    let rows = w.ok(pm, "budget.get", json!({"project": p}));
    let amount = |c: u64| rows["rows"].as_array().unwrap().iter().find(|r| r["cost_code"] == c).map(|r| r["original"].clone()).unwrap();
    assert_eq!((amount(concrete), amount(steel), amount(general)), (json!("22716.63"), json!("84000.00"), json!("14406.74")));
    w.ok(pm, "budget.lock", json!({"project": p}));
    w.err(pm, "estimate.apply_to_budget", json!({"project": p, "id": id}));
    w.ok(pm, "budget.unlock", json!({"project": p}));
    w.ok(eng, "estimate.reopen", json!({"project": p, "id": id}));
    w.ok(eng, "estimate.update", json!({"project": p, "id": id, "markups": []}));
    assert_eq!(w.engine.state().estimates.iter().next().unwrap().totals().unwrap().total, Money::parse("106716.63").unwrap());
    assert_eq!(w.ok(eng, "estimate.list", json!({"project": p})).as_array().unwrap().len(), 1);
    w.ok(eng, "estimate.delete", json!({"project": p, "id": id}));
    w.ok(pm, "estimate.restore", json!({"project": p, "id": id}));
    w.assert_replay();
}

fn estimate_of(w: &World, id: u64) -> buildkiosk_model::estimate::Estimate {
    w.engine.state().estimates.get(buildkiosk_core::Id::new(id)).unwrap().clone()
}

fn setup() -> (World, buildkiosk_core::Id<buildkiosk_model::project::Project>, buildkiosk_core::Id<buildkiosk_model::directory::Person>) {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    (w, p, pm)
}

#[test]
fn alternates_are_priced_apart_until_accepted() {
    let (mut w, p, pm) = setup();
    let (concrete, steel) = (code(&w, "03-300"), code(&w, "05-100"));
    let id = w
        .ok(
            pm,
            "estimate.create",
            json!({"project": p, "name": "Bid estimate",
                   "lines": [{"cost_code": concrete, "description": "Slab", "quantity": "100", "unit": "sf", "unit_cost": "10"}],
                   "markups": [{"name": "Overhead", "percent": "10"}],
                   "alternates": [{"name": "Polished slab", "cost_code": concrete, "amount": "500"}, {"name": "Deduct canopy", "cost_code": steel, "amount": "-200", "description": "Delete steel canopy"}, {"name": "", "amount": ""}]}),
        )["id"]
        .as_u64()
        .unwrap();
    assert_eq!(estimate_of(&w, id).alternates.len(), 2, "blank rows are dropped");
    let total = |w: &World| estimate_of(w, id).totals().unwrap();
    assert_eq!(
        (total(&w).subtotal, total(&w).alternates, total(&w).total),
        (Money::parse("1000").unwrap(), Money::ZERO, Money::parse("1100").unwrap())
    );
    w.err(pm, "estimate.create", json!({"project": p, "name": "x", "alternates": [{"name": "No code", "amount": "5"}]}));
    w.err(pm, "estimate.create", json!({"project": p, "name": "x", "alternates": [{"name": "No amount", "cost_code": concrete}]}));
    // Accepting adds to the subtotal, before the markups; a deduct takes away.
    w.err(pm, "estimate.accept_alternate", json!({"project": p, "id": id, "index": 5, "accepted": true}));
    w.ok(pm, "estimate.accept_alternate", json!({"project": p, "id": id, "index": 0, "accepted": true}));
    assert_eq!(
        (total(&w).subtotal, total(&w).alternates, total(&w).total),
        (Money::parse("1500").unwrap(), Money::parse("500").unwrap(), Money::parse("1650").unwrap())
    );
    w.ok(pm, "estimate.accept_alternate", json!({"project": p, "id": id, "index": 1, "accepted": true}));
    assert_eq!(total(&w).total, Money::parse("1430").unwrap(), "(1000 + 500 - 200) plus 10%");
    // They count under their own cost codes in the budget.
    w.ok(pm, "estimate.finalize", json!({"project": p, "id": id}));
    w.ok(pm, "estimate.accept_alternate", json!({"project": p, "id": id, "index": 1, "accepted": false}));
    w.ok(pm, "estimate.apply_to_budget", json!({"project": p, "id": id}));
    let line = |w: &World, c: u64| w.engine.state().budget_lines.iter().find(|b| b.cost_code.raw() == c && b.deleted.is_none()).map(|b| b.original);
    assert_eq!((line(&w, concrete), line(&w, steel)), (Some(Money::parse("1500").unwrap()), None));
    w.err(pm, "estimate.accept_alternate", json!({"project": p, "id": id, "index": 1, "accepted": true}));
    w.assert_replay();
}

#[test]
fn a_copy_is_a_new_version_and_the_two_can_be_compared() {
    let (mut w, p, pm) = setup();
    let (concrete, steel) = (code(&w, "03-300"), code(&w, "05-100"));
    let a = w
        .ok(pm, "estimate.create", json!({"project": p, "name": "Schematic", "lines": [
            {"cost_code": concrete, "description": "Slab", "quantity": "100", "unit_cost": "10"}, {"cost_code": steel, "description": "Frame", "quantity": "1", "unit_cost": "5000"}],
            "alternates": [{"name": "Upgrade", "cost_code": concrete, "amount": "300", "accepted": true}]}))["id"]
        .as_u64()
        .unwrap();
    w.ok(pm, "estimate.finalize", json!({"project": p, "id": a}));
    let b = w.ok(pm, "estimate.duplicate", json!({"project": p, "id": a}))["id"].as_u64().unwrap();
    let copy = estimate_of(&w, b);
    assert_eq!((copy.name.as_str(), copy.number, copy.based_on.map(|x| x.raw()), copy.lines.len()), ("Schematic (copy)", 2, Some(a), 2));
    assert_eq!(copy.status, buildkiosk_model::estimate::EstimateStatus::Draft);
    assert!(!copy.alternates[0].accepted, "choices are made afresh");
    w.ok(pm, "estimate.update", json!({"project": p, "id": b, "name": "Design development", "lines": [
        {"cost_code": concrete, "description": "Slab", "quantity": "100", "unit_cost": "12"}, {"cost_code": steel, "description": "Frame", "quantity": "1", "unit_cost": "5000"}]}));
    let c = w.ok(pm, "estimate.compare", json!({"project": p, "id": b, "with": a}));
    assert_eq!((c["this"]["name"].as_str(), c["other"]["name"].as_str()), (Some("Design development"), Some("Schematic")));
    // 2,200 against 1,000 + 5,000 + the accepted 300.
    assert_eq!(
        (c["this"]["total"].as_str(), c["other"]["total"].as_str(), c["difference"].as_str()),
        (Some("6200.00"), Some("6300.00"), Some("-100.00"))
    );
    let row = c["rows"].as_array().unwrap().iter().find(|r| r["code"] == "03-300").unwrap();
    assert_eq!((row["this"].as_str(), row["other"].as_str(), row["difference"].as_str()), (Some("1200.00"), Some("1300.00"), Some("-100.00")));
    w.err(pm, "estimate.compare", json!({"project": p, "id": b, "with": 999}));
    w.assert_replay();
}

#[test]
fn the_catalogue_and_assemblies_price_lines() {
    let (mut w, p, pm) = setup();
    let reader = w.member(p, "Rae", "Reader", "Read only");
    w.ok(pm, "member.update", json!({"project": p, "person": reader, "overrides": {"estimating": "read"}}));
    let outsider = w.user("Olive", "Outside");
    let (concrete, steel) = (code(&w, "03-300"), code(&w, "05-100"));
    // Anyone who can't even read an estimate has no business with unit costs.
    w.err(outsider, "catalog.list", json!({}));
    w.err(reader, "catalog.save", json!({"name": "Ready-mix", "unit_cost": "180"}));
    w.err(pm, "catalog.save", json!({"name": "", "unit_cost": "1"}));
    w.err(pm, "catalog.save", json!({"name": "Bad", "unit_cost": "-1"}));
    w.err(pm, "catalog.save", json!({"name": "Bad", "unit_cost": "1", "cost_code": 9999}));
    let mix = w.ok(
        pm,
        "catalog.save",
        json!({"name": "Ready-mix concrete", "unit": "cy", "unit_cost": "180", "cost_code": concrete, "category": "Concrete"}),
    )["id"]
        .as_u64()
        .unwrap();
    let mesh = w.ok(pm, "catalog.save", json!({"name": "Welded wire mesh", "unit": "sf", "unit_cost": "0.45"}))["id"].as_u64().unwrap();
    w.err(pm, "catalog.save", json!({"name": "ready-mix CONCRETE", "unit_cost": "1"}));
    w.ok(
        pm,
        "catalog.save",
        json!({"id": mix, "name": "Ready-mix concrete", "unit": "cy", "unit_cost": "190", "cost_code": concrete, "category": "Concrete"}),
    );
    assert_eq!(w.ok(reader, "catalog.list", json!({"q": "mesh"})).as_array().unwrap().len(), 1);
    assert_eq!(w.ok(reader, "catalog.list", json!({"category": "concrete"})).as_array().unwrap().len(), 1);

    // A spreadsheet of prices: new names are added, known names changed; a bad line refuses all.
    let r = w.ok(
        pm,
        "catalog.import",
        json!({"text": "Name,Unit,Unit cost,Category,Cost code\nRebar #5,lf,1.25,Steel,05-100\nWelded wire mesh,sf,0.50,,\n"}),
    );
    assert_eq!((r["added"].as_u64(), r["changed"].as_u64()), (Some(1), Some(1)));
    w.err(pm, "catalog.import", json!({"text": "Good,ea,1\nBroken,ea,abc"}));
    w.err(pm, "catalog.import", json!({"text": "Bad code,ea,1,x,99-999"}));
    assert_eq!(w.ok(reader, "catalog.list", json!({})).as_array().unwrap().len(), 3);
    let rebar = w.engine.state().catalog_items.iter().find(|i| i.name == "Rebar #5").unwrap().id.raw();

    // An assembly: one sf of slab is 0.012 cy of concrete (a 4-inch slab, rounded up), a sf of mesh and some rebar.
    w.err(pm, "assembly.save", json!({"name": "Slab", "parts": []}));
    w.err(pm, "assembly.save", json!({"name": "Slab", "parts": [{"item": 999, "quantity": "1"}]}));
    w.err(pm, "assembly.save", json!({"name": "Slab", "parts": [{"item": mix, "quantity": "0"}]}));
    w.err(pm, "assembly.save", json!({"name": "Slab", "parts": [{"item": mix}, {"item": mix}]}));
    let slab = w
        .ok(pm, "assembly.save", json!({"name": "Slab on grade", "unit": "sf", "parts": [{"item": mix, "quantity": "0.012"}, {"item": mesh, "quantity": "1"}, {"item": rebar, "quantity": "0.5"}]}))["id"]
        .as_u64()
        .unwrap();
    let listed = w.ok(reader, "assembly.list", json!({}));
    assert_eq!(listed[0]["unit_cost"], "3.41", "2.28 + 0.50 + 0.63 (each part rounded to the cent)");
    w.err(pm, "catalog.delete", json!({"id": mix}));

    // Lines made from them are copies: a later price change leaves the estimate alone.
    let e = w.ok(pm, "estimate.create", json!({"project": p, "name": "Clinic"}))["id"].as_u64().unwrap();
    w.err(outsider, "estimate.add_from_catalog", json!({"project": p, "id": e, "item": mix, "quantity": "1"}));
    w.err(pm, "estimate.add_from_catalog", json!({"project": p, "id": e, "item": mix}));
    w.err(pm, "estimate.add_from_catalog", json!({"project": p, "id": e, "item": mesh, "quantity": "100"}));
    w.ok(pm, "estimate.add_from_catalog", json!({"project": p, "id": e, "item": mesh, "quantity": "100", "cost_code": concrete}));
    w.ok(pm, "estimate.add_from_catalog", json!({"project": p, "id": e, "item": mix, "quantity": "12.5", "description": "Footing concrete"}));
    let r = w.ok(pm, "estimate.add_assembly", json!({"project": p, "id": e, "assembly": slab, "quantity": "2,000", "cost_code": steel}));
    assert_eq!(r["lines_added"], 3);
    let lines = estimate_of(&w, e).lines;
    assert_eq!(lines.len(), 5);
    let part = |name: &str| lines.iter().find(|l| l.description == name).unwrap().clone();
    assert_eq!(
        (part("Slab on grade: Ready-mix concrete").quantity_milli, part("Slab on grade: Ready-mix concrete").cost_code.raw()),
        (24_000, concrete),
        "2,000 sf x 0.012 cy; the item's own code"
    );
    assert_eq!(
        (part("Slab on grade: Welded wire mesh").quantity_milli, part("Slab on grade: Welded wire mesh").cost_code.raw()),
        (2_000_000, steel),
        "no code of its own: the one asked for"
    );
    assert_eq!(part("Footing concrete").unit_cost, Money::parse("190").unwrap());
    w.ok(pm, "catalog.save", json!({"id": mix, "name": "Ready-mix concrete", "unit": "cy", "unit_cost": "999", "cost_code": concrete}));
    assert_eq!(estimate_of(&w, e).lines.iter().find(|l| l.description == "Footing concrete").unwrap().unit_cost, Money::parse("190").unwrap());
    // Issued estimates are left alone.
    w.ok(pm, "estimate.finalize", json!({"project": p, "id": e}));
    w.err(pm, "estimate.add_from_catalog", json!({"project": p, "id": e, "item": mix, "quantity": "1"}));
    w.err(pm, "estimate.add_assembly", json!({"project": p, "id": e, "assembly": slab, "quantity": "1", "cost_code": steel}));
    // The catalogue's items can go once nothing uses them.
    w.ok(pm, "assembly.delete", json!({"id": slab}));
    w.ok(pm, "catalog.delete", json!({"id": mix}));
    w.assert_replay();
}

#[test]
fn a_bid_becomes_a_lump_sum_line() {
    let (mut w, p, pm) = setup();
    let admin = w.admin();
    let concrete = code(&w, "03-300");
    let a = w.ok(admin, "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let b = w.ok(admin, "vendor.create", json!({"name": "Granite Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let pkg =
        w.ok(pm, "bid_package.create", json!({"project": p, "title": "Concrete", "cost_code": concrete, "invite": [a, b]}))["id"].as_u64().unwrap();
    let nocode = w.ok(pm, "bid_package.create", json!({"project": p, "title": "Misc"}))["id"].as_u64().unwrap();
    let e = w.ok(pm, "estimate.create", json!({"project": p, "name": "GMP"}))["id"].as_u64().unwrap();
    w.err(pm, "estimate.add_from_bid", json!({"project": p, "id": e, "package": pkg}));
    w.ok(pm, "bid_package.open", json!({"project": p, "id": pkg}));
    w.ok(pm, "bid_package.record_bid", json!({"project": p, "id": pkg, "vendor": a, "amount": "120000"}));
    w.ok(pm, "bid_package.record_bid", json!({"project": p, "id": pkg, "vendor": b, "amount": "110000"}));
    w.err(pm, "estimate.add_from_bid", json!({"project": p, "id": e, "package": nocode}));
    w.ok(pm, "estimate.add_from_bid", json!({"project": p, "id": e, "package": pkg}));
    w.ok(pm, "estimate.add_from_bid", json!({"project": p, "id": e, "package": pkg, "vendor": a}));
    w.err(pm, "estimate.add_from_bid", json!({"project": p, "id": e, "package": pkg, "vendor": 999}));
    let lines = estimate_of(&w, e).lines;
    assert_eq!(
        (lines.len(), lines[0].description.as_str(), lines[0].unit_cost, lines[0].unit.as_str()),
        (2, "Concrete: bid from Granite Concrete", Money::parse("110000").unwrap(), "ls"),
        "the lowest bid by default"
    );
    assert_eq!(lines[1].unit_cost, Money::parse("120000").unwrap());
    w.ok(pm, "bid_package.withdraw_bid", json!({"project": p, "id": pkg, "vendor": b}));
    assert_eq!(estimate_of(&w, e).lines.len(), 2, "the estimate keeps what it was given");
    w.assert_replay();
}

#[test]
fn takeoff_totals_become_lines() {
    let (mut w, p, pm) = setup();
    let concrete = code(&w, "03-300");
    let sha = "a".repeat(64);
    let d = w.ok(
        pm,
        "drawing.add",
        json!({"project": p, "number": "A-101", "title": "Floor plan", "revision": "0", "sha256": sha, "name": "a.pdf", "size": 10}),
    )["id"]
        .as_u64()
        .unwrap();
    let e = w.ok(pm, "estimate.create", json!({"project": p, "name": "Takeoff"}))["id"].as_u64().unwrap();
    // A square page whose middle stretch is 10 feet: 5,000 units across is 10 ft, so 2,500 is 5 ft.
    w.ok(
        pm,
        "drawing.scale_set",
        json!({"project": p, "id": d, "x": 2500, "y": 5000, "x2": 7500, "y2": 5000, "aspect": 1000, "length": "10", "unit": "ft"}),
    );
    let mark = |w: &mut World, args: serde_json::Value| {
        let mut base = json!({"project": p, "id": d});
        for (k, v) in args.as_object().unwrap() {
            base[k] = v.clone();
        }
        w.ok(pm, "drawing.markup_add", base);
    };
    mark(&mut w, json!({"kind": "dim", "x": 0, "y": 0, "x2": 2500, "y2": 0, "text": "Footing"}));
    mark(&mut w, json!({"kind": "dim", "x": 0, "y": 1000, "x2": 5000, "y2": 1000, "text": "Footing"}));
    mark(&mut w, json!({"kind": "dim", "x": 0, "y": 2000, "x2": 5000, "y2": 2000, "text": "Wall"}));
    mark(&mut w, json!({"kind": "area", "x": 0, "y": 0, "x2": 5000, "y2": 2500, "text": "Slab"}));
    for n in 0..3 {
        mark(&mut w, json!({"kind": "pin", "x": 1000 * (n + 1), "y": 3000, "text": "Door"}));
    }
    mark(&mut w, json!({"kind": "text", "x": 100, "y": 100, "text": "Not a measurement"}));
    let sum = w.ok(pm, "takeoff.summary", json!({"project": p, "drawing": d}));
    let find = |kind: &str, label: &str| sum.as_array().unwrap().iter().find(|g| g["kind"] == kind && g["label"] == label).unwrap().clone();
    assert_eq!(sum.as_array().unwrap().len(), 4, "footings, walls, slab, doors: text notes aren't measurements");
    assert_eq!(
        (find("dim", "Footing")["quantity"].as_str(), find("dim", "Footing")["unit"].as_str(), find("dim", "Footing")["count"].as_u64()),
        (Some("15.000"), Some("lf"), Some(2))
    );
    assert_eq!((find("area", "Slab")["quantity"].as_str(), find("area", "Slab")["unit"].as_str()), (Some("50.000"), Some("sf")));
    assert_eq!((find("pin", "Door")["quantity"].as_str(), find("pin", "Door")["unit"].as_str()), (Some("3.000"), Some("ea")));

    // A group becomes a line: quantity from the sheet, the price from the estimator.
    w.err(
        pm,
        "estimate.add_from_takeoff",
        json!({"project": p, "id": e, "drawing": d, "kind": "dim", "label": "Nothing", "cost_code": concrete, "unit_cost": "30"}),
    );
    w.err(
        pm,
        "estimate.add_from_takeoff",
        json!({"project": p, "id": e, "drawing": d, "kind": "text", "label": "Not a measurement", "cost_code": concrete, "unit_cost": "30"}),
    );
    w.err(pm, "estimate.add_from_takeoff", json!({"project": p, "id": e, "drawing": d, "kind": "dim", "label": "Footing", "cost_code": concrete}));
    w.ok(
        pm,
        "estimate.add_from_takeoff",
        json!({"project": p, "id": e, "drawing": d, "kind": "dim", "label": "Footing", "cost_code": concrete, "unit_cost": "30"}),
    );
    w.ok(pm, "estimate.add_from_takeoff", json!({"project": p, "id": e, "drawing": d, "kind": "pin", "label": "Door", "cost_code": concrete, "unit_cost": "450", "description": "Interior doors"}));
    let lines = estimate_of(&w, e).lines;
    assert_eq!(
        (lines[0].quantity_milli, lines[0].unit.as_str(), lines[0].description.as_str()),
        (15_000, "lf", "Footing (takeoff, A-101 Floor plan)")
    );
    assert_eq!(
        (lines[1].quantity_milli, lines[1].description.as_str(), lines[1].total()),
        (3_000, "Interior doors", Some(Money::parse("1350").unwrap()))
    );
    // Another unit on the same label can't be added together; the sheet has to be set in one unit.
    w.ok(
        pm,
        "drawing.scale_set",
        json!({"project": p, "id": d, "x": 2500, "y": 5000, "x2": 7500, "y2": 5000, "aspect": 1000, "length": "20", "unit": "m"}),
    );
    let sum = w.ok(pm, "takeoff.summary", json!({"project": p, "drawing": d}));
    assert!(sum.as_array().unwrap().iter().any(|g| g["kind"] == "dim" && g["unit"] == "m"), "{sum}");
    w.assert_replay();
}

#[test]
fn catalogue_prices_are_kept_by_date_and_supplier() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let admin = w.admin();
    let acme = w.ok(admin, "vendor.create", json!({"name": "Acme Supply", "kind": "supplier"}))["id"].as_u64().unwrap();
    let other = w.ok(admin, "vendor.create", json!({"name": "Other Supply", "kind": "supplier"}))["id"].as_u64().unwrap();
    let id = w.ok(pm, "catalog.save", json!({"name": "Ready-mix 4000 psi", "unit": "cy", "unit_cost": "180"}))["id"].as_u64().unwrap();
    let item = |w: &World| w.engine.state().catalog_items.iter().find(|i| i.id.raw() == id).unwrap().clone();
    assert_eq!((item(&w).prices.len(), item(&w).unit_cost.plain()), (1, "180.00".to_string()), "a first price is the first point");
    // Typing a new cost is recording today's price; typing the same cost again adds nothing.
    w.ok(pm, "catalog.save", json!({"id": id, "name": "Ready-mix 4000 psi", "unit": "cy", "unit_cost": "180"}));
    assert_eq!(item(&w).prices.len(), 1);
    w.ok(pm, "catalog.save", json!({"id": id, "name": "Ready-mix 4000 psi", "unit": "cy", "unit_cost": "185"}));
    assert_eq!((item(&w).prices.len(), item(&w).unit_cost.plain()), (1, "185.00".to_string()), "same day, same (no) supplier: replaced");

    // Quotes from suppliers, on days: the newest by date is the current price, whatever order they arrive in.
    w.err(reader, "catalog.price", json!({"id": id, "unit_cost": "170"}));
    w.err(pm, "catalog.price", json!({"id": id, "unit_cost": "-1"}));
    w.err(pm, "catalog.price", json!({"id": id, "unit_cost": "170", "date": "2099-01-01"}));
    w.err(pm, "catalog.price", json!({"id": id, "unit_cost": "170", "vendor": 9999}));
    w.err(pm, "catalog.price", json!({"id": 9999, "unit_cost": "170"}));
    w.ok(pm, "catalog.price", json!({"id": id, "unit_cost": "172.50", "date": "2026-09-01", "vendor": acme, "note": "September list"}));
    w.ok(pm, "catalog.price", json!({"id": id, "unit_cost": "168", "date": "2026-10-05", "vendor": other}));
    assert_eq!(item(&w).unit_cost.plain(), "185.00", "today's price is newer than the quotes");
    assert_eq!(item(&w).prices.iter().map(|x| x.date.to_string()).collect::<Vec<_>>(), ["2026-09-01", "2026-10-05", "2026-10-12"]);
    // A supplier's price list updates every item it names, with its date.
    w.ok(pm, "catalog.import", json!({"text": "Ready-mix 4000 psi, cy, 190\nRebar #5, ton, 900", "date": "2026-10-12", "vendor": acme}));
    assert_eq!((item(&w).unit_cost.plain(), item(&w).prices.len()), ("190.00".to_string(), 4));
    assert!(item(&w).prices.last().unwrap().vendor.is_some_and(|v| v.raw() == acme));
    let rebar = w.engine.state().catalog_items.iter().find(|i| i.name == "Rebar #5").unwrap().clone();
    assert_eq!((rebar.prices.len(), rebar.unit_cost.plain()), (1, "900.00".to_string()));
    // An older list does not push the current price back.
    w.ok(pm, "catalog.import", json!({"text": "Ready-mix 4000 psi, cy, 150", "date": "2026-06-01", "vendor": other}));
    assert_eq!(item(&w).unit_cost.plain(), "190.00");
    w.err(pm, "catalog.import", json!({"text": "X, ea, 1", "vendor": 9999}));
    w.assert_replay();
}
