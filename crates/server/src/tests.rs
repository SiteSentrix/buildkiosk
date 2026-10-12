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
