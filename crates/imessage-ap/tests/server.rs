//! The HTTP server: headers, sign-in, sessions and request checks
//! (decision 0008).

use std::net::SocketAddr;

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use imessage_ap::{
    auth,
    dirs::Dirs,
    server::{AppState, CONTENT_SECURITY_POLICY, Config, router},
    userdb,
};
use std::sync::Arc;
use tower::ServiceExt;

const LAN: &str = "192.168.1.20:8080";
const PASSWORD: &str = "correct horse battery";

struct App {
    _tmp: tempfile::TempDir,
    state: Arc<AppState>,
    dirs: Dirs,
}

fn app_with(proxies: &str) -> App {
    let tmp = tempfile::tempdir().unwrap();
    let web = tmp.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html><title>t</title>").unwrap();
    let dirs = Dirs::under(&tmp.path().join("data"));
    let state = AppState::new(Config {
        web_dir: web,
        dirs: dirs.clone(),
        trusted_proxies: auth::parse_proxies(proxies).unwrap(),
        demo: false,
    })
    .unwrap();
    App {
        _tmp: tmp,
        state,
        dirs,
    }
}

fn app() -> App {
    app_with("")
}

/// A request from `peer` to the LAN address, as a browser on the app's own
/// page sends it.
fn request(method: &str, uri: &str, body: Option<serde_json::Value>, peer: &str) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, LAN);
    if method != "GET" {
        b = b.header(header::ORIGIN, format!("http://{LAN}"));
    }
    let mut req = match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    req.extensions_mut()
        .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    req
}

async fn send(app: &App, req: Request<Body>) -> axum::response::Response {
    router(app.state.clone()).oneshot(req).await.unwrap()
}

async fn json(res: axum::response::Response) -> serde_json::Value {
    let body = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null)
}

fn set_cookie(res: &axum::response::Response) -> String {
    res.headers()
        .get(header::SET_COOKIE)
        .expect("a cookie")
        .to_str()
        .unwrap()
        .to_owned()
}

/// `name=value` from a Set-Cookie header.
fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().to_owned()
}

fn with_cookie(mut req: Request<Body>, pair: &str) -> Request<Body> {
    req.headers_mut()
        .insert(header::COOKIE, pair.parse().unwrap());
    req
}

const PEER: &str = "192.168.1.30:50000";

