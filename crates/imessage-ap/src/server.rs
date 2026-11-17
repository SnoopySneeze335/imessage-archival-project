//! HTTP server: the JSON API under `/api` and the single-page app.
//!
//! Static files need no session: the app's code holds no data. Everything
//! else under `/api`, apart from health and sign-in, needs one (decision
//! 0008).

use std::{
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{ConnectInfo, MatchedPath, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

use crate::{
    archive,
    auth::{self, IpRange, RateLimiter},
    dirs::Dirs,
    userdb,
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
    /// Where archive.db, user.db and the files are.
    pub dirs: Dirs,
    /// Reverse proxies whose `X-Forwarded-*` headers are believed.
    pub trusted_proxies: Vec<IpRange>,
    /// Serving the demo archive of invented data: the sign-in page says so.
    pub demo: bool,
}

/// Shared server state.
pub struct AppState {
    config: Config,
    user: Mutex<Connection>,
    /// The first-run setup code, while no password is set.
    setup_code: Mutex<Option<String>>,
    limiter: Mutex<RateLimiter>,
}

impl AppState {
    /// Opens user.db. With no password set yet, makes a setup code and
    /// writes it to the log: whoever sets the password must have it.
    pub fn new(config: Config) -> Result<Arc<Self>> {
        let user = userdb::open(&config.dirs)?;
        let setup_code = if userdb::password_hash(&user)?.is_none() {
            let code = auth::new_setup_code();
            tracing::warn!(
                "no password is set yet. First-run setup code: {code} \
                 (open the app and enter it to choose the password)"
            );
            Some(code)
        } else {
            None
        };
        Ok(Arc::new(Self {
            config,
            user: Mutex::new(user),
            setup_code: Mutex::new(setup_code),
            limiter: Mutex::new(RateLimiter::default()),
        }))
    }

    /// The setup code currently accepted (tests and the CLI use it).
    pub fn setup_code(&self) -> Option<String> {
        self.setup_code.lock().expect("setup code").clone()
    }

    fn user(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.user.lock().expect("user.db connection")
    }

    fn origin(&self, req_headers: &HeaderMap, peer: Option<IpAddr>) -> auth::Origin {
        auth::origin(req_headers, peer, &self.config.trusted_proxies)
    }
}

type Shared = Arc<AppState>;

#[derive(Debug)]
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        // Errors here come from SQLite or the file system: no content.
        tracing::error!("request failed: {e:#}");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "Server error".into())
    }
}

fn peer(req: &Request) -> Option<IpAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
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

#[derive(Debug, Serialize)]
struct AuthStatus {
    /// `setup` (no password yet), `signed_out` or `signed_in`.
    state: &'static str,
    /// The request reached the server over HTTPS (directly or through a
    /// trusted proxy).
    https: bool,
    demo: bool,
    min_password_chars: usize,
}

fn signed_in(state: &AppState, headers: &HeaderMap) -> Result<bool> {
    match auth::session_token(headers) {
        Some(t) => userdb::check_session(&state.user(), &auth::token_hash(&t)),
        None => Ok(false),
    }
}

async fn auth_status(
    State(state): State<Shared>,
    req: Request,
) -> Result<Json<AuthStatus>, ApiError> {
    let origin = state.origin(req.headers(), peer(&req));
    let has_password = userdb::password_hash(&state.user())?.is_some();
    let status = if !has_password {
        "setup"
    } else if signed_in(&state, req.headers())? {
        "signed_in"
    } else {
        "signed_out"
    };
    Ok(Json(AuthStatus {
        state: status,
        https: origin.https,
        demo: state.config.demo,
        min_password_chars: auth::MIN_PASSWORD_CHARS,
    }))
}

#[derive(Debug, Deserialize)]
struct SetupBody {
    code: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct LoginBody {
    password: String,
}

fn too_many(wait: std::time::Duration) -> Response {
    let secs = wait.as_secs().max(1);
    let mut r = ApiError(
        StatusCode::TOO_MANY_REQUESTS,
        format!("Too many attempts. Try again in {secs} s."),
    )
    .into_response();
    r.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from(secs));
    r
}

/// A 204 that starts a session.
fn start_session(state: &AppState, https: bool) -> Result<Response, ApiError> {
    let token = auth::new_token();
    userdb::create_session(&state.user(), &auth::token_hash(&token))?;
    let mut r = StatusCode::NO_CONTENT.into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth::session_cookie(&token, https)).expect("cookie is ASCII"),
    );
    Ok(r)
}

async fn setup(State(state): State<Shared>, req: Request) -> Result<Response, ApiError> {
    let origin = state.origin(req.headers(), peer(&req));
    let body: SetupBody = json_body(req).await?;
    let now = Instant::now();
    if let Err(wait) = state
        .limiter
        .lock()
        .expect("limiter")
        .check(origin.client, now)
    {
        return Ok(too_many(wait));
    }
    let Some(code) = state.setup_code() else {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "A password is already set. Sign in instead.".into(),
        ));
    };
    if !auth::setup_code_matches(&body.code, &code) {
        state
            .limiter
            .lock()
            .expect("limiter")
            .failed(origin.client, now);
        tracing::warn!(client = ?origin.client, "wrong setup code");
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "That setup code is not right. It is in the server's log.".into(),
        ));
    }
    if let Err(why) = auth::validate_new_password(&body.password) {
        return Err(ApiError(StatusCode::BAD_REQUEST, why));
    }
    let password = body.password;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&password))
        .await
        .map_err(|e| anyhow::anyhow!(e))??;
    {
        let user = state.user();
        // Another browser may have finished setup first.
        if userdb::password_hash(&user)?.is_some() {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "A password is already set. Sign in instead.".into(),
            ));
        }
        userdb::set_password_hash(&user, &hash)?;
    }
    *state.setup_code.lock().expect("setup code") = None;
    state
        .limiter
        .lock()
        .expect("limiter")
        .succeeded(origin.client);
    tracing::info!("password set; setup code retired");
    start_session(&state, origin.https)
}

