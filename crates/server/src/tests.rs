use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::*;

fn temp(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("buildkiosk-server-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

async fn send(app: &Shared, req: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let res = router(app.clone()).oneshot(req).await.unwrap();
    let (parts, body) = res.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status, parts.headers, String::from_utf8_lossy(&bytes).into_owned())
}

fn post(uri: &str, cookie: &str, body: String) -> Request<Body> {
    Request::post(uri).header("cookie", cookie).header("content-type", "application/x-www-form-urlencoded").body(Body::from(body)).unwrap()
}

fn cookies(h: &HeaderMap) -> String {
    h.get_all(SET_COOKIE).iter().filter_map(|v| v.to_str().ok()).filter_map(|v| v.split(';').next()).collect::<Vec<_>>().join("; ")
}

fn field(html: &str, name: &str) -> String {
    let at = html.find(&format!("name=\"{name}\" value=\"")).unwrap() + name.len() + 15;
    html[at..].split('"').next().unwrap().to_string()
}

#[tokio::test]
async fn setup_login_and_forms() {
    let dir = temp("flow");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());

    // First run redirects to setup.
    let (st, ..) = send(&app, Request::get("/").body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let token = field(&page, "_csrf");
    // A form without the right token is refused.
    let (st, ..) = send(&app, post("/setup", &anon, "_csrf=bad&name=Acme".into())).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let body = format!("_csrf={token}&name=Acme+Builders&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery");
    let (st, h, _) = send(&app, post("/setup", &anon, body)).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    let session = cookies(&h);
    assert!(session.starts_with("bk_session="));

    // Signed in: the home page and a command through the generic form handler.
    let (st, _, home) = send(&app, Request::get("/").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    let csrf = field(&home, "_csrf");
    let (st, h, _) = send(&app, post("/do/project.create", &session, format!("_csrf={csrf}&_next=/&name=Tower+One"))).await;
    assert_eq!((st, h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/?ok=")), (StatusCode::SEE_OTHER, true));
    let (_, _, home) = send(&app, Request::get("/").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(home.contains("Tower One"));
    // A command without the token, and a bad command, don't run.
    let (st, ..) = send(&app, post("/do/project.create", &session, "_csrf=nope&name=Evil".into())).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (_, h, _) = send(&app, post("/do/project.create", &session, format!("_csrf={csrf}&_next=/&name="))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/?error="));

    // The API needs a token.
    let (st, ..) = send(&app, Request::post("/api/v1/commands/project.list").body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn form_fields_become_parameters() {
    let form = parse_form(b"_csrf=x&subject=A+b%26c&assignees%5B%5D=&assignees%5B%5D=3&assignees%5B%5D=4&private=false&private=true");
    let p = form_params(&form);
    assert_eq!(p, json!({"subject": "A b&c", "assignees": ["3", "4"], "private": "true"}));
    assert_eq!(local_path("//evil.example"), "/");
    assert_eq!(local_path("/projects/1"), "/projects/1");
}

#[tokio::test]
async fn rfi_files_history_and_csv() {
    let dir = temp("files");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let session = cookies(&h);
    let (_, _, home) = send(&app, Request::get("/").header("cookie", &session).body(Body::empty()).unwrap()).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &session, format!("_csrf={csrf}&name=P"))).await;
    send(&app, post("/do/rfi.create", &session, format!("_csrf={csrf}&project=1&subject=%3Dcmd+Door&question=Which&assignees%5B%5D=1&open=true")))
        .await;

    let upload = |token: &str, content: &str| {
        let body = format!(
            "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{token}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a b.txt\"\r\nContent-Type: text/html\r\n\r\n{content}\r\n--B--\r\n"
        );
        Request::post("/projects/1/rfis/1/upload")
            .header("cookie", &session)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (st, h, _) = send(&app, upload("bad", "x")).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{h:?}");
    let (_, h, _) = send(&app, upload(&csrf, "<b>hello</b>")).await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(loc.contains("ok="), "{loc}");

    let (_, _, detail) = send(&app, Request::get("/projects/1/rfis/1").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(detail.contains("a b.txt") && detail.contains("Attach file to RFI by Ada Admin"));
    let href = detail.split("href=\"").find(|s| s.contains("/files/")).unwrap().split('"').next().unwrap().to_string();
    let (st, h, body) = send(&app, Request::get(&href).header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body, "<b>hello</b>");
    assert_eq!(h.get("content-type").unwrap(), "application/octet-stream");
    assert!(h.get("content-disposition").unwrap().to_str().unwrap().starts_with("attachment; filename=\"a_b.txt\""));
    let (st, ..) = send(&app, Request::get(&href).body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::SEE_OTHER, "signed-out people can't download");

    let (st, h, csv) = send(&app, Request::get("/projects/1/rfis.csv").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.get("content-disposition").is_some());
    assert!(csv.contains("'=cmd Door"), "formulas are defused: {csv}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn admin_adds_a_person_and_sets_their_password() {
    let dir = temp("people");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let session = cookies(&h);
    let (st, _, dirpage) = send(&app, Request::get("/directory").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    let csrf = field(&dirpage, "_csrf");
    send(
        &app,
        post(
            "/do/person.create",
            &session,
            format!("_csrf={csrf}&_next=/directory&first_name=Bo&last_name=Builder&email=bo%40example.com&is_user=true"),
        ),
    )
    .await;
    let (_, _, dirpage) = send(&app, Request::get("/directory").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(dirpage.contains("bo@example.com") && dirpage.contains("/people/2/password"));
    let (_, h, _) = send(&app, post("/people/2/password", &session, format!("_csrf={csrf}&password=short"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "weak passwords are refused");
    let (_, h, _) = send(&app, post("/people/2/password", &session, format!("_csrf={csrf}&password=bo-secret-password"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));

    // Bo can sign in, but is no admin and no directory user.
    let (_, h, page) = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let login = format!("_csrf={}&email=bo%40example.com&password=bo-secret-password", field(&page, "_csrf"));
    let (st, h, _) = send(&app, post("/login", &anon, login)).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    let bo = cookies(&h);
    let (_, _, home) = send(&app, Request::get("/").header("cookie", &bo).body(Body::empty()).unwrap()).await;
    let bo_csrf = field(&home, "_csrf");
    let (_, h, _) = send(&app, post("/people/1/password", &bo, format!("_csrf={bo_csrf}&password=hijack-password-1"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "non-admins can't set passwords");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn people_can_be_edited_and_deactivated() {
    let dir = temp("edit");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let session = cookies(&h);
    let (_, _, d) = send(&app, Request::get("/directory").header("cookie", &session).body(Body::empty()).unwrap()).await;
    let csrf = field(&d, "_csrf");
    send(&app, post("/do/person.create", &session, format!("_csrf={csrf}&first_name=Bo&last_name=Builder"))).await;
    let (st, _, edit) = send(&app, Request::get("/directory/people/2").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    assert!(edit.contains("Bo"));
    let (_, h, _) = send(&app, post("/do/person.update", &session, format!("_csrf={csrf}&_next=/directory&id=2&title=Foreman&active=false"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let (_, _, d) = send(&app, Request::get("/directory").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(d.contains("Inactive") && d.contains("Foreman"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn team_permissions_can_be_changed() {
    let dir = temp("team");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let session = cookies(&h);
    let (_, _, d) = send(&app, Request::get("/directory").header("cookie", &session).body(Body::empty()).unwrap()).await;
    let csrf = field(&d, "_csrf");
    send(&app, post("/do/project.create", &session, format!("_csrf={csrf}&name=P"))).await;
    send(&app, post("/do/person.create", &session, format!("_csrf={csrf}&first_name=Bo&last_name=Builder&email=bo%40example.com&is_user=true")))
        .await;
    send(&app, post("/do/member.add", &session, format!("_csrf={csrf}&project=1&person=2&role=Foreman"))).await;
    let (_, h, _) = send(
        &app,
        post("/do/member.update", &session, format!("_csrf={csrf}&_next=/projects/1/team&project=1&person=2&role=Superintendent&project_admin=true")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let (_, _, team) = send(&app, Request::get("/projects/1/team").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(team.contains("Superintendent") && team.contains("Project admin"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn submittal_workflow_and_files_in_the_browser_flow() {
    let dir = temp("submittals");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let session = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &session).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &session, format!("_csrf={csrf}&name=P"))).await;
    let (st, _, new) = send(&app, get("/projects/1/submittals/new")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(new.contains("Save and send for review"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/submittal.create",
            &session,
            format!("_csrf={csrf}&_next=/projects/1/submittals/%7Bid%7D&project=1&title=Roof+drawings&reviewers%5B%5D=1&open=true"),
        ),
    )
    .await;
    assert_eq!(h.get(LOCATION).unwrap().to_str().unwrap().split('?').next().unwrap(), "/projects/1/submittals/1");
    let (_, _, detail) = send(&app, get("/projects/1/submittals/1")).await;
    assert!(detail.contains("Roof drawings") && detail.contains("In review") && detail.contains("Submit review"));
    send(&app, post("/do/submittal.review", &session, format!("_csrf={csrf}&project=1&id=1&decision=approved_as_noted&comment=See+markup"))).await;
    send(&app, post("/do/submittal.close", &session, format!("_csrf={csrf}&project=1&id=1&decision=approved_as_noted"))).await;
    let (_, _, list) = send(&app, get("/projects/1/submittals?status=closed")).await;
    assert!(list.contains("Roof drawings") && list.contains("Approved as noted"));

    let upload = format!(
        "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"r.pdf\"\r\n\r\nPDF\r\n--B--\r\n"
    );
    // Closed submittals take no new files, but an admin may still resubmit and then attach.
    send(&app, post("/do/submittal.resubmit", &session, format!("_csrf={csrf}&project=1&id=1"))).await;
    let req = Request::post("/projects/1/submittals/1/upload")
        .header("cookie", &session)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(upload))
        .unwrap();
    let (_, h, _) = send(&app, req).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let (_, _, detail) = send(&app, get("/projects/1/submittals/1")).await;
    assert!(detail.contains("r.pdf") && detail.contains("revision 1"));
    let href = detail.split("href=\"").find(|s| s.contains("/files/")).unwrap().split('"').next().unwrap().to_string();
    let (st, _, body) = send(&app, get(&href)).await;
    assert_eq!((st, body.as_str()), (StatusCode::OK, "PDF"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn register_specs_notifications_and_review_files() {
    let dir = temp("round2");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str, who: &str| Request::get(uri.to_string()).header("cookie", who.to_string()).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/", &ada)).await;
    let csrf = field(&home, "_csrf");
    let run = |cmd: &str, body: String| post(&format!("/do/{cmd}"), &ada, format!("_csrf={csrf}&{body}"));
    send(&app, run("project.create", "name=P".into())).await;
    send(&app, run("person.create", "first_name=Bo&last_name=Builder&email=bo%40example.com&is_user=true".into())).await;
    send(&app, run("member.add", "project=1&person=2&template=5".into())).await;
    send(&app, post("/people/2/password", &ada, format!("_csrf={csrf}&password=bo-secret-password"))).await;

    // Specifications and the register import.
    send(&app, run("spec.import", "project=1&text=05+12+00%2CStructural+steel%0A08+71+00%2CDoor+hardware".into())).await;
    send(&app, run("submittal.import", "project=1&text=title%2Cspec_section%0AAnchor+bolts%2C05+12+00".into())).await;
    let (_, _, specs) = send(&app, get("/projects/1/specifications", &ada)).await;
    assert!(specs.contains("Structural steel") && specs.contains("/projects/1/submittals/1"), "the section lists the submittal filed under it");
    let (_, h, csv) = send(&app, get("/projects/1/submittals.csv", &ada)).await;
    assert!(csv.contains("Anchor bolts") && h.get("content-disposition").is_some());

    // Ordered review with Bo: Bo is notified and can upload a file with the review.
    send(&app, run("submittal.update", "project=1&id=1&reviewers%5B%5D=2&ordered=true".into())).await;
    send(&app, run("submittal.open", "project=1&id=1".into())).await;
    let (_, h, page) = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let login = format!("_csrf={}&email=bo%40example.com&password=bo-secret-password", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/login", &anon, login)).await;
    let bo = cookies(&h);
    let (_, _, inbox) = send(&app, get("/notifications", &bo)).await;
    assert!(inbox.contains("waiting for your review") && inbox.contains("Notifications, 1 unread"));
    let (_, _, detail) = send(&app, get("/projects/1/submittals/1", &bo)).await;
    let bo_csrf = field(&detail, "_csrf");
    let upload = format!(
        "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{bo_csrf}\r\n--B\r\nContent-Disposition: form-data; name=\"decision\"\r\n\r\napproved_as_noted\r\n--B\r\nContent-Disposition: form-data; name=\"comment\"\r\n\r\nSee markup\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"markup.pdf\"\r\n\r\nMARKUP\r\n--B--\r\n"
    );
    let req = Request::post("/projects/1/submittals/1/review")
        .header("cookie", &bo)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(upload))
        .unwrap();
    let (_, h, _) = send(&app, req).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let (_, _, detail) = send(&app, get("/projects/1/submittals/1", &ada)).await;
    assert!(detail.contains("markup.pdf") && detail.contains("Approved as noted"));
    let href = detail.split("href=\"").find(|s| s.contains("/files/")).unwrap().split('"').next().unwrap().to_string();
    let (st, _, body) = send(&app, get(&href, &ada)).await;
    assert_eq!((st, body.as_str()), (StatusCode::OK, "MARKUP"));
    let (_, _, inbox) = send(&app, get("/notifications", &ada)).await;
    assert!(inbox.contains("All reviewers have answered"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn daily_log_form_saves_lists_and_submits() {
    let dir = temp("dailylog");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;

    let (st, _, form) = send(&app, get("/projects/1/daily-logs/new")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(form.contains("manpower.0.company") && form.contains("Save and submit"));
    let fields = "date=2026-10-12&sky=rain&high=54&low=41&work_performed=Poured+footings\
        &manpower.0.company=Northline&manpower.0.trade=Electrical&manpower.0.workers=4&manpower.0.hours=7.5\
        &manpower.1.company=&manpower.1.workers=&manpower.1.hours=\
        &manpower.2.company=Keystone&manpower.2.workers=6&manpower.2.hours=8\
        &equipment.0.name=Excavator&equipment.0.hours=5.5&deliveries.0.description=Rebar&visitors.0.name=Inspector";
    let (_, h, _) = send(&app, post("/projects/1/daily-logs/save", &ada, format!("_csrf={csrf}&{fields}&_submit=1"))).await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(loc.starts_with("/projects/1/daily-logs/1?ok="), "{loc}");
    let (_, _, detail) = send(&app, get("/projects/1/daily-logs/1")).await;
    for want in ["Northline", "Keystone", "Excavator", "Rebar", "Inspector", "Submitted", "10", "78"] {
        assert!(detail.contains(want), "{want} missing");
    }
    // A second log for the same day is refused with a message, and a bad hours value too.
    let (_, h, _) = send(&app, post("/projects/1/daily-logs/save", &ada, format!("_csrf={csrf}&date=2026-10-12"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/daily-logs/new?error="));
    let (_, h, _) = send(
        &app,
        post(
            "/projects/1/daily-logs/save",
            &ada,
            format!("_csrf={csrf}&date=2026-10-13&manpower.0.company=A&manpower.0.workers=2&manpower.0.hours=99"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="));
    let (_, _, list) = send(&app, get("/projects/1/daily-logs?month=2026-10")).await;
    assert!(list.contains("Rain, 54/41"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn punch_list_in_the_browser_flow() {
    let dir = temp("punch");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;
    let (st, _, form) = send(&app, get("/projects/1/punch-list/new")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(form.contains("Add punch item"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/punch.create",
            &ada,
            format!(
                "_csrf={csrf}&_next=/projects/1/punch-list/%7Bid%7D&project=1&title=Touch+up+paint&location=Corridor&priority=high&assignees%5B%5D=1"
            ),
        ),
    )
    .await;
    assert_eq!(h.get(LOCATION).unwrap().to_str().unwrap().split('?').next().unwrap(), "/projects/1/punch-list/1");
    send(&app, post("/do/punch.complete", &ada, format!("_csrf={csrf}&project=1&id=1&comment=Done"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/punch-list/1")).await;
    assert!(detail.contains("Ready for review") && detail.contains("Not right yet?") && detail.contains("Done"));
    send(&app, post("/do/punch.close", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let upload = format!(
        "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"p.jpg\"\r\n\r\nJPG\r\n--B--\r\n"
    );
    // Reopen, then attach and download.
    send(&app, post("/do/punch.reopen", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let req = Request::post("/projects/1/punch-list/1/upload")
        .header("cookie", &ada)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(upload))
        .unwrap();
    send(&app, req).await;
    let (_, _, detail) = send(&app, get("/projects/1/punch-list/1")).await;
    let href = detail.split("href=\"").find(|s| s.contains("/files/")).unwrap().split('"').next().unwrap().to_string();
    let (st, _, body) = send(&app, get(&href)).await;
    assert_eq!((st, body.as_str()), (StatusCode::OK, "JPG"));
    let (_, _, csv) = send(&app, get("/projects/1/punch-list.csv")).await;
    assert!(csv.contains("Touch up paint"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn observation_list_in_the_browser_flow() {
    let dir = temp("observation");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;
    let (st, _, form) = send(&app, get("/projects/1/observations/new")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(form.contains("Add observation"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/observation.create",
            &ada,
            format!(
                "_csrf={csrf}&_next=/projects/1/observations/%7Bid%7D&project=1&title=Touch+up+paint&location=Corridor&priority=high&assignees%5B%5D=1"
            ),
        ),
    )
    .await;
    assert_eq!(h.get(LOCATION).unwrap().to_str().unwrap().split('?').next().unwrap(), "/projects/1/observations/1");
    send(&app, post("/do/observation.complete", &ada, format!("_csrf={csrf}&project=1&id=1&comment=Done"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/observations/1")).await;
    assert!(detail.contains("Ready for review") && detail.contains("Not right yet?") && detail.contains("Done"));
    send(&app, post("/do/observation.close", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let upload = format!(
        "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"p.jpg\"\r\n\r\nJPG\r\n--B--\r\n"
    );
    // Reopen, then attach and download.
    send(&app, post("/do/observation.reopen", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let req = Request::post("/projects/1/observations/1/upload")
        .header("cookie", &ada)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(upload))
        .unwrap();
    send(&app, req).await;
    let (_, _, detail) = send(&app, get("/projects/1/observations/1")).await;
    let href = detail.split("href=\"").find(|s| s.contains("/files/")).unwrap().split('"').next().unwrap().to_string();
    let (st, _, body) = send(&app, get(&href)).await;
    assert_eq!((st, body.as_str()), (StatusCode::OK, "JPG"));
    let (_, _, csv) = send(&app, get("/projects/1/observations.csv")).await;
    assert!(csv.contains("Touch up paint"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn inspection_checklist_in_the_browser_flow() {
    let dir = temp("inspections");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;

    let (st, _, templates) = send(&app, get("/projects/1/inspections/templates")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(templates.contains("Add the starter templates"));
    send(&app, post("/do/inspection_template.install_starters", &ada, format!("_csrf={csrf}&project=1"))).await;
    let (_, _, templates) = send(&app, get("/projects/1/inspections/templates")).await;
    assert!(templates.contains("Pre-pour concrete check") && templates.contains("Final walk-through"));

    let (_, _, form) = send(&app, get("/projects/1/inspections/new")).await;
    assert!(form.contains("Framing walk-through"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/inspection.create",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/inspections/%7Bid%7D&project=1&template=2&location=Level+2&inspector=1"),
        ),
    )
    .await;
    assert_eq!(h.get(LOCATION).unwrap().to_str().unwrap().split('?').next().unwrap(), "/projects/1/inspections/1");
    let (_, _, detail) = send(&app, get("/projects/1/inspections/1")).await;
    assert!(detail.contains("line.0.answer") && detail.contains("Save and complete") && detail.contains("0 of 7 answered"));

    // Completing with unanswered lines is refused; answering everything works.
    let some = "line.0.answer=pass&line.1.answer=fail&line.1.note=Wrong+size&line.2.answer=";
    let (_, h, _) = send(&app, post("/projects/1/inspections/1/answers", &ada, format!("_csrf={csrf}&{some}&_complete=1"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "unanswered lines stop completion");
    let all: String = (2..7).map(|n| format!("&line.{n}.answer=pass")).collect();
    let (_, h, _) = send(
        &app,
        post(
            "/projects/1/inspections/1/answers",
            &ada,
            format!("_csrf={csrf}&line.0.answer=pass&line.1.answer=fail&line.1.note=Wrong+size{all}&_complete=1"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let (_, _, detail) = send(&app, get("/projects/1/inspections/1")).await;
    assert!(detail.contains("1 failed") && detail.contains("Wrong size") && detail.contains("Raise punch items"));
    send(&app, post("/do/inspection.raise_punch", &ada, format!("_csrf={csrf}&project=1&id=1&assignees%5B%5D=1"))).await;
    let (_, _, punch) = send(&app, get("/projects/1/punch-list")).await;
    assert!(punch.contains("Inspection #1"));
    let (_, _, list) = send(&app, get("/projects/1/inspections")).await;
    assert!(list.contains("Framing walk-through") && list.contains("7 of 7"));
    let (_, _, csv) = send(&app, get("/projects/1/inspections.csv")).await;
    assert!(csv.contains("Framing walk-through"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn budget_commitments_and_direct_costs_in_the_browser_flow() {
    let dir = temp("finance");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;

    // Budget: the starter cost codes are selectable; a line and an import.
    let (st, _, budget) = send(&app, get("/projects/1/budget")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(budget.contains("Set a budget line") && budget.contains("03-300"));
    let code = |text: &str| -> String {
        let i = budget.find(&format!(">{text} ")).unwrap();
        let before = &budget[..i];
        let v = before.rfind("value=\"").unwrap() + 7;
        before[v..].split('"').next().unwrap().to_string()
    };
    let concrete = code("03-300");
    let (_, h, _) = send(
        &app,
        post(
            "/do/budget.set_line",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/budget&project=1&cost_code={concrete}&original=120,000.00&notes=Slab"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    send(&app, post("/do/budget.import", &ada, format!("_csrf={csrf}&_next=/projects/1/budget&project=1&text=05-100%2C80000"))).await;

    // A commitment from the form, approved, and a direct cost.
    let (_, _, form) = send(&app, get("/projects/1/commitments/new")).await;
    assert!(form.contains("lines.0.cost_code") && form.contains("Schedule of values"));
    let (_, h, _) = send(
        &app,
        post("/projects/1/commitments/save", &ada, format!("_csrf={csrf}&title=Footings&kind=subcontract&vendor=&description=&lines.0.cost_code={concrete}&lines.0.description=Footings&lines.0.amount=30000.50&lines.1.cost_code=&lines.1.amount=")),
    )
    .await;
    assert_eq!(h.get(LOCATION).unwrap().to_str().unwrap().split('?').next().unwrap(), "/projects/1/commitments/1");
    let (_, _, detail) = send(&app, get("/projects/1/commitments/1")).await;
    assert!(detail.contains("30,000.50") && detail.contains("Approve") && detail.contains("Draft"));
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    assert!(!budget.contains("30,000.50"), "a draft is not committed");
    send(&app, post("/do/commitment.approve", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    send(
        &app,
        post(
            "/do/direct_cost.create",
            &ada,
            format!(
                "_csrf={csrf}&_next=/projects/1/direct-costs&project=1&cost_code={concrete}&amount=1250.25&description=Pump+truck&kind=equipment"
            ),
        ),
    )
    .await;
    let (_, _, costs) = send(&app, get("/projects/1/direct-costs")).await;
    assert!(costs.contains("Pump truck") && costs.contains("1,250.25"));
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    for want in ["30,000.50", "1,250.25", "88,749.25", "200,000.00"] {
        assert!(budget.contains(want), "{want} missing from the budget");
    }
    let (_, h, csv) = send(&app, get("/projects/1/budget.csv")).await;
    assert!(csv.starts_with("Cost code,") && csv.contains("03-300") && h.get("content-disposition").is_some());
    let (_, _, list) = send(&app, get("/projects/1/commitments?status=approved")).await;
    assert!(list.contains("Footings") && list.contains("30,000.50"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn change_events_change_orders_and_budget_changes_in_the_browser_flow() {
    let dir = temp("changes");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    let code = |text: &str| -> String {
        let i = budget.find(&format!(">{text} ")).unwrap();
        let before = &budget[..i];
        let v = before.rfind("value=\"").unwrap() + 7;
        before[v..].split('"').next().unwrap().to_string()
    };
    let (concrete, steel) = (code("03-300"), code("05-100"));
    send(&app, post("/do/budget.set_line", &ada, format!("_csrf={csrf}&project=1&cost_code={concrete}&original=100000"))).await;
    send(&app, post("/do/budget.set_line", &ada, format!("_csrf={csrf}&project=1&cost_code={steel}&original=50000"))).await;
    send(
        &app,
        post(
            "/projects/1/commitments/save",
            &ada,
            format!("_csrf={csrf}&title=Slab&kind=subcontract&vendor=&description=&lines.0.cost_code={concrete}&lines.0.amount=60000"),
        ),
    )
    .await;
    send(&app, post("/do/commitment.approve", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;

    // A change event shows as pending on the budget until it is closed.
    let (st, _, form) = send(&app, get("/projects/1/change-events/new")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(form.contains("name=\"reason\"") && form.contains("lines.0.cost_code"));
    let (_, h, _) = send(
        &app,
        post(
            "/projects/1/change-events/save",
            &ada,
            format!("_csrf={csrf}&title=Thicker+slab&description=&reason=Field+condition&lines.0.cost_code={concrete}&lines.0.amount=7500"),
        ),
    )
    .await;
    assert_eq!(h.get(LOCATION).unwrap().to_str().unwrap().split('?').next().unwrap(), "/projects/1/change-events/1");
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    assert!(budget.contains("Pending change events") && budget.contains("7,500.00"));

    // A commitment change order, approved, adds to the committed cost and shows on the commitment.
    let (_, _, form) = send(&app, get("/projects/1/change-orders/new")).await;
    assert!(form.contains("Slab"), "approved commitments can be picked");
    let (_, h, _) = send(
        &app,
        post(
            "/projects/1/change-orders/save",
            &ada,
            format!("_csrf={csrf}&title=Extra+thickness&description=&commitment=1&change_event=1&lines.0.cost_code={concrete}&lines.0.amount=7500"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/change-orders/1?ok="));
    send(&app, post("/do/commitment_change.approve", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, commitment) = send(&app, get("/projects/1/commitments/1")).await;
    assert!(commitment.contains("Extra thickness") && commitment.contains("67,500.00"));

    // A budget change moves 10,000 from steel to concrete.
    send(&app, post("/projects/1/budget-changes/save", &ada, format!("_csrf={csrf}&title=Move+to+slab&description=&change_event=&lines.0.cost_code={steel}&lines.0.amount=-10000&lines.1.cost_code={concrete}&lines.1.amount=10000"))).await;
    send(&app, post("/do/budget_change.approve", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    for want in ["110,000.00", "40,000.00", "67,500.00", "42,500.00"] {
        assert!(budget.contains(want), "{want} missing from the budget");
    }
    for path in ["change-events", "change-orders", "budget-changes"] {
        let (st, _, list) = send(&app, get(&format!("/projects/1/{path}"))).await;
        assert_eq!(st, StatusCode::OK);
        assert!(list.contains("<table"), "{path} lists its record");
    }
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn prime_contract_and_owner_invoicing_in_the_browser_flow() {
    let dir = temp("invoicing");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    let code = |text: &str| -> String {
        let i = budget.find(&format!(">{text} ")).unwrap();
        let before = &budget[..i];
        let v = before.rfind("value=\"").unwrap() + 7;
        before[v..].split('"').next().unwrap().to_string()
    };
    let (concrete, steel) = (code("03-300"), code("05-100"));

    // The contract through the generic form, then approved.
    let (st, _, form) = send(&app, get("/projects/1/prime-contracts/new")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(form.contains("name=\"retainage\"") && form.contains("name=\"owner\""));
    let (_, h, _) = send(
        &app,
        post(
            "/projects/1/prime-contracts/save",
            &ada,
            format!("_csrf={csrf}&title=Clinic+contract&description=&owner=&retainage=10&lines.0.cost_code={concrete}&lines.0.description=Concrete&lines.0.amount=100000&lines.1.cost_code={steel}&lines.1.description=Steel&lines.1.amount=50000"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/prime-contracts/1?ok="));
    send(&app, post("/do/prime_contract.approve", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;

    // An invoice: start it, type amounts, submit, approve, pay.
    let (_, _, list) = send(&app, get("/projects/1/owner-invoices")).await;
    assert!(list.contains("Start an invoice") && list.contains("Clinic contract"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/owner_invoice.create",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/owner-invoices/%7Bid%7D&project=1&contract=1&period_end=2026-10-31"),
        ),
    )
    .await;
    assert_eq!(h.get(LOCATION).unwrap().to_str().unwrap().split('?').next().unwrap(), "/projects/1/owner-invoices/1");
    let (_, _, detail) = send(&app, get("/projects/1/owner-invoices/1")).await;
    assert!(detail.contains("amount.c.0") && detail.contains("Save amounts"));
    let (_, h, _) = send(
        &app,
        post("/projects/1/owner-invoices/1/save", &ada, format!("_csrf={csrf}&period_end=2026-10-31&notes=October&amount.c.0=40000&amount.c.1=")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let (_, h, _) =
        send(&app, post("/projects/1/owner-invoices/1/save", &ada, format!("_csrf={csrf}&period_end=2026-10-31&notes=&amount.c.0=100000.01"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "billing beyond the scheduled value is refused");
    let (_, _, detail) = send(&app, get("/projects/1/owner-invoices/1")).await;
    for want in ["40,000.00", "4,000.00", "36,000.00", "Submit"] {
        assert!(detail.contains(want), "{want} missing from the invoice");
    }
    for step in ["submit", "approve", "pay"] {
        send(&app, post(&format!("/do/owner_invoice.{step}"), &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    }
    let (_, _, contract) = send(&app, get("/projects/1/prime-contracts/1")).await;
    for want in ["150,000.00", "Billed to date", "40,000.00", "Retainage held", "Paid (net)", "36,000.00", "110,000.00"] {
        assert!(contract.contains(want), "{want} missing from the contract");
    }
    let (_, _, detail) = send(&app, get("/projects/1/owner-invoices/1")).await;
    assert!(detail.contains("Paid"));
    let (st, _, print) = send(&app, get("/projects/1/owner-invoices/1/print")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        print.contains("Amount due this invoice")
            && print.contains("36,000.00")
            && print.contains("Approved by")
            && !print.contains("class=\"sidebar\"")
    );
    let (st, h, csv) = send(&app, get("/projects/1/owner-invoices/1/csv")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.get("content-disposition").is_some() && csv.contains("Amount due this invoice,,,,,,36000.00"), "{csv}");
    let (st, _, cos) = send(&app, get("/projects/1/prime-change-orders")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(cos.contains("Prime change orders"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn commitment_invoices_in_the_browser_flow() {
    let dir = temp("cinvoices");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    let i = budget.find(">03-300 ").unwrap();
    let v = budget[..i].rfind("value=\"").unwrap() + 7;
    let concrete = budget[v..].split('"').next().unwrap().to_string();

    let (_, h, _) = send(
        &app,
        post("/projects/1/commitments/save", &ada, format!("_csrf={csrf}&title=Slab&kind=subcontract&vendor=&retainage=5&description=&lines.0.cost_code={concrete}&lines.0.description=Footings&lines.0.amount=40000")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let (_, _, list) = send(&app, get("/projects/1/commitment-invoices")).await;
    assert!(list.contains("Nothing is approved to invoice yet"), "a draft commitment can't be invoiced");
    send(&app, post("/do/commitment.approve", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, list) = send(&app, get("/projects/1/commitment-invoices")).await;
    assert!(list.contains("Start an invoice") && list.contains("Slab"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/commitment_invoice.create",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/commitment-invoices/%7Bid%7D&project=1&commitment=1&period_end=2026-10-31"),
        ),
    )
    .await;
    assert_eq!(h.get(LOCATION).unwrap().to_str().unwrap().split('?').next().unwrap(), "/projects/1/commitment-invoices/1");
    send(&app, post("/projects/1/commitment-invoices/1/save", &ada, format!("_csrf={csrf}&period_end=2026-10-31&notes=&amount.c.0=20000"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/commitment-invoices/1")).await;
    for want in ["20,000.00", "1,000.00", "19,000.00", "Submit"] {
        assert!(detail.contains(want), "{want} missing");
    }
    for step in ["submit", "approve", "pay"] {
        send(&app, post(&format!("/do/commitment_invoice.{step}"), &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    }
    let (_, _, commitment) = send(&app, get("/projects/1/commitments/1")).await;
    for want in ["Billing", "Billed to date", "Retainage held", "19,000.00", "20,000.00"] {
        assert!(commitment.contains(want), "{want} missing from the commitment");
    }
    let (_, _, owner) = send(&app, get("/projects/1/owner-invoices")).await;
    assert!(owner.contains("Nothing is approved to invoice yet"), "owner and commitment invoices stay separate");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn secure_mode_marks_cookies_and_sends_hsts() {
    for secure in [false, true] {
        let dir = temp(if secure { "secure" } else { "plain" });
        let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap().with_secure_cookies(secure));
        let (_, h, _) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
        let set: Vec<&str> = h.get_all(SET_COOKIE).iter().filter_map(|v| v.to_str().ok()).collect();
        assert!(!set.is_empty());
        assert!(set.iter().all(|c| c.contains("; Secure") == secure), "{set:?}");
        assert_eq!(h.contains_key("strict-transport-security"), secure);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[tokio::test]
async fn documents_upload_versions_and_download() {
    let dir = temp("documents");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let session = cookies(&h);
    let (_, _, home) = send(&app, Request::get("/").header("cookie", &session).body(Body::empty()).unwrap()).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &session, format!("_csrf={csrf}&name=P"))).await;
    send(&app, post("/do/folder.create", &session, format!("_csrf={csrf}&project=1&name=Contracts"))).await;

    let upload = |uri: &str, parts: &[(&str, &str)], content: &str| {
        let mut body = String::new();
        for (k, v) in parts {
            body.push_str(&format!("--B\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"));
        }
        body.push_str(&format!(
            "--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"plan.txt\"\r\nContent-Type: text/plain\r\n\r\n{content}\r\n--B--\r\n"
        ));
        Request::post(uri.to_string())
            .header("cookie", &session)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (_, h, _) = send(&app, upload("/projects/1/documents/upload", &[("_csrf", &csrf), ("folder", "1"), ("title", "Plan set")], "one")).await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(loc.starts_with("/projects/1/documents/1?ok="), "{loc}");
    let (_, h, _) = send(&app, upload("/projects/1/documents/1/upload", &[("_csrf", &csrf), ("note", "Rev B")], "two")).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let (st, ..) = send(&app, upload("/projects/1/documents/1/upload", &[("_csrf", "bad")], "three")).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &session).body(Body::empty()).unwrap();
    let (_, _, list) = send(&app, get("/projects/1/documents?folder=1")).await;
    assert!(list.contains("Plan set") && list.contains("v2"), "{list}");
    let (_, _, detail) = send(&app, get("/projects/1/documents/1")).await;
    assert!(detail.contains("Rev B") && detail.contains("Current"));
    let href = detail.split("href=\"").find(|s| s.contains("/files/")).unwrap().split('"').next().unwrap().to_string();
    let (st, h, body) = send(&app, get(&href)).await;
    assert_eq!((st, body.as_str()), (StatusCode::OK, "two"));
    assert_eq!(h.get("content-type").unwrap(), "application/octet-stream");
    let (_, _, found) = send(&app, get("/projects/1/documents?q=plan")).await;
    assert!(found.contains("Plan set"));
    let (st, ..) = send(&app, get("/projects/1/documents/99")).await;
    assert_eq!(st, StatusCode::OK, "pages answer not-found inside the page");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn drawings_log_revisions_and_csv() {
    let dir = temp("drawings");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let session = cookies(&h);
    let (_, _, home) = send(&app, Request::get("/").header("cookie", &session).body(Body::empty()).unwrap()).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &session, format!("_csrf={csrf}&name=P"))).await;
    send(&app, post("/do/drawing.import", &session, format!("_csrf={csrf}&project=1&text=S-201%2C+Foundation%2C+structural%2C+Level+0"))).await;

    let upload = |uri: &str, parts: &[(&str, &str)], file: Option<&str>| {
        let mut body = String::new();
        for (k, v) in parts {
            body.push_str(&format!("--B\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"));
        }
        if let Some(c) = file {
            body.push_str(&format!(
                "--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"s.pdf\"\r\nContent-Type: application/pdf\r\n\r\n{c}\r\n"
            ));
        }
        body.push_str("--B--\r\n");
        Request::post(uri.to_string())
            .header("cookie", &session)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (_, h, _) = send(
        &app,
        upload(
            "/projects/1/drawings/upload",
            &[("_csrf", &csrf), ("number", "A-101"), ("title", "Plan"), ("discipline", "architectural"), ("revision", "0")],
            Some("sheet one"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/drawings/2?ok="));
    let (_, h, _) = send(&app, upload("/projects/1/drawings/1/upload", &[("_csrf", &csrf), ("revision", "A"), ("set", "Bulletin 1")], None)).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let (_, h, _) = send(&app, upload("/projects/1/drawings/1/upload", &[("_csrf", &csrf), ("revision", "A")], None)).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "duplicate revision is refused");

    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &session).body(Body::empty()).unwrap();
    let (_, _, list) = send(&app, get("/projects/1/drawings")).await;
    assert!(list.contains("A-101") && list.contains("S-201") && list.contains("Bulletin 1"), "{list}");
    let (_, _, filtered) = send(&app, get("/projects/1/drawings?discipline=structural")).await;
    assert!(filtered.contains("S-201") && !filtered.contains("A-101"));
    let (_, _, detail) = send(&app, get("/projects/1/drawings/2")).await;
    let href = detail.split("href=\"").find(|s| s.contains("/files/")).unwrap().split('"').next().unwrap().to_string();
    let (st, _, body) = send(&app, get(&href)).await;
    assert_eq!((st, body.as_str()), (StatusCode::OK, "sheet one"));
    let (st, h, csv) = send(&app, get("/projects/1/drawings.csv")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.get("content-disposition").is_some() && csv.contains("A-101,Plan,Architectural"), "{csv}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn schedule_page_gantt_and_csv() {
    let dir = temp("schedule");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let session = cookies(&h);
    let (_, _, home) = send(&app, Request::get("/").header("cookie", &session).body(Body::empty()).unwrap()).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &session, format!("_csrf={csrf}&name=P"))).await;
    let (_, h, _) = send(
        &app,
        post("/do/schedule.add", &session, format!("_csrf={csrf}&project=1&name=Excavate&start=2026-10-12&days=3&milestone=false&parent=&company=&predecessors=&percent=&notes=&_next=%2Fprojects%2F1%2Fschedule&_back=%2Fprojects%2F1%2Fschedule")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    send(&app, post("/do/schedule.add", &session, format!("_csrf={csrf}&project=1&name=Foundations&start=2026-10-15&days=4&predecessors=1"))).await;

    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &session).body(Body::empty()).unwrap();
    let (st, _, page) = send(&app, get("/projects/1/schedule")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(page.contains("Excavate") && page.contains("Foundations") && page.contains("class=\"bar critical\""), "{page}");
    let (_, _, edit) = send(&app, get("/projects/1/schedule/2")).await;
    assert!(edit.contains("Edit task") && edit.contains("value=\"1\""));
    let (_, _, crit) = send(&app, get("/projects/1/schedule?show=late")).await;
    assert!(crit.contains("Excavate"), "late tasks are listed (dates are in the past)");
    let (st, h, csv) = send(&app, get("/projects/1/schedule.csv")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.get("content-disposition").is_some() && csv.contains("Foundations,2026-10-15,2026-10-20,4"), "{csv}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn budget_forecast_snapshot_compare_and_lock() {
    let dir = temp("forecast");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;
    send(&app, post("/do/budget.import", &ada, format!("_csrf={csrf}&project=1&text=03-300%2C100000"))).await;
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    let code = budget.split("name=\"cost_code\"").nth(1).and_then(|s| s.split("value=\"").nth(1)).unwrap().split('"').next().unwrap().to_string();
    send(&app, post("/do/budget.set_forecast", &ada, format!("_csrf={csrf}&project=1&cost_code={code}&forecast_to_complete=130000"))).await;
    send(&app, post("/do/budget.snapshot", &ada, format!("_csrf={csrf}&project=1&name=Kickoff&note="))).await;
    send(&app, post("/do/budget.set_forecast", &ada, format!("_csrf={csrf}&project=1&cost_code={code}&forecast_to_complete=90000"))).await;
    let (_, _, budget) = send(&app, get("/projects/1/budget?compare=1")).await;
    assert!(budget.contains("Projected overrun") || budget.contains("Projected cost"), "{budget}");
    assert!(budget.contains("Now compared with") && budget.contains("Kickoff") && budget.contains("130,000.00"), "{budget}");
    send(&app, post("/do/budget.lock", &ada, format!("_csrf={csrf}&project=1"))).await;
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    assert!(budget.contains("The original budget is locked") && !budget.contains("Set a budget line"));
    let (_, _, csv) = send(&app, get("/projects/1/budget.csv")).await;
    assert!(csv.contains("Forecast to complete"), "{csv}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn commitment_file_upload_and_download() {
    let dir = temp("commitfiles");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;
    send(&app, post("/do/commitment.create", &ada, format!("_csrf={csrf}&project=1&title=Concrete"))).await;
    let body = format!(
        "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"contract.txt\"\r\nContent-Type: text/plain\r\n\r\nsigned\r\n--B--\r\n"
    );
    let req = Request::post("/projects/1/commitments/1/upload")
        .header("cookie", &ada)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(body))
        .unwrap();
    let (_, h, _) = send(&app, req).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/commitments/1")).await;
    assert!(detail.contains("contract.txt"));
    let href = detail.split("href=\"").find(|s| s.contains("/files/")).unwrap().split('"').next().unwrap().to_string();
    let (st, _, body) = send(&app, get(&href)).await;
    assert_eq!((st, body.as_str()), (StatusCode::OK, "signed"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}
