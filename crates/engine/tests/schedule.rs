//! Schedule: tasks, links, critical path, rolling up, baseline, import.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::{Date, Id};
use buildkiosk_engine::sched;
use buildkiosk_model::Choice;
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::Project;
use buildkiosk_testkit::World;
use serde_json::{Value, json};

fn d(s: &str) -> Date {
    Date::parse(s).unwrap()
}

fn row(list: &Value, number: u64) -> &Value {
    list.as_array().unwrap().iter().find(|r| r["number"] == number).unwrap()
}

#[test]
fn working_day_arithmetic() {
    assert_eq!(sched::shift(d("2026-10-12"), 2), Some(d("2026-10-14")));
    assert_eq!(sched::shift(d("2026-10-15"), 3), Some(d("2026-10-20")), "over a weekend");
    assert_eq!(sched::shift(d("2026-10-20"), -3), Some(d("2026-10-15")));
    assert_eq!(sched::days_inclusive(d("2026-10-15"), d("2026-10-20")), 4);
    assert_eq!(sched::days_inclusive(d("2026-10-12"), d("2026-10-12")), 1);
    assert_eq!(sched::days_inclusive(d("2026-10-17"), d("2026-10-18")), 0, "a weekend has no working days");
    assert_eq!(sched::days_inclusive(d("2026-10-20"), d("2026-10-15")), 0);
}

#[test]
fn links_critical_path_and_recalculate() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    w.ok(pm, "schedule.add", json!({"project": p, "name": "Excavate", "start": "2026-10-12", "days": 3}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Foundations", "start": "2026-10-15", "days": 4, "predecessors": "1"}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Framing", "start": "2026-10-21", "days": 5, "predecessors": "2"}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Permit close-out", "start": "2026-10-12", "days": 2}));
    w.err(reader, "schedule.add", json!({"project": p, "name": "x", "start": "2026-10-12"}));
    w.err(pm, "schedule.add", json!({"project": p, "name": "Bad", "start": "2026-10-12", "finish": "2026-10-01"}));
    w.err(pm, "schedule.add", json!({"project": p, "name": "Bad", "start": "2026-10-12", "predecessors": "99"}));
    w.err(pm, "schedule.update", json!({"project": p, "id": 1, "predecessors": "3"}));
    w.err(pm, "schedule.update", json!({"project": p, "id": 2, "predecessors": "2"}));

    let list = w.ok(reader, "schedule.list", json!({"project": p, "today": null}));
    assert_eq!(row(&list, 2)["finish"], "2026-10-20");
    for n in [1, 2, 3] {
        assert_eq!((row(&list, n)["critical"].as_bool(), row(&list, n)["float"].as_i64()), (Some(true), Some(0)), "task {n}");
    }
    assert_eq!((row(&list, 4)["critical"].as_bool(), row(&list, 4)["float"].as_i64()), (Some(false), Some(10)));

    // Slip the first task: its successor now starts too early; recalculate pushes the chain.
    w.ok(pm, "schedule.update", json!({"project": p, "id": 1, "days": 5}));
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    assert_eq!(row(&list, 2)["conflict"], true);
    let out = w.ok(pm, "schedule.recalculate", json!({"project": p}));
    assert_eq!(out["moved"], 2);
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    assert_eq!((row(&list, 2)["start"].as_str(), row(&list, 2)["finish"].as_str()), (Some("2026-10-19"), Some("2026-10-22")));
    assert_eq!((row(&list, 3)["start"].as_str(), row(&list, 3)["finish"].as_str()), (Some("2026-10-23"), Some("2026-10-29")));
    assert_eq!(row(&list, 2)["conflict"], false);

    // Lag and overlap.
    w.ok(pm, "schedule.update", json!({"project": p, "id": 3, "predecessors": "2+2"}));
    assert_eq!(w.ok(pm, "schedule.recalculate", json!({"project": p}))["moved"], 1);

    // Baseline and slip.
    w.err(eng, "schedule.baseline", json!({"project": p}));
    w.ok(pm, "schedule.baseline", json!({"project": p}));
    w.ok(pm, "schedule.update", json!({"project": p, "id": 4, "start": "2026-10-14"}));
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    assert!(row(&list, 4)["slip"].as_i64().unwrap() > 0);
    w.ok(pm, "schedule.progress", json!({"project": p, "id": 1, "percent": 100}));
    w.err(pm, "schedule.progress", json!({"project": p, "id": 1, "percent": 101}));

    w.err(eng, "schedule.delete", json!({"project": p, "id": 4}));
    w.ok(pm, "schedule.delete", json!({"project": p, "id": 4}));
    w.ok(pm, "schedule.restore", json!({"project": p, "id": 4}));
    let other = w.project("Other");
    w.err(pm, "schedule.get", json!({"project": other, "id": 1}));
    w.assert_replay();
}

#[test]
fn summary_tasks_roll_up_and_import() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let out = w.ok(pm, "schedule.add", json!({"project": p, "name": "Site work", "start": "2026-10-12", "days": 1}));
    let parent = out["id"].as_u64().unwrap();
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Clear", "start": "2026-10-12", "days": 2, "parent": parent, "percent": 100}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Grade", "start": "2026-10-14", "days": 2, "parent": parent, "predecessors": "2"}));
    w.err(pm, "schedule.update", json!({"project": p, "id": parent, "parent": 3}));
    w.err(pm, "schedule.delete", json!({"project": p, "id": parent}));
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        (rows[0]["summary"].as_bool(), rows[0]["start"].as_str(), rows[0]["finish"].as_str()),
        (Some(true), Some("2026-10-12"), Some("2026-10-15"))
    );
    assert_eq!(rows[0]["percent"], 50, "weighted by length");
    assert_eq!(rows[1]["depth"], 1);

    let imp = w.ok(
        pm,
        "schedule.import",
        json!({"project": p, "text": "id, name, start, finish, percent, predecessors, parent\nA, Mobilize, 2026-11-02, 2026-11-03, 0, ,\nB, Pour slab, 2026-11-04, 2026-11-06, 0, A;A, \nC, Cure, 2026-11-09, 2026-11-10, 25%, B+1, B"}),
    );
    assert_eq!(imp["added"], 3);
    w.err(pm, "schedule.import", json!({"project": p, "text": "A, One, 2026-11-02, , , B\nB, Two, 2026-11-03, , , A"}));
    w.err(pm, "schedule.import", json!({"project": p, "text": "A, One, not-a-date"}));
    w.err(pm, "schedule.import", json!({"project": p, "text": "A, One, 2026-11-02, , , Z"}));
    assert_eq!(w.ok(pm, "schedule.list", json!({"project": p})).as_array().unwrap().len(), 6);
    w.assert_replay();
}

#[test]
fn link_text_is_read_and_written() {
    use buildkiosk_model::schedule::LinkKind::{FinishToFinish as FF, FinishToStart as FS, StartToFinish as SF, StartToStart as SS};
    for (text, want) in
        [("3", (3, FS, 0)), ("5+2", (5, FS, 2)), ("7SS", (7, SS, 0)), ("4ff-1", (4, FF, -1)), ("#6 SF + 3", (6, SF, 3)), (" 8 fs ", (8, FS, 0))]
    {
        assert_eq!(sched::parse_link(text), Ok(want), "{text}");
    }
    for bad in ["", "x", "3XX", "3+", "3++1", "3 5", "3+5000", "SS", "-2"] {
        assert!(sched::parse_link(bad).is_err(), "{bad:?}");
    }
    for (n, k, lag, text) in [(5, FS, 0, "5"), (5, FS, 2, "5+2"), (7, SS, 0, "7SS"), (4, FF, -1, "4FF-1"), (6, SF, 3, "6SF+3")] {
        assert_eq!(sched::format_link(n, k, lag), text);
        assert_eq!(sched::parse_link(text), Ok((n, k, lag)));
    }
}

