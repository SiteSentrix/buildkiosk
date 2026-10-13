//! Meetings: agenda, attendance, action items, minutes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn plan_hold_issue_and_follow_up() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let arch = w.member(p, "Ann", "Architect", "Owner / design team");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    let m = w.ok(
        eng,
        "meeting.create",
        json!({"project": p, "title": "Weekly coordination", "date": "2026-10-20", "category": "Coordination", "time": "9:00 AM", "location": "Site trailer",
               "invitees": [pm.raw(), arch.raw(), arch.raw()],
               "agenda": [{"title": "Look-ahead", "notes": ""}, {"title": "", "notes": ""}, {"title": "RFIs", "notes": "Door hardware"}],
               "actions": [{"description": "Confirm steel delivery", "assignee": arch.raw(), "due": "2026-10-27"}, {"description": ""}]}),
    )["id"]
        .as_u64()
        .unwrap();
    let meeting = w.engine.state().meetings.iter().next().unwrap().clone();
    assert_eq!((meeting.attendees.len(), meeting.agenda.len(), meeting.actions.len()), (2, 2, 1), "duplicates and blank rows are dropped");
    w.err(eng, "meeting.create", json!({"project": p, "date": "2026-10-20"}));
    w.err(eng, "meeting.create", json!({"project": p, "title": "x"}));
    w.err(reader, "meeting.create", json!({"project": p, "title": "x", "date": "2026-10-20"}));

    // Edit while scheduled: author or admin; record who came; attendance survives a changed invite list.
    w.err(arch, "meeting.update", json!({"project": p, "id": m, "title": "x"}));
    w.ok(eng, "meeting.update", json!({"project": p, "id": m, "present": [arch.raw()], "minutes": "All agreed."}));
    w.ok(eng, "meeting.update", json!({"project": p, "id": m, "invitees": [arch.raw(), pm.raw(), eng.raw()]}));
    let a = w.engine.state().meetings.iter().next().unwrap().attendees.clone();
    assert!(a.iter().find(|x| x.person == arch).unwrap().present);
    assert_eq!(a.len(), 3);

    // The assignee ticks their own action off; others can't.
    w.err(reader, "meeting.set_action", json!({"project": p, "id": m, "index": 0, "done": true}));
    w.err(pm, "meeting.set_action", json!({"project": p, "id": m, "index": 5, "done": true}));
    w.ok(arch, "meeting.set_action", json!({"project": p, "id": m, "index": 0, "done": true}));
    w.ok(eng, "meeting.set_action", json!({"project": p, "id": m, "index": 0, "done": false}));

    // Issuing notifies attendees, and the people with open actions get the longer message.
    w.err(arch, "meeting.issue", json!({"project": p, "id": m}));
    w.ok(eng, "meeting.issue", json!({"project": p, "id": m}));
    w.err(eng, "meeting.issue", json!({"project": p, "id": m}));
    w.err(eng, "meeting.update", json!({"project": p, "id": m, "title": "Changed"}));
    let note = w.engine.state().notifications.iter().find(|n| n.person == arch).unwrap().text.clone();
    assert!(note.contains("Meeting #1") && note.contains("you have an action"), "{note}");
    assert!(w.engine.state().notifications.iter().any(|n| n.person == pm));
    w.err(eng, "meeting.reopen", json!({"project": p, "id": m}));
    w.ok(pm, "meeting.reopen", json!({"project": p, "id": m}));

    assert_eq!(w.ok(reader, "meeting.list", json!({"project": p, "q": "steel"})).as_array().unwrap().len(), 1);
    assert_eq!(w.ok(reader, "meeting.list", json!({"project": p, "status": "issued"})).as_array().unwrap().len(), 0);
    w.ok(eng, "meeting.attach", json!({"project": p, "id": m, "sha256": "f".repeat(64), "name": "agenda.pdf", "size": 3}));
    w.ok(eng, "meeting.detach", json!({"project": p, "id": m, "sha256": "f".repeat(64)}));
    w.ok(eng, "meeting.delete", json!({"project": p, "id": m}));
    w.ok(pm, "meeting.restore", json!({"project": p, "id": m}));
    let other = w.project("Other");
    w.err(pm, "meeting.get", json!({"project": other, "id": m}));
    w.assert_replay();
}

