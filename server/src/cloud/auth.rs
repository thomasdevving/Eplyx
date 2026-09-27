//! Accounts, browser sessions, CLI device authorization and bearer tokens.
//!
//! Passwords are entered only in the browser and stored as Argon2id hashes.
//! The CLI receives a scoped token through a device-code flow the signed-in
//! browser approves. Sessions and tokens are stored as SHA-256 digests only.
use crate::{
    api::Shared,
    cloud::error::{ApiError, ApiResult},
};
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration as StdDuration, Instant},
};

pub const SESSION_COOKIE: &str = "eplyx_session";
const SESSION_DAYS: i64 = 14;
const USER_TOKEN_DAYS: i64 = 90;
const DEVICE_MINUTES: i64 = 10;
const DEVICE_INTERVAL: i64 = 3;
/// Base-20 alphabet without vowels, so codes never spell words.
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

pub fn random_token(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes::<32>())
    )
}

pub fn new_id(prefix: &str) -> String {
    let bytes = random_bytes::<10>();
    format!(
        "{prefix}{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

pub fn digest(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt =
        SaltString::encode_b64(&random_bytes::<16>()).map_err(|e| anyhow::anyhow!("salt: {e}"))?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash: {e}"))?
        .to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// Equalizes timing for unknown accounts.
static DUMMY_HASH: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| hash_password("eplyx-dummy-password").expect("dummy hash"));

/// Failed sign-in attempts per email, to slow password guessing.
#[derive(Default)]
pub struct Limiter {
    failures: Mutex<HashMap<String, (u32, Instant)>>,
}

impl Limiter {
    const WINDOW: StdDuration = StdDuration::from_secs(15 * 60);
    const LIMIT: u32 = 10;

    fn blocked(&self, key: &str) -> bool {
        let mut map = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, (_, start)| start.elapsed() < Self::WINDOW);
        map.get(key).is_some_and(|(count, _)| *count >= Self::LIMIT)
    }

    fn fail(&self, key: &str) {
        let mut map = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        let entry = map.entry(key.to_owned()).or_insert((0, Instant::now()));
        entry.0 += 1;
    }

    fn clear(&self, key: &str) {
        self.failures
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key);
    }
}

#[derive(Clone, Debug)]
pub enum Principal {
    User {
        id: String,
        email: String,
        name: String,
        session: bool,
        token_id: Option<String>,
    },
    /// One project token for hosted checks, sync and reading its own results.
    /// It cannot administer identity or activate a baseline.
    Project {
        token_id: String,
        project_id: String,
        label: String,
    },
}

impl Principal {
    pub fn label(&self) -> String {
        match self {
            Self::User { email, .. } => email.clone(),
            Self::Project { label, .. } => format!("CI token “{label}”"),
        }
    }

    pub fn via(&self) -> &'static str {
        match self {
            Self::User { .. } => "cli",
            Self::Project { .. } => "ci",
        }
    }

    pub fn user_id(&self) -> Option<&str> {
        match self {
            Self::User { id, .. } => Some(id),
            Self::Project { .. } => None,
        }
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// Resolve the caller. A bearer token wins over a cookie; neither yields None.
pub async fn principal(
    state: &crate::api::AppState,
    headers: &HeaderMap,
) -> ApiResult<Option<Principal>> {
    let client = state.identity()?.db.get().await?;
    if let Some(token) = bearer(headers) {
        let row = client
            .query_opt(
                "SELECT t.id, t.kind, t.project_id, t.label, u.id, u.email, u.name FROM api_tokens t JOIN users u ON u.id = t.user_id
                 WHERE t.token_sha256 = $1 AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > now())",
                &[&digest(token)],
            )
            .await?;
        let Some(row) = row else {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid or expired Eplyx token",
            ));
        };
        let token_id: String = row.get(0);
        client
            .execute(
                "UPDATE api_tokens SET last_used_at = now() WHERE id = $1",
                &[&token_id],
            )
            .await?;
        let kind: String = row.get(1);
        return Ok(Some(if kind == "project" {
            Principal::Project {
                token_id,
                project_id: row.get(2),
                label: row.get(3),
            }
        } else {
            Principal::User {
                id: row.get(4),
                email: row.get(5),
                name: row.get(6),
                session: false,
                token_id: Some(token_id),
            }
        }));
    }
    if let Some(session) = cookie(headers, SESSION_COOKIE) {
        let row = client
            .query_opt(
                "SELECT u.id, u.email, u.name FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.token_sha256 = $1 AND s.expires_at > now()",
                &[&digest(session)],
            )
            .await?;
        if let Some(row) = row {
            return Ok(Some(Principal::User {
                id: row.get(0),
                email: row.get(1),
                name: row.get(2),
                session: true,
                token_id: None,
            }));
        }
    }
    Ok(None)
}

