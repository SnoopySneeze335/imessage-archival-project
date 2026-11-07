//! HTTP server: the JSON API under `/api` and the single-page app.

use std::path::{Path, PathBuf};

use axum::{
    Json, Router,
    http::{HeaderName, HeaderValue, StatusCode, header},
    routing::get,
};
use serde::Serialize;
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

/// Content-Security-Policy sent with every response. The app never loads
/// anything from another origin (spec: "Deployment, access and privacy").
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
img-src 'self' data: blob:; media-src 'self' blob:; \
object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'";

/// Server configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Directory with the built web app (`web/dist`), served at `/`.
    pub web_dir: PathBuf,
}

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: crate::VERSION,
    })
}

async fn api_not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

fn spa(web_dir: &Path) -> ServeDir<ServeFile> {
    // Unknown paths fall back to index.html so client-side routes work.
    ServeDir::new(web_dir).fallback(ServeFile::new(web_dir.join("index.html")))
}

/// Builds the application router.
pub fn router(config: &Config) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .fallback(api_not_found);

    Router::new()
        .nest("/api", api)
        .fallback_service(spa(&config.web_dir))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ))
}