#[test]
fn calendars_change_what_a_working_day_is() {
    use buildkiosk_model::schedule::{Holiday, WorkCalendar};
    let six = WorkCalendar { days: [true, true, true, true, true, true, false], holidays: vec![] };
    assert_eq!(sched::shift_in(&six, d("2026-10-16"), 1), Some(d("2026-10-17")), "Saturday is worked");
    assert_eq!(sched::shift_in(&six, d("2026-10-12"), 6), Some(d("2026-10-19")), "a week of six days");
    assert_eq!(sched::days_inclusive_in(&six, d("2026-10-12"), d("2026-10-18")), 6);

    let off = |date: &str, name: &str| Holiday { date: d(date), name: name.into() };
    let hol = WorkCalendar { holidays: vec![off("2026-10-12", "Columbus Day"), off("2026-10-14", "")], ..WorkCalendar::default() };
    assert_eq!(sched::shift_in(&hol, d("2026-10-09"), 1), Some(d("2026-10-13")), "Monday is off");
    assert_eq!(sched::shift_in(&hol, d("2026-10-12"), 0), Some(d("2026-10-13")), "a start on a day off moves on");
    assert_eq!(sched::shift_in(&hol, d("2026-10-13"), -1), Some(d("2026-10-09")));
    assert_eq!(sched::shift_in(&hol, d("2026-10-13"), 2), Some(d("2026-10-16")), "Wednesday is off too");
    assert_eq!(sched::days_inclusive_in(&hol, d("2026-10-09"), d("2026-10-16")), 4, "Fri, Tue, Thu, Fri");
    // A holiday on a weekend takes nothing away.
    let wk = WorkCalendar { holidays: vec![off("2026-10-17", "")], ..WorkCalendar::default() };
    assert_eq!(sched::days_inclusive_in(&wk, d("2026-10-12"), d("2026-10-18")), 5);
    // No working days at all is no calendar.
    let none = WorkCalendar { days: [false; 7], holidays: vec![] };
    assert_eq!((sched::shift_in(&none, d("2026-10-12"), 1), none.is_valid()), (None, false));
    // The same answer whether weeks are skipped or counted day by day.
    for n in [0, 1, 4, 5, 6, 11, 40] {
        let by_hand = (0..n).fold(d("2026-10-12"), |day, _| sched::shift(day, 1).unwrap());
        assert_eq!(sched::shift(d("2026-10-12"), n), Some(by_hand), "{n}");
    }
}

#[test]
fn start_to_start_finish_to_finish_and_start_to_finish_links() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    // 1: Mon 12 to Fri 16 Oct.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Pour slab", "start": "2026-10-12", "days": 5}));
    // Starts two working days after 1 starts (Wed 14).
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Rebar", "start": "2026-10-12", "days": 3, "predecessors": "1SS+2"}));
    // Finishes when 1 finishes (Fri 16), so starts Thu 15.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Inspect", "start": "2026-10-12", "days": 2, "predecessors": "1FF"}));
    // Can't finish before 1 starts: three days ending Mon 12 start Thu 8, so no conflict at the start of the 12th.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Prep", "start": "2026-10-12", "days": 3, "predecessors": "1SF"}));
    // The usual kind with a lag: Tue 20.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Frame", "start": "2026-10-20", "days": 1, "predecessors": "1+1"}));
    w.err(pm, "schedule.add", json!({"project": p, "name": "Bad", "start": "2026-10-12", "predecessors": "1XX"}));

    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    assert_eq!(
        [2, 3, 4, 5].map(|n| row(&list, n)["conflict"].as_bool().unwrap()),
        [true, true, false, false],
        "Rebar starts before 1 has begun two days; Inspect finishes before 1 does"
    );
    let out = w.ok(pm, "schedule.recalculate", json!({"project": p}));
    assert_eq!(out["moved"], 2);
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    let span = |n| (row(&list, n)["start"].as_str().unwrap().to_string(), row(&list, n)["finish"].as_str().unwrap().to_string());
    assert_eq!(span(2), ("2026-10-14".into(), "2026-10-16".into()));
    assert_eq!(span(3), ("2026-10-15".into(), "2026-10-16".into()));
    assert_eq!(span(4), ("2026-10-12".into(), "2026-10-14".into()), "already allowed, so not moved");
    // 1 leads to Frame by a finish-to-start link, so it is critical; Rebar can slip until the project ends.
    assert_eq!((row(&list, 1)["critical"].as_bool(), row(&list, 1)["float"].as_i64()), (Some(true), Some(0)));
    assert_eq!(row(&list, 2)["float"], 2);
    assert_eq!(w.ok(pm, "schedule.get", json!({"project": p, "id": 3}))["predecessors"][0]["kind"], "ff");
    w.assert_replay();
}

#[test]
fn working_calendar_and_look_ahead() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    assert_eq!(w.ok(reader, "schedule.calendar", json!({"project": p}))["days"], json!([true, true, true, true, true, false, false]));

    w.err(reader, "schedule.calendar_set", json!({"project": p, "holidays": "2026-10-14"}));
    for bad in [
        json!({"days": "none"}),
        json!({"days": "fri-mon"}),
        json!({"days": ""}),
        json!({"holidays": "next tuesday"}),
        json!({"holidays": "2026-10-14\n2026-10-14 again"}),
    ] {
        let mut params = bad.clone();
        params["project"] = json!(p);
        w.err(pm, "schedule.calendar_set", params);
    }
    w.ok(pm, "schedule.calendar_set", json!({"project": p, "days": "mon-fri", "holidays": "2026-10-19 Plant shutdown\n2026-10-14 Founders Day"}));
    let cal = w.ok(pm, "schedule.calendar", json!({"project": p}));
    assert_eq!(
        (cal["holidays"][0]["date"].as_str(), cal["holidays"][1]["name"].as_str()),
        (Some("2026-10-14"), Some("Plant shutdown")),
        "kept in date order"
    );

    // Three working days from Monday skip the Wednesday off.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Dig", "start": "2026-10-12", "days": 3, "company": null}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Pour", "start": "2026-10-20", "days": 2, "percent": 50}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Done", "start": "2026-10-12", "days": 1, "percent": 100}));
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    assert_eq!((row(&list, 1)["finish"].as_str(), row(&list, 1)["duration"].as_i64()), (Some("2026-10-15"), Some(3)));

    // The look ahead: week one is the 12th to the 18th, week two the 19th to the 25th; finished work is left out.
    let la = w.ok(reader, "schedule.lookahead", json!({"project": p, "from": "2026-10-12", "weeks": 2}));
    let names = |i: usize| la["weeks"][i]["tasks"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!((names(0), names(1)), (vec!["Dig".to_string()], vec!["Pour".to_string()]));
    assert_eq!(la["weeks"][1]["start"], "2026-10-19");
    w.err(reader, "schedule.lookahead", json!({"project": p, "weeks": 13}));

    // Changing the calendar later leaves dates alone; a task that no longer fits is flagged.
    w.ok(pm, "schedule.calendar_set", json!({"project": p, "holidays": ""}));
    assert_eq!(w.ok(pm, "schedule.calendar", json!({"project": p}))["holidays"], json!([]));
    w.assert_replay();
}

#[test]
fn imported_links_carry_kinds() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    w.ok(
        pm,
        "schedule.import",
        json!({"project": p, "text": "a, Dig, 2026-10-12, 2026-10-14, 0, ,\nb, Form, 2026-10-14, 2026-10-16, 0, aSS+1,\nc, Pour, 2026-10-15, 2026-10-16, 0, bff-1;a,"}),
    );
    let kinds = |n: u64| {
        let id = w.engine.state().schedule_tasks.iter().find(|t| t.number == n as u32).unwrap().predecessors.clone();
        id.iter().map(|l| (l.kind.key(), l.lag)).collect::<Vec<_>>()
    };
    assert_eq!(kinds(2), vec![("ss", 1)]);
    assert_eq!(kinds(3), vec![("ff", -1), ("fs", 0)]);
    w.assert_replay();
}