async fn setup(app: &App) -> String {
    let code = app
        .state
        .setup_code()
        .expect("a setup code before the first password");
    let res = send(
        app,
        request(
            "POST",
            "/api/auth/setup",
            Some(serde_json::json!({ "code": code, "password": PASSWORD })),
            PEER,
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    cookie_pair(&set_cookie(&res))
}

#[tokio::test]
async fn health_reports_ok_and_version() {
    let app = app();
    let res = send(&app, request("GET", "/api/health", None, PEER)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = json(res).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn every_response_carries_strict_csp() {
    for uri in [
        "/api/health",
        "/",
        "/some/client/route",
        "/api/missing",
        "/api/archive",
    ] {
        let app = app();
        let res = send(&app, request("GET", uri, None, PEER)).await;
        let csp = res.headers().get(header::CONTENT_SECURITY_POLICY);
        assert_eq!(
            csp.and_then(|v| v.to_str().ok()),
            Some(CONTENT_SECURITY_POLICY),
            "{uri}"
        );
        assert!(CONTENT_SECURITY_POLICY.starts_with("default-src 'self'"));
    }
}

#[tokio::test]
async fn static_files_need_no_session_but_the_api_does() {
    let app = app();
    let res = send(&app, request("GET", "/conversations/42", None, PEER)).await;
    assert_eq!(res.status(), StatusCode::OK);
    for uri in ["/api/archive", "/api/nope"] {
        let res = send(&app, request("GET", uri, None, PEER)).await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(
            res.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
    }
    let cookie = setup(&app).await;
    let res = send(
        &app,
        with_cookie(request("GET", "/api/nope", None, PEER), &cookie),
    )
    .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn first_run_needs_the_setup_code() {
    let app = app();
    let status = json(send(&app, request("GET", "/api/auth/status", None, PEER)).await).await;
    assert_eq!(status["state"], "setup");
    assert_eq!(status["https"], false);

    let wrong = send(
        &app,
        request(
            "POST",
            "/api/auth/setup",
            Some(serde_json::json!({ "code": "AAAA-AAAA", "password": PASSWORD })),
            PEER,
        ),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let user = userdb::open(&app.dirs).unwrap();
    assert!(userdb::password_hash(&user).unwrap().is_none());

    let code = app.state.setup_code().unwrap();
    let short = send(
        &app,
        request(
            "POST",
            "/api/auth/setup",
            Some(serde_json::json!({ "code": code, "password": "short" })),
            PEER,
        ),
    )
    .await;
    assert_eq!(short.status(), StatusCode::BAD_REQUEST);

    let cookie = setup(&app).await;
    // Plain HTTP: no Secure flag, or the browser would drop the cookie.
    assert!(cookie.starts_with("session="));
    assert!(app.state.setup_code().is_none());
    let hash = userdb::password_hash(&user).unwrap().unwrap();
    assert!(hash.starts_with("$argon2id$") && !hash.contains(PASSWORD));

    let status = json(
        send(
            &app,
            with_cookie(request("GET", "/api/auth/status", None, PEER), &cookie),
        )
        .await,
    )
    .await;
    assert_eq!(status["state"], "signed_in");
    let summary = send(
        &app,
        with_cookie(request("GET", "/api/archive", None, PEER), &cookie),
    )
    .await;
    assert_eq!(summary.status(), StatusCode::OK);
    assert_eq!(json(summary).await["present"], false);

    // Setup cannot run twice.
    let again = send(
        &app,
        request(
            "POST",
            "/api/auth/setup",
            Some(serde_json::json!({ "code": code, "password": PASSWORD })),
            PEER,
        ),
    )
    .await;
    assert_eq!(again.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn sign_in_and_out() {
    let app = app();
    setup(&app).await;
    let wrong = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            Some(serde_json::json!({ "password": "nope nope nope" })),
            PEER,
        ),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert!(wrong.headers().get(header::SET_COOKIE).is_none());

    let ok = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            Some(serde_json::json!({ "password": PASSWORD })),
            PEER,
        ),
    )
    .await;
    assert_eq!(ok.status(), StatusCode::NO_CONTENT);
    let cookie = cookie_pair(&set_cookie(&ok));
    let res = send(
        &app,
        with_cookie(request("GET", "/api/archive", None, PEER), &cookie),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    let out = send(
        &app,
        with_cookie(request("POST", "/api/auth/logout", None, PEER), &cookie),
    )
    .await;
    assert_eq!(out.status(), StatusCode::NO_CONTENT);
    let cleared: Vec<_> = out
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    assert!(cleared.iter().all(|c| c.contains("Max-Age=0")));
    let res = send(
        &app,
        with_cookie(request("GET", "/api/archive", None, PEER), &cookie),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_new_password_signs_everyone_out() {
    let app = app();
    let cookie = setup(&app).await;
    let user = userdb::open(&app.dirs).unwrap();
    userdb::set_password_hash(&user, &auth::hash_password("another password").unwrap()).unwrap();
    let res = send(
        &app,
        with_cookie(request("GET", "/api/archive", None, PEER), &cookie),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn guessing_is_rate_limited_per_client() {
    let app = app();
    setup(&app).await;
    let attempt = |peer: &str, password: &str| {
        request(
            "POST",
            "/api/auth/login",
            Some(serde_json::json!({ "password": password })),
            peer,
        )
    };
    for _ in 0..5 {
        let r = send(&app, attempt(PEER, "wrong guess")).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }
    // Locked out, even with the right password.
    let r = send(&app, attempt(PEER, PASSWORD)).await;
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(r.headers().get(header::RETRY_AFTER).is_some());
    // Another device is not.
    let r = send(&app, attempt("192.168.1.31:50000", PASSWORD)).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn cross_origin_posts_are_refused() {
    let app = app();
    let code = app.state.setup_code().unwrap();
    let mut req = request(
        "POST",
        "/api/auth/setup",
        Some(serde_json::json!({ "code": code, "password": PASSWORD })),
        PEER,
    );
    req.headers_mut()
        .insert(header::ORIGIN, "http://evil.example.com".parse().unwrap());
    let res = send(&app, req).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert!(app.state.setup_code().is_some(), "nothing changed");

    // The same app reached by its server name works: the check compares
    // with the request's own Host.
    let mut req = request(
        "POST",
        "/api/auth/setup",
        Some(serde_json::json!({ "code": code, "password": PASSWORD })),
        PEER,
    );
    req.headers_mut()
        .insert(header::HOST, "snoopyserver:8080".parse().unwrap());
    req.headers_mut()
        .insert(header::ORIGIN, "http://snoopyserver:8080".parse().unwrap());
    assert_eq!(send(&app, req).await.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn https_through_a_trusted_proxy_gets_a_secure_cookie() {
    let proxy = "172.17.0.1:40000";
    for (proxies, secure) in [("172.17.0.0/16", true), ("", false)] {
        let app = app_with(proxies);
        let code = app.state.setup_code().unwrap();
        let mut req = request(
            "POST",
            "/api/auth/setup",
            Some(serde_json::json!({ "code": code, "password": PASSWORD })),
            proxy,
        );
        let h = req.headers_mut();
        h.insert(header::HOST, "snoopy.example.ts.net".parse().unwrap());
        h.insert(
            header::ORIGIN,
            "https://snoopy.example.ts.net".parse().unwrap(),
        );
        h.insert("x-forwarded-proto", "https".parse().unwrap());
        let res = send(&app, req).await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT, "proxies {proxies:?}");
        let c = set_cookie(&res);
        assert_eq!(
            c.starts_with("__Host-session=") && c.contains("Secure"),
            secure,
            "{c}"
        );

        let mut status = with_cookie(
            request("GET", "/api/auth/status", None, proxy),
            &cookie_pair(&c),
        );
        status
            .headers_mut()
            .insert("x-forwarded-proto", "https".parse().unwrap());
        let body = json(send(&app, status).await).await;
        assert_eq!(body["https"], secure);
        assert_eq!(body["state"], "signed_in");
    }
}
