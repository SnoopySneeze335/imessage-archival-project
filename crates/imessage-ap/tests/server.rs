use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use imessage_ap::server::{CONTENT_SECURITY_POLICY, Config, router};
use tower::ServiceExt;

fn make_app() -> (tempfile::TempDir, axum::Router) {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(
        web.path().join("index.html"),
        "<!doctype html><title>t</title>",
    )
    .unwrap();
    let app = router(&Config {
        web_dir: web.path().to_path_buf(),
    });
    (web, app)
}

async fn get(app: axum::Router, uri: &str) -> axum::response::Response {
    app.oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn health_reports_ok_and_version() {
    let (_web, app) = make_app();
    let res = get(app, "/api/health").await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn every_response_carries_strict_csp() {
    for uri in ["/api/health", "/", "/some/client/route", "/api/missing"] {
        let (_web, app) = make_app();
        let res = get(app, uri).await;
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
async fn unknown_routes_fall_back_to_index_but_api_404s() {
    let (_web, app) = make_app();
    let res = get(app, "/conversations/42").await;
    assert_eq!(res.status(), StatusCode::OK);

    let (_web, app) = make_app();
    let res = get(app, "/api/nope").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