#[test]
fn constraints_hold_tasks_to_dates() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Demo", "start": "2026-10-12", "days": 2}));
    // Follows 1 (so could start Wed 14) but not before the 19th.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Framing", "start": "2026-10-14", "days": 2, "predecessors": "1", "constraint": "snet", "constraint_date": "2026-10-19"}));
    // Must start Wed 21 whatever else.
    w.ok(
        pm,
        "schedule.add",
        json!({"project": p, "name": "Crane day", "start": "2026-10-12", "days": 2, "constraint": "mso", "constraint_date": "2026-10-21"}),
    );
    // A deadline the dates already miss.
    w.ok(
        pm,
        "schedule.add",
        json!({"project": p, "name": "Permit", "start": "2026-10-12", "days": 3, "constraint": "fnlt", "constraint_date": "2026-10-13"}),
    );
    for bad in [
        json!({"constraint": "snet"}),
        json!({"constraint": "sometime", "constraint_date": "2026-10-19"}),
        json!({"constraint": "mso", "constraint_date": "next week"}),
    ] {
        let mut params = json!({"project": p, "name": "Bad", "start": "2026-10-12"});
        for (k, v) in bad.as_object().unwrap() {
            params[k] = v.clone();
        }
        w.err(pm, "schedule.add", params);
    }
    w.err(reader, "schedule.update", json!({"project": p, "id": 2, "constraint": "none"}));

    let list = w.ok(reader, "schedule.list", json!({"project": p}));
    let broken = |l: &Value, n| row(l, n)["broken"].as_bool().unwrap();
    assert_eq!([2, 3, 4].map(|n| broken(&list, n)), [true, true, true]);
    assert_eq!((row(&list, 2)["constraint"].as_str(), row(&list, 2)["constraint_date"].as_str()), (Some("snet"), Some("2026-10-19")));
    assert_eq!((row(&list, 4)["float"].as_i64(), row(&list, 4)["critical"].as_bool()), (Some(-1), Some(true)), "a missed deadline is negative float");
    assert!(!broken(&list, 1));

    // Recalculate moves the floor and the fixed start, and leaves the deadline alone.
    assert_eq!(w.ok(pm, "schedule.recalculate", json!({"project": p}))["moved"], 2);
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    let span = |n| (row(&list, n)["start"].as_str().unwrap().to_string(), row(&list, n)["finish"].as_str().unwrap().to_string());
    assert_eq!(span(2), ("2026-10-19".into(), "2026-10-20".into()));
    assert_eq!(span(3), ("2026-10-21".into(), "2026-10-22".into()));
    assert_eq!(span(4), ("2026-10-12".into(), "2026-10-14".into()));
    assert_eq!([2, 3, 4].map(|n| broken(&list, n)), [false, false, true]);

    // Changing one: a date alone keeps the kind; "none" clears both.
    w.ok(pm, "schedule.update", json!({"project": p, "id": 4, "constraint_date": "2026-10-14"}));
    assert!(!broken(&w.ok(pm, "schedule.list", json!({"project": p})), 4));
    w.err(pm, "schedule.update", json!({"project": p, "id": 1, "constraint_date": "2026-10-14"}));
    w.ok(pm, "schedule.update", json!({"project": p, "id": 4, "constraint": "none"}));
    let t = w.ok(pm, "schedule.get", json!({"project": p, "id": 4}));
    assert!(t["constraint_date"].is_null() && t["constraint"] == "none", "{t}");
    w.assert_replay();
}

#[test]
fn resources_and_loading() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Framing", "start": "2026-10-12", "days": 3, "resources": "Carpenter 4, Laborer 2"}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Blocking", "start": "2026-10-13", "days": 2, "resources": "Carpenter 3"}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Trim", "start": "2026-10-19", "days": 1, "resources": "carpenter 2, CARPENTER 3"}));
    w.ok(
        pm,
        "schedule.add",
        json!({"project": p, "name": "Done work", "start": "2026-10-12", "days": 3, "percent": 100, "resources": "Carpenter 50"}),
    );
    for bad in ["Carpenter 0", "Carpenter 99999", "4", "a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r, s, t, u"] {
        w.err(pm, "schedule.add", json!({"project": p, "name": "Bad", "start": "2026-10-12", "resources": bad}));
    }
    assert_eq!(
        w.ok(pm, "schedule.get", json!({"project": p, "id": 3}))["resources"],
        json!([{"name": "carpenter", "units": 5}]),
        "the same name adds up, in the first spelling"
    );

    let load = w.ok(reader, "schedule.loading", json!({"project": p, "from": "2026-10-12", "weeks": 2}));
    assert_eq!(load[0]["name"], "Carpenter");
    assert_eq!((load[0]["peaks"].clone(), load[0]["limit"].clone()), (json!([7, 5]), json!(null)), "finished work needs nothing");
    assert_eq!(load[1]["peaks"], json!([2, 0]));

    w.err(reader, "schedule.resource_limits", json!({"project": p, "text": "Carpenter 6"}));
    w.err(pm, "schedule.resource_limits", json!({"project": p, "text": "Carpenter 0"}));
    w.ok(pm, "schedule.resource_limits", json!({"project": p, "text": "Carpenter 6\nElectrician 2"}));
    let load = w.ok(reader, "schedule.loading", json!({"project": p, "from": "2026-10-12", "weeks": 2}));
    assert_eq!(load.as_array().unwrap().len(), 3, "a limit with no work yet still shows");
    assert_eq!((load[0]["limit"].clone(), load[0]["over"].clone()), (json!(6), json!([true, false])));
    assert_eq!(load[1]["name"], "Electrician");
    w.err(reader, "schedule.loading", json!({"project": p, "weeks": 27}));
    // Editing a task's resources replaces them.
    w.ok(pm, "schedule.update", json!({"project": p, "id": 2, "resources": ""}));
    assert_eq!(w.ok(reader, "schedule.loading", json!({"project": p, "from": "2026-10-12", "weeks": 1}))[0]["peaks"], json!([4]));
    w.assert_replay();
}

#[test]
fn importing_a_project_planning_xml_file() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Project xmlns="http://schemas.microsoft.com/project"><Tasks>
<Task><UID>0</UID><Name>Whole project</Name><OutlineLevel>0</OutlineLevel></Task>
<Task><UID>10</UID><Name>Site work, phase 1</Name><OutlineLevel>1</OutlineLevel><Start>2026-10-12T08:00:00</Start><Finish>2026-10-16T17:00:00</Finish></Task>
<Task><UID>11</UID><Name>Clear</Name><OutlineLevel>2</OutlineLevel><Start>2026-10-12T08:00:00</Start><Finish>2026-10-13T17:00:00</Finish><PercentComplete>40</PercentComplete></Task>
<Task><UID>12</UID><Name>Grade</Name><OutlineLevel>2</OutlineLevel><Start>2026-10-14T08:00:00</Start><Finish>2026-10-16T17:00:00</Finish>
 <PredecessorLink><PredecessorUID>11</PredecessorUID><Type>3</Type><LinkLag>4800</LinkLag></PredecessorLink>
 <ConstraintType>4</ConstraintType><ConstraintDate>2026-10-14T08:00:00</ConstraintDate></Task>
<Task><UID>13</UID><Name>Site ready</Name><OutlineLevel>1</OutlineLevel><Milestone>1</Milestone><Start>2026-10-19T08:00:00</Start><Finish>2026-10-19T08:00:00</Finish>
 <PredecessorLink><PredecessorUID>12</PredecessorUID><Type>1</Type><LinkLag>0</LinkLag></PredecessorLink></Task>