fn meeting(w: &World, id: u64) -> buildkiosk_model::meeting::Meeting {
    w.engine.state().meetings.get(buildkiosk_core::Id::new(id)).unwrap().clone()
}

#[test]
fn a_meeting_can_be_repeated_on_a_schedule() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let arch = w.member(p, "Ann", "Architect", "Owner / design team");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let m = w
        .ok(
            pm,
            "meeting.create",
            json!({"project": p, "title": "OAC", "date": "2026-10-31", "category": "OAC", "time": "10:00", "location": "Trailer", "invitees": [arch.raw()],
                   "agenda": [{"title": "Schedule", "notes": "Last week's notes"}], "actions": [{"description": "Send RFI log"}], "minutes": "Done."}),
        )["id"]
        .as_u64()
        .unwrap();
    w.err(reader, "meeting.repeat", json!({"project": p, "id": m, "every": "weekly", "count": 2}));
    w.err(pm, "meeting.repeat", json!({"project": p, "id": m, "every": "daily", "count": 2}));
    w.err(pm, "meeting.repeat", json!({"project": p, "id": m, "every": "weekly"}));
    w.err(pm, "meeting.repeat", json!({"project": p, "id": m, "every": "weekly", "count": 0}));
    w.err(pm, "meeting.repeat", json!({"project": p, "id": m, "every": "weekly", "count": 27}));
    assert_eq!(w.engine.state().meetings.iter().count(), 1, "a refused repeat makes nothing");

    let r = w.ok(pm, "meeting.repeat", json!({"project": p, "id": m, "every": "weekly", "count": 3}));
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|i| i.as_u64().unwrap()).collect();
    let dates: Vec<String> = ids.iter().map(|i| meeting(&w, *i).date.to_string()).collect();
    assert_eq!(dates, ["2026-11-07", "2026-11-14", "2026-11-21"]);
    let copy = meeting(&w, ids[0]);
    assert_eq!(
        (copy.title.as_str(), copy.category.as_str(), copy.time.as_str(), copy.location.as_str(), copy.number),
        ("OAC", "OAC", "10:00", "Trailer", 2)
    );
    assert_eq!(
        (copy.attendees.len(), copy.attendees[0].present, copy.agenda[0].title.as_str(), copy.agenda[0].notes.as_str()),
        (1, false, "Schedule", "")
    );
    assert!(copy.actions.is_empty() && copy.minutes.is_empty() && copy.status == buildkiosk_model::meeting::MeetingStatus::Scheduled);
    // Monthly keeps the day, or the end of a shorter month.
    let months = w.ok(pm, "meeting.repeat", json!({"project": p, "id": m, "every": "monthly", "count": 2}));
    let dates: Vec<String> = months["ids"].as_array().unwrap().iter().map(|i| meeting(&w, i.as_u64().unwrap()).date.to_string()).collect();
    assert_eq!(dates, ["2026-11-30", "2026-12-31"]);
    w.assert_replay();
}

