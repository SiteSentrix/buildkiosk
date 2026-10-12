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