<Task><UID>14</UID><Name>Undated</Name></Task>
</Tasks></Project>"#;
    w.err(reader, "schedule.import_xml", json!({"project": p, "text": xml}));
    w.err(pm, "schedule.import_xml", json!({"project": p, "text": "<Project/>"}));
    w.err(pm, "schedule.import_xml", json!({"project": p, "text": "plain text"}));
    let out = w.ok(pm, "schedule.import_xml", json!({"project": p, "text": xml}));
    assert_eq!((out["added"].as_u64(), out["skipped"].as_u64()), (Some(4), Some(1)));

    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    assert_eq!(list.as_array().unwrap().len(), 4);
    assert_eq!(
        (row(&list, 1)["summary"].as_bool(), row(&list, 2)["depth"].as_u64(), row(&list, 3)["depth"].as_u64()),
        (Some(true), Some(1), Some(1))
    );
    assert_eq!(
        (row(&list, 2)["percent"].as_u64(), row(&list, 4)["milestone"].as_bool(), row(&list, 4)["depth"].as_u64()),
        (Some(40), Some(true), Some(0))
    );
    let grade = w.ok(pm, "schedule.get", json!({"project": p, "id": 3}));
    assert_eq!((grade["predecessors"][0]["kind"].as_str(), grade["predecessors"][0]["lag"].as_i64()), (Some("ss"), Some(1)));
    assert_eq!((grade["constraint"].as_str(), grade["constraint_date"].as_str()), (Some("snet"), Some("2026-10-14")));
    // The text import takes the same extra columns.
    w.ok(pm, "schedule.import", json!({"project": p, "text": "x, Pour, 2026-10-20, 2026-10-21, 0, , , no, mso, 2026-10-20"}));
    assert_eq!(w.ok(pm, "schedule.get", json!({"project": p, "id": 5}))["constraint"], "mso");
    w.err(pm, "schedule.import", json!({"project": p, "text": "y, Bad, 2026-10-20, 2026-10-21, 0, , , no, mso, soon"}));
    w.assert_replay();
}

#[test]
fn moving_and_resizing_tasks() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    w.ok(pm, "schedule.calendar_set", json!({"project": p, "holidays": "2026-10-19 Plant shutdown"}));
    let parent = w.ok(pm, "schedule.add", json!({"project": p, "name": "Phase", "start": "2026-10-12", "days": 1}))["id"].as_u64().unwrap();
    // Wed 14 to Fri 16: three working days.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Slab", "start": "2026-10-14", "days": 3, "parent": parent}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Inspect", "start": "2026-10-14", "milestone": true, "parent": parent}));
    let get = |w: &mut World, id: u64| {
        let t = w.ok(pm, "schedule.get", json!({"project": p, "id": id}));
        (t["start"].as_str().unwrap().to_string(), t["finish"].as_str().unwrap().to_string())
    };
    w.err(reader, "schedule.move", json!({"project": p, "id": 2, "shift": 1}));
    w.err(pm, "schedule.move", json!({"project": p, "id": parent, "shift": 1}));
    w.err(pm, "schedule.move", json!({"project": p, "id": 2}));
    w.err(pm, "schedule.move", json!({"project": p, "id": 2, "shift": 1, "finish_shift": 1}));
    w.err(pm, "schedule.move", json!({"project": p, "id": 2, "shift": 99999}));
    w.err(pm, "schedule.move", json!({"project": p, "id": 3, "finish_shift": 1}));

    // Two days later is Fri 16: three working days from there is Fri 16, Tue 20 and Wed 21 (Mon 19 is off).
    assert_eq!(w.ok(pm, "schedule.move", json!({"project": p, "id": 2, "shift": 2}))["finish"], "2026-10-21");
    assert_eq!(get(&mut w, 2), ("2026-10-16".into(), "2026-10-21".into()));
    // A move onto a weekend lands on the next working day forward, or the previous one back.
    w.ok(pm, "schedule.move", json!({"project": p, "id": 3, "shift": 3}));
    assert_eq!(get(&mut w, 3).0, "2026-10-20", "Sat 17 snaps on past the weekend and the holiday");
    w.ok(pm, "schedule.move", json!({"project": p, "id": 3, "shift": -3}));
    assert_eq!(get(&mut w, 3).0, "2026-10-16", "Sun 18 snaps back to Friday");
    // Resizing only moves the finish.
    w.ok(pm, "schedule.move", json!({"project": p, "id": 2, "finish_shift": 1}));
    assert_eq!(get(&mut w, 2), ("2026-10-16".into(), "2026-10-22".into()));
    w.err(pm, "schedule.move", json!({"project": p, "id": 2, "finish_shift": -20}));
    w.assert_replay();
}

#[test]
fn the_schedule_exports_to_project_xml_and_reads_back() {
    use buildkiosk_engine::cmd::schedule_xml::export_xml;
    let mut w = World::new();
    let p = w.project("Riverside & Sons <Clinic>");
    let other = w.project("Copy");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let template = buildkiosk_engine::cmd::company::template_named(w.engine.state(), "Project manager").unwrap();
    w.ok(w.admin(), "member.add", json!({"project": other, "person": pm, "template": template}));
    let phase = w.ok(pm, "schedule.add", json!({"project": p, "name": "Foundations & \"footings\"", "start": "2026-10-12", "days": 1}))["id"]
        .as_u64()
        .unwrap();
    let a = w.ok(pm, "schedule.add", json!({"project": p, "name": "Excavate", "start": "2026-10-12", "days": 3, "parent": phase}))["id"]
        .as_u64()
        .unwrap();
    let b = w.ok(pm, "schedule.add", json!({"project": p, "name": "Pour <slab>", "start": "2026-10-16", "days": 2, "parent": phase, "predecessors": format!("{}FS+1", 2), "constraint": "snet", "constraint_date": "2026-10-16"}));
    let _ = (a, b);
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Inspection", "start": "2026-10-22", "milestone": true}));

    let today = buildkiosk_core::Date::parse("2026-10-12").unwrap();
    let xml = export_xml(w.engine.state(), p, today);
    assert!(
        xml.starts_with("<?xml")
            && xml.contains("<Name>Riverside &amp; Sons &lt;Clinic&gt;</Name>")
            && xml.contains("Foundations &amp; &quot;footings&quot;")
            && xml.contains("Pour &lt;slab&gt;"),
        "{xml}"
    );
    assert!(
        xml.contains("<OutlineLevel>2</OutlineLevel>")
            && xml.contains("<Milestone>1</Milestone>")
            && xml.contains("<ConstraintType>4</ConstraintType>")
            && xml.contains("<PredecessorUID>2</PredecessorUID>"),
        "{xml}"
    );

    // Reading the file into another project gives the same tasks, nesting, links and constraint.
    w.ok(pm, "schedule.import_xml", json!({"project": other, "text": xml}));
    let rows = |w: &World, project| {
        buildkiosk_engine::sched::rows(w.engine.state(), project, today)
            .into_iter()
            .map(|r| (r.name, r.depth, r.start.to_string(), r.finish.to_string(), r.milestone, r.constraint))
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&w, p), rows(&w, other));
    let links = |w: &World, project| {
        w.engine.state().schedule_tasks.iter().filter(|t| t.project == project && t.deleted.is_none()).map(|t| t.predecessors.len()).sum::<usize>()
    };
    assert_eq!((links(&w, p), links(&w, other)), (1, 1));
    // An empty schedule is still a well-formed file.
    let third = w.project("Empty");
    assert!(export_xml(w.engine.state(), third, today).contains("<Tasks>"));
}

