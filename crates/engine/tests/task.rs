//! The tasks workflow and its permissions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::task::TaskStatus;
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn work_done_review_send_back_close_and_reopen() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let other = w.member(p, "Olga", "Other", "Subcontractor");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let unread = |w: &mut World, who| w.ok(who, "notification.list", json!({"unread": true})).as_array().unwrap().len();

    let id = w.ok(pm, "task.create", json!({"project": p, "title": "Order door hardware", "location": "Level 1 corridor", "priority": "high", "assignees": [sub, sub], "due": "2026-10-30"}))["id"].as_u64().unwrap();
    let get = |w: &World| w.engine.state().tasks.iter().next().unwrap().clone();
    assert_eq!(get(&w).assignees, vec![sub], "assigned once");
    assert_eq!(get(&w).ball_in_court, vec![sub]);
    assert_eq!(unread(&mut w, sub), 1);

    // Only assignees and the manager mark work done; readers can't even comment.
    w.err(other, "task.complete", json!({"project": p, "id": id}));
    w.err(reader, "task.comment", json!({"project": p, "id": id, "body": "hi"}));
    w.ok(other, "task.comment", json!({"project": p, "id": id, "body": "Is the lift available?"}));
    w.ok(sub, "task.complete", json!({"project": p, "id": id, "comment": "Painted two coats."}));
    assert_eq!((get(&w).status, get(&w).ball_in_court.clone()), (TaskStatus::ReadyForReview, vec![pm]));
    assert_eq!(
        (unread(&mut w, pm), unread(&mut w, sub)),
        (1, 2),
        "the comment goes to the assignee, who has the ball; the manager hears the work is done"
    );
    w.err(sub, "task.close", json!({"project": p, "id": id}));
    w.err(pm, "task.send_back", json!({"project": p, "id": id}));
    w.ok(pm, "task.send_back", json!({"project": p, "id": id, "comment": "Still see a patch near the frame."}));
    assert_eq!((get(&w).status, get(&w).comments.len()), (TaskStatus::Open, 3));
    w.ok(sub, "task.complete", json!({"project": p, "id": id}));
    w.ok(pm, "task.close", json!({"project": p, "id": id}));
    assert_eq!(get(&w).status, TaskStatus::Closed);
    w.err(sub, "task.update", json!({"project": p, "id": id, "title": "x"}));
    w.err(sub, "task.comment", json!({"project": p, "id": id, "body": "late"}));
    w.err(sub, "task.reopen", json!({"project": p, "id": id}));
    w.ok(pm, "task.reopen", json!({"project": p, "id": id, "comment": "Water stain returned."}));
    assert_eq!(get(&w).status, TaskStatus::Open);
    w.assert_replay();
}

#[test]
fn task_files_and_isolation() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let id = w.ok(pm, "task.create", json!({"project": p, "title": "Collect warranties"}))["id"].as_u64().unwrap();
    w.ok(pm, "task.attach", json!({"project": p, "id": id, "sha256": "f".repeat(64), "name": "list.xlsx", "size": 10}));
    let other = w.project("Other");
    w.err(pm, "task.get", json!({"project": other, "id": id}));
    assert_eq!(w.ok(pm, "task.list", json!({"project": p})).as_array().unwrap().len(), 1);
    w.assert_replay();
}

fn get_task(w: &World, id: u64) -> buildkiosk_model::task::Task {
    w.engine.state().tasks.get(buildkiosk_core::Id::new(id)).unwrap().clone()
}

#[test]
fn subtasks_go_one_level_deep_and_hold_up_their_parent() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let other = w.project("Harbor Office");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let template = buildkiosk_engine::cmd::company::template_named(w.engine.state(), "Project manager").unwrap();
    w.ok(w.admin(), "member.add", json!({"project": other, "person": pm, "template": template}));
    let parent = w.ok(pm, "task.create", json!({"project": p, "title": "Close out level 1"}))["id"].as_u64().unwrap();
    let a = w.ok(pm, "task.create", json!({"project": p, "title": "Punch walk", "parent": parent}))["id"].as_u64().unwrap();
    let b = w.ok(pm, "task.create", json!({"project": p, "title": "Clean", "parent": parent}))["id"].as_u64().unwrap();
    assert_eq!(get_task(&w, a).parent.map(|x| x.raw()), Some(parent));

    // One level only; no cycles; the parent must be a task of this project.
    w.err(pm, "task.create", json!({"project": p, "title": "Too deep", "parent": a}));
    w.err(pm, "task.update", json!({"project": p, "id": parent, "parent": a}));
    w.err(pm, "task.update", json!({"project": p, "id": parent, "parent": parent}));
    w.err(pm, "task.update", json!({"project": p, "id": parent, "parent": b}));
    let foreign = w.ok(pm, "task.create", json!({"project": other, "title": "Elsewhere"}))["id"].as_u64().unwrap();
    w.err(pm, "task.create", json!({"project": p, "title": "x", "parent": foreign}));
    w.err(pm, "task.create", json!({"project": p, "title": "x", "parent": 999}));
    w.err(pm, "task.create", json!({"project": p, "title": "x", "parent": a, "repeat": "weekly", "due": "2026-10-20"}));

    // A parent can't be closed while a subtask is open.
    let e = w.err(pm, "task.close", json!({"project": p, "id": parent}));
    assert!(e.to_string().contains("2 open subtasks"), "{e}");
    w.ok(pm, "task.close", json!({"project": p, "id": a}));
    assert!(w.err(pm, "task.close", json!({"project": p, "id": parent})).to_string().contains("1 open subtask"));
    w.ok(pm, "task.delete", json!({"project": p, "id": b}));
    w.ok(pm, "task.close", json!({"project": p, "id": parent}));
    // Taking a task out of a family is allowed.
    w.ok(pm, "task.update", json!({"project": p, "id": a, "parent": ""}));
    assert_eq!(get_task(&w, a).parent, None);
    w.assert_replay();
}