#[test]
fn open_actions_carry_forward_without_being_counted_twice() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let arch = w.member(p, "Ann", "Architect", "Owner / design team");
    let first = w
        .ok(
            pm,
            "meeting.create",
            json!({"project": p, "title": "OAC 1", "date": "2026-10-13", "category": "OAC",
                   "actions": [{"description": "Order hardware", "assignee": arch.raw(), "due": "2026-10-20"}, {"description": "Done already", "done": true}, {"description": "Revise schedule"}]}),
        )["id"]
        .as_u64()
        .unwrap();
    let other_kind = w.ok(
        pm,
        "meeting.create",
        json!({"project": p, "title": "Safety", "date": "2026-10-14", "category": "Safety", "actions": [{"description": "Toolbox talk"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    let second = w.ok(pm, "meeting.create", json!({"project": p, "title": "OAC 2", "date": "2026-10-20", "category": "OAC"}))["id"].as_u64().unwrap();

    w.err(arch, "meeting.carry_actions", json!({"project": p, "id": second}));
    w.err(pm, "meeting.carry_actions", json!({"project": p, "id": first}));
    w.err(pm, "meeting.carry_actions", json!({"project": p, "id": second, "from": second}));
    // By default it takes the latest earlier meeting of the same type, skipping the safety meeting in between.
    let r = w.ok(pm, "meeting.carry_actions", json!({"project": p, "id": second}));
    assert_eq!(r["carried"], 2);
    let (a, b) = (meeting(&w, first), meeting(&w, second));
    assert_eq!(b.actions.iter().map(|x| x.description.as_str()).collect::<Vec<_>>(), ["Order hardware", "Revise schedule"]);
    assert_eq!(
        (b.actions[0].assignee, b.actions[0].due.map(|d| d.to_string()), b.actions[0].from_meeting.map(|m| m.raw())),
        (Some(arch), Some("2026-10-20".to_string()), Some(first))
    );
    assert_eq!((a.actions[0].carried, a.actions[1].carried, a.actions[2].carried), (true, false, true), "the done one stays where it was");
    // The same action is never carried twice, and the old copy no longer counts as open.
    assert_eq!(w.ok(pm, "meeting.carry_actions", json!({"project": p, "id": second}))["carried"], 0);
    assert_eq!(b.actions.len(), 2);
    let summary = w.ok(pm, "report.summary", json!({"project": p}));
    let row = summary.to_string();
    let open = summary.as_array().and_then(|rows| rows.iter().find(|r| r["tool"] == "Meeting action items")).map(|r| r["open"].clone());
    assert_eq!(open, Some(json!(3)), "two carried to OAC 2 and the safety one: {row}");
    // An explicit source works too.
    let r = w.ok(pm, "meeting.carry_actions", json!({"project": p, "id": second, "from": other_kind}));
    assert_eq!(r["carried"], 1);
    // Editing a meeting through the form keeps what the form can't show.
    w.ok(pm, "meeting.update", json!({"project": p, "id": first, "actions": [{"description": "Order hardware", "assignee": arch.raw(), "due": "2026-10-20"}, {"description": "Done already", "done": true}, {"description": "Revise schedule"}]}));
    assert!(meeting(&w, first).actions[0].carried, "an edit that leaves the text alone keeps it carried");
    w.assert_replay();
}

#[test]
fn an_action_becomes_a_task_and_closing_the_task_finishes_the_action() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let arch = w.member(p, "Ann", "Architect", "Owner / design team");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let m = w
        .ok(pm, "meeting.create", json!({"project": p, "title": "OAC", "date": "2026-10-13", "actions": [{"description": "Order hardware", "assignee": arch.raw(), "due": "2026-10-20"}, {"description": "Nothing yet"}]}))["id"]
        .as_u64()
        .unwrap();
    w.err(reader, "meeting.action_to_task", json!({"project": p, "id": m, "index": 0}));
    w.err(pm, "meeting.action_to_task", json!({"project": p, "id": m, "index": 9}));
    let r = w.ok(pm, "meeting.action_to_task", json!({"project": p, "id": m, "index": 0}));
    let task = w.engine.state().tasks.get(buildkiosk_core::Id::new(r["task"].as_u64().unwrap())).unwrap().clone();
    assert_eq!(
        (task.title.as_str(), task.assignees.clone(), task.due.map(|d| d.to_string()), task.meeting.map(|x| x.raw())),
        ("Order hardware", vec![arch], Some("2026-10-20".to_string()), Some(m))
    );
    assert_eq!(meeting(&w, m).actions[0].task, Some(task.id));
    w.err(pm, "meeting.action_to_task", json!({"project": p, "id": m, "index": 0}));
    // The edit form (which knows nothing of the link) doesn't lose it.
    w.ok(pm, "meeting.update", json!({"project": p, "id": m, "actions": [{"description": "Order hardware", "assignee": arch.raw(), "due": "2026-10-20"}, {"description": "Nothing yet"}]}));
    assert_eq!(meeting(&w, m).actions[0].task, Some(task.id));
    // Closing the task finishes the action.
    assert!(!meeting(&w, m).actions[0].done);
    w.ok(pm, "task.close", json!({"project": p, "id": task.id.raw()}));
    assert!(meeting(&w, m).actions[0].done);
    assert!(!meeting(&w, m).actions[1].done);
    w.err(pm, "meeting.action_to_task", json!({"project": p, "id": m, "index": 0}));
    w.assert_replay();
}

#[test]
fn templates_start_a_meeting_with_its_usual_people_place_and_agenda() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    // A template from typed-in values: the agenda is one topic a line.
    w.err(reader, "meeting_template.save", json!({"project": p, "name": "Weekly"}));
    let t = w.ok(
        pm,
        "meeting_template.save",
        json!({"project": p, "name": "Weekly coordination", "category": "Coordination", "time": "9:00 AM", "location": "Site trailer",
               "invitees": [pm.raw(), eng.raw()], "agenda": "- Look-ahead\n\n* RFIs\nSafety"}),
    )["id"]
        .as_u64()
        .unwrap();
    let tpl = w.engine.state().meeting_templates.iter().next().unwrap().clone();
    assert_eq!((tpl.agenda.as_slice(), tpl.invitees.len()), (&["Look-ahead".to_string(), "RFIs".to_string(), "Safety".to_string()][..], 2));
    // Saving under the same name replaces it.
    assert_eq!(w.ok(pm, "meeting_template.save", json!({"project": p, "name": "weekly COORDINATION", "location": "Room 2"}))["id"].as_u64(), Some(t));
    assert_eq!(w.engine.state().meeting_templates.iter().filter(|x| x.deleted.is_none()).count(), 1);
    w.ok(pm, "meeting_template.save", json!({"project": p, "name": "Weekly coordination", "category": "Coordination", "time": "9:00 AM", "location": "Site trailer", "invitees": [pm.raw(), eng.raw()], "agenda": "Look-ahead\nRFIs"}));

    // A new meeting takes what the form leaves blank from the template; what is typed wins.
    let m = w.ok(
        eng,
        "meeting.create",
        json!({"project": p, "title": "Week 42", "date": "2026-10-20", "template": t, "location": "Room 5", "category": ""}),
    )["id"]
        .as_u64()
        .unwrap();
    let made = w.engine.state().meetings.iter().find(|x| x.id.raw() == m).unwrap().clone();
    assert_eq!((made.category.as_str(), made.time.as_str(), made.location.as_str()), ("Coordination", "9:00 AM", "Room 5"));
    assert_eq!((made.attendees.len(), made.agenda.iter().map(|a| a.title.as_str()).collect::<Vec<_>>()), (2, vec!["Look-ahead", "RFIs"]));
    w.err(eng, "meeting.create", json!({"project": p, "title": "x", "date": "2026-10-20", "template": 9999}));

    // A template from an existing meeting; another project's meeting can't be used.
    let t2 = w.ok(eng, "meeting_template.save", json!({"project": p, "name": "From week 42", "from_meeting": m}))["id"].as_u64().unwrap();
    assert_eq!(w.engine.state().meeting_templates.iter().find(|x| x.id.raw() == t2).unwrap().agenda.len(), 2);
    let other = w.project("Other");
    let foreign = w.ok(w.admin(), "meeting.create", json!({"project": other, "title": "Elsewhere", "date": "2026-10-20"}))["id"].as_u64().unwrap();
    w.err(eng, "meeting_template.save", json!({"project": p, "name": "Bad", "from_meeting": foreign}));
    w.err(eng, "meeting.create", json!({"project": other, "title": "x", "date": "2026-10-20", "template": t}));
    assert_eq!(w.ok(reader, "meeting_template.list", json!({"project": p})).as_array().unwrap().len(), 2);
    w.ok(eng, "meeting_template.delete", json!({"project": p, "id": t2}));
    w.err(eng, "meeting_template.delete", json!({"project": p, "id": t2}));
    w.err(eng, "meeting.create", json!({"project": p, "title": "x", "date": "2026-10-20", "template": t2}));
    w.assert_replay();
}