#[test]
fn more_constraints_and_company_holiday_lists() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let span = |list: &Value, n| (row(list, n)["start"].as_str().unwrap().to_string(), row(list, n)["finish"].as_str().unwrap().to_string());
    // 1: must finish on Wed 21; 2: finish no earlier than Thu 15; 3: start no later than Tue 13 but starts Wed 14.
    w.ok(
        pm,
        "schedule.add",
        json!({"project": p, "name": "Pour", "start": "2026-10-12", "days": 2, "constraint": "mfo", "constraint_date": "2026-10-21"}),
    );
    w.ok(
        pm,
        "schedule.add",
        json!({"project": p, "name": "Inspect", "start": "2026-10-12", "days": 1, "constraint": "fnet", "constraint_date": "2026-10-15"}),
    );
    w.ok(
        pm,
        "schedule.add",
        json!({"project": p, "name": "Late", "start": "2026-10-14", "days": 1, "constraint": "snlt", "constraint_date": "2026-10-13"}),
    );
    // 4 delivers for 5, which must start Wed 21: delivery can wait until Tuesday 20.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Deliver", "start": "2026-10-12", "days": 1, "constraint": "alap"}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Install", "start": "2026-10-21", "days": 2, "predecessors": "4", "constraint": "mso", "constraint_date": "2026-10-21"}));
    w.err(pm, "schedule.add", json!({"project": p, "name": "Bad", "start": "2026-10-12", "constraint": "mfo"}));
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    assert!(row(&list, 1)["broken"].as_bool().unwrap() && row(&list, 2)["broken"].as_bool().unwrap() && row(&list, 3)["broken"].as_bool().unwrap());
    assert!(!row(&list, 4)["broken"].as_bool().unwrap(), "as late as possible is never broken");
    assert!(w.ok(pm, "schedule.get", json!({"project": p, "id": 4}))["constraint_date"].is_null());

    w.ok(pm, "schedule.recalculate", json!({"project": p}));
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    assert_eq!(span(&list, 1), ("2026-10-20".into(), "2026-10-21".into()), "finish fixed, start follows");
    assert_eq!(span(&list, 2), ("2026-10-15".into(), "2026-10-15".into()), "pushed until it finishes on the 15th");
    assert_eq!(span(&list, 3), ("2026-10-14".into(), "2026-10-14".into()), "a late start is flagged, not moved");
    assert!(row(&list, 3)["broken"].as_bool().unwrap() && !row(&list, 1)["broken"].as_bool().unwrap() && !row(&list, 2)["broken"].as_bool().unwrap());
    assert_eq!(span(&list, 4), ("2026-10-20".into(), "2026-10-20".into()), "pushed to the latest day before Install");
    assert_eq!(span(&list, 5), ("2026-10-21".into(), "2026-10-22".into()));

    // Company holiday lists: admins keep them, projects subscribe, and every duration follows.
    let rae = w.member(p, "Rae", "Reader", "Read only");
    w.err(pm, "holiday_list.save", json!({"name": "Federal 2026", "holidays": "2026-10-12 Columbus Day"}));
    let l =
        w.ok(w.admin(), "holiday_list.save", json!({"name": "Federal 2026", "holidays": "2026-11-11 Veterans Day\n2026-10-12 Columbus Day"}))["id"]
            .as_u64()
            .unwrap();
    w.err(w.admin(), "holiday_list.save", json!({"name": "federal 2026", "holidays": ""}));
    w.err(w.admin(), "holiday_list.save", json!({"name": "Bad", "holidays": "soon"}));
    w.err(w.admin(), "holiday_list.save", json!({"name": "Twice", "holidays": "2026-10-12\n2026-10-12"}));
    assert_eq!(w.ok(rae, "holiday_list.list", json!({})).as_array().unwrap().len(), 1);
    assert!(!sched::calendar(w.engine.state(), p).holiday_on(d("2026-10-12")));
    w.err(rae, "schedule.calendar_set", json!({"project": p, "lists": [l]}));
    w.err(pm, "schedule.calendar_set", json!({"project": p, "lists": [9999]}));
    w.ok(pm, "schedule.calendar_set", json!({"project": p, "lists": [l], "holidays": "2026-12-25 Christmas"}));
    let cal = sched::calendar(w.engine.state(), p);
    assert!(cal.holiday_on(d("2026-10-12")) && cal.holiday_on(d("2026-11-11")) && cal.holiday_on(d("2026-12-25")));
    assert_eq!(sched::shift_in(&cal, d("2026-10-09"), 1), Some(d("2026-10-13")), "Columbus Day is skipped");
    // The project's own list is stored on its own; the company days are not copied in.
    assert_eq!(w.engine.state().projects.get(p).unwrap().calendar.holidays.len(), 1);
    // Changing the company list changes the project; deleting it lets go.
    w.ok(w.admin(), "holiday_list.save", json!({"id": l, "name": "Federal 2026", "holidays": "2026-11-11 Veterans Day"}));
    assert!(!sched::calendar(w.engine.state(), p).holiday_on(d("2026-10-12")));
    w.ok(w.admin(), "holiday_list.delete", json!({"id": l}));
    assert!(w.engine.state().projects.get(p).unwrap().holiday_lists.is_empty());
    assert!(!sched::calendar(w.engine.state(), p).holiday_on(d("2026-11-11")));
    w.assert_replay();
}

#[test]
fn several_named_baselines_and_the_one_in_use() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Demo", "start": "2026-10-12", "days": 2}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Frame", "start": "2026-10-14", "days": 3, "predecessors": "1"}));
    w.err(pm, "schedule.baseline_save", json!({"project": p}));
    w.err(sup, "schedule.baseline_save", json!({"project": p, "name": "Contract"}));
    let a = w.ok(pm, "schedule.baseline_save", json!({"project": p, "name": "Contract"}))["id"].as_u64().unwrap();
    let baseline_of = |w: &World, n: u32| {
        let t = w.engine.state().schedule_tasks.iter().find(|t| t.number == n).unwrap();
        t.baseline_finish.map(|d| d.to_string())
    };
    assert_eq!((baseline_of(&w, 1).as_deref(), baseline_of(&w, 2).as_deref()), (Some("2026-10-13"), Some("2026-10-16")));

    // The schedule slips and a new baseline is saved; the chart follows the one in use.
    w.ok(pm, "schedule.move", json!({"project": p, "id": 1, "shift": 2}));
    let b = w.ok(pm, "schedule.baseline_save", json!({"project": p, "name": "After the redesign"}))["id"].as_u64().unwrap();
    assert_ne!(a, b);
    assert_eq!(baseline_of(&w, 1).as_deref(), Some("2026-10-15"));
    let list = w.ok(pm, "schedule.baselines", json!({"project": p}));
    let in_use: Vec<(&str, bool)> = list.as_array().unwrap().iter().map(|r| (r["name"].as_str().unwrap(), r["in_use"].as_bool().unwrap())).collect();
    assert_eq!(in_use, [("After the redesign", true), ("Contract", false)], "newest first");
    // Back to the first: the slip is measured against it again.
    w.ok(pm, "schedule.baseline_use", json!({"project": p, "id": a}));
    assert_eq!(baseline_of(&w, 1).as_deref(), Some("2026-10-13"));
    let rows = w.ok(pm, "schedule.list", json!({"project": p}));
    assert_eq!(row(&rows, 1)["slip"].as_i64(), Some(2), "{rows}");
    // Saving under the same name replaces it; a task added later is not in the older baselines.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Roof", "start": "2026-10-19", "days": 2}));
    assert_eq!(baseline_of(&w, 3), None);
    w.ok(pm, "schedule.baseline_save", json!({"project": p, "name": "contract"}));
    assert_eq!(w.engine.state().schedule_baselines.iter().filter(|b| b.deleted.is_none()).count(), 2);
    assert_eq!(baseline_of(&w, 3).as_deref(), Some("2026-10-20"));
    // The plain baseline command leaves the saved ones aside; deleting the one in use clears the chart.
    w.ok(pm, "schedule.baseline", json!({"project": p}));
    assert!(w.engine.state().schedule_baselines.iter().all(|b| !b.in_use));
    w.ok(pm, "schedule.baseline_use", json!({"project": p, "id": b}));
    w.ok(pm, "schedule.baseline_delete", json!({"project": p, "id": b}));
    assert_eq!(baseline_of(&w, 1), None);
    w.err(pm, "schedule.baseline_use", json!({"project": p, "id": b}));
    let other = w.project("Other");
    w.err(pm, "schedule.baseline_use", json!({"project": other, "id": a}));
    w.assert_replay();
}

