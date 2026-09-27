//! Identity and authorization for the one hosted Eplyx service. Postgres stores
//! no authoritative project name, run, report, capture or analytical result.
pub mod auth;
pub mod db;
pub mod error;
pub mod sync;
pub mod views;
pub mod web;
pub mod workspaces;
use crate::api::{AppState, Shared};
use anyhow::{Context, Result};
use axum::{
    routing::{get, post},
    Router,
};

#[derive(Clone)]
pub struct IdentityConfig {
    pub database_url: String,
    pub public_url: String,
    pub signup_code: Option<String>,
    pub demo_project: Option<String>,
}
impl IdentityConfig {
    pub fn from_env() -> Result<Option<Self>> {
        let var = |key: &str| std::env::var(key).ok().filter(|s| !s.trim().is_empty());
        let Some(database_url) = var("EPLYX_DATABASE_URL") else {
            return Ok(None);
        };
        let public_url = eplyx_engine::cloud::credentials::normalize_server(
            &var("EPLYX_PUBLIC_URL")
                .context("EPLYX_PUBLIC_URL is required with identity storage")?,
        )?;
        let demo_project = var("EPLYX_DEMO_PROJECT");
        anyhow::ensure!(
            demo_project
                .as_ref()
                .is_none_or(|s| eplyx_engine::cloud::contract::is_project_id(s)),
            "invalid EPLYX_DEMO_PROJECT"
        );
        Ok(Some(Self {
            database_url,
            public_url,
            signup_code: var("EPLYX_SIGNUP_CODE"),
            demo_project,
        }))
    }
    pub fn secure_cookies(&self) -> bool {
        self.public_url.starts_with("https://")
    }
}
pub struct Identity {
    pub db: deadpool_postgres::Pool,
    pub config: IdentityConfig,
    pub limiter: auth::Limiter,
    pub project_creation: tokio::sync::Mutex<()>,
}
impl Identity {
    pub async fn connect(config: IdentityConfig) -> Result<Self> {
        let db = db::connect(&config.database_url)?;
        db::migrate(&db).await?;
        Ok(Self {
            db,
            config,
            limiter: auth::Limiter::default(),
            project_creation: tokio::sync::Mutex::new(()),
        })
    }
}
impl AppState {
    pub fn identity(&self) -> error::ApiResult<&Identity> {
        self.identity.as_ref().ok_or_else(|| {
            error::ApiError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "identity storage is not configured",
            )
        })
    }
}
pub fn identity_id(id: &str, prefix: &str) -> bool {
    eplyx_engine::cloud::contract::is_cloud_id(id, prefix)
}
pub fn router() -> Router<Shared> {
    Router::new()
        .route(
            "/v1/workspaces",
            get(workspaces::list_workspaces).post(workspaces::create_workspace),
        )
        .route(
            "/v1/workspaces/{workspace}/members",
            get(workspaces::list_members).post(workspaces::add_member),
        )
        .route(
            "/v1/workspaces/{workspace}/members/{member}",
            axum::routing::delete(workspaces::remove_member),
        )
        .route(
            "/v1/workspaces/{workspace}/projects",
            post(workspaces::create_project),
        )
        .route(
            "/v1/projects/{project_id}/workspace",
            get(workspaces::get_project),
        )
        .route(
            "/v1/projects/{project_id}/links",
            post(workspaces::link_project),
        )
        .route("/v1/auth/signup", post(auth::signup))
        .route("/v1/auth/login", post(auth::login))
        .route("/v1/auth/logout", post(auth::logout))
        .route("/v1/auth/me", get(auth::me))
        .route("/v1/auth/token", axum::routing::delete(auth::revoke_token))
        .route("/v1/auth/device", post(auth::device_start))
        .route("/v1/auth/device/lookup", get(auth::device_lookup))
        .route("/v1/auth/device/approve", post(auth::device_approve))
        .route("/v1/auth/device/token", post(auth::device_token))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .merge(sync::router())
        .merge(views::router())
        .merge(web::router())
}

/// Cookie mutations require the configured first-party origin. Bearer requests
/// keep MAIN's CI contract and cannot be authenticated by an ambient cookie.
pub async fn cookie_write_guard(
    axum::extract::State(state): axum::extract::State<Shared>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::{
        http::{header, Method},
        response::IntoResponse,
    };
    let headers = request.headers();
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) && headers.contains_key(header::COOKIE)
        && !headers.contains_key(header::AUTHORIZATION)
    {
        if let Some(identity) = &state.identity {
            let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
            let site = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok());
            if !(origin == Some(identity.config.public_url.as_str())
                || (origin.is_none() && site == Some("same-origin")))
            {
                return error::ApiError::forbidden("cross-site request refused").into_response();
            }
        }
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    for (name,value) in [
        ("x-frame-options","DENY"),("referrer-policy","no-referrer"),
        ("cross-origin-resource-policy","same-origin"),("cross-origin-opener-policy","same-origin"),
        ("content-security-policy","default-src 'self'; img-src 'self' data:; style-src 'self'; style-src-attr 'unsafe-inline'; script-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"),
    ] {response.headers_mut().insert(axum::http::HeaderName::from_static(name),value.parse().unwrap());}
    if state
        .identity
        .as_ref()
        .is_some_and(|i| i.config.secure_cookies())
    {
        response.headers_mut().insert(
            header::STRICT_TRANSPORT_SECURITY,
            "max-age=31536000".parse().unwrap(),
        );
    }
    response
}