pub async fn require(state: &crate::api::AppState, headers: &HeaderMap) -> ApiResult<Principal> {
    principal(state, headers)
        .await?
        .ok_or_else(ApiError::unauthorized)
}

/// A signed-in person (browser session or CLI user token), never a CI token.
pub async fn require_user(
    state: &crate::api::AppState,
    headers: &HeaderMap,
) -> ApiResult<Principal> {
    match require(state, headers).await? {
        user @ Principal::User { .. } => Ok(user),
        Principal::Project { .. } => Err(ApiError::forbidden(
            "Project tokens can submit checks and sync to their own project; account management requires a user",
        )),
    }
}

/// Cookie-authenticated writes must come from this site's own pages: the
/// request's Origin (or Fetch metadata) must match the public origin.
pub fn same_origin(
    state: &crate::api::AppState,
    headers: &HeaderMap,
    principal: &Principal,
) -> ApiResult<()> {
    if !matches!(principal, Principal::User { session: true, .. }) {
        return Ok(());
    }
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let site = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok());
    if origin == Some(state.identity()?.config.public_url.as_str())
        || (origin.is_none() && site == Some("same-origin"))
    {
        Ok(())
    } else {
        Err(ApiError::forbidden("cross-site request refused"))
    }
}

fn session_cookie(
    state: &crate::api::AppState,
    value: &str,
    max_age: i64,
) -> ApiResult<HeaderValue> {
    let secure = if state.identity()?.config.secure_cookies() {
        "; Secure"
    } else {
        ""
    };
    Ok(HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}"
    ))
    .expect("cookie header is ASCII"))
}

pub fn parse_json<T: serde::de::DeserializeOwned>(body: &[u8]) -> ApiResult<T> {
    serde_json::from_slice(body).map_err(|_| ApiError::invalid("invalid request body"))
}

pub fn clean_name(name: &str, what: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(ApiError::invalid(format!("{what} must be 1–80 characters")));
    }
    Ok(name.to_owned())
}

fn clean_email(email: &str) -> ApiResult<String> {
    let email = email.trim().to_ascii_lowercase();
    let valid = email.len() <= 254
        && !email.contains(char::is_whitespace)
        && email
            .split_once('@')
            .is_some_and(|(user, domain)| !user.is_empty() && domain.contains('.'));
    if valid {
        Ok(email)
    } else {
        Err(ApiError::invalid("enter a valid email address"))
    }
}

fn user_json(id: &str, email: &str, name: &str) -> Value {
    json!({"id": id, "email": email, "name": name})
}