#[test]
fn importing_a_planning_program_exchange_file() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    // A small made-up file in the tab-separated layout: two projects, a work breakdown, tasks and links.
    let xer = [
        "ERMHDR\t19.12\t2026-10-12\tProject\tadmin\tdemo\tdbname\tProject Management\tUSD",
        "%T\tPROJECT",
        "%F\tproj_id\tproj_short_name",
        "%R\t100\tCLINIC",
        "%R\t200\tOTHER",
        "%T\tPROJWBS",
        "%F\twbs_id\tproj_id\tparent_wbs_id\twbs_short_name\twbs_name\tproj_node_flag",
        "%R\t1\t100\t\tCLINIC\tRiverside Clinic\tY",
        "%R\t2\t100\t1\tSITE\tSite work\tN",
        "%R\t3\t100\t1\tEMPTY\tNothing under this\tN",
        "%R\t9\t200\t\tOTHER\tOther project\tY",
        "%T\tTASK",
        "%F\ttask_id\tproj_id\twbs_id\ttask_name\ttask_type\tstatus_code\ttarget_start_date\ttarget_end_date\tact_start_date\tact_end_date\tphys_complete_pct\tcstr_type\tcstr_date",
        "%R\t11\t100\t2\tClear\tTT_Task\tTK_Complete\t2026-10-12 08:00\t2026-10-13 17:00\t2026-10-12 08:00\t2026-10-13 17:00\t100\t\t",
        "%R\t12\t100\t2\tGrade\tTT_Task\tTK_Active\t2026-10-14 08:00\t2026-10-16 17:00\t2026-10-14 08:00\t\t40\tCS_MSOA\t2026-10-14 00:00",
        "%R\t13\t100\t1\tSite ready\tTT_FinMile\tTK_NotStart\t2026-10-19 08:00\t2026-10-19 08:00\t\t\t\t\t",
        "%R\t14\t100\t2\tNo dates\tTT_Task\tTK_NotStart\t\t\t\t\t\t\t",
        "%R\t15\t100\t2\tSummary line\tTT_WBS\tTK_NotStart\t2026-10-12 08:00\t2026-10-16 17:00\t\t\t\t\t",
        "%R\t21\t200\t9\tElsewhere\tTT_Task\tTK_NotStart\t2026-10-12 08:00\t2026-10-13 17:00\t\t\t\t\t",
        "%T\tTASKPRED",
        "%F\ttask_pred_id\ttask_id\tpred_task_id\tproj_id\tpred_type\tlag_hr_cnt",
        "%R\t1\t12\t11\t100\tPR_SS\t8",
        "%R\t2\t13\t12\t100\tPR_FS\t0",
        "%R\t3\t13\t21\t100\tPR_FS\t0",
        "%E",
    ]
    .join("\r\n");
    w.err(reader, "schedule.import_xer", json!({"project": p, "text": xer}));
    w.err(pm, "schedule.import_xer", json!({"project": p, "text": "plain text"}));
    w.err(pm, "schedule.import_xer", json!({"project": p, "text": xer, "proj_id": "999"}));
    let out = w.ok(pm, "schedule.import_xer", json!({"project": p, "text": xer}));
    // The work breakdown node with tasks becomes a summary task; the empty one and the root don't.
    assert_eq!((out["added"].as_u64(), out["skipped"].as_u64()), (Some(4), Some(1)));
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    let by_name = |n: &str| list.as_array().unwrap().iter().find(|r| r["name"] == n).unwrap().clone();
    assert_eq!(by_name("Site work")["summary"], true);
    assert_eq!((by_name("Clear")["depth"].as_u64(), by_name("Site ready")["milestone"].as_bool()), (Some(1), Some(true)));
    assert_eq!((by_name("Clear")["percent"].as_u64(), by_name("Grade")["percent"].as_u64()), (Some(100), Some(40)));
    let grade = w.ok(pm, "schedule.get", json!({"project": p, "id": by_name("Grade")["id"].as_u64().unwrap()}));
    assert_eq!((grade["predecessors"][0]["kind"].as_str(), grade["predecessors"][0]["lag"].as_i64()), (Some("ss"), Some(1)));
    assert_eq!((grade["constraint"].as_str(), grade["constraint_date"].as_str()), (Some("snet"), Some("2026-10-14")));
    // The link to a task in the other project is dropped, not an error.
    let ready = w.ok(pm, "schedule.get", json!({"project": p, "id": by_name("Site ready")["id"].as_u64().unwrap()}));
    assert_eq!(ready["predecessors"].as_array().unwrap().len(), 1);
    // The second project in the file can be chosen.
    let other = w.project("Harbor Office");
    let template = buildkiosk_engine::cmd::company::template_named(w.engine.state(), "Project manager").unwrap();
    w.ok(w.admin(), "member.add", json!({"project": other, "person": pm, "template": template}));
    let pm2 = pm;
    w.ok(pm2, "schedule.import_xer", json!({"project": other, "text": xer, "proj_id": "200"}));
    assert_eq!(w.ok(pm2, "schedule.list", json!({"project": other})).as_array().unwrap().len(), 1);
    w.assert_replay();
}

/// Framing and Blocking both need carpenters on 13 and 14 October; Punch follows Blocking and ends the project.
fn leveling_world() -> (World, Id<Project>, Id<Person>, Id<Person>) {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Framing", "start": "2026-10-12", "days": 3, "resources": "Carpenter 4"}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Blocking", "start": "2026-10-13", "days": 2, "resources": "Carpenter 3"}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Punch", "start": "2026-10-15", "days": 10, "predecessors": "2"}));
    w.ok(pm, "schedule.resource_limits", json!({"project": p, "text": "Carpenter 5"}));
    (w, p, pm, reader)
}

#[test]
fn leveling_resources_delays_work_and_what_follows() {
    let (mut w, p, pm, reader) = leveling_world();
    // The plan changes nothing and anyone who can read may ask for it.
    let plan = w.ok(reader, "schedule.level_plan", json!({"project": p}));
    assert_eq!(plan["moves"].as_array().unwrap().len(), 2);
    assert_eq!(plan["unresolved"], json!([]));
    let blocking = &plan["moves"][0];
    assert_eq!(
        (blocking["number"].clone(), blocking["start"].clone(), blocking["finish"].clone()),
        (json!(2), json!("2026-10-15"), json!("2026-10-16"))
    );
    assert_eq!(blocking["days"], 2);
    let punch = &plan["moves"][1];
    assert_eq!((punch["number"].clone(), punch["start"].clone(), punch["finish"].clone()), (json!(3), json!("2026-10-19"), json!("2026-10-30")));
    assert_eq!(
        (plan["finish_before"].clone(), plan["finish_after"].clone(), plan["finish_slip"].clone()),
        (json!("2026-10-28"), json!("2026-10-30"), json!(2))
    );
    let list = w.ok(reader, "schedule.list", json!({"project": p}));
    assert_eq!(row(&list, 2)["start"], "2026-10-13", "planning moves nothing");

    // Levelling needs the right to change the schedule.
    w.err(reader, "schedule.level", json!({"project": p}));
    let done = w.ok(pm, "schedule.level", json!({"project": p}));
    assert_eq!(done["moved"], 2);
    let list = w.ok(reader, "schedule.list", json!({"project": p}));
    assert_eq!(
        (row(&list, 1)["start"].clone(), row(&list, 1)["finish"].clone()),
        (json!("2026-10-12"), json!("2026-10-14")),
        "the first claim on the carpenters stays"
    );
    assert_eq!((row(&list, 2)["start"].clone(), row(&list, 2)["finish"].clone()), (json!("2026-10-15"), json!("2026-10-16")));
    assert_eq!((row(&list, 3)["start"].clone(), row(&list, 3)["finish"].clone()), (json!("2026-10-19"), json!("2026-10-30")), "the link is kept");
    // No day is over the limit now, and a second run has nothing to do.
    let load = w.ok(reader, "schedule.loading", json!({"project": p, "from": "2026-10-12", "weeks": 1}));
    assert_eq!(load[0]["over"], json!([false]));
    assert_eq!(w.ok(pm, "schedule.level", json!({"project": p}))["moved"], 0);
    // Another project's id is not found.
    w.err(pm, "schedule.level", json!({"project": 9999}));
    w.assert_replay();
}