#[test]
fn repeating_tasks_make_the_next_one_when_closed() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let unread = |w: &mut World, who| w.ok(who, "notification.list", json!({"unread": true})).as_array().unwrap().len();

    w.err(pm, "task.create", json!({"project": p, "title": "No date", "repeat": "weekly"}));
    w.err(pm, "task.create", json!({"project": p, "title": "x", "repeat": "fortnightly", "due": "2026-10-14"}));
    w.err(pm, "task.create", json!({"project": p, "title": "x", "repeat_until": "2026-12-01", "due": "2026-10-14"}));
    w.err(pm, "task.create", json!({"project": p, "title": "x", "repeat": "weekly", "due": "2026-10-14", "repeat_until": "2026-10-01"}));
    let t = w.ok(
        pm,
        "task.create",
        json!({"project": p, "title": "Inspect fall protection", "kind": "Safety", "priority": "high", "assignees": [sub], "due": "2026-10-14", "repeat": "weekly", "repeat_until": "2026-10-30"}),
    )["id"]
        .as_u64()
        .unwrap();
    let before = unread(&mut w, sub);
    w.ok(pm, "task.close", json!({"project": p, "id": t}));
    let closed = get_task(&w, t);
    let next = get_task(&w, closed.next.unwrap().raw());
    assert_eq!(
        (next.title.as_str(), next.due.unwrap().to_string(), next.assignees.clone(), next.status),
        ("Inspect fall protection", "2026-10-21".to_string(), vec![sub], TaskStatus::Open)
    );
    assert_eq!((next.kind.as_str(), next.priority, next.repeat, next.repeat_until), ("Safety", closed.priority, closed.repeat, closed.repeat_until));
    assert_eq!(next.number, 2);
    assert_eq!(unread(&mut w, sub), before + 2, "the assignee hears it was closed and about the next one");
    // Reopening and closing again doesn't make a second.
    w.ok(pm, "task.reopen", json!({"project": p, "id": t}));
    w.ok(pm, "task.close", json!({"project": p, "id": t}));
    assert_eq!(w.engine.state().tasks.iter().count(), 2);
    // The series stops at repeat_until: 28 Oct is within it, 4 Nov is not.
    let n2 = w.ok(pm, "task.close", json!({"project": p, "id": next.id.raw()}));
    assert!(n2["id"].is_u64());
    let third = get_task(&w, get_task(&w, next.id.raw()).next.unwrap().raw());
    assert_eq!(third.due.unwrap().to_string(), "2026-10-28");
    w.ok(pm, "task.close", json!({"project": p, "id": third.id.raw()}));
    assert_eq!(get_task(&w, third.id.raw()).next, None);
    assert_eq!(w.engine.state().tasks.iter().count(), 3);

    // Closed late: the next one is the first due date still ahead (today is 12 Oct).
    let late =
        w.ok(pm, "task.create", json!({"project": p, "title": "Weekly report", "due": "2026-09-01", "repeat": "weekly"}))["id"].as_u64().unwrap();
    w.ok(pm, "task.close", json!({"project": p, "id": late}));
    assert_eq!(get_task(&w, get_task(&w, late).next.unwrap().raw()).due.unwrap().to_string(), "2026-10-13");
    // Monthly keeps the day, or the last day of a shorter month.
    let m =
        w.ok(pm, "task.create", json!({"project": p, "title": "Pay application", "due": "2026-10-31", "repeat": "monthly"}))["id"].as_u64().unwrap();
    w.ok(pm, "task.close", json!({"project": p, "id": m}));
    assert_eq!(get_task(&w, get_task(&w, m).next.unwrap().raw()).due.unwrap().to_string(), "2026-11-30");
    // Subtasks don't come along.
    let parent =
        w.ok(pm, "task.create", json!({"project": p, "title": "Monthly close", "due": "2026-10-20", "repeat": "monthly"}))["id"].as_u64().unwrap();
    let kid = w.ok(pm, "task.create", json!({"project": p, "title": "Reconcile", "parent": parent}))["id"].as_u64().unwrap();
    w.ok(pm, "task.close", json!({"project": p, "id": kid}));
    w.ok(pm, "task.close", json!({"project": p, "id": parent}));
    let again = get_task(&w, get_task(&w, parent).next.unwrap().raw());
    assert_eq!(w.engine.state().tasks.iter().filter(|x| x.parent == Some(again.id)).count(), 0);
    w.assert_replay();
}

