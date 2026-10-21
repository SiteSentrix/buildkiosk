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
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/?") && h.get(LOCATION).unwrap().to_str().unwrap().contains("error="));

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

    // A response carries files of its own, which download like the RFI's.
    let respond = |token: &str, text: &str| {
        let body = format!(
            "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{token}\r\n--B\r\nContent-Disposition: form-data; name=\"body\"\r\n\r\n{text}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"sketch.txt\"\r\nContent-Type: text/plain\r\n\r\nsketch-bytes\r\n--B--\r\n"
        );
        Request::post("/projects/1/rfis/1/respond")
            .header("cookie", &session)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (st, ..) = send(&app, respond("bad", "x")).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (_, h, _) = send(&app, respond(&csrf, "Use+the+detail")).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, Request::get("/projects/1/rfis/1").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(detail.contains("sketch.txt"));
    let href =
        detail.split("href=\"").filter(|s| s.contains("/files/")).find(|s| s.contains("sketch.txt")).unwrap().split('"').next().unwrap().to_string();
    let (st, _, body) = send(&app, Request::get(&href).header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert_eq!((st, body.as_str()), (StatusCode::OK, "sketch-bytes"));

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

    // Per-tool overrides: page lists the tools, a saved override sticks, "inherit" clears it.
    send(&app, post("/do/member.update", &session, format!("_csrf={csrf}&project=1&person=2&project_admin=false"))).await;
    let (st, _, access) = send(&app, Request::get("/projects/1/team/2").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(st == StatusCode::OK && access.contains("overrides.rfis") && access.contains("Use the template"), "{access}");
    let (_, h, _) = send(
        &app,
        post(
            "/do/member.update",
            &session,
            format!(
                "_csrf={csrf}&_next=/projects/1/team/2&project=1&person=2&overrides.rfis=none&overrides.budget=read&overrides.submittals=inherit"
            ),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, access) = send(&app, Request::get("/projects/1/team/2").header("cookie", &session).body(Body::empty()).unwrap()).await;
    let row = |key: &str| access.split(&format!("name=\"overrides.{key}\"")).nth(1).unwrap().split("</select>").next().unwrap().to_string();
    assert!(row("rfis").contains("selected") && row("rfis").contains("value=\"none\" selected"), "{}", row("rfis"));
    assert!(row("budget").contains("value=\"read\" selected"));
    assert!(row("submittals").contains("value=\"inherit\" selected"));

    // Project and company settings pages edit through the same commands.
    let (st, _, ps) = send(&app, Request::get("/projects/1/settings").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(st == StatusCode::OK && ps.contains("Project settings") && ps.contains("name=\"stage\""), "{ps}");
    let (_, h, _) = send(&app, post("/do/project.update", &session, format!("_csrf={csrf}&_next=/projects/1/settings&project=1&name=Tower+Two&stage=closeout&value=1250000.50&start=2026-01-05&completion=2026-12-01&active=true"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, ps) = send(&app, Request::get("/projects/1/settings").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(ps.contains("Tower Two") && ps.contains("1250000.50") && ps.contains("2026-12-01") && ps.contains("value=\"closeout\" selected"), "{ps}");
    let (st, _, cs) = send(&app, Request::get("/settings").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(st == StatusCode::OK && cs.contains("Company settings") && cs.contains("Acme"));
    let (_, h, _) = send(&app, post("/do/company.update", &session, format!("_csrf={csrf}&_next=/settings&phone=555-0100&rfi_due_days=5"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, cs) = send(&app, Request::get("/settings").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(cs.contains("555-0100"));
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

    // Register import from an uploaded file, then the log and its PDF.
    let import = |token: &str, csv: &str| {
        let body = format!(
            "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{token}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"register.csv\"\r\nContent-Type: text/csv\r\n\r\n{csv}\r\n--B--\r\n"
        );
        Request::post("/projects/1/submittals/import")
            .header("cookie", &session)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (st, ..) = send(&app, import("bad", "title\nX")).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (_, h, _) =
        send(&app, import(&csrf, "title,spec_section,kind,due\nAnchor bolts,05 12 00,Product data,2026-12-01\nDoor hardware,08 71 00,Samples,"))
            .await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(loc.contains("ok=") && loc.contains("Imported%202"), "{loc}");
    let (_, h, _) = send(&app, import(&csrf, "title\n")).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "an empty register imports nothing, without an error");
    let (st, _, log) = send(&app, get("/projects/1/submittals/log")).await;
    assert!(st == StatusCode::OK && log.contains("Submittal log") && log.contains("Anchor bolts") && log.contains("05 12 00"), "{log}");
    let (st, h, pdf) = send(&app, get("/projects/1/submittals/log/pdf")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(pdf.starts_with("%PDF") && h.get(CONTENT_TYPE).unwrap() == "application/pdf");
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
    // Files on the log.
    let upload = |token: &str| {
        let body = format!(
            "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{token}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"site.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nsite-photo\r\n--B--\r\n"
        );
        Request::post("/projects/1/daily-logs/1/upload")
            .header("cookie", &ada)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (st, ..) = send(&app, upload("bad")).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (_, h, _) = send(&app, upload(&csrf)).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/daily-logs/1")).await;
    let href =
        detail.split("href=\"").filter(|x| x.contains("/files/")).find(|x| x.contains("site.jpg")).unwrap().split('"').next().unwrap().to_string();
    let (st, _, bytes) = send(&app, get(&href)).await;
    assert_eq!((st, bytes.as_str()), (StatusCode::OK, "site-photo"));

    // Copy yesterday's crews, and the weekly summary with its PDF.
    let (_, _, plain_form) = send(&app, get("/projects/1/daily-logs/new")).await;
    assert!(plain_form.contains("Copy crews and equipment from") && !plain_form.contains("value=\"Northline\""));
    let (_, _, copied) = send(&app, get("/projects/1/daily-logs/new?copy=1")).await;
    assert!(copied.contains("value=\"Northline\"") && copied.contains("value=\"Excavator\""), "{copied}");
    let (st, _, week) = send(&app, get("/projects/1/daily-logs/week?date=2026-10-14")).await;
    assert!(st == StatusCode::OK && week.contains("Week of Oct 12, 2026") && week.contains("Poured footings") && week.contains("No log"), "{week}");
    let (st, h, pdf) = send(&app, get("/projects/1/daily-logs/week/pdf?date=2026-10-12")).await;
    assert!(st == StatusCode::OK && pdf.starts_with("%PDF") && h.get(CONTENT_TYPE).unwrap() == "application/pdf");
    // A second log for the same day is refused with a message, and a bad hours value too.
    let (_, h, _) = send(&app, post("/projects/1/daily-logs/save", &ada, format!("_csrf={csrf}&date=2026-10-12"))).await;
    assert!(
        h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/daily-logs/new?")
            && h.get(LOCATION).unwrap().to_str().unwrap().contains("error=")
    );
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

    // Bulk import from a file, then filter, group and print the list.
    let import = |token: &str, csv: &str| {
        let body = format!(
            "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{token}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"walk.csv\"\r\nContent-Type: text/csv\r\n\r\n{csv}\r\n--B--\r\n"
        );
        Request::post("/projects/1/punch-list/import")
            .header("cookie", &ada)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (st, ..) = send(&app, import("bad", "title\nX")).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (_, h, _) = send(&app, import(&csrf, "title,location,kind,priority,due,assignees\nCracked tile,Lobby,Tile,high,2026-12-01,ada@example.com\nLoose handrail,Stair 2,Carpentry,,,\nTouch-up,Lobby,Paint,low,,")).await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(loc.contains("ok=") && loc.contains("Imported%203"), "{loc}");
    let (_, h, _) = send(&app, import(&csrf, "title\nGood\nBad,Here,,sideways")).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "one bad line adds nothing");
    let (_, _, all) = send(&app, get("/projects/1/punch-list")).await;
    assert!(!all.contains(">Good<"));
    let (_, _, by_loc) = send(&app, get("/projects/1/punch-list?group=location")).await;
    assert!(by_loc.contains("group-row") && by_loc.contains("Lobby") && by_loc.contains("Stair 2"));
    let (_, _, lobby) = send(&app, get("/projects/1/punch-list?location=lobby")).await;
    assert!(lobby.contains("Cracked tile") && !lobby.contains("Loose handrail"));
    let (_, _, mine) = send(&app, get("/projects/1/punch-list?assignee=1")).await;
    assert!(mine.contains("Cracked tile") && !mine.contains("Touch-up"));
    let (st, h, pdf) = send(&app, get("/projects/1/punch-list.pdf?group=location&location=Lobby")).await;
    assert!(st == StatusCode::OK && pdf.starts_with("%PDF") && h.get(CONTENT_TYPE).unwrap() == "application/pdf");
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
    assert!(form.contains("Add observation") && form.contains("name=\"kind\" placeholder"), "free text until the project sets categories");
    send(&app, post("/do/project.update", &ada, format!("_csrf={csrf}&project=1&observation_types=Safety%0AHousekeeping"))).await;
    let (_, _, form) = send(&app, get("/projects/1/observations/new")).await;
    assert!(form.contains("<select id=\"kind\"") && form.contains("value=\"Housekeeping\""), "{form}");
    let (st, _, trends) = send(&app, get("/projects/1/observations/trends")).await;
    assert!(st == StatusCode::OK && trends.contains("Observation trends"));
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
    let (st, h, pdf) = send(&app, get("/projects/1/inspections/1/pdf")).await;
    assert!(st == StatusCode::OK && pdf.starts_with("%PDF") && h.get(CONTENT_TYPE).unwrap() == "application/pdf");
    let (_, h, _) = send(
        &app,
        post(
            "/do/inspection_template.update",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/inspections/templates&project=1&id=2&name=Framing+check&items=%23+Studs%0AStuds+plumb"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, templates) = send(&app, get("/projects/1/inspections/templates")).await;
    assert!(templates.contains("Framing check") && templates.contains("Studs plumb") && templates.contains("Edit this template"));

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

    // A signed, repeating inspection with a photo on a line, and the section scores.
    let (_, h, _) = send(&app, post("/do/inspection.create", &ada, format!("_csrf={csrf}&_next=/projects/1/inspections/%7Bid%7D&project=1&title=Weekly+walk&items=%23+Site%0AFences+up%0AExits+clear&require_signature=true&repeat_days=7&inspector=1"))).await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().split('?').next().unwrap().to_string();
    let (_, _, detail) = send(&app, get(&loc)).await;
    assert!(
        detail.contains("name=\"signature\"")
            && detail.contains("Every 7 days")
            && detail.contains("Sections")
            && detail.contains("Add a photo to a line"),
        "{detail}"
    );
    let line_upload = |token: &str| {
        let body = format!(
            "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{token}\r\n--B\r\nContent-Disposition: form-data; name=\"line\"\r\n\r\n2\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"pallets.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nphoto-bytes\r\n--B--\r\n"
        );
        Request::post(format!("{loc}/line-file"))
            .header("cookie", &ada)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (st, ..) = send(&app, line_upload("bad")).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (_, h, _) = send(&app, line_upload(&csrf)).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get(&loc)).await;
    let href =
        detail.split("href=\"").filter(|x| x.contains("/files/")).find(|x| x.contains("pallets.jpg")).unwrap().split('"').next().unwrap().to_string();
    let (st, _, bytes) = send(&app, get(&href)).await;
    assert_eq!((st, bytes.as_str()), (StatusCode::OK, "photo-bytes"));
    let answers = format!("_csrf={csrf}&line.0.answer=pass&line.1.answer=fail&line.1.note=Blocked&_complete=1");
    let (_, h, _) = send(&app, post(&format!("{loc}/answers"), &ada, answers.clone())).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "unsigned");
    let (_, h, _) = send(&app, post(&format!("{loc}/answers"), &ada, format!("{answers}&signature=Ada+Admin"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get(&loc)).await;
    assert!(detail.contains("Ada Admin") && detail.contains("Next inspection"), "{detail}");
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

    // A budget file upload, with a cost code the project doesn't have yet.
    let upload = |fields: &[(&str, &str)], file: &str| {
        let mut body = String::new();
        for (k, v) in fields {
            body.push_str(&format!("--B\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"));
        }
        body.push_str(&format!(
            "--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"budget.csv\"\r\nContent-Type: text/csv\r\n\r\n{file}\r\n--B--\r\n"
        ));
        Request::post("/projects/1/budget/import-file")
            .header("cookie", &ada)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let sheet = "Cost code,Description,Amount\n99-100,Owner allowances,\"25,000\"\n03-300,,121000\n";
    let (_, h, _) = send(&app, upload(&[("_csrf", &csrf)], sheet)).await;
    let to = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(to.contains("error=") && to.contains("99-100"), "a new code is refused unless asked for: {to}");
    let (_, h, _) = send(&app, upload(&[("_csrf", &csrf), ("add_codes", "true")], sheet)).await;
    let to = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(to.contains("ok=Imported%202%20budget%20lines.%20Added%201%20cost%20code"), "{to}");
    let (_, _, csv) = send(&app, get("/projects/1/budget.csv")).await;
    assert!(csv.contains("99-100,Owner allowances,25000.00") && csv.contains("03-300,Cast-in-place concrete,121000.00"), "{csv}");
    let (st, _, _) = send(&app, upload(&[("_csrf", "wrong")], sheet)).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Signed contracts: a project can require one before approval; the card on the commitment records it.
    let (_, _, detail) = send(&app, get("/projects/1/commitments/1")).await;
    assert!(detail.contains("Contract signature"), "{detail}");
    let (_, _, list) = send(&app, get("/projects/1/commitments")).await;
    assert!(list.contains("Require a signed contract before approval"), "{list}");
    send(&app, post("/do/commitment.require_signed", &ada, format!("_csrf={csrf}&_next=/projects/1/commitments&project=1&required=true"))).await;
    let (_, _, list) = send(&app, get("/projects/1/commitments")).await;
    assert!(list.contains("Stop requiring signatures"), "{list}");
    send(
        &app,
        post("/projects/1/commitments/save", &ada, format!("_csrf={csrf}&title=Piers&kind=subcontract&vendor=&description=&lines.0.cost_code={concrete}&lines.0.description=Piers&lines.0.amount=9000&lines.1.cost_code=&lines.1.amount=")),
    )
    .await;
    let (_, h, _) = send(&app, post("/do/commitment.approve", &ada, format!("_csrf={csrf}&_next=/projects/1/commitments/2&project=1&id=2"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "an unsigned contract can't be approved: {h:?}");
    let (_, h, _) = send(
        &app,
        post(
            "/do/commitment.send_for_signature",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/commitments/2&project=1&id=2&to=sam%40example.com&on="),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/commitments/2")).await;
    assert!(detail.contains("Out for signature") && detail.contains("sam@example.com") && detail.contains("Record signature"), "{detail}");
    send(
        &app,
        post(
            "/do/commitment.record_signature",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/commitments/2&project=1&id=2&signer=Sam+Sub&on=&note="),
        ),
    )
    .await;
    let (_, _, detail) = send(&app, get("/projects/1/commitments/2")).await;
    assert!(detail.contains("Signed") && detail.contains("Sam Sub"), "{detail}");
    let (_, h, _) = send(&app, post("/do/commitment.approve", &ada, format!("_csrf={csrf}&_next=/projects/1/commitments/2&project=1&id=2"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");

    // A saved budget view: chosen columns, grouped by division, and the same columns in the CSV.
    assert!(budget.contains("Budget views") && budget.contains("columns[]"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/budget.view_save",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/budget&project=1&name=Cash&group=division&columns[]=original&columns[]=projected"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, plain) = send(&app, get("/projects/1/budget")).await;
    assert!(plain.contains("href=\"/projects/1/budget?view=1\"") && plain.contains(">Cash<"), "{plain}");
    let (_, _, viewed) = send(&app, get("/projects/1/budget?view=1")).await;
    assert!(viewed.contains("Division 03") && viewed.contains("Division 05") && viewed.contains("Subtotal"), "{viewed}");
    assert!(viewed.contains("Projected cost") && !viewed.contains("Committed</th>"), "{viewed}");
    let (_, _, csv) = send(&app, get("/projects/1/budget.csv?view=1")).await;
    assert!(csv.starts_with("Cost code,Description,Original budget,Projected cost\r\n"), "{csv}");
    // The PDF follows the view, and a view can be each person's default.
    let (st, h, _) = send(&app, get("/projects/1/budget.pdf?view=1")).await;
    assert!(st == StatusCode::OK && h.get(CONTENT_TYPE).unwrap() == "application/pdf");
    send(&app, post("/do/budget.view_default", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, dflt) = send(&app, get("/projects/1/budget")).await;
    assert!(dflt.contains("Division 03") && dflt.contains("This is the view you open by default."), "{dflt}");
    let (_, _, standard) = send(&app, get("/projects/1/budget?view=0")).await;
    assert!(!standard.contains("Division 03") && standard.contains("Open the standard layout by default"), "{standard}");
    send(&app, post("/do/budget.view_default", &ada, format!("_csrf={csrf}&project=1&id="))).await;
    let (_, _, cleared) = send(&app, get("/projects/1/budget")).await;
    assert!(!cleared.contains("Division 03"), "{cleared}");
    // Forecasting by committed cost: the 05-100 line (nothing spent) stops being assumed spent.
    let (_, h, _) =
        send(&app, post("/do/budget.set_method", &ada, format!("_csrf={csrf}&_next=/projects/1/budget&project=1&method=committed"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    assert!(budget.contains("only what is committed or spent") && budget.contains("40,250.75"), "{budget}");
    let (_, _, gone) = send(&app, post("/do/budget.view_delete", &ada, format!("_csrf={csrf}&_next=/projects/1/budget&project=1&id=1"))).await;
    let _ = gone;
    let (_, _, back) = send(&app, get("/projects/1/budget?view=1")).await;
    assert!(!back.contains("Division 03"), "a deleted view falls back to the standard table");
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
    // Signing the contract with the owner is tracked on the contract's page.
    assert!(contract.contains("Mark as sent for signature") && contract.contains("Record the signature"), "{contract}");
    send(&app, post("/do/prime_contract.record_signature", &ada, format!("_csrf={csrf}&project=1&id=1&signer=Olga+Owner&on=2026-10-01"))).await;
    let (_, _, contract) = send(&app, get("/projects/1/prime-contracts/1")).await;
    assert!(contract.contains("Olga Owner") && contract.contains("Clear the signature"), "{contract}");
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

    // Several files at once, a zip of the folder, moving a folder, and the deleted-files page.
    let many = |token: &str| {
        let mut body = format!(
            "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{token}\r\n--B\r\nContent-Disposition: form-data; name=\"folder\"\r\n\r\n1\r\n"
        );
        for (name, content) in [("a.txt", "alpha"), ("b.txt", "beta"), ("a.txt", "alpha again")] {
            body.push_str(&format!(
                "--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: text/plain\r\n\r\n{content}\r\n"
            ));
        }
        body.push_str("--B--\r\n");
        Request::post("/projects/1/documents/upload-many")
            .header("cookie", &session)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (st, ..) = send(&app, many("bad")).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (_, h, _) = send(&app, many(&csrf)).await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(loc.starts_with("/projects/1/documents?folder=1&ok=") && loc.contains("Uploaded%203"), "{loc}");
    let res = router(app.clone())
        .oneshot(Request::get("/projects/1/documents/zip?folder=1").header("cookie", &session).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(CONTENT_TYPE).unwrap(), "application/zip");
    let zip = res.into_body().collect().await.unwrap().to_bytes();
    assert!(zip.starts_with(b"PK\x03\x04"));
    let text = String::from_utf8_lossy(&zip);
    assert!(
        text.contains("a.txt") && text.contains("a (2).txt") && text.contains("b.txt") && text.contains("alpha again") && text.contains("plan.txt"),
        "unique names, current versions"
    );
    let (st, ..) = send(&app, Request::get("/projects/1/documents/zip").body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    let (_, h, _) = send(&app, Request::get("/projects/1/documents/zip?folder=99").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(h.get(LOCATION).is_none(), "an unknown folder is just not found");
    send(&app, post("/do/folder.create", &session, format!("_csrf={csrf}&project=1&name=Archive"))).await;
    let (_, _, folder_page) =
        send(&app, Request::get("/projects/1/documents?folder=1").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(folder_page.contains("Move this folder into") && folder_page.contains("Download all as zip"));
    let (_, h, _) =
        send(&app, post("/do/folder.update", &session, format!("_csrf={csrf}&_next=/projects/1/documents&project=1&id=1&parent=2"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, moved) = send(&app, Request::get("/projects/1/documents?folder=1").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(moved.contains("Archive"));
    send(&app, post("/do/document.delete", &session, format!("_csrf={csrf}&project=1&id=2"))).await;
    let (st, _, trash) = send(&app, Request::get("/projects/1/documents/trash").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(st == StatusCode::OK && trash.contains("Deleted files") && trash.contains("a.txt") && trash.contains("Restore"), "{trash}");
    send(&app, post("/do/document.restore", &session, format!("_csrf={csrf}&project=1&id=2"))).await;
    let (_, _, trash) = send(&app, Request::get("/projects/1/documents/trash").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(trash.contains("Nothing deleted"));
    // A picture (and a PDF) is previewed on the document page; a restricted document is hidden from others.
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(40, 30, |x, y| image::Rgb([x as u8, y as u8, 7])))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n").as_bytes());
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"site.png\"\r\nContent-Type: image/png\r\n\r\n");
    body.extend_from_slice(&png);
    body.extend_from_slice(b"\r\n--B--\r\n");
    let req = Request::post("/projects/1/documents/upload")
        .header("cookie", &session)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(body))
        .unwrap();
    let (_, h, _) = send(&app, req).await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    let did = loc.trim_start_matches("/projects/1/documents/").split('?').next().unwrap().to_string();
    let (_, _, detail) = send(&app, get(&format!("/projects/1/documents/{did}"))).await;
    let view = detail.split("src=\"").find(|s| s.contains("/view/")).map(|s| s.split('"').next().unwrap().replace("&amp;", "&"));
    let view = view.unwrap_or_else(|| panic!("a preview image: {detail}"));
    let res = router(app.clone()).oneshot(get(&view)).await.unwrap();
    assert_eq!((res.status(), res.headers().get(CONTENT_TYPE).unwrap().to_str().unwrap().to_string()), (StatusCode::OK, "image/png".to_string()));
    let (st, ..) = send(&app, Request::get(view.clone()).body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::SEE_OTHER, "signed-out people don't see previews");
    assert!(detail.contains("Who can open it"), "{detail}");
    send(&app, post("/do/document.update", &session, format!("_csrf={csrf}&project=1&id={did}&restricted=true&access%5B%5D="))).await;
    let (_, _, detail) = send(&app, get(&format!("/projects/1/documents/{did}"))).await;
    assert!(detail.contains("Restricted"), "{detail}");
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

    // A set of sheets at once: new numbers become sheets, a known number gets a revision.
    let set = |token: &str, files: &[(&str, &str)]| {
        let mut body = format!(
            "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{token}\r\n--B\r\nContent-Disposition: form-data; name=\"revision\"\r\n\r\nSet-1\r\n--B\r\nContent-Disposition: form-data; name=\"set\"\r\n\r\nPermit set\r\n"
        );
        for (name, content) in files {
            body.push_str(&format!(
                "--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/pdf\r\n\r\n{content}\r\n"
            ));
        }
        body.push_str("--B--\r\n");
        Request::post("/projects/1/drawings/upload-set")
            .header("cookie", &session)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (st, ..) = send(&app, set("bad", &[("S-201 Framing.pdf", "x")])).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (_, h, _) = send(&app, set(&csrf, &[("S-201 Framing plan.pdf", "framing"), ("FP-001_Sprinklers.pdf", "sprinklers")])).await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(loc.contains("ok=") && loc.contains("1%20new%20sheet%20and%201%20new%20revision"), "{loc}");
    let (_, h, _) = send(&app, set(&csrf, &[("s-201 Framing plan.pdf", "again")])).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "the same revision label twice is refused: {h:?}");
    let (_, _, list) = send(&app, Request::get("/projects/1/drawings").header("cookie", &session).body(Body::empty()).unwrap()).await;
    assert!(
        list.contains("S-201")
            && list.contains("Framing plan")
            && list.contains("FP-001")
            && list.contains("Sprinklers")
            && list.contains("Upload a set of sheets")
    );
    assert_eq!(sheet_from_file_name("A-101 Floor plan.pdf"), ("A-101".to_string(), "Floor plan".to_string()));
    assert_eq!(sheet_from_file_name("E-2_Panel_schedule.pdf"), ("E-2".to_string(), "Panel schedule".to_string()));
    assert_eq!(discipline_for_sheet("FP-001"), "fire_protection");
    assert_eq!(discipline_for_sheet("S-201"), "structural");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn resource_levelling_preview_and_apply() {
    let dir = temp("level");
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
    for (name, start, days, res) in [("Framing", "2026-10-12", 3, "Carpenter+4"), ("Blocking", "2026-10-13", 2, "Carpenter+3")] {
        send(&app, post("/do/schedule.add", &session, format!("_csrf={csrf}&project=1&name={name}&start={start}&days={days}&resources={res}"))).await;
    }
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &session).body(Body::empty()).unwrap();
    // With no limits there is nothing to level.
    let (_, _, res) = send(&app, get("/projects/1/schedule/resources?from=2026-10-12")).await;
    assert!(!res.contains("Level resources"), "{res}");
    send(&app, post("/do/schedule.resource_limits", &session, format!("_csrf={csrf}&project=1&text=Carpenter+5"))).await;
    let (st, _, res) = send(&app, get("/projects/1/schedule/resources?from=2026-10-12")).await;
    assert!(st == StatusCode::OK && res.contains("Level resources") && res.contains("Preview, keeping the finish date"), "{res}");
    assert!(!res.contains("Apply this plan"), "no plan is shown until one is asked for");
    // The preview lists what would move and offers to apply it; nothing has changed yet.
    let (_, _, res) = send(&app, get("/projects/1/schedule/resources?from=2026-10-12&level=all")).await;
    assert!(res.contains("Blocking") && res.contains("Apply this plan") && res.contains("Working days later"), "{res}");
    let (_, _, list) = send(&app, get("/projects/1/schedule.csv")).await;
    assert!(list.contains("2026-10-13"), "{list}");
    // Applying moves the task, so the page then finds nothing to do.
    let (_, h, _) =
        send(&app, post("/do/schedule.level", &session, format!("_csrf={csrf}&project=1&within_float=false&_next=%2Fprojects%2F1%2Fschedule"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, res) = send(&app, get("/projects/1/schedule/resources?from=2026-10-12&level=all")).await;
    assert!(res.contains("nothing needs to move") && !res.contains("Apply this plan"), "{res}");
    let (_, _, list) = send(&app, get("/projects/1/schedule.csv")).await;
    assert!(list.contains("Blocking") && list.contains("2026-10-15"), "{list}");

    // A project file uploaded with the boxes ticked brings its resources, limits and calendar.
    let xml = r#"<Project xmlns="http://schemas.microsoft.com/project"><Calendars><Calendar><UID>1</UID><IsBaseCalendar>1</IsBaseCalendar><WeekDays><WeekDay><DayType>1</DayType><DayWorking>0</DayWorking></WeekDay><WeekDay><DayType>2</DayType><DayWorking>1</DayWorking></WeekDay><WeekDay><DayType>7</DayType><DayWorking>1</DayWorking></WeekDay></WeekDays></Calendar></Calendars><Resources><Resource><UID>1</UID><Name>Mason</Name><MaxUnits>3</MaxUnits></Resource></Resources><Tasks><Task><UID>1</UID><Name>Pour</Name><Start>2026-11-02T08:00:00</Start><Finish>2026-11-03T17:00:00</Finish></Task></Tasks><Assignments><Assignment><TaskUID>1</TaskUID><ResourceUID>1</ResourceUID><Units>2</Units></Assignment></Assignments></Project>"#;
    let mut body = String::new();
    for (k, v) in [("_csrf", csrf.as_str()), ("resources", "true"), ("limits", "true"), ("calendar", "true")] {
        body.push_str(&format!("--B\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"));
    }
    body.push_str(&format!(
        "--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"plan.xml\"\r\nContent-Type: text/xml\r\n\r\n{xml}\r\n--B--\r\n"
    ));
    let upload = Request::post("/projects/1/schedule/import-xml")
        .header("cookie", &session)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(body))
        .unwrap();
    let (_, h, _) = send(&app, upload).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, res) = send(&app, get("/projects/1/schedule/resources?from=2026-11-02&weeks=1")).await;
    assert!(res.contains("Mason") && res.contains(">3<"), "the limit from the file is shown: {res}");
    let (_, _, cal) = send(&app, get("/projects/1/schedule/calendar")).await;
    assert!(cal.contains("value=\"mon, sat\""), "the file's weekdays, Monday and Saturday only here");
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
    assert!(page.contains("id=\"daysoff\"") && page.contains("class=\"dayoff\"") && page.contains("<use href=\"#daysoff\">"), "days off are shaded");
    let (_, _, edit) = send(&app, get("/projects/1/schedule/2")).await;
    assert!(edit.contains("Edit task") && edit.contains("value=\"1\""));
    let (_, _, crit) = send(&app, get("/projects/1/schedule?show=late")).await;
    assert!(crit.contains("Excavate"), "late tasks are listed (dates are in the past)");
    let (st, h, csv) = send(&app, get("/projects/1/schedule.csv")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.get("content-disposition").is_some() && csv.contains("Foundations,2026-10-15,2026-10-20,4"), "{csv}");

    // Dragging bars needs the browser module; only a build that carries it offers it, with its own page policy.
    let (_, ph, drag) = send(&app, get("/projects/1/schedule")).await;
    let csp = ph.get("content-security-policy").and_then(|v| v.to_str().ok()).unwrap_or_default();
    if viewer_assets::START_JS.is_some() {
        assert!(
            drag.contains("id=\"wasm-gantt\"")
                && drag.contains("data-task=\"1\"")
                && drag.contains("name=\"finish_shift\"")
                && drag.contains("class=\"handle\""),
            "{drag}"
        );
        assert!(csp.contains("script-src 'self' 'wasm-unsafe-eval'"), "{csp}");
    } else {
        assert!(!drag.contains("wasm-gantt") && csp.contains("script-src 'none'"), "{csp}");
    }
    // Moving a task through the form the chart posts.
    let (_, h, _) = send(
        &app,
        post(
            "/do/schedule.move",
            &session,
            format!("_csrf={csrf}&project=1&id=1&shift=7&_next=%2Fprojects%2F1%2Fschedule&_back=%2Fprojects%2F1%2Fschedule"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, moved) = send(&app, get("/projects/1/schedule.csv")).await;
    assert!(moved.contains("Excavate,2026-10-19,2026-10-21"), "{moved}");

    // The working calendar: set by the form, shown on its page, used by the dates.
    let (st, _, cal) = send(&app, get("/projects/1/schedule/calendar")).await;
    assert!(st == StatusCode::OK && cal.contains("name=\"days\" value=\"mon-fri\"") && cal.contains("Save calendar"), "{cal}");
    let (_, h, _) = send(
        &app,
        post(
            "/do/schedule.calendar_set",
            &session,
            format!("_csrf={csrf}&project=1&days=mon-sat&holidays=2026-10-14+Founders+Day&_next=%2Fprojects%2F1%2Fschedule%2Fcalendar&_back=%2Fprojects%2F1%2Fschedule%2Fcalendar"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, cal) = send(&app, get("/projects/1/schedule/calendar")).await;
    assert!(cal.contains("value=\"mon-sat\"") && cal.contains("2026-10-14 Founders Day"), "{cal}");
    let (_, h, _) = send(
        &app,
        post(
            "/do/schedule.calendar_set",
            &session,
            format!("_csrf={csrf}&project=1&days=nope&_next=%2F&_back=%2Fprojects%2F1%2Fschedule%2Fcalendar"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "a bad weekday is refused");
    // Named baselines: saved from the page, listed with the one in use marked.
    send(&app, post("/do/schedule.baseline_save", &session, format!("_csrf={csrf}&project=1&name=Contract"))).await;
    let (_, _, sched_page) = send(&app, get("/projects/1/schedule")).await;
    assert!(sched_page.contains("Baselines") && sched_page.contains("Contract") && sched_page.contains("In use"), "{sched_page}");
    // A company holiday list, taken by the project, shows on the calendar page.
    send(&app, post("/do/holiday_list.save", &session, format!("_csrf={csrf}&name=Federal+2026&holidays=2026-11-11+Veterans+Day"))).await;
    let (_, _, cal) = send(&app, get("/projects/1/schedule/calendar")).await;
    assert!(cal.contains("Company holiday lists") && cal.contains("Federal 2026"), "{cal}");
    send(&app, post("/do/schedule.calendar_set", &session, format!("_csrf={csrf}&project=1&lists%5B%5D=&lists%5B%5D=1"))).await;
    let (_, _, cal) = send(&app, get("/projects/1/schedule/calendar")).await;
    assert!(cal.contains("Days off from company lists") && cal.contains("Veterans Day") && cal.contains("2026-10-14 Founders Day"), "{cal}");

    // A link with a kind, shown the way it is written, and the look ahead.
    send(&app, post("/do/schedule.update", &session, format!("_csrf={csrf}&project=1&id=2&predecessors=1SS%2B1"))).await;
    let (_, _, edit) = send(&app, get("/projects/1/schedule/2")).await;
    assert!(edit.contains("value=\"1SS+1\"") && edit.contains("start to start"), "{edit}");
    // A constraint it breaks is flagged; resources show on the loading page, with a limit.
    send(
        &app,
        post(
            "/do/schedule.update",
            &session,
            format!("_csrf={csrf}&project=1&id=1&constraint=fnlt&constraint_date=2026-10-13&resources=Carpenter+4%2C+Laborer+2"),
        ),
    )
    .await;
    let (_, _, flagged) = send(&app, get("/projects/1/schedule")).await;
    assert!(flagged.contains(">Constraint<") && flagged.contains("Break a constraint"), "{flagged}");
    let (_, _, task) = send(&app, get("/projects/1/schedule/1")).await;
    assert!(task.contains("name=\"constraint_date\" value=\"2026-10-13\"") && task.contains("value=\"Carpenter 4, Laborer 2\""), "{task}");
    send(&app, post("/do/schedule.resource_limits", &session, format!("_csrf={csrf}&project=1&text=Carpenter+3"))).await;
    let (st, _, res) = send(&app, get("/projects/1/schedule/resources?weeks=4&from=2026-10-12")).await;
    assert!(st == StatusCode::OK && res.contains("Carpenter") && res.contains("class=\"right over\"") && res.contains("Save limits"), "{res}");
    // A project-planning XML file, uploaded.
    let xml = r#"<Project xmlns="http://schemas.microsoft.com/project"><Tasks><Task><UID>1</UID><Name>Imported pour</Name><OutlineLevel>1</OutlineLevel><Start>2026-11-02T08:00:00</Start><Finish>2026-11-03T17:00:00</Finish></Task></Tasks></Project>"#;
    let multipart = |fields: &[(&str, &str)], file: &str| {
        let mut body = String::new();
        for (k, v) in fields {
            body.push_str(&format!("--B\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"));
        }
        body.push_str(&format!(
            "--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"plan.xml\"\r\nContent-Type: text/xml\r\n\r\n{file}\r\n--B--\r\n"
        ));
        Request::post("/projects/1/schedule/import-xml")
            .header("cookie", &session)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (_, h, _) = send(&app, multipart(&[("_csrf", &csrf)], xml)).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok=Imported%201%20task"), "{h:?}");
    let (_, h, _) = send(&app, multipart(&[("_csrf", &csrf)], "not a project file")).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "{h:?}");
    let (st, _, _) = send(&app, multipart(&[("_csrf", "wrong")], xml)).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    // An exchange (.xer) file, in a Windows code page (the é is one byte), uploaded.
    let mut xer = b"ERMHDR\t19.12\n%T\tTASK\n%F\ttask_id\tproj_id\ttask_name\ttask_type\ttarget_start_date\ttarget_end_date\n%R\t5\t1\tCaf".to_vec();
    xer.push(0xE9);
    xer.extend_from_slice(b" slab\tTT_Task\t2026-11-09 08:00\t2026-11-10 17:00\n%E\n");
    let mut body = Vec::new();
    body.extend_from_slice(format!("--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"plan.xer\"\r\nContent-Type: text/plain\r\n\r\n").as_bytes());
    body.extend_from_slice(&xer);
    body.extend_from_slice(b"\r\n--B--\r\n");
    let req = Request::post("/projects/1/schedule/import-xer")
        .header("cookie", &session)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(body))
        .unwrap();
    let (_, h, _) = send(&app, req).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok=Imported%201%20task"), "{h:?}");
    let (_, _, xer_page) = send(&app, get("/projects/1/schedule")).await;
    assert!(xer_page.contains("Caf\u{e9} slab") && xer_page.contains("Import an exchange file"), "{xer_page}");
    let (_, _, after) = send(&app, get("/projects/1/schedule")).await;
    assert!(after.contains("Imported pour"), "{after}");
    let (st, _, ahead) = send(&app, get("/projects/1/schedule/lookahead?weeks=2&from=2026-10-12")).await;
    assert!(st == StatusCode::OK && ahead.contains("Week 1") && ahead.contains("Excavate") && ahead.contains("Foundations"), "{ahead}");
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

#[tokio::test]
async fn transmittal_form_send_and_files() {
    let dir = temp("transmittals");
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
    let (_, _, new) = send(&app, get("/projects/1/transmittals/new")).await;
    assert!(new.contains("items.0.description") && new.contains("Save draft"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/transmittal.create",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/transmittals/%7Bid%7D&_back=/projects/1/transmittals/new&project=1&subject=Permit+set&purpose=for_review&to%5B%5D=1&to%5B%5D=&cc%5B%5D=&due=&private=false&message=&items.0.description=Drawings&items.0.copies=3&items.1.description=&items.1.copies="),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/transmittals/1?ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/transmittals/1")).await;
    assert!(detail.contains("Drawings") && detail.contains("Send") && detail.contains("Draft"));
    send(&app, post("/do/transmittal.send", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, list) = send(&app, get("/projects/1/transmittals?status=sent")).await;
    assert!(list.contains("Permit set") && list.contains("Sent") && list.contains("0 of 1"), "{list}");

    // Items linked to a document and a drawing; the recipient acknowledges; the cover sheet shows both.
    let sha = "d".repeat(64);
    send(
        &app,
        post(
            "/do/document.add",
            &ada,
            format!("_csrf={csrf}&project=1&title=Geotech+report&sha256={sha}&name=geo.pdf&size=10&mime=application%2Fpdf"),
        ),
    )
    .await;
    send(
        &app,
        post("/do/drawing.add", &ada, format!("_csrf={csrf}&project=1&number=A-101&title=Floor+plan&revision=B&sha256={sha}&name=a.pdf&size=10")),
    )
    .await;
    let (_, _, form) = send(&app, get("/projects/1/transmittals/new")).await;
    assert!(form.contains("items.0.document") && form.contains("Geotech report") && form.contains("A-101 Floor plan"), "{form}");
    let (_, h, _) = send(
        &app,
        post("/do/transmittal.create", &ada, format!("_csrf={csrf}&_next=/projects/1/transmittals/%7Bid%7D&project=1&subject=Linked&to%5B%5D=1&items.0.description=&items.0.document=1&items.0.copies=1&items.1.description=&items.1.drawing=1&items.1.copies=2")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/transmittals/2?ok="), "{h:?}");
    send(&app, post("/do/transmittal.send", &ada, format!("_csrf={csrf}&project=1&id=2"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/transmittals/2")).await;
    assert!(
        detail.contains("Geotech report") && detail.contains("v1") && detail.contains("A-101 Floor plan") && detail.contains("rev B"),
        "{detail}"
    );
    assert!(
        detail.contains("href=\"/projects/1/documents/1\"")
            && detail.contains("href=\"/projects/1/drawings/1\"")
            && detail.contains("Acknowledge receipt"),
        "{detail}"
    );
    let (_, h, _) =
        send(&app, post("/do/transmittal.acknowledge", &ada, format!("_csrf={csrf}&_next=/projects/1/transmittals/2&project=1&id=2&person="))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/transmittals/2")).await;
    assert!(detail.contains("1 of 1 have acknowledged") && detail.contains("Received"), "{detail}");
    let (st, h, pdf) = send(&app, get("/projects/1/transmittals/2/pdf")).await;
    assert!(st == StatusCode::OK && h.get(CONTENT_TYPE).unwrap() == "application/pdf" && pdf.starts_with("%PDF"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn meeting_form_issue_and_actions() {
    let dir = temp("meetings");
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
    let (_, _, new) = send(&app, get("/projects/1/meetings/new")).await;
    assert!(new.contains("agenda.0.title") && new.contains("actions.0.assignee"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/meeting.create",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/meetings/%7Bid%7D&_back=/projects/1/meetings/new&project=1&title=Kickoff&date=2026-10-20&time=9+AM&category=&location=&minutes=&invitees%5B%5D=&invitees%5B%5D=1&agenda.0.title=Look-ahead&agenda.0.notes=&actions.0.description=Order+steel&actions.0.assignee=1&actions.0.due=2026-10-27"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/meetings/1?ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/meetings/1")).await;
    assert!(detail.contains("Look-ahead") && detail.contains("Order steel") && detail.contains("Mark done") && detail.contains("Issue minutes"));
    assert!(detail.contains("Make a task") && detail.contains("Schedule more of these meetings") && detail.contains("Sign-in sheet"), "{detail}");
    // The action becomes a task; the meeting is repeated; the open action is carried into the next one.
    let (_, h, _) =
        send(&app, post("/do/meeting.action_to_task", &ada, format!("_csrf={csrf}&_next=/projects/1/meetings/1&project=1&id=1&index=0"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, h, _) =
        send(&app, post("/do/meeting.repeat", &ada, format!("_csrf={csrf}&_next=/projects/1/meetings&project=1&id=1&every=weekly&count=2"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, next) = send(&app, get("/projects/1/meetings/2")).await;
    assert!(next.contains("Carry open actions forward") && next.contains("Oct 27, 2026"), "{next}");
    let (_, h, _) = send(&app, post("/do/meeting.carry_actions", &ada, format!("_csrf={csrf}&_next=/projects/1/meetings/2&project=1&id=2"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, next) = send(&app, get("/projects/1/meetings/2")).await;
    assert!(next.contains("Order steel") && next.contains("From") && next.contains("href=\"/projects/1/tasks/1\""), "{next}");
    let (_, _, first) = send(&app, get("/projects/1/meetings/1")).await;
    assert!(first.contains("Carried forward"), "{first}");
    let (st, h, pdf) = send(&app, get("/projects/1/meetings/1/signin.pdf")).await;
    assert!(st == StatusCode::OK && h.get(CONTENT_TYPE).unwrap() == "application/pdf" && pdf.starts_with("%PDF"));
    send(&app, post("/do/task.close", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, next) = send(&app, get("/projects/1/meetings/2")).await;
    assert!(next.contains("Done"), "closing the task finishes the carried action too: {next}");
    send(&app, post("/do/meeting.set_action", &ada, format!("_csrf={csrf}&project=1&id=1&index=0&done=true"))).await;
    send(&app, post("/do/meeting.issue", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, list) = send(&app, get("/projects/1/meetings")).await;
    assert!(list.contains("Kickoff") && list.contains("Minutes issued") && !list.contains("Open action items"), "{list}");
    // A meeting saved as a template starts the next one.
    let (_, _, first) = send(&app, get("/projects/1/meetings/1")).await;
    assert!(first.contains("Save as a template"), "{first}");
    send(
        &app,
        post("/do/meeting_template.save", &ada, format!("_csrf={csrf}&_next=/projects/1/meetings/1&project=1&from_meeting=1&name=Kickoff+template")),
    )
    .await;
    let (_, _, new) = send(&app, get("/projects/1/meetings/new")).await;
    assert!(new.contains("Start from a template") && new.contains("Kickoff template"), "{new}");
    let (_, h, _) = send(&app, post("/do/meeting.create", &ada, format!("_csrf={csrf}&_next=/projects/1/meetings/%7Bid%7D&_back=/projects/1/meetings/new&project=1&title=Second+kickoff&date=2026-11-03&time=&category=&location=&minutes=&template=1&invitees%5B%5D="))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, again) = send(&app, get("/projects/1/meetings/4")).await;
    assert!(again.contains("Look-ahead") && again.contains("9 AM"), "the agenda and time came from the template: {again}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn tasks_use_the_shared_work_pages() {
    let dir = temp("tasks");
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
    let (st, _, form) = send(&app, get("/projects/1/tasks/new")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(form.contains("Add task"));
    let (st, _, form) = send(&app, get("/projects/1/incidents/new")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(form.contains("Report incident") && form.contains("Root cause") && form.contains("Near miss"));
    assert!(form.contains("name=\"private\"") && form.contains("involved[]"), "{form}");

    // A recordable, private incident: the printable report, the injury log with hours and rates, and the CSV.
    let (_, h, _) = send(
        &app,
        post("/do/incident.create", &ada, format!("_csrf={csrf}&_next=/projects/1/incidents&project=1&title=Fell+from+ladder&severity=lost_time&recordable=true&occurred=2026-06-15&private=true&involved[]=1&people_involved=J.+Smith")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/incidents/1")).await;
    assert!(detail.contains("Printable report") && detail.contains("Private"), "{detail}");
    let (st, h, pdf) = send(&app, get("/projects/1/incidents/1/pdf")).await;
    assert!(st == StatusCode::OK && h.get(CONTENT_TYPE).unwrap() == "application/pdf" && pdf.starts_with("%PDF"));
    let (_, _, list) = send(&app, get("/projects/1/incidents")).await;
    assert!(list.contains("Injury log"), "{list}");
    let (st, _, log) = send(&app, get("/projects/1/incidents/log?year=2026")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(log.contains("Injury log, 2026") && log.contains("Fell from ladder") && log.contains("Rates need hours"), "{log}");
    let (_, h, _) =
        send(&app, post("/do/incident.set_hours", &ada, format!("_csrf={csrf}&_next=/projects/1/incidents/log&project=1&year=2026&hours=10000")))
            .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, log) = send(&app, get("/projects/1/incidents/log?year=2026")).await;
    assert!(log.contains("20.00") && !log.contains("Rates need hours"), "1 case in 10,000 hours is 20 per 200,000: {log}");
    let (st, h, csv) = send(&app, get("/projects/1/incidents/log.csv?year=2026")).await;
    assert!(st == StatusCode::OK && h.get("content-disposition").is_some() && csv.contains("Fell from ladder"), "{csv}");
    let (st, h, pdf) = send(&app, get("/projects/1/incidents/log.pdf?year=2026")).await;
    assert!(st == StatusCode::OK && h.get(CONTENT_TYPE).unwrap() == "application/pdf" && pdf.starts_with("%PDF"));
    let (_, _, none) = send(&app, get("/projects/1/incidents/log?year=2020")).await;
    assert!(none.contains("No recordable cases in 2020"), "{none}");
    let (_, _, all) = send(&app, get("/injury-log?year=2026")).await;
    assert!(all.contains("Injury log across projects, 2026") && all.contains("20.00") && all.contains("All projects"), "{all}");
    let (st, h, csv) = send(&app, get("/injury-log.csv?year=2026")).await;
    assert!(
        st == StatusCode::OK
            && h.get("content-disposition").is_some()
            && csv.starts_with("Project,Incidents,Recordable")
            && csv.contains("All projects,"),
        "{csv}"
    );
    let (_, _, link) = send(&app, get("/reports")).await;
    assert!(link.contains("/injury-log"), "{link}");
    send(&app, post("/do/task.create", &ada, format!("_csrf={csrf}&_next=/projects/1/tasks&project=1&title=Order+hardware&priority=high"))).await;
    let (_, _, list) = send(&app, get("/projects/1/tasks")).await;
    assert!(list.contains("Order hardware") && list.contains("Tasks"), "{list}");
    let (st, h, csv) = send(&app, get("/projects/1/tasks.csv")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.get("content-disposition").is_some() && csv.contains("Order hardware"));

    // The form carries the task-only fields; blank ones mean "none". A repeating task, a subtask, an RFI link.
    assert!(form_has_task_fields(&send(&app, get("/projects/1/tasks/new")).await.2));
    send(
        &app,
        post("/do/rfi.create", &ada, format!("_csrf={csrf}&_next=/projects/1/rfis&project=1&subject=Footing+depth&question=Which+governs%3F")),
    )
    .await;
    let (_, h, _) = send(
        &app,
        post("/do/task.create", &ada, format!("_csrf={csrf}&_next=/projects/1/tasks&project=1&title=Weekly+safety+walk&due=2026-10-14&repeat=weekly&repeat_until=&parent=&rfi=1&meeting=&assignees[]=1")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/tasks/2")).await;
    assert!(
        detail.contains("Weekly") && detail.contains("From RFI") && detail.contains("Footing depth") && detail.contains("Add subtask"),
        "{detail}"
    );
    let (_, h, _) = send(
        &app,
        post("/do/task.create", &ada, format!("_csrf={csrf}&_next=/projects/1/tasks/2&project=1&parent=2&assignees[]=1&title=Check+harnesses")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/tasks/2")).await;
    assert!(detail.contains("Check harnesses") && detail.contains("Subtasks"), "{detail}");
    let (_, _, kid) = send(&app, get("/projects/1/tasks/3")).await;
    assert!(kid.contains("Subtask of") && kid.contains("Weekly safety walk"), "{kid}");
    // The parent waits for its subtask; closing both makes the next week's task.
    let (_, h, _) = send(&app, post("/do/task.close", &ada, format!("_csrf={csrf}&_next=/projects/1/tasks/2&project=1&id=2"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "{h:?}");
    send(&app, post("/do/task.close", &ada, format!("_csrf={csrf}&_next=/projects/1/tasks/3&project=1&id=3"))).await;
    send(&app, post("/do/task.close", &ada, format!("_csrf={csrf}&_next=/projects/1/tasks/2&project=1&id=2"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/tasks/2")).await;
    assert!(detail.contains("Next occurrence"), "{detail}");

    // My tasks: the open ones assigned to Ada, across projects; closed ones on request.
    let (st, _, mine) = send(&app, get("/my-tasks")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(mine.contains("Weekly safety walk") && mine.contains("My tasks") && !mine.contains("Check harnesses"), "{mine}");
    let (_, _, everything) = send(&app, get("/my-tasks?all=1")).await;
    assert!(everything.contains("Order hardware"), "{everything}");
    let (_, _, closed) = send(&app, get("/my-tasks?closed=1")).await;
    assert!(closed.contains("Check harnesses") && closed.contains("Subtask"), "{closed}");
    send(&app, post("/do/task.create", &ada, format!("_csrf={csrf}&_next=/projects/1/tasks&project=1&title=Hand+over+keys&due=2026-10-15"))).await;
    let (st, _, cal) = send(&app, get("/projects/1/tasks/calendar?month=2026-10")).await;
    assert!(
        st == StatusCode::OK
            && cal.contains("Hand over keys")
            && cal.contains("Oct 2026")
            && cal.contains("month=2026-09")
            && cal.contains("month=2026-11"),
        "{cal}"
    );
    assert!(cal.contains("without a due date"), "the undated task is counted, not drawn: {cal}");
    let (_, _, other) = send(&app, get("/projects/1/tasks/calendar?month=2026-11")).await;
    assert!(!other.contains("Hand over keys"), "{other}");
    let (_, _, tasks_list) = send(&app, get("/projects/1/tasks")).await;
    assert!(tasks_list.contains("/projects/1/tasks/calendar"), "{tasks_list}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn photos_upload_gallery_and_inline_image() {
    let dir = temp("photos");
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

    // A PNG header, an HTML file pretending to be a picture, and a real GIF header.
    let mut body: Vec<u8> = Vec::new();
    let text = |b: &mut Vec<u8>, k: &str, v: &str| {
        b.extend_from_slice(format!("--B\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes())
    };
    text(&mut body, "_csrf", &csrf);
    text(&mut body, "album", "Foundations");
    text(&mut body, "caption", "Forms");
    for (name, content) in
        [("a.png", &b"\x89PNG\r\n\x1a\n-pixels"[..]), ("evil.png", &b"<script>alert(1)</script>"[..]), ("b.gif", &b"GIF89a-pixels"[..])]
    {
        body.extend_from_slice(
            format!("--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: image/png\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--B--\r\n");
    let req = Request::post("/projects/1/photos/upload")
        .header("cookie", &ada)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(body))
        .unwrap();
    let (_, h, _) = send(&app, req).await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(loc.contains("ok=") && loc.contains("2+added") || loc.contains("2%20added"), "{loc}");
    let (_, _, gallery) = send(&app, get("/projects/1/photos?album=foundations")).await;
    assert!(
        gallery.contains("/projects/1/photos/1/thumb") && gallery.contains("/projects/1/photos/2/thumb") && !gallery.contains("photos/3/"),
        "{gallery}"
    );
    let (st, h, body) = send(&app, get("/projects/1/photos/1/image")).await;
    assert_eq!((st, h.get("content-type").unwrap().to_str().unwrap()), (StatusCode::OK, "image/png"));
    assert!(body.starts_with("\u{89}PNG") || !body.is_empty());
    assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
    let (_, h, _) = send(&app, get("/projects/1/photos/1/download")).await;
    assert_eq!(h.get("content-type").unwrap(), "application/octet-stream");
    let (st, ..) = send(&app, Request::get("/projects/1/photos/1/image").body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::SEE_OTHER, "signed-out people don't see photos");
    let (_, _, detail) = send(&app, get("/projects/1/photos/1")).await;
    assert!(detail.contains("Forms") && detail.contains("Download original"));

    // A real, large picture gets a real, small thumbnail; one that can't be decoded falls back to itself.
    let mut big = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(1600, 1200, |x, y| image::Rgb([(x % 251) as u8, (y % 241) as u8, 90])))
        .write_to(&mut std::io::Cursor::new(&mut big), image::ImageFormat::Png)
        .unwrap();
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n").as_bytes());
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"big.png\"\r\nContent-Type: image/png\r\n\r\n");
    body.extend_from_slice(&big);
    body.extend_from_slice(b"\r\n--B--\r\n");
    let req = Request::post("/projects/1/photos/upload")
        .header("cookie", &ada)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(body))
        .unwrap();
    let (_, h, _) = send(&app, req).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let res = router(app.clone()).oneshot(get("/projects/1/photos/3/thumb")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let thumb = res.into_body().collect().await.unwrap().to_bytes();
    let decoded = image::load_from_memory(&thumb).unwrap();
    assert!(decoded.width() <= 360 && decoded.height() <= 360 && decoded.width() >= 300, "{}x{}", decoded.width(), decoded.height());
    assert!(thumb.len() < big.len() / 3, "{} vs {}", thumb.len(), big.len());
    let (st, h, _) = send(&app, get("/projects/1/photos/1/thumb")).await;
    assert_eq!((st, h.get("content-type").unwrap().to_str().unwrap()), (StatusCode::OK, "image/png"), "an undecodable picture is sent as it is");
    let (st, ..) = send(&app, Request::get("/projects/1/photos/3/thumb").body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::SEE_OTHER, "signed-out people don't see thumbnails");

    // The picture's size is recorded; a sideways phone picture (EXIF orientation 6) gets an upright thumbnail,
    // and the camera's date and name are read from it.
    let photo4 = app.lock().tenant.as_ref().unwrap().engine.state().photos.get(buildkiosk_core::Id::new(3)).unwrap().clone();
    assert_eq!((photo4.width, photo4.height), (1600, 1200));
    let mut jpeg = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(800, 400, |x, _| image::Rgb([(x % 251) as u8, 120, 90])))
        .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
        .unwrap();
    let mut tiff: Vec<u8> = b"MM\0*\0\0\0\x08".to_vec();
    tiff.extend([0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0, 0]);
    let mut seg = b"Exif\0\0".to_vec();
    seg.extend(tiff);
    let mut with_exif = vec![0xFF, 0xD8, 0xFF, 0xE1];
    with_exif.extend(((seg.len() + 2) as u16).to_be_bytes());
    with_exif.extend(seg);
    with_exif.extend(&jpeg[2..]);
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n").as_bytes());
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"side.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n");
    body.extend_from_slice(&with_exif);
    body.extend_from_slice(b"\r\n--B--\r\n");
    let req = Request::post("/projects/1/photos/upload")
        .header("cookie", &ada)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(body))
        .unwrap();
    send(&app, req).await;
    let res = router(app.clone()).oneshot(get("/projects/1/photos/4/thumb")).await.unwrap();
    let thumb = res.into_body().collect().await.unwrap().to_bytes();
    let decoded = image::load_from_memory(&thumb).unwrap();
    assert!(decoded.height() > decoded.width(), "turned upright: {}x{}", decoded.width(), decoded.height());

    // Two photos side by side.
    let (_, _, side) = send(&app, get("/projects/1/photos/compare?a=3&b=1")).await;
    assert!(
        side.contains("Compare photos")
            && side.contains("/projects/1/photos/3/image")
            && side.contains("/projects/1/photos/1/image")
            && side.contains("Swap sides"),
        "{side}"
    );
    let (_, _, nope) = send(&app, get("/projects/1/photos/compare?a=3&b=999")).await;
    assert!(!nope.contains("/projects/1/photos/3/image"), "an unknown photo gives not found");
    let (_, _, det) = send(&app, get("/projects/1/photos/3")).await;
    assert!(det.contains("Compare with") && det.contains("Side by side"), "{det}");
    // Tag a photo to a punch item and see it on the item's page; slideshow and zip.
    send(&app, post("/do/punch.create", &ada, format!("_csrf={csrf}&project=1&title=Touch+up+paint"))).await;
    send(&app, post("/do/photo.link", &ada, format!("_csrf={csrf}&project=1&id=3&tag=punch%3A1"))).await;
    let (_, _, punch) = send(&app, get("/projects/1/punch-list/1")).await;
    assert!(punch.contains("/projects/1/photos/3/thumb"), "{punch}");
    let (_, _, detail) = send(&app, get("/projects/1/photos/3")).await;
    assert!(detail.contains("Touch up paint") && detail.contains("1600 × 1200 px"), "{detail}");
    let (_, _, show) = send(&app, get("/projects/1/photos/slideshow?i=1")).await;
    assert!(show.contains("Picture 2 of 4") && !show.contains("http-equiv=\"refresh\""), "{show}");
    let (_, _, play) = send(&app, get("/projects/1/photos/slideshow?i=1&play=1")).await;
    assert!(play.contains("http-equiv=\"refresh\"") && play.contains("i=2") && play.contains("Pause"), "{play}");
    let res = router(app.clone()).oneshot(get("/projects/1/photos/album.zip?album=foundations")).await.unwrap();
    assert_eq!(res.headers().get("content-type").unwrap(), "application/zip");
    let zip = res.into_body().collect().await.unwrap().to_bytes();
    assert!(zip.starts_with(b"PK"), "a zip file");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn correspondence_send_reply_and_close() {
    let dir = temp("correspondence");
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
    let (_, _, new) = send(&app, get("/projects/1/correspondence/new")).await;
    assert!(new.contains("Save draft") && new.contains("Reply wanted by"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/correspondence.create",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/correspondence/%7Bid%7D&_back=/projects/1/correspondence/new&project=1&subject=Notice+of+delay&kind=Notice&date=2026-10-12&reply_by=&private=false&to%5B%5D=1&cc%5B%5D=&body=Steel+is+late"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/correspondence/1?ok="), "{h:?}");
    send(&app, post("/do/correspondence.send", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    send(&app, post("/do/correspondence.reply", &ada, format!("_csrf={csrf}&project=1&id=1&body=Received+with+thanks"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/correspondence/1")).await;
    assert!(detail.contains("Received with thanks") && detail.contains("Open") && detail.contains("Close"));
    send(&app, post("/do/correspondence.close", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, list) = send(&app, get("/projects/1/correspondence?status=closed")).await;
    assert!(list.contains("Notice of delay"));
    // Types give each kind its own numbering; the letter print and the incoming forms work.
    send(&app, post("/do/correspondence_type.save", &ada, format!("_csrf={csrf}&project=1&name=Notice&prefix=ntc"))).await;
    let (_, _, types) = send(&app, get("/projects/1/correspondence/types")).await;
    assert!(types.contains("NTC") && types.contains("New type"), "{types}");
    let (_, _, new) = send(&app, get("/projects/1/correspondence/new")).await;
    assert!(new.contains("Notice (NTC)"), "{new}");
    send(
        &app,
        post("/do/correspondence.create", &ada, format!("_csrf={csrf}&project=1&subject=Second&kind=Notice&date=2026-10-13&to%5B%5D=1&body=Hello")),
    )
    .await;
    let (_, _, list) = send(&app, get("/projects/1/correspondence")).await;
    assert!(list.contains("NTC-001"), "{list}");
    let (st, _, letter) = send(&app, get("/projects/1/correspondence/2/letter.pdf")).await;
    assert_eq!(st, StatusCode::OK, "{letter}");
    let (_, _, inc) = send(&app, get("/projects/1/correspondence/incoming")).await;
    assert!(inc.contains("Paste an email") && inc.contains("Type it in"), "{inc}");
    send(&app, post("/do/correspondence.file_incoming", &ada, format!("_csrf={csrf}&project=1&subject=Price+change&from_name=Sam+Supplier&from_email=&kind=Notice&date=2026-10-13&to%5B%5D=&to%5B%5D=1&body=Up"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/correspondence/3")).await;
    assert!(detail.contains("Sam Supplier") && detail.contains("outside the company"), "{detail}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn forms_template_fill_and_submit() {
    let dir = temp("forms");
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
    let (_, _, tpl) = send(&app, get("/projects/1/forms/templates")).await;
    assert!(tpl.contains("fields.0.label") && tpl.contains("Save template"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/form_template.create",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/forms/templates&project=1&name=Pre-pour&description=&fields.0.label=Rebar+inspected&fields.0.kind=yes_no&fields.0.options=&fields.0.required=true&fields.1.label=Mix&fields.1.kind=choice&fields.0.show_if_question=&fields.0.show_if_answer=&fields.1.show_if_question=1&fields.1.show_if_answer=yes&fields.1.options=3000+psi%2C+4000+psi&fields.2.label=&fields.2.kind=text&fields.2.options=&signers=Inspector"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, h, _) = send(
        &app,
        post(
            "/do/form.create",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/forms/%7Bid%7D&_back=/projects/1/forms&project=1&template=1&title=Slab+A"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/forms/1?ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/forms/1")).await;
    assert!(detail.contains("Rebar inspected") && detail.contains("name=\"answers.f1\"") && detail.contains("4000 psi"));
    let (_, h, _) = send(&app, post("/do/form.submit", &ada, format!("_csrf={csrf}&_back=/projects/1/forms/1&project=1&id=1"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "required answer missing");
    send(&app, post("/do/form.update", &ada, format!("_csrf={csrf}&project=1&id=1&title=Slab+A&answers.f1=yes&answers.f2=4000+psi"))).await;
    send(&app, post("/do/form.submit", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/forms/1")).await;
    assert!(detail.contains("Submitted") && detail.contains("Reopen") && detail.contains("4000 psi"), "{detail}");
    let (_, _, list) = send(&app, get("/projects/1/forms?status=submitted")).await;
    assert!(list.contains("Pre-pour"));
    // Signing as the one role, then the report and the recurring-form schedule.
    assert!(detail.contains("Type your full name"), "{detail}");
    send(&app, post("/do/form.sign", &ada, format!("_csrf={csrf}&project=1&id=1&role=Inspector&name=Ada+Admin"))).await;
    let (_, _, signed) = send(&app, get("/projects/1/forms/1")).await;
    assert!(signed.contains("Ada Admin") && !signed.contains("Not signed"), "{signed}");
    let (st, _, pdf) = send(&app, get("/projects/1/forms/1/pdf")).await;
    assert_eq!(st, StatusCode::OK, "{pdf}");
    let (_, _, rep) = send(&app, get("/projects/1/forms/report?template=1")).await;
    assert!(rep.contains("Rebar inspected") && rep.contains("4000 psi") && rep.contains("Export CSV"), "{rep}");
    let (st, h, csv) = send(&app, get("/projects/1/forms/report.csv?template=1&status=submitted")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        h.get("content-disposition").is_some()
            && csv.starts_with("Number,Title,Status,Started,Started by,Rebar inspected,Mix")
            && csv.contains("Slab A,Submitted"),
        "{csv}"
    );
    send(
        &app,
        post("/do/form_schedule.save", &ada, format!("_csrf={csrf}&project=1&template=1&every=weekly&assignee=1&title=Weekly&next=2026-10-01")),
    )
    .await;
    let (_, _, tpl) = send(&app, get("/projects/1/forms/templates")).await;
    assert!(tpl.contains("Recurring forms") && tpl.contains("Weekly") && tpl.contains("Only if question no."), "{tpl}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn timesheets_week_submit_and_approve() {
    let dir = temp("timesheets");
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
    let (_, _, page) = send(&app, get("/projects/1/timesheets?week=2026-10-14")).await;
    assert!(page.contains("Week of Oct 12, 2026") && page.contains("Log hours"), "{page}");
    send(&app, post("/do/timesheet.add", &ada, format!("_csrf={csrf}&project=1&date=2026-10-13&hours=7.5&cost_code=&description=Footings"))).await;
    let (_, _, page) = send(&app, get("/projects/1/timesheets?week=2026-10-12")).await;
    assert!(page.contains("Footings") && page.contains("7.5") && page.contains("Submit the week"));
    send(&app, post("/do/timesheet.submit", &ada, format!("_csrf={csrf}&project=1&from=2026-10-12&to=2026-10-18"))).await;
    let (_, _, page) = send(&app, get("/projects/1/timesheets?week=2026-10-12")).await;
    assert!(page.contains("Waiting for approval") && page.contains("Approve"));
    send(&app, post("/do/timesheet.approve", &ada, format!("_csrf={csrf}&project=1&person=1&from=2026-10-12&to=2026-10-18"))).await;
    let (st, h, csv) = send(&app, get("/projects/1/timesheets.csv?week=2026-10-12")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.get("content-disposition").is_some() && csv.contains("2026-10-13,Ada Admin,7.5") && csv.contains("Approved"), "{csv}");
    // Labour cost: rules, a rate, the report, then posting to direct costs.
    send(&app, post("/do/timesheet.set_rules", &ada, format!("_csrf={csrf}&project=1&daily_hours=6&weekly_hours=0&multiplier=150"))).await;
    send(&app, post("/do/timesheet.set_rate", &ada, format!("_csrf={csrf}&project=1&person=1&hourly=40&from=2026-10-01"))).await;
    let (st, _, cost) = send(&app, get("/projects/1/timesheets/cost?from=2026-10-12&to=2026-10-18")).await;
    assert_eq!(st, StatusCode::OK);
    // 7.5 hours with overtime after 6: 6 regular, 1.5 overtime = 240.00 + 90.00
    assert!(
        cost.contains("Ada Admin") && cost.contains("330.00") && cost.contains("Post to direct costs") && cost.contains("Overtime rules"),
        "{cost}"
    );
    let (_, _, csv) = send(&app, get("/projects/1/timesheets/cost.csv?from=2026-10-12&to=2026-10-18")).await;
    assert!(csv.contains("2026-10-13,Ada Admin") && csv.contains("6,1.5,40.00,330.00,Approved,no"), "{csv}");
    let (_, h, _) = send(
        &app,
        post(
            "/do/timesheet.post_costs",
            &ada,
            format!("_csrf={csrf}&project=1&from=2026-10-12&to=2026-10-18&default_cost_code=1&_next=/projects/1/timesheets/cost"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).is_some(), "{h:?}");
    let (_, _, csv) = send(&app, get("/projects/1/timesheets/cost.csv?from=2026-10-12&to=2026-10-18")).await;
    assert!(csv.ends_with("Approved,yes\r\n"), "{csv}");
    // A saved crew can be logged together.
    send(&app, post("/do/crew.save", &ada, format!("_csrf={csrf}&project=1&name=Concrete+crew&members%5B%5D=&members%5B%5D=1"))).await;
    let (_, _, crew_page) = send(&app, get("/projects/1/timesheets?week=2026-10-19")).await;
    assert!(crew_page.contains("Concrete crew") && crew_page.contains("A saved crew") && crew_page.contains("Add crew"), "{crew_page}");
    send(
        &app,
        post("/do/timesheet.add", &ada, format!("_csrf={csrf}&project=1&date=2026-10-20&hours=6&cost_code=&description=&crew=1&people%5B%5D=")),
    )
    .await;
    let (_, _, crew_week) = send(&app, get("/projects/1/timesheets.csv?week=2026-10-19")).await;
    assert!(crew_week.contains("2026-10-20,Ada Admin,6"), "{crew_week}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn project_report_and_home_waiting_list() {
    let dir = temp("report");
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
    send(&app, post("/do/task.create", &ada, format!("_csrf={csrf}&project=1&title=Order+hardware&due=2020-01-01"))).await;
    let (_, _, report) = send(&app, get("/projects/1/reports")).await;
    assert!(report.contains("Project report") && report.contains("Tasks") && report.contains("Past due"), "{report}");
    let (_, _, project) = send(&app, get("/projects/1")).await;
    assert!(project.contains("Waiting on me") && project.contains("Order hardware"), "{project}");
    let (st, h, csv) = send(&app, get("/projects/1/reports.csv")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.get("content-disposition").is_some() && csv.contains("Tasks,1,1,1"), "{csv}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn action_plan_outline_assign_and_complete() {
    let dir = temp("actionplans");
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
    let (_, _, new) = send(&app, get("/projects/1/action-plans/new")).await;
    assert!(new.contains("Sections and items"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/action_plan.create",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/action-plans/%7Bid%7D&_back=/projects/1/action-plans/new&project=1&title=Closeout&manager=1&description=&outline=%23+Documents%0A-+Collect+warranties%0A-+Record+drawings"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/action-plans/1?ok="), "{h:?}");
    send(&app, post("/do/action_plan.set_item", &ada, format!("_csrf={csrf}&project=1&id=1&section=0&item=0&assignee=1&due=2026-11-15&note=Binder")))
        .await;
    send(&app, post("/do/action_plan.set_approvers", &ada, format!("_csrf={csrf}&project=1&id=1&approvers%5B%5D=&approvers%5B%5D=1"))).await;
    send(&app, post("/do/action_plan.activate", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, waiting) = send(&app, get("/projects/1/action-plans/1")).await;
    assert!(waiting.contains("Waiting") && waiting.contains("Choose approvers") && waiting.contains("Start the plan"), "{waiting}");
    send(&app, post("/do/action_plan.approve", &ada, format!("_csrf={csrf}&project=1&id=1&note=Looks+right&approved=true"))).await;
    send(&app, post("/do/action_plan.activate", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    send(&app, post("/do/action_plan.comment", &ada, format!("_csrf={csrf}&project=1&id=1&section=0&item=0&text=Binder+is+in+the+trailer"))).await;
    send(&app, post("/do/action_plan.template_save", &ada, format!("_csrf={csrf}&project=1&id=1&name=Closeout+basics"))).await;
    let (_, _, newer) = send(&app, get("/projects/1/action-plans/new")).await;
    assert!(newer.contains("Closeout basics") && newer.contains("Start from a template"), "{newer}");
    let (_, _, detail) = send(&app, get("/projects/1/action-plans/1")).await;
    assert!(
        detail.contains("Approved")
            && detail.contains("Looks right")
            && detail.contains("Comments (1)")
            && detail.contains("Binder is in the trailer"),
        "{detail}"
    );
    assert!(
        detail.contains("Collect warranties") && detail.contains("Binder") && detail.contains("Mark done") && detail.contains("0 of 2 items done"),
        "{detail}"
    );
    // A file on the plan itself.
    let body = format!(
        "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"programme.txt\"\r\nContent-Type: text/plain\r\n\r\nplan-programme\r\n--B--\r\n"
    );
    let up = Request::post("/projects/1/action-plans/1/upload")
        .header("cookie", &ada)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(body))
        .unwrap();
    let (_, h, _) = send(&app, up).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, withfile) = send(&app, get("/projects/1/action-plans/1")).await;
    let href = withfile
        .split("href=\"")
        .filter(|x| x.contains("/files/"))
        .find(|x| x.contains("programme.txt"))
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    let (st, _, bytes) = send(&app, get(&href)).await;
    assert_eq!((st, bytes.as_str()), (StatusCode::OK, "plan-programme"));
    for item in 0..2 {
        send(&app, post("/do/action_plan.set_item", &ada, format!("_csrf={csrf}&project=1&id=1&section=0&item={item}&done=true"))).await;
    }
    send(&app, post("/do/action_plan.complete", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, list) = send(&app, get("/projects/1/action-plans?status=complete")).await;
    assert!(list.contains("Closeout") && list.contains("2 of 2 items"), "{list}");
    let (_, _, done) = send(&app, get("/projects/1/action-plans/1")).await;
    assert!(done.contains("Signed off"), "{done}");
    let (st, _, pdf) = send(&app, get("/projects/1/action-plans/1/pdf")).await;
    assert_eq!(st, StatusCode::OK, "{pdf}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bidding_package_bids_and_award() {
    let dir = temp("bidding");
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
    send(&app, post("/do/vendor.create", &ada, format!("_csrf={csrf}&name=Keystone+Concrete&kind=subcontractor"))).await;
    send(&app, post("/do/vendor.create", &ada, format!("_csrf={csrf}&name=Northline+Concrete&kind=subcontractor"))).await;
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    let code = budget.split("name=\"cost_code\"").nth(1).and_then(|s| s.split("value=\"").nth(1)).unwrap().split('"').next().unwrap().to_string();
    let (_, _, new) = send(&app, get("/projects/1/bidding/new")).await;
    assert!(new.contains("Keystone Concrete") && new.contains("Scope of work"));
    let (_, h, _) = send(
        &app,
        post("/do/bid_package.create", &ada, format!("_csrf={csrf}&_next=/projects/1/bidding/%7Bid%7D&_back=/projects/1/bidding/new&project=1&title=Concrete&due=&cost_code={code}&scope=Slab&invite%5B%5D=&invite%5B%5D=1&invite%5B%5D=2")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/bidding/1?ok="), "{h:?}");
    send(&app, post("/do/bid_package.open", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    send(
        &app,
        post("/do/bid_package.record_bid", &ada, format!("_csrf={csrf}&project=1&id=1&vendor=1&amount=120000&received=&notes=Excludes+rebar")),
    )
    .await;
    send(&app, post("/do/bid_package.record_bid", &ada, format!("_csrf={csrf}&project=1&id=1&vendor=2&amount=98000.50&received=&notes="))).await;
    let (_, _, detail) = send(&app, get("/projects/1/bidding/1")).await;
    assert!(
        detail.contains("98,000.50")
            && detail.contains("120,000.00")
            && detail.contains("Low")
            && detail.contains("Excludes rebar")
            && detail.contains("Award and start a commitment"),
        "{detail}"
    );
    // Scoring criteria and a score typed into the form (the blank box stays unscored).
    send(&app, post("/do/bid_package.set_criteria", &ada, format!("_csrf={csrf}&project=1&id=1&text=Price%2C+60%2C+price%0ASafety%2C+40"))).await;
    send(&app, post("/do/bid_package.score", &ada, format!("_csrf={csrf}&project=1&id=1&vendor=2&scores.2=9"))).await;
    send(&app, post("/do/bid_package.score", &ada, format!("_csrf={csrf}&project=1&id=1&vendor=1&scores.2="))).await;
    let (_, _, scored) = send(&app, get("/projects/1/bidding/1")).await;
    assert!(scored.contains("Scoring") && scored.contains("Total (of 10)") && scored.contains("not complete") && scored.contains("9.60"), "{scored}");
    send(&app, post("/do/bid_package.award", &ada, format!("_csrf={csrf}&project=1&id=1&vendor=2&commitment=true"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/bidding/1")).await;
    assert!(detail.contains("Awarded") && detail.contains("A draft commitment was started"), "{detail}");
    let (_, _, commitments) = send(&app, get("/projects/1/commitments")).await;
    assert!(commitments.contains("Concrete") && commitments.contains("98,000.50"), "{commitments}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bid_form_leveling_addenda_and_files() {
    let dir = temp("bidform");
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
    send(&app, post("/do/vendor.create", &ada, format!("_csrf={csrf}&name=Keystone+Concrete&kind=subcontractor"))).await;
    send(&app, post("/do/vendor.create", &ada, format!("_csrf={csrf}&name=Northline+Concrete&kind=subcontractor"))).await;
    let (_, _, new) = send(&app, get("/projects/1/bidding/new")).await;
    assert!(new.contains("scope_items.0.text") && new.contains("form_items.0.description") && new.contains("bond_percent"), "{new}");
    let (_, h, _) = send(
        &app,
        post("/do/bid_package.create", &ada, format!("_csrf={csrf}&_next=/projects/1/bidding/%7Bid%7D&project=1&title=Concrete&scope=Slab&invite%5B%5D=1&invite%5B%5D=2&scope_items.0.text=Footings&scope_items.1.text=Sealing&scope_items.2.text=&form_items.0.description=Slab&form_items.0.unit=sf&form_items.0.quantity=1000&form_items.1.description=&form_items.1.unit=&form_items.1.quantity=&alternates.0.name=Polished&alternates.1.name=&bond_percent=5")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/bidding/1?ok="), "{h:?}");
    send(&app, post("/do/bid_package.open", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/bidding/1")).await;
    assert!(
        detail.contains("Unit prices") && detail.contains("What the bid covers") && detail.contains("Issue addendum") && detail.contains("Bid bond"),
        "{detail}"
    );
    let (_, h, _) = send(
        &app,
        post("/do/bid_package.record_bid", &ada, format!("_csrf={csrf}&_next=/projects/1/bidding/1&project=1&id=1&vendor=1&amount=&received=&notes=&lines.0.item=0&lines.0.unit_price=21&scope.0.item=0&scope.0.answer=included&scope.0.adjustment=&scope.1.item=1&scope.1.answer=excluded&scope.1.adjustment=2500&alternates.0.index=0&alternates.0.amount=4000&bond_received=true&bond_amount=1050")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    send(&app, post("/do/bid_package.record_bid", &ada, format!("_csrf={csrf}&project=1&id=1&vendor=2&amount=23000&scope.0.item=0&scope.0.answer=included&scope.0.adjustment=&scope.1.item=1&scope.1.answer=included&scope.1.adjustment="))).await;
    let (_, _, detail) = send(&app, get("/projects/1/bidding/1")).await;
    assert!(
        detail.contains("Leveling")
            && detail.contains("23,500.00")
            && detail.contains("Excluded")
            && detail.contains("4,000.00")
            && detail.contains("No bond"),
        "{detail}"
    );
    // Addenda are issued and acknowledged; files attach to the package and to a bid and download.
    send(
        &app,
        post("/do/bid_package.addendum", &ada, format!("_csrf={csrf}&_next=/projects/1/bidding/1&project=1&id=1&title=Slab+thicker&body=Six+inches")),
    )
    .await;
    let (_, _, detail) = send(&app, get("/projects/1/bidding/1")).await;
    assert!(
        detail.contains("Addendum 1: Slab thicker") && detail.contains("Keystone Concrete acknowledged") && detail.contains("not acknowledged"),
        "{detail}"
    );
    send(&app, post("/do/bid_package.acknowledge", &ada, format!("_csrf={csrf}&project=1&id=1&vendor=1&number=1"))).await;
    let upload = |vendor: Option<&str>, content: &str| {
        let mut body = format!("--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n");
        if let Some(v) = vendor {
            body.push_str(&format!("--B\r\nContent-Disposition: form-data; name=\"vendor\"\r\n\r\n{v}\r\n"));
        }
        body.push_str(&format!(
            "--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"f.pdf\"\r\nContent-Type: application/pdf\r\n\r\n{content}\r\n--B--\r\n"
        ));
        Request::post("/projects/1/bidding/1/upload")
            .header("cookie", &ada)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (_, h, _) = send(&app, upload(None, "scope documents")).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, h, _) = send(&app, upload(Some("1"), "keystone proposal")).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, h, _) = send(&app, upload(Some("9"), "nobody")).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/bidding/1")).await;
    let shas: Vec<String> = detail.split("/projects/1/bidding/1/files/").skip(1).map(|t| t.split('"').next().unwrap().to_string()).collect();
    assert_eq!(shas.len(), 2, "{detail}");
    let mut bodies = Vec::new();
    for sha in &shas {
        let (st, h, bytes) = send(&app, get(&format!("/projects/1/bidding/1/files/{sha}"))).await;
        assert!(st == StatusCode::OK && h.get("content-disposition").is_some());
        bodies.push(bytes);
    }
    bodies.sort();
    assert_eq!(bodies, ["keystone proposal", "scope documents"]);
    assert_eq!(send(&app, get("/projects/1/bidding/1/files/nothing")).await.0, StatusCode::NOT_FOUND);
    // The award waits for the bond: Northline sent none, Keystone's is in.
    let (_, h, _) =
        send(&app, post("/do/bid_package.award", &ada, format!("_csrf={csrf}&_next=/projects/1/bidding/1&project=1&id=1&vendor=2"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "{h:?}");
    let (_, h, _) =
        send(&app, post("/do/bid_package.award", &ada, format!("_csrf={csrf}&_next=/projects/1/bidding/1&project=1&id=1&vendor=1"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn company_reports_save_chart_and_export() {
    let dir = temp("companyreports");
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
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=Riverside+Clinic"))).await;
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=Harbor+Office"))).await;
    send(&app, post("/do/budget.import", &ada, format!("_csrf={csrf}&project=1&text=03-300%2C+100000"))).await;
    send(&app, post("/do/budget.import", &ada, format!("_csrf={csrf}&project=2&text=03-300%2C+50000"))).await;
    for n in 0..3 {
        send(&app, post("/do/task.create", &ada, format!("_csrf={csrf}&project=1&title=T{n}"))).await;
    }
    let (st, _, page) = send(&app, get("/reports")).await;
    assert!(
        st == StatusCode::OK
            && page.contains("Riverside Clinic")
            && page.contains("Harbor Office")
            && page.contains("150,000.00")
            && page.contains("<svg"),
        "{page}"
    );
    assert!(page.contains("Save a report") && page.contains("columns[]") && page.contains("Reports"), "{page}");
    // Save one, run it, chart another column, export it.
    let (_, h, _) = send(
        &app,
        post("/do/company_report.save", &ada, format!("_csrf={csrf}&_next=/reports&name=Open+work&columns[]=open&columns[]=rfis&columns[]=revised&stage=&sort=open&descending=true&shared=true")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, page) = send(&app, get("/reports?report=1&chart=revised")).await;
    assert!(
        page.contains("Open work")
            && page.contains("Change this report")
            && page.contains("Delete this report")
            && page.contains("Revised budget by project"),
        "{page}"
    );
    assert!(page.find("Riverside Clinic").unwrap() < page.find("Harbor Office").unwrap(), "most open work first");
    let (st, h, csv) = send(&app, get("/reports.csv?report=1")).await;
    assert!(st == StatusCode::OK && h.get("content-disposition").is_some(), "{csv}");
    assert!(
        csv.starts_with("Project,Needs action,Open RFIs,Revised budget\r\n")
            && csv.contains("Riverside Clinic,3,0,100000.00")
            && csv.contains("Harbor Office,0,0,50000.00")
            && csv.contains("Total,3,0,150000.00"),
        "{csv}"
    );
    let (st, h, pdf) = send(&app, get("/reports.pdf?report=1")).await;
    assert!(st == StatusCode::OK && h.get(CONTENT_TYPE).unwrap() == "application/pdf" && pdf.starts_with("%PDF"));
    assert_eq!(send(&app, get("/reports.csv?report=99")).await.0, StatusCode::NOT_FOUND);
    // The schedule leaves as a project file.
    send(&app, post("/do/schedule.add", &ada, format!("_csrf={csrf}&project=1&name=Excavate&start=2026-10-12&days=3"))).await;
    let (st, h, xml) = send(&app, get("/projects/1/schedule.xml")).await;
    assert!(
        st == StatusCode::OK
            && h.get("content-disposition").unwrap().to_str().unwrap().contains("schedule.xml")
            && xml.contains("<Name>Excavate</Name>"),
        "{xml}"
    );
    let (_, _, sched) = send(&app, get("/projects/1/schedule")).await;
    assert!(sched.contains("schedule.xml"), "{sched}");
    // Deleting it.
    let (_, h, _) = send(&app, post("/do/company_report.delete", &ada, format!("_csrf={csrf}&_next=/reports&id=1"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn estimate_form_totals_apply_and_csv() {
    let dir = temp("estimating");
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
    let code = budget.split("name=\"cost_code\"").nth(1).and_then(|s| s.split("value=\"").nth(1)).unwrap().split('"').next().unwrap().to_string();
    let (_, _, new) = send(&app, get("/projects/1/estimating/new")).await;
    assert!(new.contains("lines.0.cost_code") && new.contains("markups.0.name"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/estimate.create",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/estimating/%7Bid%7D&_back=/projects/1/estimating/new&project=1&name=Schematic&notes=&lines.0.cost_code={code}&lines.0.description=Slab&lines.0.quantity=1000&lines.0.unit=sf&lines.0.unit_cost=8.5&lines.1.cost_code=&lines.1.description=&lines.1.quantity=&lines.1.unit=&lines.1.unit_cost=&markups.0.name=Overhead&markups.0.percent=10"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/projects/1/estimating/1?ok="), "{h:?}");
    send(&app, post("/do/estimate.finalize", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/estimating/1")).await;
    assert!(
        detail.contains("8,500.00") && detail.contains("Overhead (10%)") && detail.contains("9,350.00") && detail.contains("Put in the budget"),
        "{detail}"
    );
    send(&app, post("/do/estimate.apply_to_budget", &ada, format!("_csrf={csrf}&project=1&id=1&markup_cost_code="))).await;
    let (_, _, budget) = send(&app, get("/projects/1/budget")).await;
    assert!(budget.contains("8,500.00"));
    let (st, h, csv) = send(&app, get("/projects/1/estimating/1/csv")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.get("content-disposition").is_some() && csv.contains("Total,,,,,9350.00"), "{csv}");

    // The company catalogue, an assembly, lines from them, a new version and a comparison.
    let (st, _, cat) = send(&app, get("/catalog")).await;
    assert!(st == StatusCode::OK && cat.contains("Add an item") && cat.contains("Import a price list"), "{cat}");
    let (_, h, _) = send(
        &app,
        post(
            "/do/catalog.save",
            &ada,
            format!("_csrf={csrf}&_next=/catalog&name=Ready-mix&unit=cy&unit_cost=185&category=Concrete&cost_code={code}&notes="),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    send(&app, post("/do/catalog.price", &ada, format!("_csrf={csrf}&_next=/catalog&id=1&unit_cost=182.50&date=2026-10-01&vendor=&note=Quote")))
        .await;
    let (_, _, hist) = send(&app, get("/catalog?edit=1")).await;
    assert!(
        hist.contains("Prices for Ready-mix") && hist.contains("182.50") && hist.contains("Record a price") && hist.contains("Import a price list"),
        "{hist}"
    );
    let (_, h, _) = send(&app, post("/do/catalog.import", &ada, format!("_csrf={csrf}&_next=/catalog&text=Mesh%2C+sf%2C+0.45%2C+Concrete"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, h, _) = send(&app, post("/do/assembly.save", &ada, format!("_csrf={csrf}&_next=/catalog&name=Slab&unit=sf&parts.0.item=1&parts.0.quantity=0.012&parts.1.item=2&parts.1.quantity=1&parts.2.item=&parts.2.quantity="))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, cat) = send(&app, get("/catalog?q=mesh")).await;
    assert!(cat.contains("Mesh") && !cat.contains("Ready-mix</strong>") && cat.contains("2.67"), "{cat}");
    send(&app, post("/do/estimate.reopen", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let (_, _, detail) = send(&app, get("/projects/1/estimating/1")).await;
    assert!(
        detail.contains("Add lines from the catalogue")
            && detail.contains("From the catalogue")
            && detail.contains("Add its lines")
            && detail.contains("New version"),
        "{detail}"
    );
    let (_, h, _) = send(
        &app,
        post(
            "/do/estimate.add_from_catalog",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/estimating/1&project=1&id=1&item=1&quantity=10&cost_code="),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, h, _) = send(
        &app,
        post(
            "/do/estimate.add_assembly",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/estimating/1&project=1&id=1&assembly=1&quantity=100&cost_code={code}"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/projects/1/estimating/1")).await;
    assert!(detail.contains("Ready-mix") && detail.contains("Slab: Mesh"), "{detail}");
    let (_, h, _) = send(&app, post("/do/estimate.duplicate", &ada, format!("_csrf={csrf}&_next=/projects/1/estimating/1&project=1&id=1"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, list) = send(&app, get("/projects/1/estimating")).await;
    assert!(list.contains("(copy)") && list.contains("from #1") && list.contains("Catalogue"), "{list}");
    send(&app, post("/do/estimate.update", &ada, format!("_csrf={csrf}&project=1&id=2&alternates.0.name=Polished&alternates.0.cost_code={code}&alternates.0.amount=750&alternates.0.accepted=false&alternates.1.name=&alternates.1.cost_code=&alternates.1.amount="))).await;
    let (_, _, two) = send(&app, get("/projects/1/estimating/2")).await;
    assert!(two.contains("Alternates") && two.contains("Polished") && two.contains("Accept") && two.contains("A new version of"), "{two}");
    send(&app, post("/do/estimate.accept_alternate", &ada, format!("_csrf={csrf}&project=1&id=2&index=0&accepted=true"))).await;
    let (st, _, cmp) = send(&app, get("/projects/1/estimating/compare?a=2&b=1")).await;
    assert!(st == StatusCode::OK && cmp.contains("Compare estimates") && cmp.contains("Difference") && cmp.contains("750.00"), "{cmp}");
    let (_, _, csv) = send(&app, get("/projects/1/estimating/2/csv")).await;
    assert!(csv.contains("Alternate: Polished"), "{csv}");
    // Save as a template, then start a new estimate from it.
    let (_, _, one) = send(&app, get("/projects/1/estimating/1")).await;
    assert!(one.contains("Save as a template"), "{one}");
    send(&app, post("/do/estimate.template_save", &ada, format!("_csrf={csrf}&project=1&id=1&name=Standard+slab"))).await;
    let (_, _, newer) = send(&app, get("/projects/1/estimating/new")).await;
    assert!(newer.contains("Start from a template") && newer.contains("Standard slab"), "{newer}");
    let (_, h, _) = send(&app, post("/do/estimate.create", &ada, format!("_csrf={csrf}&_next=/projects/1/estimating/%7Bid%7D&project=1&name=From+template&template=1&lines.0.cost_code=&lines.0.description=&lines.0.quantity=&lines.0.unit=&lines.0.unit_cost="))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, made) = send(&app, get("/projects/1/estimating/3")).await;
    assert!(made.contains("From template") && made.contains("Slab"), "{made}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn prequalification_record_review_and_lapse_warning() {
    let dir = temp("prequal");
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
    send(&app, post("/do/vendor.create", &ada, format!("_csrf={csrf}&name=Keystone+Concrete&kind=subcontractor"))).await;
    let (_, _, page) = send(&app, get("/prequalification")).await;
    assert!(page.contains("Add or update a record") && page.contains("Keystone Concrete"), "{page}");
    send(&app, post("/do/prequal.save", &ada, format!("_csrf={csrf}&vendor=1&valid_until=&insurance_expires=2020-01-01&license_expires=&emr=0.85&bonding_capacity=2500000&notes=Good+references"))).await;
    send(&app, post("/do/prequal.review", &ada, format!("_csrf={csrf}&vendor=1&decision=approved"))).await;
    let (_, _, page) = send(&app, get("/prequalification")).await;
    assert!(page.contains("Approved") && page.contains("0.85") && page.contains("2,500,000.00") && page.contains("lapsed"), "{page}");
    let (_, _, page) = send(&app, get("/prequalification?attention=1")).await;
    assert!(page.contains("Keystone Concrete"));
    let (_, _, page) = send(&app, get("/prequalification?status=rejected")).await;
    assert!(!page.contains("Good references"));

    // A questionnaire, the company's answers, the points, and a certificate.
    let (st, _, forms) = send(&app, get("/prequalification/forms")).await;
    assert!(st == StatusCode::OK && forms.contains("New questionnaire") && forms.contains("Require prequalification"), "{forms}");
    let text = "%23+Safety%0ASafety+program%3F+%7C+20%0AEMR+under+1%3F+%7C+10%0A%23+Financial%0AAudited+statements+%7C+30";
    let (_, h, _) =
        send(&app, post("/do/prequal.form_save", &ada, format!("_csrf={csrf}&_next=/prequalification/forms&name=Subs&text={text}"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, forms) = send(&app, get("/prequalification/forms?edit=1")).await;
    assert!(forms.contains("Edit questionnaire") && forms.contains("Safety program?"), "{forms}");
    let (st, _, detail) = send(&app, get("/prequalification/1")).await;
    assert!(st == StatusCode::OK && detail.contains("Keystone Concrete") && detail.contains("Use this questionnaire"), "{detail}");
    send(&app, post("/do/prequal.answer", &ada, format!("_csrf={csrf}&_next=/prequalification/1&vendor=1&form=1"))).await;
    let (_, h, _) = send(
        &app,
        post("/do/prequal.answer", &ada, format!("_csrf={csrf}&_next=/prequalification/1&vendor=1&replies.0.section=0&replies.0.question=0&replies.0.text=Yes&replies.1.section=1&replies.1.question=0&replies.1.text=FY25")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, h, _) = send(
        &app,
        post("/do/prequal.grade", &ada, format!("_csrf={csrf}&_next=/prequalification/1&vendor=1&points.0.section=0&points.0.question=0&points.0.points=20&points.1.section=0&points.1.question=1&points.1.points=&points.2.section=1&points.2.question=0&points.2.points=15")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/prequalification/1")).await;
    assert!(detail.contains("35 of 60 points") && detail.contains("58%") && detail.contains("Save points and score"), "{detail}");
    // The upload and the download.
    let body = format!(
        "--B\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"coi.pdf\"\r\nContent-Type: application/pdf\r\n\r\ncertificate bytes\r\n--B--\r\n"
    );
    let up = Request::post("/prequalification/1/upload")
        .header("cookie", &ada)
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(body))
        .unwrap();
    let (_, h, _) = send(&app, up).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, detail) = send(&app, get("/prequalification/1")).await;
    let sha = detail.split("/prequalification/1/files/").nth(1).unwrap().split('"').next().unwrap().to_string();
    let (st, h, bytes) = send(&app, get(&format!("/prequalification/1/files/{sha}"))).await;
    assert!(st == StatusCode::OK && bytes == "certificate bytes" && h.get("content-disposition").unwrap().to_str().unwrap().contains("coi.pdf"));
    assert_eq!(send(&app, get("/prequalification/1/files/nothing")).await.0, StatusCode::NOT_FOUND);
    assert!(!send(&app, get("/prequalification/99")).await.2.contains("Use this questionnaire"), "an unknown company has no page");
    // The requirement shows on the questionnaires page.
    send(&app, post("/do/prequal.require", &ada, format!("_csrf={csrf}&_next=/prequalification/forms&required=true"))).await;
    let (_, _, forms) = send(&app, get("/prequalification/forms")).await;
    assert!(forms.contains("needs a current prequalification"), "{forms}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn directory_edit_template_levels_and_company() {
    let dir = temp("dirtemplates");
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
    let (_, _, dir_page) = send(&app, get("/directory")).await;
    assert!(dir_page.contains("Edit levels") && dir_page.contains("Add a permission template"));
    send(&app, post("/do/template.create", &ada, format!("_csrf={csrf}&_next=/directory&name=Estimator&description=Preconstruction+staff"))).await;
    let (_, _, edit) = send(&app, get("/directory/templates/7")).await;
    assert!(
        edit.contains("Permission template: Estimator") && edit.contains("name=\"levels.rfis\"") && edit.contains("name=\"levels.estimating\""),
        "{edit}"
    );
    let (_, h, _) = send(&app, post("/do/template.update", &ada, format!("_csrf={csrf}&_next=/directory/templates/7&id=7&name=Estimator&description=Preconstruction+staff&levels.estimating=admin&levels.bidding=standard&levels.rfis=read&levels.budget=none"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, edit) = send(&app, get("/directory/templates/7")).await;
    let selected = |tool: &str, level: &str| {
        let at = edit.find(&format!("name=\"levels.{tool}\"")).unwrap();
        let chunk = &edit[at..];
        let end = chunk.find("</select>").unwrap();
        chunk[..end].contains(&format!("<option value=\"{level}\" selected"))
    };
    assert!(selected("estimating", "admin") && selected("bidding", "standard") && selected("rfis", "read"));

    send(&app, post("/do/vendor.create", &ada, format!("_csrf={csrf}&name=Keystone&kind=subcontractor"))).await;
    let (_, _, company) = send(&app, get("/directory/companies/1")).await;
    assert!(company.contains("value=\"Keystone\""));
    send(
        &app,
        post(
            "/do/vendor.update",
            &ada,
            format!("_csrf={csrf}&_next=/directory&id=1&name=Keystone+Concrete&kind=subcontractor&trade=Concrete&city=Springfield&notes="),
        ),
    )
    .await;
    let (_, _, dir_page) = send(&app, get("/directory")).await;
    assert!(dir_page.contains("Keystone Concrete") && dir_page.contains("Concrete"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn mcp_over_http_needs_a_token_and_runs_commands() {
    let dir = temp("mcphttp");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    send(&app, post("/setup", &anon, body)).await;
    let token = {
        let mut inner = app.lock();
        let person = inner.tenant.as_ref().unwrap().engine.state().people.iter().next().unwrap().id.raw();
        let t = inner.creds.create_token(person, "agent", Timestamp::from_millis(now_ms())).unwrap();
        let t2 = inner.tenant.as_ref().unwrap().store.save_credentials(&inner.creds);
        t2.unwrap();
        t
    };
    let rpc = |auth: Option<&str>, msg: &str| {
        let mut r = Request::post("/mcp").header("content-type", "application/json");
        if let Some(a) = auth {
            r = r.header("authorization", format!("Bearer {a}"));
        }
        r.body(Body::from(msg.to_string())).unwrap()
    };
    let (st, ..) = send(&app, rpc(None, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, h, body) = send(&app, rpc(Some(&token), r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(h.get("content-type").unwrap(), "application/json");
    assert!(body.contains("project_create") && body.contains("rfi_list"), "{}", &body[..body.len().min(300)]);
    let (_, _, body) = send(
        &app,
        rpc(Some(&token), r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project_create","arguments":{"name":"Via MCP"}}}"#),
    )
    .await;
    assert!(body.contains("\"id\":2") && !body.contains("\"isError\":true"), "{body}");
    let (_, _, home) = send(
        &app,
        Request::get("/")
            .header("cookie", {
                let (_, h, p) = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
                let anon = cookies(&h);
                let b = format!("_csrf={}&email=ada%40example.com&password=correct-horse-battery", field(&p, "_csrf"));
                let (_, h, _) = send(&app, post("/login", &anon, b)).await;
                cookies(&h)
            })
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(home.contains("Via MCP"), "the project made over MCP shows up in the app");
    let (st, ..) = send(&app, rpc(Some(&token), r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)).await;
    assert_eq!(st, StatusCode::ACCEPTED);
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn account_password_and_tokens() {
    let dir = temp("account");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, acct) = send(&app, get("/account")).await;
    let csrf = field(&acct, "_csrf");
    assert!(acct.contains("Change password") && acct.contains("You have no tokens"));

    // A wrong current password, a mismatch and a weak password are refused.
    for (cur, new, again, want) in [
        ("nope", "a-new-long-password", "a-new-long-password", "not%20right"),
        ("correct-horse-battery", "a-new-long-password", "other-long-password", "different"),
        ("correct-horse-battery", "short", "short", "at%20least"),
    ] {
        let (_, h, _) = send(&app, post("/account/password", &ada, format!("_csrf={csrf}&current={cur}&password={new}&confirm={again}"))).await;
        let loc = h.get(LOCATION).unwrap().to_str().unwrap();
        assert!(loc.contains("error=") && loc.contains(want), "{loc}");
    }
    let (_, h, _) = send(
        &app,
        post(
            "/account/password",
            &ada,
            format!("_csrf={csrf}&current=correct-horse-battery&password=a-new-long-password&confirm=a-new-long-password"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let ada = cookies(&h);
    // The new password works and the old one doesn't.
    let (_, h2, p2) = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
    let a2 = cookies(&h2);
    let t2 = field(&p2, "_csrf");
    let (st, ..) = send(&app, post("/login", &a2, format!("_csrf={t2}&email=ada%40example.com&password=correct-horse-battery"))).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, ..) = send(&app, post("/login", &a2, format!("_csrf={t2}&email=ada%40example.com&password=a-new-long-password"))).await;
    assert_eq!(st, StatusCode::SEE_OTHER);

    // A token is shown once, works for the API, and can be revoked.
    let (_, _, acct) = send(&app, Request::get("/account").header("cookie", &ada).body(Body::empty()).unwrap()).await;
    let csrf = field(&acct, "_csrf");
    let (st, h, made) = send(&app, post("/account/token", &ada, format!("_csrf={csrf}&name=Laptop"))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(h.get("cache-control").unwrap(), "no-store");
    let token = made.split("not shown again):").nth(1).unwrap().split("<code>").nth(1).unwrap().split("</code>").next().unwrap().to_string();
    assert!(token.starts_with("bk_") && made.contains("Laptop"));
    let api = |t: &str| Request::post("/api/v1/commands/project.list").header("authorization", format!("Bearer {t}")).body(Body::empty()).unwrap();
    assert_eq!(send(&app, api(&token)).await.0, StatusCode::OK);
    let (_, _, again) = send(&app, Request::get("/account").header("cookie", &ada).body(Body::empty()).unwrap()).await;
    assert!(again.contains("Laptop") && !again.contains(&token), "the secret is never shown again");
    let id = again.split("name=\"id\" value=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
    let (_, h, _) = send(&app, post("/account/token/revoke", &ada, format!("_csrf={csrf}&id=nope"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="));
    send(&app, post("/account/token/revoke", &ada, format!("_csrf={csrf}&id={id}"))).await;
    assert_eq!(send(&app, api(&token)).await.0, StatusCode::UNAUTHORIZED);
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn pdf_downloads_for_documents_and_lists() {
    let dir = temp("pdfs");
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
    send(
        &app,
        post("/do/rfi.create", &ada, format!("_csrf={csrf}&project=1&subject=Door+hardware&question=Which+hinge%3F&assignees%5B%5D=1&open=true")),
    )
    .await;
    send(&app, post("/do/task.create", &ada, format!("_csrf={csrf}&project=1&title=Order+hardware"))).await;
    send(&app, post("/do/schedule.add", &ada, format!("_csrf={csrf}&project=1&name=Excavate&start=2026-10-12&days=3"))).await;
    send(&app, post("/do/meeting.create", &ada, format!("_csrf={csrf}&project=1&title=Kickoff&date=2026-10-20"))).await;
    for (uri, want) in [
        ("/projects/1/rfis/1/pdf", "rfi-1.pdf"),
        ("/projects/1/tasks.pdf", "tasks.pdf"),
        ("/projects/1/schedule.pdf", "schedule.pdf"),
        ("/projects/1/schedule/lookahead.pdf?weeks=2&from=2026-10-12", "look-ahead.pdf"),
        ("/projects/1/budget.pdf", "budget.pdf"),
        ("/projects/1/reports.pdf", "project-report.pdf"),
        ("/projects/1/drawings.pdf", "drawing-log.pdf"),
        ("/projects/1/meetings/1/pdf", "meeting-1.pdf"),
    ] {
        let (st, h, body) = send(&app, get(uri)).await;
        assert_eq!(st, StatusCode::OK, "{uri}");
        assert_eq!(h.get("content-type").unwrap(), "application/pdf", "{uri}");
        assert!(h.get("content-disposition").unwrap().to_str().unwrap().contains(want), "{uri}: {h:?}");
        assert!(body.starts_with("%PDF-1.4") && body.trim_end().ends_with("%%EOF"), "{uri}");
    }
    let (st, ..) = send(&app, get("/projects/1/rfis/99/pdf")).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, ..) = send(&app, Request::get("/projects/1/rfis/1/pdf").body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::SEE_OTHER, "signed-out people don't get documents");
    let (_, _, page) = send(&app, get("/projects/1/rfis/1")).await;
    assert!(page.contains("/projects/1/rfis/1/pdf"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_forms_post_to_the_save_route() {
    assert_eq!(save_action("/projects/1/daily-logs/new"), "/projects/1/daily-logs/save");
    assert_eq!(save_action("/projects/1/commitments/7/edit"), "/projects/1/commitments/7/save");
    assert_eq!(save_action("/projects/1/x"), "/projects/1/x");
}

#[tokio::test]
async fn refused_saves_keep_what_was_typed() {
    let dir = temp("drafts");
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

    // A generic form: a refused RFI keeps its subject, question and priority.
    let (_, h, _) = send(
        &app,
        post(
            "/do/rfi.create",
            &ada,
            format!("_csrf={csrf}&_next=/projects/1/rfis/%7Bid%7D&_back=/projects/1/rfis/new&project=1&subject=Door+%22hardware%22&question=Which+hinge%3F&due=not-a-date"),
        ),
    )
    .await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(loc.starts_with("/projects/1/rfis/new?") && loc.contains("error=") && loc.contains("draft="), "{loc}");
    let (_, _, form) = send(&app, get(&loc)).await;
    assert!(form.contains("Door &quot;hardware&quot;") && form.contains("Which hinge?"), "{form}");

    // Someone else, or a made-up marker, sees an empty form.
    let (_, _, plain_form) = send(&app, get("/projects/1/rfis/new?draft=nope")).await;
    assert!(!plain_form.contains("Which hinge?"));
    // The CSRF token and password-like fields are never replayed from a draft.
    assert!(form.contains(&format!("value=\"{csrf}\"")) || form.contains("name=\"_csrf\""));

    // A custom save form: a refused daily log keeps its text.
    let (_, h, _) = send(
        &app,
        post(
            "/projects/1/daily-logs/save",
            &ada,
            format!("_csrf={csrf}&date=2026-10-12&sky=rain&high=abc&low=&weather_notes=&work_performed=Poured+footings&delays=&safety=&notes="),
        ),
    )
    .await;
    let loc = h.get(LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(loc.contains("draft="), "{loc}");
    let (_, _, form) = send(&app, get(&loc)).await;
    assert!(form.contains("Poured footings") && form.contains("value=\"abc\""), "{form}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A minimal SMTP server that accepts every message and returns what it received.
fn fake_smtp() -> (u16, std::sync::mpsc::Receiver<String>) {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let mut out = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let _ = out.write_all(b"220 fake ready\r\n");
            let (mut data, mut in_data, mut got) = (false, false, String::new());
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if in_data {
                    if line == ".\r\n" {
                        in_data = false;
                        data = true;
                        let _ = out.write_all(b"250 queued\r\n");
                    } else {
                        got.push_str(&line);
                    }
                    continue;
                }
                let up = line.to_ascii_uppercase();
                let reply: &[u8] = if up.starts_with("EHLO") {
                    b"250 fake\r\n"
                } else if up.starts_with("DATA") {
                    in_data = true;
                    b"354 go\r\n"
                } else if up.starts_with("QUIT") {
                    let _ = out.write_all(b"221 bye\r\n");
                    break;
                } else {
                    b"250 ok\r\n"
                };
                let _ = out.write_all(reply);
            }
            if data {
                let _ = tx.send(got);
            }
        }
    });
    (port, rx)
}

#[tokio::test]
async fn notifications_are_emailed() {
    let dir = temp("email");
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

    // Nothing is configured: the page asks, a bad form is refused, a notification waits.
    let (_, _, page) = send(&app, get("/admin/email", &ada)).await;
    assert!(page.contains("smtp.example.com"));
    let (port, rx) = fake_smtp();
    let form = |host: &str, from: &str| {
        format!("_csrf={csrf}&host={host}&port={port}&security=none&username=&password=&from={from}&base_url=https%3A%2F%2Fbk.example.com%2F")
    };
    let (_, h, _) = send(&app, post("/admin/email", &ada, form("127.0.0.1", "not+an+address"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="));
    let (_, h, _) = send(&app, post("/admin/email", &ada, form("127.0.0.1", "BuildKiosk+%3Cnoreply%40example.com%3E"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    assert_eq!(app.lock().creds.smtp.as_ref().unwrap().base_url, "https://bk.example.com");

    // The test button reaches the server.
    let (_, h, _) = send(&app, post("/admin/email/test", &ada, format!("_csrf={csrf}"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{:?}", h.get(LOCATION));
    let got = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(got.contains("BuildKiosk test email") && got.contains("ada@example.com"), "{got}");

    // A notification for Bo goes out once, with a link; turning email off stops it.
    send(&app, run("spec.import", "project=1&text=05+12+00%2CStructural+steel".into())).await;
    send(&app, run("submittal.import", "project=1&text=title%2Cspec_section%0AAnchor+bolts%2C05+12+00".into())).await;
    send(&app, run("submittal.update", "project=1&id=1&reviewers%5B%5D=2&ordered=true".into())).await;
    send(&app, run("submittal.open", "project=1&id=1".into())).await;
    let a = app.clone();
    assert_eq!(tokio::task::spawn_blocking(move || mail_pass(&a)).await.unwrap(), 1);
    let got = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(got.contains("bo@example.com") && got.contains("https://bk.example.com/projects/1/submittals/1"), "{got}");
    let a = app.clone();
    assert_eq!(tokio::task::spawn_blocking(move || mail_pass(&a)).await.unwrap(), 0, "already sent");

    let (_, h, page) = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let login = format!("_csrf={}&email=bo%40example.com&password=bo-secret-password", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/login", &anon, login)).await;
    let bo = cookies(&h);
    let (_, _, acct) = send(&app, get("/account", &bo)).await;
    let bo_csrf = field(&acct, "_csrf");
    send(&app, post("/account/email", &bo, format!("_csrf={bo_csrf}"))).await;
    assert!(app.lock().creds.email_off.contains(&2));
    // Only admins reach the settings.
    let (_, h, _) = send(&app, get("/admin/email", &bo)).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with("/account?"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn sheet_viewer_markup_and_compare() {
    let dir = temp("viewer");
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

    // Two revisions of a sheet, as real PDFs with a box in different places.
    let upload = |uri: &str, fields: &[(&str, &str)], pdf: Vec<u8>| {
        let mut body: Vec<u8> = Vec::new();
        for (k, v) in fields {
            body.extend_from_slice(format!("--B\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
        }
        body.extend_from_slice(
            b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a101.pdf\"\r\nContent-Type: application/pdf\r\n\r\n",
        );
        body.extend_from_slice(&pdf);
        body.extend_from_slice(b"\r\n--B--\r\n");
        Request::post(uri.to_string())
            .header("cookie", &ada)
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(body))
            .unwrap()
    };
    let (_, h, _) = send(
        &app,
        upload(
            "/projects/1/drawings/upload",
            &[("_csrf", &csrf), ("number", "A-101"), ("title", "Plan"), ("revision", "A")],
            render::tests::tiny_pdf(50),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));
    let (_, h, _) = send(&app, upload("/projects/1/drawings/1/upload", &[("_csrf", &csrf), ("revision", "B")], render::tests::tiny_pdf(250))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="));

    let (_, _, sheet) = send(&app, get("/projects/1/drawings/1")).await;
    assert!(sheet.contains("Open in viewer"));
    let (st, _, viewer) = send(&app, get("/projects/1/drawings/1/view")).await;
    assert!(
        st == StatusCode::OK && viewer.contains("Sheet viewer") && viewer.contains("view.png?page=1&amp;w=900") && viewer.contains("Compare with"),
        "{viewer}"
    );
    // The picture is a real PNG of the right shape, and signed-out people don't get it.
    let res = router(app.clone()).oneshot(get("/projects/1/drawings/1/view.png?page=1&w=800")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(CONTENT_TYPE).unwrap(), "image/png");
    let png = res.into_body().collect().await.unwrap().to_bytes();
    let img = image::load_from_memory(&png).unwrap();
    assert_eq!((img.width(), img.height()), (800, 600));
    let (st, ..) = send(&app, Request::get("/projects/1/drawings/1/view.png").body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    let (st, ..) = send(&app, get("/projects/1/drawings/1/view.png?rev=nope")).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, ..) = send(&app, get("/projects/1/drawings/1/view.png?page=9")).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // Clicking with the pin tool turns pixels into a position and offers the form.
    let (_, _, pinning) = send(&app, get("/projects/1/drawings/1/view?tool=pin&w=900&at.x=450&at.y=675")).await;
    assert!(
        pinning.contains("Add a pin") && pinning.contains("name=\"x\" value=\"5000\"") && pinning.contains("name=\"y\" value=\"10000\""),
        "{pinning}"
    );
    let (_, _, rect1) = send(&app, get("/projects/1/drawings/1/view?tool=rect&w=900&at.x=90&at.y=0")).await;
    assert!(
        rect1.contains("Click the opposite corner")
            && rect1.contains("name=\"x1\" value=\"1000\"")
            && rect1.contains("name=\"y1\" value=\"0\"")
            && !rect1.contains("Add a box"),
        "{rect1}"
    );
    let (_, _, rect2) = send(&app, get("/projects/1/drawings/1/view?tool=rect&w=900&x1=1000&y1=0&at.x=450&at.y=675")).await;
    assert!(
        rect2.contains("Add a box")
            && rect2.contains("name=\"x2\" value=\"5000\"")
            && rect2.contains("name=\"y2\" value=\"10000\"")
            && rect2.contains("name=\"x\" value=\"1000\""),
        "{rect2}"
    );
    let (_, h, _) = send(&app, post("/do/drawing.markup_add", &ada, format!("_csrf={csrf}&_next=/projects/1/drawings/1/view%3Ftool%3Dpin&project=1&id=1&revision=B&page=1&kind=pin&x=5000&y=5000&text=Check+this+door"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, marked) = send(&app, get("/projects/1/drawings/1/view")).await;
    assert!(marked.contains("<circle cx=\"450\" cy=\"338\"") && marked.contains("Check this door") && marked.contains("Resolve"), "{marked}");
    let (_, _, other_rev) = send(&app, get("/projects/1/drawings/1/view?rev=A")).await;
    // Revision B's markup is on A's page only in a hidden list (the browser viewer switches between them), and the overlay shows none.
    assert!(other_rev.contains("<div class=\"marklist\" data-rev=\"B\" data-page=\"1\" hidden>"), "markups belong to their revision");
    assert!(!other_rev.contains("<circle cx=\"450\" cy=\"338\""), "no overlay for another revision");
    assert!(marked.contains("<div class=\"marklist\" data-rev=\"B\" data-page=\"1\">"), "{marked}");

    // The browser viewer (WebAssembly) is offered only by a build that carries it, and only its own
    // page may run scripts.
    let csp = |h: &HeaderMap| h.get("content-security-policy").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let (_, vh, vpage) = send(&app, get("/projects/1/drawings/1/view")).await;
    let (st, ..) = send(&app, get("/static/viewer/viewer_bg.wasm")).await;
    let (_, ph, _) = send(&app, get("/projects/1/drawings")).await;
    assert!(csp(&ph).contains("script-src 'none'"), "{}", csp(&ph));
    if viewer_assets::START_JS.is_some() {
        assert_eq!(st, StatusCode::OK);
        // Compressed for browsers that accept it, plain for the rest.
        let zipped = router(app.clone())
            .oneshot(Request::get("/static/viewer/viewer_bg.wasm").header("accept-encoding", "br, gzip;q=0.8").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(zipped.headers().get("content-encoding").and_then(|v| v.to_str().ok()), Some("gzip"));
        let zipped_len = zipped.into_body().collect().await.unwrap().to_bytes().len();
        let raw = router(app.clone()).oneshot(get("/static/viewer/viewer_bg.wasm")).await.unwrap();
        assert!(raw.headers().get("content-encoding").is_none());
        let raw = raw.into_body().collect().await.unwrap().to_bytes();
        assert!(raw.starts_with(b"\0asm") && zipped_len < raw.len() / 2, "{zipped_len} vs {}", raw.len());
        assert!(
            vpage.contains("id=\"wasm-viewer\"")
                && vpage.contains("data-sheet=")
                && vpage.contains("/projects/1/drawings/1/files/")
                && vpage.contains("class=\"chip cmp-btn\"")
                && vpage.contains("class=\"marklist\""),
            "{vpage}"
        );
        assert!(vpage.contains("&quot;kind&quot;:&quot;pin&quot;") && vpage.contains("&quot;can_mark&quot;:true"), "{vpage}");
        assert!(vpage.contains("src=\"/static/viewer/start.js\"") && vpage.contains("id=\"mk-x\""), "{vpage}");
        assert!(csp(&vh).contains("script-src 'self' 'wasm-unsafe-eval'") && !csp(&vh).contains("unsafe-inline"), "{}", csp(&vh));
        // A comparison is a picture the server draws, so it keeps the strict policy.
        let (_, ch, cpage) = send(&app, get("/projects/1/drawings/1/view?compare=A")).await;
        assert!(csp(&ch).contains("script-src 'none'") && !cpage.contains("wasm-viewer"));
    } else {
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert!(!vpage.contains("wasm-viewer") && csp(&vh).contains("script-src 'none'"), "{vpage}");
    }

    // Compare: the old position is blue, the new one red.
    let res = router(app.clone()).oneshot(get("/projects/1/drawings/1/compare.png?a=A&b=B&page=1&w=800")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let png = res.into_body().collect().await.unwrap().to_bytes();
    let diff = image::load_from_memory(&png).unwrap().to_rgb8();
    assert_eq!((diff.get_pixel(140, 340).0, diff.get_pixel(540, 340).0), ([30, 90, 220], [220, 30, 30]));
    let (_, _, cmp) = send(&app, get("/projects/1/drawings/1/view?compare=A")).await;
    assert!(cmp.contains("compare.png?a=A&amp;b=B") && cmp.contains("only on revision B"), "{cmp}");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn scheduled_backups_run_while_the_server_is_up_and_keep_the_newest() {
    let dir = temp("sched-backup");
    let backups = temp("sched-backup-out");
    let app = App::open(DataDir::open(&dir).unwrap(), None).unwrap();
    assert!(app.with_backups(dir.join("inside"), 24, 3).is_err(), "not inside the data directory");
    let app = App::open(DataDir::open(temp("sched-backup-other")).unwrap(), None).unwrap();
    assert!(app.with_backups(backups.clone(), 0, 3).is_err());
    let app = App::open(DataDir::open(temp("sched-backup-other")).unwrap(), None).unwrap();
    assert!(app.with_backups(backups.clone(), 24, 0).is_err());
    let _ = std::fs::remove_dir_all(temp("sched-backup-other"));
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap().with_backups(backups.clone(), 24, 2).unwrap());
    // Set the company up through the browser so there is something to back up.
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    send(&app, post("/setup", &anon, body)).await;

    let day = 86_400;
    let pass = |now: u64| {
        let a = app.clone();
        tokio::task::spawn_blocking(move || backup_pass(&a, now))
    };
    // Something this scheduler didn't make is never touched; an interrupted backup is cleared.
    std::fs::create_dir_all(backups.join("my-notes")).unwrap();
    std::fs::create_dir_all(backups.join("backup-5.partial")).unwrap();
    let first = pass(1_000_000).await.unwrap().unwrap().unwrap();
    assert!(first.ends_with("backup-1000000") && !backups.join("backup-5.partial").exists() && backups.join("my-notes").is_dir());
    // It loads, with the company in it, and the live server still runs.
    let copy = DataDir::open(&first).unwrap();
    assert_eq!(copy.companies().unwrap().len(), 1);
    drop(copy);
    let (st, _, _) = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    // Not due again until the interval has passed.
    assert!(pass(1_000_000 + day - 1).await.unwrap().unwrap().is_none());
    assert!(pass(1_000_000 + day).await.unwrap().unwrap().is_some());
    assert!(pass(1_000_000 + 2 * day).await.unwrap().unwrap().is_some());
    // Only the newest two are kept.
    let mut left: Vec<String> = std::fs::read_dir(&backups).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(left, ["backup-1086400", "backup-1172800", "my-notes"]);
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&backups);
}

#[tokio::test]
async fn admins_back_up_now_and_each_backup_is_copied_to_a_second_folder() {
    let dir = temp("now-backup");
    let backups = temp("now-backup-out");
    let second = temp("now-backup-second");
    let plain = App::open(DataDir::open(temp("now-backup-plain")).unwrap(), None).unwrap();
    assert!(plain.with_backup_copy(second.clone()).is_err(), "a second copy needs backups");
    let app = App::open(DataDir::open(&dir).unwrap(), None).unwrap().with_backups(backups.clone(), 24, 2).unwrap();
    assert!(app.with_backup_copy(backups.join("nested")).is_err(), "not inside the backup folder");
    let app = App::open(DataDir::open(&dir).unwrap(), None).unwrap().with_backups(backups.clone(), 24, 2).unwrap();
    assert!(app.with_backup_copy(dir.join("copy")).is_err(), "not inside the data directory");
    let app = Arc::new(
        App::open(DataDir::open(&dir).unwrap(), None)
            .unwrap()
            .with_backups(backups.clone(), 24, 2)
            .unwrap()
            .with_backup_copy(second.clone())
            .unwrap(),
    );
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");

    let (st, ..) = send(&app, Request::get("/admin/backups").body(Body::empty()).unwrap()).await;
    assert_eq!(st, StatusCode::SEE_OTHER, "signed-out people are sent to sign in");
    let (_, _, empty) = send(&app, get("/admin/backups")).await;
    assert!(empty.contains("Back up now") && empty.contains("None yet"), "{empty}");
    let (st, ..) = send(&app, post("/admin/backups/run", &ada, "_csrf=bad".to_string())).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (_, h, _) = send(&app, post("/admin/backups/run", &ada, format!("_csrf={csrf}"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let count =
        |d: &std::path::Path| std::fs::read_dir(d).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("backup-")).count();
    assert_eq!((count(&backups), count(&second)), (1, 1));
    assert_eq!(DataDir::open(std::fs::read_dir(&second).unwrap().flatten().next().unwrap().path()).unwrap().companies().unwrap().len(), 1);
    let (_, _, listed) = send(&app, get("/admin/backups")).await;
    assert!(listed.contains("Copied") && listed.contains("backup-"), "{listed}");
    let (_, _, account) = send(&app, get("/account")).await;
    assert!(account.contains("/admin/backups"), "{account}");
    drop(app);
    for d in [&dir, &backups, &second] {
        let _ = std::fs::remove_dir_all(d);
    }
    let _ = std::fs::remove_dir_all(temp("now-backup-plain"));
}

fn form_has_task_fields(form: &str) -> bool {
    ["name=\"repeat\"", "name=\"repeat_until\"", "name=\"rfi\"", "name=\"meeting\""].iter().all(|f| form.contains(f))
}

#[tokio::test]
async fn mcp_over_http_can_follow_a_resource_and_stream_the_changes() {
    let dir = temp("mcpsub");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    send(&app, post("/setup", &anon, body)).await;
    let (token, v1, v2) = {
        let mut inner = app.lock();
        let person = inner.tenant.as_ref().unwrap().engine.state().people.iter().next().unwrap().id.raw();
        let t = inner.creds.create_token(person, "agent", Timestamp::from_millis(now_ms())).unwrap();
        let blobs = &inner.tenant.as_ref().unwrap().store.blobs;
        (t, blobs.put(b"first").unwrap(), blobs.put(b"second").unwrap())
    };
    let rpc = |session: Option<&str>, msg: String| {
        let mut r = Request::post("/mcp").header("content-type", "application/json").header("authorization", format!("Bearer {token}"));
        if let Some(s) = session {
            r = r.header("mcp-session-id", s);
        }
        r.body(Body::from(msg)).unwrap()
    };
    let call = |id: u32, name: &str, args: Value| {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string()
    };
    send(&app, rpc(None, call(1, "project_create", json!({"name": "P"})))).await;
    let (_, _, out) = send(
        &app,
        rpc(
            None,
            call(2, "document_add", json!({"project": 1, "title": "Notes", "sha256": v1.0, "size": v1.1, "name": "n.txt", "mime": "text/plain"})),
        ),
    )
    .await;
    assert!(!out.contains("\"isError\":true"), "{out}");

    // Without a session there are no subscriptions; initialize starts one and names it in a header.
    let sub = json!({"jsonrpc": "2.0", "id": 3, "method": "resources/subscribe", "params": {"uri": "buildkiosk://document/1/1"}}).to_string();
    let (_, _, out) = send(&app, rpc(None, sub.clone())).await;
    assert!(out.contains("-32601"), "{out}");
    let (st, h, out) = send(&app, rpc(None, json!({"jsonrpc": "2.0", "id": 4, "method": "initialize", "params": {}}).to_string())).await;
    let session = h.get("mcp-session-id").unwrap().to_str().unwrap().to_string();
    assert!(st == StatusCode::OK && out.contains("\"subscribe\":true") && session.len() == 32, "{out}");
    let (_, _, out) = send(&app, rpc(Some(&session), sub)).await;
    assert!(out.contains("\"result\":{}"), "{out}");
    // A made-up session is gone, not silently stateless.
    let (st, ..) = send(&app, rpc(Some("nope"), json!({"jsonrpc": "2.0", "id": 5, "method": "ping"}).to_string())).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // The stream reports a change made by anyone: here, a new version through another connection.
    let stream =
        Request::get("/mcp").header("authorization", format!("Bearer {token}")).header("mcp-session-id", &session).body(Body::empty()).unwrap();
    let res = router(app.clone()).oneshot(stream).await.unwrap();
    assert_eq!(res.headers().get(CONTENT_TYPE).unwrap(), "text/event-stream");
    let mut body = res.into_body();
    let (_, _, out) = send(
        &app,
        rpc(
            None,
            call(6, "document_add_version", json!({"project": 1, "id": 1, "sha256": v2.0, "size": v2.1, "name": "n.txt", "mime": "text/plain"})),
        ),
    )
    .await;
    assert!(!out.contains("\"isError\":true"), "{out}");
    let frame = tokio::time::timeout(std::time::Duration::from_secs(10), body.frame()).await.expect("a frame within 10 seconds").unwrap().unwrap();
    let text = String::from_utf8_lossy(frame.data_ref().unwrap()).into_owned();
    assert!(text.starts_with("data: ") && text.contains("notifications/resources/updated") && text.contains("buildkiosk://document/1/1"), "{text}");
    drop(body);

    // The stream needs the connection's own session and token; DELETE ends the connection.
    let other = Request::get("/mcp").header("authorization", format!("Bearer {token}")).header("mcp-session-id", "nope").body(Body::empty()).unwrap();
    assert_eq!(send(&app, other).await.0, StatusCode::NOT_FOUND);
    let anon_stream = Request::get("/mcp").header("mcp-session-id", &session).body(Body::empty()).unwrap();
    assert_eq!(send(&app, anon_stream).await.0, StatusCode::UNAUTHORIZED);
    let del =
        Request::delete("/mcp").header("authorization", format!("Bearer {token}")).header("mcp-session-id", &session).body(Body::empty()).unwrap();
    assert_eq!(send(&app, del).await.0, StatusCode::NO_CONTENT);
    let (st, ..) = send(&app, rpc(Some(&session), json!({"jsonrpc": "2.0", "id": 7, "method": "ping"}).to_string())).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn each_backup_is_uploaded_to_object_storage_and_the_page_says_how_it_went() {
    let dir = temp("s3-backup");
    let backups = temp("s3-backup-out");
    let (endpoint, rx) = crate::s3::tests::serve("200 OK");
    let target = crate::s3::S3Target::new(&endpoint, "bucket", "us-east-1", "bk", "AK", "SK", true).unwrap();
    let plain = App::open(DataDir::open(temp("s3-backup-plain")).unwrap(), None).unwrap();
    assert!(plain.with_backup_object_store(target.clone()).is_err(), "uploads need backups");
    let app = Arc::new(
        App::open(DataDir::open(&dir).unwrap(), None)
            .unwrap()
            .with_backups(backups.clone(), 24, 2)
            .unwrap()
            .with_backup_object_store(target)
            .unwrap(),
    );
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    let (_, _, before) = send(&app, get("/admin/backups")).await;
    assert!(before.contains("Nothing uploaded since the server started") && before.contains("/bucket/bk/"), "{before}");
    let (_, h, _) = send(&app, post("/admin/backups/run", &ada, format!("_csrf={csrf}"))).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let sent: Vec<_> = rx.try_iter().collect();
    assert!(
        sent.len() >= 2 && sent.iter().all(|(path, ..)| path.starts_with("/bucket/bk/backup-")),
        "{:?}",
        sent.iter().map(|s| &s.0).collect::<Vec<_>>()
    );
    assert!(sent.iter().any(|(path, ..)| path.ends_with("/instance.json")));
    let (_, _, after) = send(&app, get("/admin/backups")).await;
    assert!(after.contains("files uploaded"), "{after}");
    drop(app);
    for d in [&dir, &backups] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[tokio::test]
async fn pages_come_out_in_the_language_chosen() {
    let dir = temp("language");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str, extra: &str| Request::get(uri.to_string()).header("cookie", format!("{ada}{extra}")).body(Body::empty()).unwrap();

    // English by default, with the choice offered and the one in use marked.
    let (_, h, en) = send(&app, get("/", "")).await;
    assert!(en.contains("<html lang=\"en\">") && en.contains(">Projects<") && h.get("content-language").is_none(), "{en}");
    assert!(en.contains("href=\"/language?to=en\" aria-current=\"true\"") && en.contains(">Español<") && en.contains(">Nederlands<"), "{en}");

    // The cookie wins; the browser's preference is the fallback.
    let (_, h, es) = send(&app, get("/", "; bk_lang=es")).await;
    assert!(es.contains("<html lang=\"es\">") && es.contains(">Proyectos<") && !es.contains(">Projects<"), "{es}");
    assert_eq!(h.get("content-language").unwrap(), "es");
    assert!(es.contains("href=\"/language?to=es\" aria-current=\"true\"") && es.contains(">English<"), "{es}");
    let by_header = Request::get("/").header("cookie", &ada).header("accept-language", "es-MX,es;q=0.9,en;q=0.5").body(Body::empty()).unwrap();
    assert!(send(&app, by_header).await.2.contains(">Proyectos<"));
    let (_, _, back_to_en) = send(&app, get("/", "; bk_lang=en")).await;
    assert!(back_to_en.contains(">Projects<"));

    // Choosing sets the cookie and returns to the page the link was on; nowhere else.
    let choose = |to: &str, referer: &str| Request::get(format!("/language?to={to}")).header("referer", referer).body(Body::empty()).unwrap();
    let (st, h, _) = send(&app, choose("fr", "http://localhost:8080/projects/1/rfis?status=open")).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert_eq!(h.get(LOCATION).unwrap(), "/projects/1/rfis?status=open");
    let set = h.get(SET_COOKIE).unwrap().to_str().unwrap();
    assert!(set.starts_with("bk_lang=fr;") && set.contains("Max-Age=31536000") && set.contains("SameSite=Lax"), "{set}");
    for referer in ["http://x/language?to=es", "http://x//evil.example", "nonsense", ""] {
        let (_, h, _) = send(&app, choose("nl", referer)).await;
        assert_eq!(h.get(LOCATION).unwrap(), "/", "{referer}");
    }
    let (_, h, _) = send(&app, choose("tlh", "http://x/a")).await;
    assert_eq!((h.get(LOCATION).unwrap().to_str().unwrap(), h.get(SET_COOKIE).is_none()), ("/", true));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bidders_use_their_links_without_an_account() {
    let dir = temp("portal");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let bare = |uri: &str| Request::get(uri.to_string()).body(Body::empty()).unwrap();
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;
    send(&app, post("/do/vendor.create", &ada, format!("_csrf={csrf}&name=Keystone+Concrete&kind=subcontractor"))).await;
    send(&app, post("/do/vendor.create", &ada, format!("_csrf={csrf}&name=Northline+Concrete&kind=subcontractor"))).await;
    send(&app, post("/do/bid_package.create", &ada, format!("_csrf={csrf}&project=1&title=Concrete&due=2099-01-01&scope=Slab&invite%5B%5D=1"))).await;
    for (v, e) in [(1, "bids%40keystone.example"), (2, "bids%40northline.example")] {
        send(&app, post("/do/bid_package.invite", &ada, format!("_csrf={csrf}&project=1&id=1&vendor={v}&email={e}"))).await;
    }
    send(&app, post("/do/bid_package.open", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let slug = app.lock().tenant.as_ref().unwrap().slug.clone();
    let link = |n: u32, g: u32| portal::token(app.secret(), &slug, 1, n, g);
    let (a, b) = (link(1, 0), link(2, 0));

    // The link opens the package for that company: no sign-in, no cookie, nothing about the other company.
    let (st, h, page) = send(&app, bare(&format!("/bid/{a}"))).await;
    assert_eq!(st, StatusCode::OK);
    assert!(page.contains("Keystone Concrete") && page.contains("Concrete") && !page.contains("Northline"), "{page}");
    assert!(h.get(SET_COOKIE).is_none());
    assert_eq!(h.get("referrer-policy").unwrap(), "no-referrer");
    assert_eq!(h.get("cache-control").unwrap(), "no-store");
    assert!(h.get("content-security-policy").unwrap().to_str().unwrap().contains("script-src 'none'"));

    // Anything else looks the same: forged, wrong generation, wrong company, nonsense.
    let tampered = format!("{}x", &a[..a.len() - 1]);
    let wrong_generation = link(1, 4);
    let mut answers = Vec::new();
    for bad in [tampered.as_str(), wrong_generation.as_str(), "garbage", "1.1.0.", "99.1.0.abc"] {
        let (st, _, body) = send(&app, bare(&format!("/bid/{bad}"))).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{bad}");
        answers.push(body);
    }
    assert!(answers.windows(2).all(|w| w[0] == w[1]));
    assert!(answers[0].contains("doesn't work") && !answers[0].contains("Keystone"));
    // A link signed for another company's server isn't ours either.
    let foreign = portal::token(app.secret(), "someone-else", 1, 1, 0);
    assert_eq!(send(&app, bare(&format!("/bid/{foreign}"))).await.0, StatusCode::NOT_FOUND);

    // Sending a bid: needs the form token, and lands sealed.
    let form = field(&page, "_csrf");
    let post_bare = |uri: &str, body: String| {
        Request::post(uri.to_string()).header("content-type", "application/x-www-form-urlencoded").body(Body::from(body)).unwrap()
    };
    let (st, ..) = send(&app, post_bare(&format!("/bid/{a}/submit"), "_csrf=nope&amount=98000.50".into())).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, ..) = send(&app, post_bare(&format!("/bid/{b}/submit"), format!("_csrf={form}&amount=98000.50"))).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "a form token belongs to its own link");
    let (st, h, _) = send(&app, post_bare(&format!("/bid/{a}/submit"), format!("_csrf={form}&amount=98000.50&notes=Excludes+rebar"))).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().starts_with(&format!("/bid/{a}?ok=")), "{h:?}");
    assert_eq!(h.get("referrer-policy").unwrap(), "no-referrer");
    {
        let inner = app.lock();
        let pkg = inner.tenant.as_ref().unwrap().engine.state().bid_packages.iter().next().unwrap().clone();
        assert_eq!((pkg.sealed_bids.len(), pkg.bids.len()), (1, 0));
    }
    // The bidder sees their own bid; the team sees that one arrived but not what it says.
    let (_, _, again) = send(&app, bare(&format!("/bid/{a}"))).await;
    assert!(again.contains("98000.50") && again.contains("Withdraw"));
    let (_, _, detail) = send(&app, get("/projects/1/bidding/1")).await;
    assert!(detail.contains("sealed until bidding closes") && !detail.contains("98,000.50") && !detail.contains("Excludes rebar"), "{detail}");

    // The team can copy a link; sending it again replaces it and the old one stops working at once.
    let (st, _, shown) = send(&app, get("/projects/1/bidding/1/links/1")).await;
    assert!(st == StatusCode::OK && shown.contains(&format!("/bid/{a}")));
    assert_eq!(send(&app, bare("/projects/1/bidding/1/links/1")).await.0, StatusCode::SEE_OTHER, "the team page needs a sign-in");
    send(&app, post("/do/bid_package.resend_invite", &ada, format!("_csrf={csrf}&project=1&id=1&number=1"))).await;
    assert_eq!(send(&app, bare(&format!("/bid/{a}"))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(send(&app, bare(&format!("/bid/{}", link(1, 1)))).await.0, StatusCode::OK);
    send(&app, post("/do/bid_package.revoke_invite", &ada, format!("_csrf={csrf}&project=1&id=1&number=2"))).await;
    assert_eq!(send(&app, bare(&format!("/bid/{b}"))).await.0, StatusCode::NOT_FOUND);

    // Links go out by email only with outgoing mail set up and the server's public address known.
    assert!(portal::pending(&app).is_none());
    app.lock().creds.smtp = Some(buildkiosk_store::creds::SmtpSettings {
        host: "mail.example".into(),
        port: 25,
        security: "none".into(),
        from: "bids@example.com".into(),
        base_url: "https://bids.example".into(),
        ..Default::default()
    });
    let (_, out) = portal::pending(&app).unwrap();
    assert_eq!(out.len(), 1, "the cancelled link is not sent");
    assert_eq!((out[0].number, out[0].generation, out[0].mail.to.as_str()), (1, 1, "bids@keystone.example"));
    assert!(out[0].mail.body.contains(&format!("https://bids.example/bid/{}", link(1, 1))), "{}", out[0].mail.body);
    assert!(!out[0].mail.body.contains(&link(1, 0)));

    // Questions: asked on the bidder's page, answered by the team, shared without the company's name.
    let q1 = link(1, 1);
    let (_, _, page) = send(&app, bare(&format!("/bid/{q1}"))).await;
    assert!(page.contains("Ask the team a question"));
    let form = field(&page, "_csrf");
    let (st, h, _) = send(&app, post_bare(&format!("/bid/{q1}/ask"), format!("_csrf={form}&question=What+depth+are+the+footings%3F"))).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (st, ..) = send(&app, post_bare(&format!("/bid/{q1}/ask"), "_csrf=nope&question=x".into())).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (_, _, detail) = send(&app, get("/projects/1/bidding/1")).await;
    assert!(
        detail.contains("Questions from bidders") && detail.contains("What depth are the footings?") && detail.contains("Keystone Concrete"),
        "{detail}"
    );
    send(
        &app,
        post(
            "/do/bid_package.answer_question",
            &ada,
            format!("_csrf={csrf}&project=1&id=1&number=1&text=What+depth%3F&answer=Four+feet&publish=false&publish=true&addendum=false"),
        ),
    )
    .await;
    let (_, _, mine) = send(&app, bare(&format!("/bid/{q1}"))).await;
    assert!(mine.contains("Your question") && mine.contains("Four feet"), "{mine}");

    // A link posts only so often.
    let fresh = link(1, 1);
    let (_, _, page) = send(&app, bare(&format!("/bid/{fresh}"))).await;
    let form = field(&page, "_csrf");
    let mut last = StatusCode::OK;
    for _ in 0..12 {
        last = send(&app, post_bare(&format!("/bid/{fresh}/submit"), format!("_csrf={form}&amount=90000"))).await.0;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

fn multipart(boundary: &str, fields: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, filename, data) in fields {
        out.extend(format!("--{boundary}\r\n").as_bytes());
        match filename {
            Some(f) => out.extend(
                format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"\r\nContent-Type: application/pdf\r\n\r\n").as_bytes(),
            ),
            None => out.extend(format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes()),
        }
        out.extend(*data);
        out.extend(b"\r\n");
    }
    out.extend(format!("--{boundary}--\r\n").as_bytes());
    out
}

#[tokio::test]
async fn bid_files_receipts_and_the_open_link() {
    let dir = temp("portal-more");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let ada = cookies(&h);
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &ada).body(Body::empty()).unwrap();
    let bare = |uri: &str| Request::get(uri.to_string()).body(Body::empty()).unwrap();
    let post_bare = |uri: &str, body: String| {
        Request::post(uri.to_string()).header("content-type", "application/x-www-form-urlencoded").body(Body::from(body)).unwrap()
    };
    let (_, _, home) = send(&app, get("/")).await;
    let csrf = field(&home, "_csrf");
    send(&app, post("/do/project.create", &ada, format!("_csrf={csrf}&name=P"))).await;
    send(&app, post("/do/vendor.create", &ada, format!("_csrf={csrf}&name=Keystone+Concrete&kind=subcontractor"))).await;
    send(&app, post("/do/bid_package.create", &ada, format!("_csrf={csrf}&project=1&title=Concrete&due=2099-01-01&scope=Slab&invite%5B%5D=1"))).await;
    send(&app, post("/do/bid_package.invite", &ada, format!("_csrf={csrf}&project=1&id=1&vendor=1&email=bids%40keystone.example"))).await;
    send(&app, post("/do/bid_package.set_public", &ada, format!("_csrf={csrf}&project=1&id=1&enabled=true"))).await;
    send(&app, post("/do/bid_package.open", &ada, format!("_csrf={csrf}&project=1&id=1"))).await;
    let slug = app.lock().tenant.as_ref().unwrap().slug.clone();
    let a = portal::token(app.secret(), &slug, 1, 1, 0);

    // A bidder attaches a file to their own bid (after sending one), downloads it back, and removes it.
    let (_, _, page) = send(&app, bare(&format!("/bid/{a}"))).await;
    let form = field(&page, "_csrf");
    let boundary = "XBOUNDARYX";
    let upload = |data: &'static [u8], form: &str| {
        let body = multipart(boundary, &[("_csrf", None, form.as_bytes()), ("file", Some("proposal.pdf"), data)]);
        Request::post(format!("/bid/{a}/upload"))
            .header("content-type", format!("multipart/form-data; boundary={boundary}"))
            .body(Body::from(body))
            .unwrap()
    };
    let (st, h, _) = send(&app, upload(b"%PDF-1.4 proposal", &form)).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "files need a bid first: {h:?}");
    send(&app, post_bare(&format!("/bid/{a}/submit"), format!("_csrf={form}&amount=5000"))).await;
    let (st, h, _) = send(&app, upload(b"%PDF-1.4 proposal", "nope")).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{h:?}");
    let (_, h, _) = send(&app, upload(b"%PDF-1.4 proposal", &form)).await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, page) = send(&app, bare(&format!("/bid/{a}"))).await;
    assert!(page.contains("proposal.pdf") && page.contains("Remove"), "{page}");
    let sha = page.split("/files/").nth(1).unwrap().split('"').next().unwrap().to_string();
    let res = router(app.clone()).oneshot(bare(&format!("/bid/{a}/files/{sha}"))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().get("content-disposition").unwrap().to_str().unwrap().starts_with("attachment"));
    assert_eq!(res.into_body().collect().await.unwrap().to_bytes().as_ref(), b"%PDF-1.4 proposal");
    assert_eq!(send(&app, bare(&format!("/bid/{}/files/{sha}", portal::token(app.secret(), &slug, 1, 1, 9)))).await.0, StatusCode::NOT_FOUND);
    send(&app, post_bare(&format!("/bid/{a}/remove"), format!("_csrf={form}&sha256={sha}"))).await;
    let (_, _, page) = send(&app, bare(&format!("/bid/{a}"))).await;
    assert!(!page.contains("proposal.pdf"));

    // The receipt names the reference and amount, and carries the bid page when the public address is known.
    {
        let mut inner = app.lock();
        assert!(portal::receipt(&inner, 1, 1, &format!("/bid/{a}"), &json!({})).is_none(), "no outgoing email, no receipt");
        inner.creds.smtp = Some(buildkiosk_store::creds::SmtpSettings {
            host: "mail.example".into(),
            port: 25,
            security: "none".into(),
            from: "bids@example.com".into(),
            base_url: "https://bids.example".into(),
            ..Default::default()
        });
        let (_, m) =
            portal::receipt(&inner, 1, 1, &format!("/bid/{a}"), &json!({"reference": "BP1-1", "amount": "5000.00", "received": "2026-10-14"}))
                .unwrap();
        assert_eq!(m.to, "bids@keystone.example");
        assert!(m.body.contains("BP1-1") && m.body.contains("5000.00") && m.body.contains(&format!("https://bids.example/bid/{a}")), "{}", m.body);
        inner.creds.smtp = None;
    }

    // The open link: a page anyone can reach that only asks for a request, never shows the documents.
    let (st, _, shown) = send(&app, get("/projects/1/bidding/1/open-link")).await;
    let open = format!("/bid/open/{}", portal::open_token(app.secret(), &slug, 1, 0));
    assert!(st == StatusCode::OK && shown.contains(&open), "{shown}");
    let (st, h, page) = send(&app, bare(&open)).await;
    assert_eq!(st, StatusCode::OK);
    assert!(page.contains("Ask to bid") && page.contains("Concrete") && h.get("referrer-policy").unwrap() == "no-referrer");
    let form = field(&page, "_csrf");
    for bad in ["/bid/open/1.5.abc", "/bid/open/garbage", &format!("/bid/open/{}", portal::open_token(app.secret(), &slug, 1, 3))] {
        assert_eq!(send(&app, bare(bad)).await.0, StatusCode::NOT_FOUND, "{bad}");
    }
    let (st, ..) = send(&app, post_bare(&format!("{open}/register"), "_csrf=nope&company=X".into())).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    // A filled-in hidden field is a program: same thank-you, nothing recorded.
    let (st, h, _) = send(
        &app,
        post_bare(&format!("{open}/register"), format!("_csrf={form}&company=Bot&contact=Bot&email=bot%40bot.example&website=http%3A%2F%2Fspam")),
    )
    .await;
    assert_eq!((st, h.get(LOCATION).unwrap().to_str().unwrap().contains("ok=")), (StatusCode::SEE_OTHER, true));
    assert!(app.lock().tenant.as_ref().unwrap().engine.state().bid_packages.iter().next().unwrap().registrations.is_empty());
    let (_, h, _) = send(
        &app,
        post_bare(&format!("{open}/register"), format!("_csrf={form}&company=Granite+Works&contact=Sam&email=bids%40granite.example&phone=")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, h, _) = send(
        &app,
        post_bare(&format!("{open}/register"), format!("_csrf={form}&company=Granite+Works&contact=Sam&email=bids%40granite.example&phone=")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "a second request from the same address is refused: {h:?}");

    // The team sees the request, approves it, and the new company gets its own private link.
    let (_, _, detail) = send(&app, get("/projects/1/bidding/1")).await;
    assert!(detail.contains("Granite Works") && detail.contains("Approve") && detail.contains("Open bid link"), "{detail}");
    send(&app, post("/do/bid_package.approve_registration", &ada, format!("_csrf={csrf}&project=1&id=1&number=1&vendor="))).await;
    {
        let inner = app.lock();
        let state = inner.tenant.as_ref().unwrap().engine.state();
        let pkg = state.bid_packages.iter().next().unwrap();
        assert_eq!(pkg.invitations.len(), 2);
        assert!(state.vendors.iter().any(|v| v.name == "Granite Works"));
    }
    assert_eq!(send(&app, bare(&format!("/bid/{}", portal::token(app.secret(), &slug, 1, 2, 0)))).await.0, StatusCode::OK);

    // Replacing the open link ends the old one.
    send(&app, post("/do/bid_package.set_public", &ada, format!("_csrf={csrf}&project=1&id=1&rotate=true"))).await;
    assert_eq!(send(&app, bare(&open)).await.0, StatusCode::NOT_FOUND);
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn budget_grouped_by_cost_type_page() {
    let dir = temp("budget-types");
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
    send(&app, post("/do/cost_code.create", &session, format!("_csrf={csrf}&project=1&code=77-100&description=Paving"))).await;
    send(&app, post("/do/cost_code.create", &session, format!("_csrf={csrf}&project=1&code=77-200&description=Curbs"))).await;
    send(&app, post("/do/budget.import", &session, format!("_csrf={csrf}&project=1&text=77-100%2C+1000%0A77-200%2C+2000"))).await;
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &session).body(Body::empty()).unwrap();
    let (st, _, page) = send(&app, get("/projects/1/budget")).await;
    assert!(st == StatusCode::OK && page.contains("Cost types") && page.contains("Save cost types"), "the admin sees the cost types card");
    // Types are set from the page's form; a wrong line changes nothing and says so.
    let (_, h, _) = send(
        &app,
        post("/do/cost_code.set_types", &session, format!("_csrf={csrf}&project=1&text=77-100+labor%0A99-999+labor&_next=%2Fprojects%2F1%2Fbudget")),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("error="), "{h:?}");
    send(&app, post("/do/cost_code.set_types", &session, format!("_csrf={csrf}&project=1&text=77-100+labor%0A77-200+material"))).await;
    let (_, _, page) = send(&app, get("/projects/1/budget")).await;
    assert!(page.contains("77-100 labor") && page.contains("77-200 material"), "the card shows what is set");
    // A view grouped by cost type puts labor before material, each with a subtotal.
    send(&app, post("/do/budget.view_save", &session, format!("_csrf={csrf}&project=1&name=Types&columns=original&group=type"))).await;
    let (_, _, page) = send(&app, get("/projects/1/budget?view=1")).await;
    let (labor, material) = (page.find(">Labor</th>"), page.find(">Material</th>"));
    assert!(matches!((labor, material), (Some(a), Some(b)) if a < b), "labor then material");
    assert!(page.contains("Subtotal") && page.contains(", by cost type"), "subtotals and the view's grouping are named");
    let (st, h, _) = send(&app, get("/projects/1/budget.pdf?view=1")).await;
    assert!(st == StatusCode::OK && h.get("content-type").unwrap().to_str().unwrap().contains("pdf"), "the PDF follows the view");
}

#[tokio::test]
async fn company_report_chart_kinds_and_email_schedule() {
    let dir = temp("report-email");
    let app = Arc::new(App::open(DataDir::open(&dir).unwrap(), None).unwrap());
    let (_, h, page) = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    let anon = cookies(&h);
    let body =
        format!("_csrf={}&name=Acme&first_name=Ada&last_name=Admin&email=ada%40example.com&password=correct-horse-battery", field(&page, "_csrf"));
    let (_, h, _) = send(&app, post("/setup", &anon, body)).await;
    let session = cookies(&h);
    let (_, _, home) = send(&app, Request::get("/").header("cookie", &session).body(Body::empty()).unwrap()).await;
    let csrf = field(&home, "_csrf");
    for name in ["Alpha", "Bravo"] {
        send(&app, post("/do/project.create", &session, format!("_csrf={csrf}&name={name}"))).await;
    }
    for (project, title) in [(1, "A1"), (1, "A2"), (1, "A3"), (2, "B1")] {
        send(&app, post("/do/task.create", &session, format!("_csrf={csrf}&project={project}&title={title}"))).await;
    }
    send(&app, post("/do/company_report.save", &session, format!("_csrf={csrf}&name=Tasks&columns%5B%5D=tasks&columns%5B%5D=original&shared=true")))
        .await;
    let get = |uri: &str| Request::get(uri.to_string()).header("cookie", &session).body(Body::empty()).unwrap();

    // The chart is drawn as bars, columns or a pie; a pie shows each share.
    let (_, _, bars) = send(&app, get("/reports?report=1&chart=tasks")).await;
    assert!(bars.contains("one bar per project") && bars.contains("Kind of chart"), "bars are the default");
    let (_, _, cols) = send(&app, get("/reports?report=1&chart=tasks&kind=columns")).await;
    assert!(cols.contains("one column per project"), "columns");
    let (_, _, pie) = send(&app, get("/reports?report=1&chart=tasks&kind=pie")).await;
    assert!(pie.contains("each project&#39;s share of the total") || pie.contains("each project's share of the total"), "{pie}");
    assert!(pie.contains("Alpha: 3 (75%)") && pie.contains("Bravo: 1 (25%)"), "the key gives each share");
    // No project has an original budget, so the figures add up to nothing and a pie says so.
    let (_, _, none) = send(&app, get("/reports?report=1&chart=original&kind=pie")).await;
    assert!(none.contains("A pie needs figures that are not negative"), "no figures");

    // Scheduling: the form is on the report's page; saving shows the rhythm.
    assert!(bars.contains("Email this report") && bars.contains("Not emailed"));
    let (_, h, _) = send(
        &app,
        post(
            "/do/company_report.schedule",
            &session,
            format!("_csrf={csrf}&id=1&rhythm=weekly&weekday=friday&day=1&people%5B%5D=1&_next=%2Freports%3Freport%3D1"),
        ),
    )
    .await;
    assert!(h.get(LOCATION).unwrap().to_str().unwrap().contains("ok="), "{h:?}");
    let (_, _, bars) = send(&app, get("/reports?report=1")).await;
    assert!(bars.contains("Emailed every Friday to 1 person"), "{bars}");

    // What goes in the email: the table, the totals, a link and the CSV, as that person sees it.
    let (subject, body, csv) = {
        let inner = app.lock();
        let t = inner.tenant.as_ref().unwrap();
        let req = buildkiosk_web::layout::Req {
            engine: &t.engine,
            person: buildkiosk_core::Id::new(1),
            csrf: "",
            now: buildkiosk_core::Timestamp::from_millis(super::now_ms()),
            flash: None,
        };
        buildkiosk_web::company_reports::delivery_email(&req, buildkiosk_core::Id::new(1), "https://bk.example.com").unwrap()
    };
    assert!(subject.starts_with("Tasks"), "{subject}");
    assert!(
        body.contains("Alpha") && body.contains("Bravo") && body.contains("Total") && body.contains("https://bk.example.com/reports?report=1"),
        "{body}"
    );
    assert!(csv.starts_with("Project,Open tasks,Original budget") && csv.contains("Alpha,3"), "{csv}");
}