async fn login(State(state): State<Shared>, req: Request) -> Result<Response, ApiError> {
    let origin = state.origin(req.headers(), peer(&req));
    let body: LoginBody = json_body(req).await?;
    let now = Instant::now();
    if let Err(wait) = state
        .limiter
        .lock()
        .expect("limiter")
        .check(origin.client, now)
    {
        return Ok(too_many(wait));
    }
    let Some(stored) = userdb::password_hash(&state.user())? else {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "No password is set yet. Use the setup code.".into(),
        ));
    };
    let ok = if body.password.len() > auth::MAX_PASSWORD_BYTES {
        false
    } else {
        let password = body.password;
        tokio::task::spawn_blocking(move || auth::verify_password(&password, &stored))
            .await
            .map_err(|e| anyhow::anyhow!(e))?
    };
    if !ok {
        state
            .limiter
            .lock()
            .expect("limiter")
            .failed(origin.client, now);
        tracing::warn!(client = ?origin.client, "wrong password");
        return Err(ApiError(StatusCode::UNAUTHORIZED, "Wrong password.".into()));
    }
    state
        .limiter
        .lock()
        .expect("limiter")
        .succeeded(origin.client);
    start_session(&state, origin.https)
}

async fn logout(State(state): State<Shared>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(t) = auth::session_token(&headers) {
        userdb::delete_session(&state.user(), &auth::token_hash(&t))?;
    }
    let mut r = StatusCode::NO_CONTENT.into_response();
    for c in auth::clear_cookies() {
        r.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_str(&c).expect("cookie is ASCII"),
        );
    }
    Ok(r)
}

async fn json_body<T: serde::de::DeserializeOwned>(req: Request) -> Result<T, ApiError> {
    let bytes = axum::body::to_bytes(req.into_body(), 64 * 1024)
        .await
        .map_err(|_| ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Request too large".into()))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Malformed request".into()))
}

/// What the home screen shows until the conversation list arrives (4a-2):
/// counts only.
#[derive(Debug, Serialize)]
struct ArchiveSummary {
    present: bool,
    messages: i64,
    chats: i64,
    attachments: i64,
    backups: i64,
}

async fn archive_summary(State(state): State<Shared>) -> Result<Json<ArchiveSummary>, ApiError> {
    let dirs = state.config.dirs.clone();
    let summary = tokio::task::spawn_blocking(move || -> Result<ArchiveSummary> {
        if !archive::path(&dirs).is_file() {
            return Ok(ArchiveSummary {
                present: false,
                messages: 0,
                chats: 0,
                attachments: 0,
                backups: 0,
            });
        }
        let db = archive::open_read_only(&dirs)?;
        let n = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0));
        Ok(ArchiveSummary {
            present: true,
            messages: n("SELECT COUNT(*) FROM message")?,
            chats: n("SELECT COUNT(*) FROM chat")?,
            attachments: n("SELECT COUNT(*) FROM attachment")?,
            backups: n("SELECT COUNT(*) FROM source")?,
        })
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    Ok(Json(summary))
}

async fn api_not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

/// Refuses requests under `/api` that need a session and have none.
async fn require_session(State(state): State<Shared>, req: Request, next: Next) -> Response {
    match signed_in(&state, req.headers()) {
        Ok(true) => next.run(req).await,
        Ok(false) => ApiError(StatusCode::UNAUTHORIZED, "Sign in first.".into()).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// Refuses requests that change something unless they come from the app's
/// own pages (decision 0008).
async fn check_origin(State(state): State<Shared>, req: Request, next: Next) -> Response {
    let origin = state.origin(req.headers(), peer(&req));
    if auth::same_origin(req.method(), req.headers(), &origin) {
        next.run(req).await
    } else {
        tracing::warn!(method = %req.method(), "refused a cross-origin request");
        ApiError(
            StatusCode::FORBIDDEN,
            "Cross-origin request refused.".into(),
        )
        .into_response()
    }
}

/// One log line per API request: method, route pattern, status and time.
/// Never the query string or the body, which may hold text the user typed.
async fn log_request(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "(unmatched)".to_owned(), |p| p.as_str().to_owned());
    let started = Instant::now();
    let res = next.run(req).await;
    tracing::info!(
        %method,
        route,
        status = res.status().as_u16(),
        ms = started.elapsed().as_millis() as u64,
        "api"
    );
    res
}

fn spa(web_dir: &Path) -> ServeDir<ServeFile> {
    // Unknown paths fall back to index.html so client-side routes work.
    ServeDir::new(web_dir).fallback(ServeFile::new(web_dir.join("index.html")))
}

/// Builds the application router.
pub fn router(state: Shared) -> Router {
    let protected = Router::new()
        .route("/archive", get(archive_summary))
        .fallback(api_not_found)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_session,
        ));

    let api = Router::new()
        .route("/health", get(health))
        .route("/auth/status", get(auth_status))
        .route("/auth/setup", post(setup))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .merge(protected)
        .layer(middleware::from_fn(log_request))
        .layer(middleware::from_fn_with_state(state.clone(), check_origin))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .with_state(state.clone());

    Router::new()
        .nest("/api", api)
        .fallback_service(spa(&state.config.web_dir))
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