#[test]
fn tasks_link_to_the_rfi_or_meeting_they_came_from() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let other = w.project("Harbor Office");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let template = buildkiosk_engine::cmd::company::template_named(w.engine.state(), "Project manager").unwrap();
    w.ok(w.admin(), "member.add", json!({"project": other, "person": pm, "template": template}));
    let rfi = w.ok(pm, "rfi.create", json!({"project": p, "subject": "Footing depth", "question": "Which governs?"}))["id"].as_u64().unwrap();
    let foreign_rfi = w.ok(pm, "rfi.create", json!({"project": other, "subject": "Door swing", "question": "Which way?"}))["id"].as_u64().unwrap();
    let meeting = w.ok(pm, "meeting.create", json!({"project": p, "title": "OAC", "date": "2026-10-20"}))["id"].as_u64().unwrap();
    let t = w.ok(pm, "task.create", json!({"project": p, "title": "Confirm with engineer", "rfi": rfi, "meeting": meeting}))["id"].as_u64().unwrap();
    assert_eq!((get_task(&w, t).rfi.map(|x| x.raw()), get_task(&w, t).meeting.map(|x| x.raw())), (Some(rfi), Some(meeting)));
    w.err(pm, "task.create", json!({"project": p, "title": "x", "rfi": foreign_rfi}));
    w.err(pm, "task.create", json!({"project": p, "title": "x", "meeting": 99}));
    w.ok(pm, "task.update", json!({"project": p, "id": t, "rfi": "", "meeting": ""}));
    assert_eq!((get_task(&w, t).rfi, get_task(&w, t).meeting), (None, None));
    w.assert_replay();
}

#[test]
fn my_tasks_come_from_every_project_the_person_can_read() {
    let mut w = World::new();
    let a = w.project("Riverside Clinic");
    let b = w.project("Harbor Office");
    let c = w.project("Private Annex");
    let pm = w.member(a, "Paula", "Manager", "Project manager");
    let template = buildkiosk_engine::cmd::company::template_named(w.engine.state(), "Project manager").unwrap();
    w.ok(w.admin(), "member.add", json!({"project": b, "person": pm, "template": template}));
    let boss = w.member(c, "Bo", "Boss", "Project manager");
    let titles = |w: &mut World, who, args| -> Vec<String> {
        w.ok(who, "task.mine", args).as_array().unwrap().iter().map(|t| t["title"].as_str().unwrap().to_string()).collect()
    };
    w.ok(pm, "task.create", json!({"project": a, "title": "Later", "assignees": [pm], "due": "2026-11-01"}));
    w.ok(pm, "task.create", json!({"project": b, "title": "Sooner", "assignees": [pm], "due": "2026-10-15"}));
    w.ok(pm, "task.create", json!({"project": b, "title": "No date", "assignees": [pm]}));
    w.ok(pm, "task.create", json!({"project": a, "title": "Someone else's", "manager": null, "assignees": []}));
    let done = w.ok(pm, "task.create", json!({"project": a, "title": "Done", "assignees": [pm], "due": "2026-10-01"}))["id"].as_u64().unwrap();
    w.ok(pm, "task.close", json!({"project": a, "id": done}));
    // A task in a project the person isn't on is not theirs to see, even when assigned.
    w.ok(boss, "task.create", json!({"project": c, "title": "Annex", "assignees": [pm]}));

    // Soonest first, undated last; closed ones hidden; the project and overdue flag come with each.
    assert_eq!(titles(&mut w, pm, json!({})), ["Sooner", "Later", "No date"]);
    assert_eq!(titles(&mut w, pm, json!({"include_closed": true})), ["Done", "Sooner", "Later", "No date"]);
    let all = titles(&mut w, pm, json!({"all": true}));
    assert!(all.contains(&"Someone else's".to_string()) && !all.contains(&"Annex".to_string()), "{all:?}");
    let first = w.ok(pm, "task.mine", json!({}))[0].clone();
    assert_eq!((first["project_name"].as_str().unwrap(), first["overdue"].as_bool().unwrap()), ("Harbor Office", false));
    assert_eq!(titles(&mut w, boss, json!({})), ["Annex"], "Bo manages it, so it is his too");
    w.err(buildkiosk_engine::Actor::System, "task.mine", json!({}));
}