#[test]
fn leveling_within_float_keeps_the_finish() {
    let (mut w, p, pm, _reader) = leveling_world();
    let plan = w.ok(pm, "schedule.level_plan", json!({"project": p, "within_float": true}));
    assert_eq!(plan["moves"], json!([]), "Blocking has no float, so nothing may move");
    assert_eq!(plan["unresolved"].as_array().unwrap().len(), 1);
    assert_eq!(plan["unresolved"][0]["number"], 2);
    assert!(plan["unresolved"][0]["reason"].as_str().unwrap().contains("float"));
    assert_eq!(plan["finish_slip"], 0);
    // With a long closing task behind it, Blocking has float and may give way without moving the finish.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Closeout", "start": "2026-10-12", "days": 20}));
    w.ok(pm, "schedule.update", json!({"project": p, "id": 3, "predecessors": ""}));
    let plan = w.ok(pm, "schedule.level_plan", json!({"project": p, "within_float": true}));
    assert_eq!(plan["unresolved"], json!([]));
    assert_eq!(plan["moves"].as_array().unwrap().len(), 1);
    assert_eq!((plan["moves"][0]["number"].clone(), plan["moves"][0]["start"].clone()), (json!(2), json!("2026-10-15")));
    assert_eq!(plan["finish_slip"], 0, "within float the project finish never moves");
    w.assert_replay();
}

#[test]
fn leveling_leaves_finished_started_and_pinned_work_alone() {
    let (mut w, p, pm, _reader) = leveling_world();
    // Started work and work pinned to a date hold their place and their carpenters.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Started", "start": "2026-10-19", "days": 2, "percent": 40, "resources": "Carpenter 5"}));
    w.ok(
        pm,
        "schedule.add",
        json!({"project": p, "name": "Pinned", "start": "2026-10-21", "days": 1, "resources": "Carpenter 5", "constraint": "mso", "constraint_date": "2026-10-21"}),
    );
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Done", "start": "2026-10-12", "days": 3, "percent": 100, "resources": "Carpenter 5"}));
    w.ok(pm, "schedule.level", json!({"project": p}));
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    for (number, start) in [(4, "2026-10-19"), (5, "2026-10-21"), (6, "2026-10-12")] {
        assert_eq!(row(&list, number)["start"], start);
    }
    // Finished work used nothing, so Blocking still lands on the 15th; Punch waits for the started and pinned work's days only if it needs the carpenters (it does not).
    assert_eq!(row(&list, 2)["start"], "2026-10-15");
    // A task that needs more than the limit by itself is reported, not moved.
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Too big", "start": "2026-11-02", "days": 1, "resources": "carpenter 9"}));
    let plan = w.ok(pm, "schedule.level_plan", json!({"project": p}));
    let big = plan["unresolved"].as_array().unwrap().iter().find(|u| u["name"] == "Too big").unwrap();
    assert!(big["reason"].as_str().unwrap().contains("more than the limit of 5"));
    assert!(plan["moves"].as_array().unwrap().iter().all(|m| m["name"] != "Too big"));
    w.assert_replay();
}

const PLAN_WITH_RESOURCES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Project xmlns="http://schemas.microsoft.com/project">
  <Name>Clinic</Name>
  <CalendarUID>2</CalendarUID>
  <Calendars>
    <Calendar><UID>1</UID><Name>Standard</Name><IsBaseCalendar>1</IsBaseCalendar></Calendar>
    <Calendar><UID>2</UID><Name>Six days</Name><IsBaseCalendar>1</IsBaseCalendar>
      <WeekDays>
        <WeekDay><DayType>1</DayType><DayWorking>0</DayWorking></WeekDay>
        <WeekDay><DayType>2</DayType><DayWorking>1</DayWorking></WeekDay>
        <WeekDay><DayType>3</DayType><DayWorking>1</DayWorking></WeekDay>
        <WeekDay><DayType>4</DayType><DayWorking>1</DayWorking></WeekDay>
        <WeekDay><DayType>5</DayType><DayWorking>1</DayWorking></WeekDay>
        <WeekDay><DayType>6</DayType><DayWorking>1</DayWorking></WeekDay>
        <WeekDay><DayType>7</DayType><DayWorking>1</DayWorking></WeekDay>
      </WeekDays>
      <Exceptions>
        <Exception><Name>Thanksgiving</Name><TimePeriod><FromDate>2026-11-26T00:00:00</FromDate><ToDate>2026-11-27T23:59:00</ToDate></TimePeriod><DayWorking>0</DayWorking></Exception>
        <Exception><Name>Make-up day</Name><TimePeriod><FromDate>2026-12-05T00:00:00</FromDate><ToDate>2026-12-05T23:59:00</ToDate></TimePeriod><DayWorking>1</DayWorking></Exception>
      </Exceptions>
    </Calendar>
  </Calendars>
  <Resources>
    <Resource><UID>0</UID><Name>Unassigned</Name></Resource>
    <Resource><UID>1</UID><Name>Carpenter, crew A</Name><MaxUnits>4</MaxUnits></Resource>
    <Resource><UID>2</UID><Name>Laborer</Name><MaxUnits>1.5</MaxUnits></Resource>
    <Resource><UID>3</UID><Name>Idle</Name><MaxUnits>9</MaxUnits></Resource>
  </Resources>
  <Tasks>
    <Task><UID>1</UID><Name>Framing</Name><OutlineLevel>1</OutlineLevel><Start>2026-10-12T08:00:00</Start><Finish>2026-10-14T17:00:00</Finish></Task>
    <Task><UID>2</UID><Name>Cleanup</Name><OutlineLevel>1</OutlineLevel><Start>2026-10-15T08:00:00</Start><Finish>2026-10-15T17:00:00</Finish></Task>
  </Tasks>
  <Assignments>
    <Assignment><TaskUID>1</TaskUID><ResourceUID>1</ResourceUID><Units>3</Units></Assignment>
    <Assignment><TaskUID>1</TaskUID><ResourceUID>2</ResourceUID><Units>1</Units></Assignment>
    <Assignment><TaskUID>1</TaskUID><ResourceUID>2</ResourceUID><Units>0.5</Units></Assignment>
    <Assignment><TaskUID>2</TaskUID><ResourceUID>2</ResourceUID><Units>1</Units></Assignment>
    <Assignment><TaskUID>2</TaskUID><ResourceUID>0</ResourceUID><Units>1</Units></Assignment>
    <Assignment><TaskUID>2</TaskUID><ResourceUID>-65535</ResourceUID><Units>1</Units></Assignment>
  </Assignments>
</Project>"#;