async fn start_session(state: &crate::api::AppState, user_id: &str) -> ApiResult<HeaderValue> {
    let token = random_token("eplyx_s_");
    let client = state.identity()?.db.get().await?;
    client
        .execute(
            "INSERT INTO sessions (token_sha256, user_id, expires_at) VALUES ($1, $2, $3)",
            &[
                &digest(&token),
                &user_id,
                &(Utc::now() + Duration::days(SESSION_DAYS)),
            ],
        )
        .await?;
    session_cookie(state, &token, SESSION_DAYS * 86400)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Signup {
    email: String,
    password: String,
    name: String,
    #[serde(default)]
    signup_code: Option<String>,
}

pub async fn signup(
    State(state): State<Shared>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Response> {
    origin_for_anonymous(&state, &headers)?;
    let request: Signup = parse_json(&body)?;
    if let Some(code) = &state.identity()?.config.signup_code {
        if request.signup_code.as_deref() != Some(code.as_str()) {
            return Err(ApiError::forbidden(
                "a valid sign-up code is required on this server",
            ));
        }
    }
    let email = clean_email(&request.email)?;
    let name = clean_name(&request.name, "name")?;
    if request.password.chars().count() < 10 || request.password.len() > 256 {
        return Err(ApiError::invalid(
            "use a password of at least 10 characters",
        ));
    }
    let password = request.password.clone();
    let hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(ApiError::internal)??;
    let user_id = new_id("usr_");
    let workspace_id = new_id("ws_");
    let mut client = state.identity()?.db.get().await?;
    let tx = client.transaction().await?;
    let inserted = tx
        .execute(
            "INSERT INTO users (id, email, name, password_hash) VALUES ($1, $2, $3, $4) ON CONFLICT (email) DO NOTHING",
            &[&user_id, &email, &name, &hash],
        )
        .await?;
    if inserted == 0 {
        return Err(ApiError::conflict(
            "an account with this email already exists; sign in instead",
        ));
    }
    tx.execute(
        "INSERT INTO workspaces (id, name, created_by) VALUES ($1, $2, $3)",
        &[&workspace_id, &format!("{name}’s workspace"), &user_id],
    )
    .await?;
    tx.execute(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ($1, $2, 'owner')",
        &[&workspace_id, &user_id],
    )
    .await?;
    tx.commit().await?;
    let cookie = start_session(&state, &user_id).await?;
    let mut response = (
        StatusCode::CREATED,
        Json(json!({"user": user_json(&user_id, &email, &name), "workspace_id": workspace_id})),
    )
        .into_response();
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

/// Sign-up and sign-in have no session yet; they still must come from this
/// site's pages so another site cannot log a visitor into a chosen account.
fn origin_for_anonymous(state: &crate::api::AppState, headers: &HeaderMap) -> ApiResult<()> {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    match origin {
        Some(origin) if origin != state.identity()?.config.public_url => {
            Err(ApiError::forbidden("cross-site request refused"))
        }
        _ => Ok(()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    email: String,
    password: String,
}

pub async fn login(
    State(state): State<Shared>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Response> {
    origin_for_anonymous(&state, &headers)?;
    let request: Login = parse_json(&body)?;
    if request.password.len() > 256 || request.email.len() > 254 {
        return Err(ApiError::invalid("invalid sign-in fields"));
    }
    let email = request.email.trim().to_ascii_lowercase();
    if state.identity()?.limiter.blocked(&email) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too many failed sign-in attempts; wait 15 minutes",
        ));
    }
    let client = state.identity()?.db.get().await?;
    let row = client
        .query_opt(
            "SELECT id, name, password_hash FROM users WHERE email = $1",
            &[&email],
        )
        .await?;
    let (hash, found) = match &row {
        Some(row) => (row.get::<_, String>(2), true),
        None => (DUMMY_HASH.clone(), false),
    };
    let password = request.password.clone();
    let valid = tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .map_err(ApiError::internal)?;
    let Some(row) = row.filter(|_| found && valid) else {
        state.identity()?.limiter.fail(&email);
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "email or password is incorrect",
        ));
    };
    state.identity()?.limiter.clear(&email);
    let user_id: String = row.get(0);
    let name: String = row.get(1);
    let cookie = start_session(&state, &user_id).await?;
    let mut response = Json(json!({"user": user_json(&user_id, &email, &name)})).into_response();
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

pub async fn logout(State(state): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(principal) = principal(&state, &headers).await? {
        same_origin(&state, &headers, &principal)?;
    }
    if let Some(session) = cookie(&headers, SESSION_COOKIE) {
        let client = state.identity()?.db.get().await?;
        client
            .execute(
                "DELETE FROM sessions WHERE token_sha256 = $1",
                &[&digest(session)],
            )
            .await?;
    }
    let mut response = Json(json!({"signed_out": true})).into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, session_cookie(&state, "", 0)?);
    Ok(response)
}

pub async fn me(State(state): State<Shared>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    match require(&state, &headers).await? {
        Principal::User {
            id, email, name, ..
        } => Ok(Json(
            json!({"user": user_json(&id, &email, &name), "demo_project": state.identity()?.config.demo_project}),
        )),
        Principal::Project {
            project_id, label, ..
        } => Ok(Json(
            json!({"ci": {"project_id": project_id, "label": label}}),
        )),
    }
}

/// Revoke the bearer token presented with this request (`eplyx logout`).
pub async fn revoke_token(
    State(state): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let principal = require(&state, &headers).await?;
    let token_id = match &principal {
        Principal::User {
            token_id: Some(id), ..
        } => id.clone(),
        Principal::Project { token_id, .. } => token_id.clone(),
        Principal::User { .. } => {
            return Err(ApiError::bad_request(
                "present the token to revoke as a bearer token",
            ))
        }
    };
    let client = state.identity()?.db.get().await?;
    client
        .execute(
            "UPDATE api_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
            &[&token_id],
        )
        .await?;
    Ok(Json(json!({"revoked": true})))
}

// ------------------------------------------------------ device authorization

fn user_code() -> String {
    let bytes = random_bytes::<8>();
    let chars: String = bytes
        .iter()
        .map(|b| USER_CODE_ALPHABET[*b as usize % USER_CODE_ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

fn normalize_code(code: &str) -> String {
    let letters: String = code
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if letters.len() == 8 {
        format!("{}-{}", &letters[..4], &letters[4..])
    } else {
        String::new()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceStart {
    client: String,
}

pub async fn device_start(
    State(state): State<Shared>,
    body: axum::body::Bytes,
) -> ApiResult<Json<Value>> {
    let request: DeviceStart = parse_json(&body)?;
    let client_label = clean_name(&request.client, "client label")?;
    let device_code = random_token("");
    let code = user_code();
    let db = state.identity()?.db.get().await?;
    db.execute(
        "DELETE FROM device_codes WHERE expires_at < now() - interval '1 day'",
        &[],
    )
    .await?;
    db.execute(
        "INSERT INTO device_codes (device_sha256, user_code, client, expires_at) VALUES ($1, $2, $3, $4)",
        &[
            &digest(&device_code),
            &code,
            &client_label,
            &(Utc::now() + Duration::minutes(DEVICE_MINUTES)),
        ],
    )
    .await?;
    let verification = format!("{}/device", state.identity()?.config.public_url);
    Ok(Json(json!({
        "device_code": device_code,
        "user_code": code,
        "verification_uri": verification,
        "verification_uri_complete": format!("{verification}?code={code}"),
        "interval": DEVICE_INTERVAL,
        "expires_in": DEVICE_MINUTES * 60,
    })))
}

#[derive(Deserialize)]
pub struct CodeQuery {
    code: String,
}

pub async fn device_lookup(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<CodeQuery>,
) -> ApiResult<Json<Value>> {
    let principal = require_user(&state, &headers).await?;
    if !matches!(principal, Principal::User { session: true, .. }) {
        return Err(ApiError::forbidden("approve sign-in codes in the browser"));
    }
    let code = normalize_code(&query.code);
    let db = state.identity()?.db.get().await?;
    let row = db
        .query_opt(
            "SELECT client, created_at, expires_at, approved_by IS NOT NULL, denied, consumed_at IS NOT NULL FROM device_codes WHERE user_code = $1",
            &[&code],
        )
        .await?
        .ok_or_else(|| ApiError::not_found("unknown or expired code"))?;
    let expires: DateTime<Utc> = row.get(2);
    let state_text = if row.get::<_, bool>(5) {
        "used"
    } else if row.get::<_, bool>(4) {
        "denied"
    } else if row.get::<_, bool>(3) {
        "approved"
    } else if expires < Utc::now() {
        "expired"
    } else {
        "pending"
    };
    Ok(Json(json!({
        "user_code": code,
        "client": row.get::<_, String>(0),
        "created_at": row.get::<_, DateTime<Utc>>(1),
        "expires_at": expires,
        "state": state_text,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceDecision {
    user_code: String,
    approve: bool,
}

pub async fn device_approve(
    State(state): State<Shared>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Json<Value>> {
    let principal = require_user(&state, &headers).await?;
    if !matches!(principal, Principal::User { session: true, .. }) {
        return Err(ApiError::forbidden("approve sign-in codes in the browser"));
    }
    same_origin(&state, &headers, &principal)?;
    let request: DeviceDecision = parse_json(&body)?;
    let code = normalize_code(&request.user_code);
    let db = state.identity()?.db.get().await?;
    let updated = db
        .execute(
            "UPDATE device_codes SET approved_by = CASE WHEN $2 THEN $3 ELSE NULL END, approved_at = now(), denied = NOT $2
             WHERE user_code = $1 AND expires_at > now() AND approved_by IS NULL AND NOT denied AND consumed_at IS NULL",
            &[&code, &request.approve, &principal.user_id().unwrap_or_default()],
        )
        .await?;
    if updated == 0 {
        return Err(ApiError::not_found(
            "this code is unknown, expired or already used",
        ));
    }
    Ok(Json(json!({"approved": request.approve})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceToken {
    device_code: String,
}

pub async fn device_token(
    State(state): State<Shared>,
    body: axum::body::Bytes,
) -> ApiResult<Json<Value>> {
    let request: DeviceToken = parse_json(&body)?;
    let pending = |error: &str| ApiError::bad_request(error);
    let mut db = state.identity()?.db.get().await?;
    let tx = db.transaction().await?;
    let row = tx
        .query_opt(
            "SELECT client, expires_at, approved_by, denied, consumed_at IS NOT NULL, last_poll_at FROM device_codes WHERE device_sha256 = $1 FOR UPDATE",
            &[&digest(&request.device_code)],
        )
        .await?
        .ok_or_else(|| pending("expired_token"))?;
    let expires: DateTime<Utc> = row.get(1);
    if row.get::<_, bool>(4) || expires < Utc::now() {
        return Err(pending("expired_token"));
    }
    if row.get::<_, bool>(3) {
        return Err(pending("access_denied"));
    }
    let last_poll: Option<DateTime<Utc>> = row.get(5);
    tx.execute(
        "UPDATE device_codes SET last_poll_at = now() WHERE device_sha256 = $1",
        &[&digest(&request.device_code)],
    )
    .await?;
    let Some(user_id) = row.get::<_, Option<String>>(2) else {
        tx.commit().await?;
        if last_poll.is_some_and(|t| Utc::now() - t < Duration::seconds(DEVICE_INTERVAL - 1)) {
            return Err(pending("slow_down"));
        }
        return Err(pending("authorization_pending"));
    };
    let token = random_token("eplyx_u_");
    let token_id = new_id("tok_");
    let expires_at = Utc::now() + Duration::days(USER_TOKEN_DAYS);
    let client_label: String = row.get(0);
    tx.execute(
        "INSERT INTO api_tokens (id, token_sha256, kind, user_id, label, expires_at) VALUES ($1, $2, 'user', $3, $4, $5)",
        &[&token_id, &digest(&token), &user_id, &client_label, &expires_at],
    )
    .await?;
    tx.execute(
        "UPDATE device_codes SET consumed_at = now() WHERE device_sha256 = $1",
        &[&digest(&request.device_code)],
    )
    .await?;
    let user = tx
        .query_one("SELECT email, name FROM users WHERE id = $1", &[&user_id])
        .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "access_token": token,
        "token_type": "Bearer",
        "scope": "user",
        "expires_at": expires_at,
        "user": {"email": user.get::<_, String>(0), "name": user.get::<_, String>(1)},
    })))
}

/// Legacy MAIN filesystem tokens retain validity and revocation semantics while
/// workspace-issued tokens use the same eplyx_proj_ capability in Postgres.
pub async fn require_for(
    state: &crate::api::AppState,
    headers: &HeaderMap,
    project_id: &str,
) -> ApiResult<Principal> {
    if let Some(secret) = bearer(headers) {
        if let Ok(token) = state.registry.authenticate_token(project_id, secret) {
            state.registry.note_token_use(&token);
            return Ok(Principal::Project {
                token_id: token.token_id,
                project_id: project_id.into(),
                label: token.label,
            });
        }
    }
    require(state, headers).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_tokens_are_random_and_well_formed() {
        let code = user_code();
        assert_eq!(code.len(), 9);
        assert_eq!(normalize_code(&code.to_lowercase().replace('-', " ")), code);
        assert_ne!(random_token("eplyx_u_"), random_token("eplyx_u_"));
        assert!(random_token("eplyx_u_").len() > 40);
        assert!(crate::cloud::identity_id(&new_id("ws_"), "ws_"));
        assert_eq!(normalize_code("abc"), "");
    }

    #[test]
    fn passwords_verify_only_with_their_hash() {
        let hash = hash_password("correct horse battery").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("wrong horse battery", &hash));
    }
}