#[test]
fn importing_resources_limits_and_calendar_from_a_project_file() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    // Left alone unless asked for, as before: only tasks come across.
    let q = w.project("Other");
    let pm_q = w.member(q, "Quinn", "Manager", "Project manager");
    let out = w.ok(pm_q, "schedule.import_xml", json!({"project": q, "text": PLAN_WITH_RESOURCES}));
    assert_eq!(out["added"], 2);
    assert!(w.ok(pm_q, "schedule.get", json!({"project": q, "id": 1}))["resources"].is_null(), "no resources unless asked for");
    assert_eq!(w.ok(pm_q, "schedule.calendar", json!({"project": q}))["days"], json!([true, true, true, true, true, false, false]));

    // Resources: units round to whole numbers (at least one), a resource named twice adds up, commas in a name become spaces, and the unassigned rows are dropped.
    let out = w.ok(pm, "schedule.import_xml", json!({"project": p, "text": PLAN_WITH_RESOURCES, "resources": true}));
    assert_eq!(out["added"], 2);
    let framing = w.ok(pm, "schedule.get", json!({"project": p, "id": 3}));
    assert_eq!(framing["resources"], json!([{"name": "Carpenter crew A", "units": 3}, {"name": "Laborer", "units": 2}]));
    let cleanup = w.ok(pm, "schedule.get", json!({"project": p, "id": 4}));
    assert_eq!(cleanup["resources"], json!([{"name": "Laborer", "units": 1}]));

    // The limits and the calendar belong to the project, so they take a project admin.
    w.err(eng, "schedule.import_xml", json!({"project": p, "text": PLAN_WITH_RESOURCES, "limits": true}));
    w.err(eng, "schedule.import_xml", json!({"project": p, "text": PLAN_WITH_RESOURCES, "calendar": true}));
    let admin = w.admin();
    let out = w.ok(admin, "schedule.import_xml", json!({"project": p, "text": PLAN_WITH_RESOURCES, "limits": true, "calendar": true}));
    assert_eq!((out["limits"].clone(), out["calendar"].clone(), out["calendar_left_out"].clone()), (json!(2), json!(true), json!(1)));
    let cal = w.ok(pm, "schedule.calendar", json!({"project": p}));
    assert_eq!(cal["days"], json!([true, true, true, true, true, true, false]), "Monday to Saturday");
    let off: Vec<&str> = cal["holidays"].as_array().unwrap().iter().map(|h| h["date"].as_str().unwrap()).collect();
    assert_eq!(off, ["2026-11-26", "2026-11-27"], "the working-day exception can't be kept and is counted");
    assert_eq!(cal["holidays"][0]["name"], "Thanksgiving");
    // The limit came from MaxUnits for resources that are used (1.5 rounds to 2); the unused one is left out.
    let load = w.ok(pm, "schedule.loading", json!({"project": p, "from": "2026-10-12", "weeks": 1}));
    let limits: Vec<(String, u64)> =
        load.as_array().unwrap().iter().filter_map(|l| Some((l["name"].as_str()?.to_string(), l["limit"].as_u64()?))).collect();
    assert_eq!(limits, [("Carpenter crew A".to_string(), 4), ("Laborer".to_string(), 2)]);
    // A file with no calendar leaves the project's alone, and says so.
    let bare = r#"<Project xmlns="http://schemas.microsoft.com/project"><Tasks><Task><UID>1</UID><Name>X</Name><Start>2026-10-12T08:00:00</Start><Finish>2026-10-12T17:00:00</Finish></Task></Tasks></Project>"#;
    let out = w.ok(admin, "schedule.import_xml", json!({"project": p, "text": bare, "calendar": true}));
    assert_eq!(out["calendar"], false);
    assert_eq!(w.ok(pm, "schedule.calendar", json!({"project": p}))["days"], json!([true, true, true, true, true, true, false]));
    // The text import takes an eleventh column of resources too.
    let out = w.ok(pm, "schedule.import", json!({"project": p, "text": "10\tPour\t2026-11-02\t2026-11-03\t0\t\t\tno\t\t\tMason 2, Laborer\n"}));
    assert_eq!(out["added"], 1);
    w.err(pm, "schedule.import", json!({"project": p, "text": "11\tBad\t2026-11-02\t2026-11-03\t0\t\t\tno\t\t\tMason 0\n"}));
    w.assert_replay();
}

#[test]
fn exchange_file_resources_limits_and_calendar() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let admin = w.admin();
    // Monday to Friday worked (1 is Sunday, 7 Saturday), 26 November off, 27 November worked a half day.
    let week: String =
        (1..=7).map(|d| if (2..=6).contains(&d) { format!("(0||{d}()( (0||0(s|08:00|f|17:00)()))) ") } else { format!("(0||{d}()()) ") }).collect();
    let data = format!(
        "(0||CalendarData()( (0||DaysOfWeek()( {week}) (0||VIEW(ShowTotal|Y)()) (0||Exceptions()( (0||0(d|46352)()) (0||1(d|46353)( (0||0(s|08:00|f|12:00)()))) ))))"
    );
    let xer = [
        "ERMHDR\t19.12\t2026-10-12\tProject\tadmin\tdemo\tdbname\tProject Management\tUSD".to_string(),
        "%T\tPROJECT".into(),
        "%F\tproj_id\tproj_short_name\tclndr_id".into(),
        "%R\t100\tCLINIC\t7".into(),
        "%T\tCALENDAR".into(),
        "%F\tclndr_id\tdefault_flag\tclndr_data".into(),
        "%R\t3\tY\t(0||CalendarData()( (0||DaysOfWeek()( (0||1()()) (0||2()()) )))".into(),
        format!("%R\t7\tN\t{data}"),
        "%T\tPROJWBS".into(),
        "%F\twbs_id\tproj_id\tparent_wbs_id\twbs_short_name\twbs_name\tproj_node_flag".into(),
        "%R\t1\t100\t\tCLINIC\tRiverside Clinic\tY".into(),
        "%T\tTASK".into(),
        "%F\ttask_id\tproj_id\twbs_id\ttask_name\ttask_type\tstatus_code\ttarget_start_date\ttarget_end_date".into(),
        "%R\t11\t100\t1\tFraming\tTT_Task\tTK_NotStart\t2026-10-12 08:00\t2026-10-14 17:00".into(),
        "%R\t12\t100\t1\tBlocking\tTT_Task\tTK_NotStart\t2026-10-13 08:00\t2026-10-14 17:00".into(),
        "%T\tRSRC".into(),
        "%F\trsrc_id\trsrc_name\trsrc_short_name".into(),
        "%R\t5\tCarpenter\tCARP".into(),
        "%R\t6\tElectrician, senior\tELEC".into(),
        "%R\t7\tUnused\tUNUS".into(),
        "%T\tTASKRSRC".into(),
        "%F\ttask_id\trsrc_id\tproj_id\ttarget_qty_per_hr".into(),
        "%R\t11\t5\t100\t0.5".into(),
        "%R\t12\t5\t100\t0.375".into(),
        "%R\t12\t6\t100\t0.25".into(),
        "%T\tRSRCRATE".into(),
        "%F\trsrc_id\tmax_qty_per_hr".into(),
        "%R\t5\t0.75".into(),
        "%R\t7\t1".into(),
        "%E".into(),
    ]
    .join("\r\n");
    // Without the options only tasks come across.
    let plain = w.project("Plain");
    let template = buildkiosk_engine::cmd::company::template_named(w.engine.state(), "Project manager").unwrap();
    w.ok(admin, "member.add", json!({"project": plain, "person": pm, "template": template}));
    w.ok(pm, "schedule.import_xer", json!({"project": plain, "text": xer}));
    let plain_list = w.ok(pm, "schedule.list", json!({"project": plain}));
    let first = plain_list.as_array().unwrap()[0]["id"].as_u64().unwrap();
    assert_eq!(w.ok(pm, "schedule.get", json!({"project": plain, "id": first}))["resources"], serde_json::Value::Null);

    // The limits and the calendar take a project admin.
    w.ok(admin, "member.update", json!({"project": p, "person": eng, "overrides": {"schedule": "standard"}}));
    w.err(eng, "schedule.import_xer", json!({"project": p, "text": xer, "limits": true}));
    w.err(eng, "schedule.import_xer", json!({"project": p, "text": xer, "calendar": true}));
    let out = w.ok(admin, "schedule.import_xer", json!({"project": p, "text": xer, "resources": true, "limits": true, "calendar": true}));
    assert_eq!(
        (out["added"].as_u64(), out["limits"].as_u64(), out["calendar"].as_bool(), out["calendar_left_out"].as_u64()),
        (Some(2), Some(1), Some(true), Some(1))
    );

    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    let task = |n: &str| list.as_array().unwrap().iter().find(|r| r["name"] == n).unwrap().clone();
    let mut resources = |n: &str| w.ok(pm, "schedule.get", json!({"project": p, "id": task(n)["id"].as_u64().unwrap()}))["resources"].clone();
    assert_eq!(resources("Framing"), json!([{"name": "Carpenter", "units": 4}]));
    assert_eq!(resources("Blocking"), json!([{"name": "Carpenter", "units": 3}, {"name": "Electrician senior", "units": 2}]));
    // The limit is the largest an hour (0.75) times eight, for resources used in the project only.
    let proj = w.engine.state().projects.get(p).unwrap();
    assert_eq!(proj.resource_limits.len(), 1);
    assert_eq!((proj.resource_limits[0].name.as_str(), proj.resource_limits[0].units), ("Carpenter", 6));
    // The calendar: Monday to Friday, 26 November off; the working half day on the 27th is left out.
    assert_eq!(proj.calendar.days, [true, true, true, true, true, false, false]);
    assert_eq!(proj.calendar.holidays.len(), 1);
    assert_eq!(proj.calendar.holidays[0].date.to_string(), "2026-11-26");
    w.assert_replay();
}
