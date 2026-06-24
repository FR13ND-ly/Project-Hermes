use axum::{
    extract::{State, Path, Query},
    http::{StatusCode, HeaderMap},
    Json,
};
use uuid::Uuid;
use sha2::{Sha256, Digest};

use crate::app_state::AppState;
use crate::models::app_user_model::AppUser;
use crate::dtos::app_user_dto::{
    AssignRoleRequest, RemoveRoleRequest, AppUserWithRolesResponse,
    AppUserRegisterRequest, AppUserLoginRequest, AppUserAuthResponse,
    RefreshTokenRequest, LogoutRequest,
    VerifyTokenRequest, VerifyTokenResponse, VerifyKeyRequest, VerifyKeyResponse,
    AuthIntegrationResponse,
};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::models::baas_model::BaasService;
use crate::utils::error::AppError;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBaasServiceRequest {
    pub project_id: Uuid,
    pub name: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaasServiceResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<BaasService> for BaasServiceResponse {
    fn from(s: BaasService) -> Self {
        BaasServiceResponse {
            id: s.id,
            project_id: s.project_id,
            name: s.name,
            slug: s.slug,
            created_at: s.created_at,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct UsersQuery {
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub search: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedUsersResponse {
    pub users: Vec<AppUserWithRolesResponse>,
    pub total: i64,
    pub page: i64,
    pub limit: i64,
    pub pages: i64,
}


/// Claims embedded in a BaaS access token. `extra` carries backend-supplied custom
/// claims (flattened to the top level); on verification it captures any unknown keys.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct AppUserClaims {
    pub sub: Uuid,
    pub baas_id: Uuid,
    pub identifier: String,
    pub roles: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    pub iat: i64,
    pub exp: i64,
    pub jti: Uuid,
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Access-token lifetime (seconds). Short by design; clients refresh. Override with
/// `BAAS_ACCESS_EXPIRY`.
fn access_ttl_secs() -> i64 {
    std::env::var("BAAS_ACCESS_EXPIRY").ok().and_then(|s| s.parse().ok()).unwrap_or(900)
}

/// Refresh-token lifetime (seconds). Override with `BAAS_REFRESH_EXPIRY`. Default 30d.
fn refresh_ttl_secs() -> i64 {
    std::env::var("BAAS_REFRESH_EXPIRY").ok().and_then(|s| s.parse().ok()).unwrap_or(2_592_000)
}

/// Claim names the backend may NOT override via `additionalClaims` — they are owned
/// by Hermes and define the token's identity/authorization.
const RESERVED_CLAIMS: &[&str] = &[
    "sub", "baas_id", "identifier", "roles", "permissions", "iat", "exp", "jti", "iss", "aud", "nbf",
];

#[derive(Debug, sqlx::FromRow)]
struct AppAuthCtx {
    workspace_id: Uuid,
    project_id: Uuid,
    auth_roles_config: serde_json::Value,
}

async fn fetch_app_ctx(pool: &sqlx::PgPool, baas_id: Uuid) -> Result<AppAuthCtx, AppError> {
    sqlx::query_as::<_, AppAuthCtx>(
        "SELECT workspace_id, project_id, auth_roles_config FROM baas_services WHERE id = $1",
    )
    .bind(baas_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application not found.".to_string()))
}

async fn fetch_roles(pool: &sqlx::PgPool, baas_id: Uuid, user_id: Uuid) -> Result<Vec<String>, AppError> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT role FROM app_user_roles WHERE baas_id = $1 AND app_user_id = $2",
    )
    .bind(baas_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

/// Confirm the app exists in the caller's workspace (admin/dashboard endpoints).
async fn ensure_app_in_ws(pool: &sqlx::PgPool, baas_id: Uuid, ws_id: Uuid) -> Result<(), AppError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM baas_services WHERE id = $1 AND workspace_id = $2)",
    )
    .bind(baas_id)
    .bind(ws_id)
    .fetch_one(pool)
    .await?;
    if !exists {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }
    Ok(())
}

struct IssuedTokens {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

/// Mint an access token (signed with the app's secret) + a fresh rotating refresh
/// token (stored only as a SHA-256 hash, with the custom claims persisted so a later
/// refresh re-issues them).
async fn issue_token_pair(
    pool: &sqlx::PgPool,
    baas_id: Uuid,
    user_id: Uuid,
    identifier: &str,
    roles: &[String],
    permissions: &[String],
    secret: &str,
    extra: serde_json::Map<String, serde_json::Value>,
) -> Result<IssuedTokens, AppError> {
    let now = chrono::Utc::now();
    let access_ttl = access_ttl_secs();
    let exp = now + chrono::Duration::seconds(access_ttl);

    let claims = AppUserClaims {
        sub: user_id,
        baas_id,
        identifier: identifier.to_string(),
        roles: roles.to_vec(),
        permissions: permissions.to_vec(),
        iat: now.timestamp(),
        exp: exp.timestamp(),
        jti: Uuid::new_v4(),
        extra: extra.clone(),
    };

    let access_token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Fatal(anyhow::anyhow!(e.to_string())))?;

    let refresh_token = format!("{}.{}", Uuid::new_v4(), Uuid::new_v4());
    let refresh_hash = format!("{:x}", Sha256::digest(refresh_token.as_bytes()));
    let refresh_exp = now + chrono::Duration::seconds(refresh_ttl_secs());

    sqlx::query(
        "INSERT INTO app_refresh_tokens (id, baas_id, app_user_id, token_hash, additional_claims, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(baas_id)
    .bind(user_id)
    .bind(&refresh_hash)
    .bind(serde_json::Value::Object(extra))
    .bind(refresh_exp)
    .execute(pool)
    .await?;

    Ok(IssuedTokens { access_token, refresh_token, expires_in: access_ttl })
}

/// Resolve the custom claims to embed. Only a server-to-server caller that proves it
/// is the app backend — by sending the app's signing secret in `X-Hermes-Auth-Secret`
/// — may inject claims; otherwise a present `additionalClaims` is rejected (so a
/// public end user can't escalate). Reserved claim names are always stripped.
fn resolve_additional_claims(
    headers: &axum::http::HeaderMap,
    requested: Option<serde_json::Value>,
    app_secret: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, AppError> {
    let obj = match requested {
        None | Some(serde_json::Value::Null) => return Ok(serde_json::Map::new()),
        Some(serde_json::Value::Object(m)) if m.is_empty() => return Ok(serde_json::Map::new()),
        Some(serde_json::Value::Object(m)) => m,
        Some(_) => return Err(AppError::Validation("additionalClaims must be a JSON object.".to_string())),
    };

    // Constant-time-ish secret check (compare digests, not the raw secrets).
    let provided = headers
        .get("x-hermes-auth-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let ok = !provided.is_empty()
        && Sha256::digest(provided.as_bytes()) == Sha256::digest(app_secret.as_bytes());
    if !ok {
        return Err(AppError::Permission(
            "additionalClaims requires the app auth secret in the X-Hermes-Auth-Secret header.".to_string(),
        ));
    }

    Ok(obj.into_iter().filter(|(k, _)| !RESERVED_CLAIMS.contains(&k.as_str())).collect())
}

/// Client IP honouring the reverse proxy's forwarding headers (for rate-limiting).
fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Throttle public BaaS auth attempts per (app, client IP) to blunt brute-force.
/// Shared across replicas via the Postgres-backed limiter (20 / 5 min).
async fn enforce_baas_rate_limit(state: &AppState, headers: &HeaderMap, baas_id: Uuid) -> Result<(), AppError> {
    let key = format!("baas:{}:{}", baas_id, client_ip(headers));
    if !crate::utils::locks::check_rate_limit(&state.pool, &key, 20, 300).await {
        return Err(AppError::RateLimited(
            "Too many authentication attempts. Please try again in a few minutes.".to_string(),
        ));
    }
    Ok(())
}

pub async fn assign_user_role_to_app(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(baas_id): Path<Uuid>,
    Json(payload): Json<AssignRoleRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;
    ensure_app_in_ws(&state.pool, baas_id, ws_id).await?;

    let user_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM app_users WHERE baas_id = $1 AND identifier = $2",
    )
    .bind(baas_id)
    .bind(payload.identifier.trim())
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("No user with this identifier in this application.".to_string()))?;

    sqlx::query(
        "INSERT INTO app_user_roles (id, baas_id, app_user_id, role)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (baas_id, app_user_id, role) DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(baas_id)
    .bind(user_id)
    .bind(payload.role.trim().to_lowercase())
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::OK)
}

pub async fn remove_user_role_from_app(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(baas_id): Path<Uuid>,
    Json(payload): Json<RemoveRoleRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM baas_services WHERE id = $1 AND workspace_id = $2", baas_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    sqlx::query!(
        "DELETE FROM app_user_roles WHERE baas_id = $1 AND app_user_id = $2 AND role = $3",
        baas_id, payload.app_user_id, payload.role.trim().to_lowercase()
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, sqlx::FromRow)]
struct UserRoleRow {
    app_user_id: Uuid,
    identifier: String,
    status: String,
    last_login: Option<chrono::DateTime<chrono::Utc>>,
    roles: Vec<String>,
}

pub async fn list_app_users_with_roles(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(baas_id): Path<Uuid>,
    Query(query): Query<UsersQuery>,
) -> Result<Json<PaginatedUsersResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;
    ensure_app_in_ws(&state.pool, baas_id, ws_id).await?;

    let search_pattern = query.search.as_ref().map(|s| format!("%{}%", s.trim().to_lowercase()));

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT au.id)
         FROM app_users au
         JOIN app_user_roles aur ON au.id = aur.app_user_id
         WHERE aur.baas_id = $1
           AND ($2::text IS NULL OR au.identifier ILIKE $2)",
    )
    .bind(baas_id)
    .bind(search_pattern.clone())
    .fetch_one(&state.pool)
    .await?;

    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(10);
    let offset = (page - 1) * limit;

    let records = sqlx::query_as::<_, UserRoleRow>(
        r#"
        SELECT
            au.id          AS app_user_id,
            au.identifier  AS identifier,
            au.status      AS status,
            au.last_login  AS last_login,
            array_agg(aur.role) AS roles
        FROM app_users au
        JOIN app_user_roles aur ON au.id = aur.app_user_id
        WHERE aur.baas_id = $1
          AND ($2::text IS NULL OR au.identifier ILIKE $2)
        GROUP BY au.id, au.identifier, au.status, au.last_login
        ORDER BY au.identifier ASC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(baas_id)
    .bind(search_pattern)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let users = records
        .into_iter()
        .map(|rec| AppUserWithRolesResponse {
            app_user_id: rec.app_user_id,
            identifier: rec.identifier,
            status: rec.status,
            last_login: rec.last_login,
            roles: rec.roles,
        })
        .collect();

    let pages = (total as f64 / limit as f64).ceil() as i64;

    Ok(Json(PaginatedUsersResponse {
        users,
        total,
        page,
        limit,
        pages,
    }))
}

pub async fn register_public_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(baas_id): Path<Uuid>,
    Json(payload): Json<AppUserRegisterRequest>,
) -> Result<(StatusCode, Json<AppUserAuthResponse>), AppError> {
    enforce_baas_rate_limit(&state, &headers, baas_id).await?;
    let app = fetch_app_ctx(&state.pool, baas_id).await?;

    let identifier = payload.identifier.trim().to_string();
    if identifier.is_empty() {
        return Err(AppError::Validation("identifier is required.".to_string()));
    }
    if payload.password.len() < 8 {
        return Err(AppError::Validation("Password must be at least 8 characters.".to_string()));
    }

    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM app_users WHERE baas_id = $1 AND identifier = $2)",
    )
    .bind(baas_id)
    .bind(&identifier)
    .fetch_one(&state.pool)
    .await?;
    if exists {
        return Err(AppError::Conflict(
            "This identifier is already registered in this application.".to_string(),
        ));
    }

    let hashed_password = crate::utils::crypto::hash_password(&payload.password)?;
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO app_users (id, baas_id, identifier, password_hash) VALUES ($1, $2, $3, $4)")
        .bind(user_id)
        .bind(baas_id)
        .bind(&identifier)
        .bind(hashed_password)
        .execute(&state.pool)
        .await?;

    let roles = vec!["user".to_string()];
    sqlx::query("INSERT INTO app_user_roles (id, baas_id, app_user_id, role) VALUES ($1, $2, $3, $4)")
        .bind(Uuid::new_v4())
        .bind(baas_id)
        .bind(user_id)
        .bind(&roles[0])
        .execute(&state.pool)
        .await?;

    let permissions = crate::utils::app_auth::permissions_for_roles(&app.auth_roles_config, &roles);
    let secret = crate::utils::app_auth::get_or_create_secret(
        &state.pool, baas_id, app.workspace_id, app.project_id,
    ).await?;
    let extra = resolve_additional_claims(&headers, payload.additional_claims, &secret)?;

    let tokens = issue_token_pair(
        &state.pool, baas_id, user_id, &identifier, &roles, &permissions, &secret, extra,
    ).await?;

    Ok((
        StatusCode::CREATED,
        Json(AppUserAuthResponse {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            token_type: "Bearer".to_string(),
            expires_in: tokens.expires_in,
            app_user_id: user_id,
            identifier,
            roles,
            permissions,
        }),
    ))
}

pub async fn login_public_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(baas_id): Path<Uuid>,
    Json(payload): Json<AppUserLoginRequest>,
) -> Result<Json<AppUserAuthResponse>, AppError> {
    enforce_baas_rate_limit(&state, &headers, baas_id).await?;
    let app = fetch_app_ctx(&state.pool, baas_id).await?;
    let identifier = payload.identifier.trim().to_string();

    let user = sqlx::query_as::<_, AppUser>(
        "SELECT * FROM app_users WHERE baas_id = $1 AND identifier = $2",
    )
    .bind(baas_id)
    .bind(&identifier)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Auth("Invalid credentials.".to_string()))?;

    if !crate::utils::crypto::verify_password(&payload.password, &user.password_hash)? {
        return Err(AppError::Auth("Invalid credentials.".to_string()));
    }
    if user.status != "active" {
        return Err(AppError::Auth("This account is suspended or inactive.".to_string()));
    }

    sqlx::query("UPDATE app_users SET last_login = now() WHERE id = $1")
        .bind(user.id)
        .execute(&state.pool)
        .await?;

    let roles = fetch_roles(&state.pool, baas_id, user.id).await?;
    let permissions = crate::utils::app_auth::permissions_for_roles(&app.auth_roles_config, &roles);
    let secret = crate::utils::app_auth::get_or_create_secret(
        &state.pool, baas_id, app.workspace_id, app.project_id,
    ).await?;
    let extra = resolve_additional_claims(&headers, payload.additional_claims, &secret)?;

    let tokens = issue_token_pair(
        &state.pool, baas_id, user.id, &user.identifier, &roles, &permissions, &secret, extra,
    ).await?;

    Ok(Json(AppUserAuthResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: tokens.expires_in,
        app_user_id: user.id,
        identifier: user.identifier,
        roles,
        permissions,
    }))
}

#[derive(Debug, sqlx::FromRow)]
struct ConsumedRefresh {
    app_user_id: Uuid,
    additional_claims: serde_json::Value,
}

/// PUBLIC: exchange a valid refresh token for a new access + refresh pair. The old
/// refresh token is single-use (deleted on exchange); the custom claims captured at
/// the original issue are carried into the new access token.
pub async fn refresh_app_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(baas_id): Path<Uuid>,
    Json(payload): Json<RefreshTokenRequest>,
) -> Result<Json<AppUserAuthResponse>, AppError> {
    enforce_baas_rate_limit(&state, &headers, baas_id).await?;
    let app = fetch_app_ctx(&state.pool, baas_id).await?;
    let token_hash = format!("{:x}", Sha256::digest(payload.refresh_token.as_bytes()));

    let consumed = sqlx::query_as::<_, ConsumedRefresh>(
        "DELETE FROM app_refresh_tokens
         WHERE baas_id = $1 AND token_hash = $2 AND expires_at > now()
         RETURNING app_user_id, additional_claims",
    )
    .bind(baas_id)
    .bind(&token_hash)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Auth("Invalid or expired refresh token.".to_string()))?;

    let extra = match consumed.additional_claims {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };

    let user = sqlx::query_as::<_, AppUser>(
        "SELECT * FROM app_users WHERE id = $1 AND baas_id = $2",
    )
    .bind(consumed.app_user_id)
    .bind(baas_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Auth("Account no longer exists.".to_string()))?;

    if user.status != "active" {
        return Err(AppError::Auth("This account is suspended or inactive.".to_string()));
    }

    let roles = fetch_roles(&state.pool, baas_id, user.id).await?;
    let permissions = crate::utils::app_auth::permissions_for_roles(&app.auth_roles_config, &roles);
    let secret = crate::utils::app_auth::get_or_create_secret(
        &state.pool, baas_id, app.workspace_id, app.project_id,
    ).await?;

    let tokens = issue_token_pair(
        &state.pool, baas_id, user.id, &user.identifier, &roles, &permissions, &secret, extra,
    ).await?;

    Ok(Json(AppUserAuthResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: tokens.expires_in,
        app_user_id: user.id,
        identifier: user.identifier,
        roles,
        permissions,
    }))
}

/// PUBLIC: revoke a refresh token (logout). Idempotent — unknown tokens are a no-op.
/// Access tokens are short-lived and expire on their own.
pub async fn logout_app_user(
    State(state): State<AppState>,
    Path(baas_id): Path<Uuid>,
    Json(payload): Json<LogoutRequest>,
) -> Result<StatusCode, AppError> {
    let token_hash = format!("{:x}", Sha256::digest(payload.refresh_token.as_bytes()));
    sqlx::query("DELETE FROM app_refresh_tokens WHERE baas_id = $1 AND token_hash = $2")
        .bind(baas_id)
        .bind(&token_hash)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// Update app user status
pub async fn update_app_user_status(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((baas_id, user_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<crate::dtos::app_user_dto::UpdateUserStatusRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;
    ensure_app_in_ws(&state.pool, baas_id, ws_id).await?;

    let status_clean = payload.status.trim().to_lowercase();
    if status_clean != "active" && status_clean != "suspended" {
        return Err(AppError::Validation("Invalid status. Must be active or suspended.".to_string()));
    }

    sqlx::query("UPDATE app_users SET status = $1, updated_at = now() WHERE id = $2 AND baas_id = $3")
        .bind(&status_clean)
        .bind(user_id)
        .bind(baas_id)
        .execute(&state.pool)
        .await?;

    // A suspended user must not be able to mint new access tokens via refresh.
    if status_clean == "suspended" {
        let _ = sqlx::query("DELETE FROM app_refresh_tokens WHERE app_user_id = $1")
            .bind(user_id)
            .execute(&state.pool)
            .await;
    }

    Ok(StatusCode::OK)
}

// Reset app user password
pub async fn reset_app_user_password(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((baas_id, user_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<crate::dtos::app_user_dto::ResetPasswordRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;
    ensure_app_in_ws(&state.pool, baas_id, ws_id).await?;

    if payload.new_password.len() < 8 {
        return Err(AppError::Validation("Password must be at least 8 characters.".to_string()));
    }
    let hashed_password = crate::utils::crypto::hash_password(&payload.new_password)?;

    sqlx::query("UPDATE app_users SET password_hash = $1, updated_at = now() WHERE id = $2 AND baas_id = $3")
        .bind(hashed_password)
        .bind(user_id)
        .bind(baas_id)
        .execute(&state.pool)
        .await?;

    // Force re-login everywhere after a password reset.
    let _ = sqlx::query("DELETE FROM app_refresh_tokens WHERE app_user_id = $1")
        .bind(user_id)
        .execute(&state.pool)
        .await;

    Ok(StatusCode::OK)
}

// Get JSON auth configurations (roles & permissions)
pub async fn get_app_auth_config(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(baas_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let svc = sqlx::query!(
        "SELECT auth_roles_config FROM baas_services WHERE id = $1 AND workspace_id = $2",
        baas_id,
        ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Auth service not found.".to_string()))?;

    Ok(Json(svc.auth_roles_config))
}

// Update JSON auth configurations (roles & permissions)
pub async fn update_app_auth_config(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(baas_id): Path<Uuid>,
    Json(payload): Json<crate::dtos::app_user_dto::UpdateAuthConfigRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let svc_exists = sqlx::query!("SELECT id FROM baas_services WHERE id = $1 AND workspace_id = $2", baas_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if svc_exists.is_none() {
        return Err(AppError::NotFound("Auth service not found in this workspace.".to_string()));
    }

    sqlx::query!(
        "UPDATE baas_services SET auth_roles_config = $1, updated_at = now() WHERE id = $2",
        payload.auth_roles_config,
        baas_id
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::OK)
}

// List API keys
pub async fn list_app_api_keys(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(baas_id): Path<Uuid>,
) -> Result<Json<Vec<crate::dtos::app_user_dto::ApiKeyResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM baas_services WHERE id = $1 AND workspace_id = $2", baas_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    let keys = sqlx::query_as::<_, crate::models::app_user_model::AppApiKey>(
        "SELECT id, baas_id, name, key_hash, key_prefix, created_at, expires_at, last_used_at FROM app_api_keys WHERE baas_id = $1 ORDER BY created_at DESC",
    )
    .bind(baas_id)
    .fetch_all(&state.pool)
    .await?;

    let result = keys
        .into_iter()
        .map(|k| crate::dtos::app_user_dto::ApiKeyResponse {
            id: k.id,
            name: k.name,
            key_prefix: k.key_prefix,
            created_at: k.created_at,
            expires_at: k.expires_at,
            last_used_at: k.last_used_at,
        })
        .collect();

    Ok(Json(result))
}

// Generate new API Key
pub async fn create_app_api_key(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(baas_id): Path<Uuid>,
    Json(payload): Json<crate::dtos::app_user_dto::CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<crate::dtos::app_user_dto::CreateApiKeyResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_row = sqlx::query!("SELECT project_id FROM baas_services WHERE id = $1 AND workspace_id = $2", baas_id, ws_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Application not found in this workspace.".to_string()))?;
    let project_id = app_row.project_id;

    let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let prefix: String = format!(
        "hm_{}",
        (0..8)
            .map(|_| {
                let idx = (rand::random::<u32>() as usize) % chars.len();
                chars[idx] as char
            })
            .collect::<String>()
    );

    let secret: String = (0..32)
        .map(|_| {
            let idx = (rand::random::<u32>() as usize) % chars.len();
            chars[idx] as char
        })
        .collect::<String>();

    let raw_key = format!("{}.{}", prefix, secret);
    let key_hash = crate::utils::crypto::hash_password(&secret)?;

    let new_id = Uuid::new_v4();
    let now = chrono::Utc::now();

    sqlx::query!(
        "INSERT INTO app_api_keys (id, baas_id, name, key_hash, key_prefix, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        new_id,
        baas_id,
        payload.name.trim(),
        key_hash,
        prefix,
        now,
        payload.expires_at
    )
    .execute(&state.pool)
    .await?;

    // Publish the BaaS API key into the app's project pool so any app in the
    // project can opt into it (e.g. to call this app's authenticated backend).
    let key_name = format!(
        "{}_API_KEY",
        crate::utils::app_env::sanitize_key_fragment(&payload.name, "BAAS")
    );
    let _ = crate::utils::app_env::publish_project_env(
        &state.pool, ws_id, project_id, &key_name, &raw_key, true, "baas", new_id,
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(crate::dtos::app_user_dto::CreateApiKeyResponse {
            id: new_id,
            name: payload.name.trim().to_string(),
            key_prefix: prefix,
            raw_key,
            created_at: now,
            expires_at: payload.expires_at,
        }),
    ))
}

// Revoke/Delete API Key
pub async fn delete_app_api_key(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((baas_id, key_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM baas_services WHERE id = $1 AND workspace_id = $2", baas_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    sqlx::query!(
        "DELETE FROM app_api_keys WHERE id = $1 AND baas_id = $2",
        key_id,
        baas_id
    )
    .execute(&state.pool)
    .await?;

    // Remove the published project-pool var and reload any linked apps.
    let linked = crate::utils::app_env::unpublish_project_env(&state.pool, "baas", key_id).await;
    for inst in linked {
        crate::controllers::env_variable_controller::hot_reload_if_running(&state.pool, inst);
    }

    Ok(StatusCode::NO_CONTENT)
}

// PUBLIC: verify an end-user JWT minted by Hermes for this app. Convenience/
// fallback for stacks without a JWT library — normal verification is done locally
// in the app with HERMES_AUTH_SECRET. Permissions are recomputed from the app's
// current auth_roles_config so config changes take effect on existing tokens.
pub async fn verify_app_token(
    State(state): State<AppState>,
    Path(baas_id): Path<Uuid>,
    Json(payload): Json<VerifyTokenRequest>,
) -> Result<Json<VerifyTokenResponse>, AppError> {
    let app = sqlx::query!(
        "SELECT workspace_id, project_id, auth_roles_config FROM baas_services WHERE id = $1",
        baas_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application not found.".to_string()))?;

    let secret = crate::utils::app_auth::get_or_create_secret(
        &state.pool, baas_id, app.workspace_id, app.project_id,
    ).await?;

    let invalid = || VerifyTokenResponse {
        valid: false,
        app_user_id: None,
        identifier: None,
        roles: vec![],
        permissions: vec![],
        expires_at: None,
    };

    let decoded = jsonwebtoken::decode::<AppUserClaims>(
        &payload.token,
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &jsonwebtoken::Validation::default(),
    );

    let data = match decoded {
        Ok(d) => d,
        Err(_) => return Ok(Json(invalid())),
    };

    // Reject tokens that were signed for a different app.
    if data.claims.baas_id != baas_id {
        return Ok(Json(invalid()));
    }

    let permissions =
        crate::utils::app_auth::permissions_for_roles(&app.auth_roles_config, &data.claims.roles);

    Ok(Json(VerifyTokenResponse {
        valid: true,
        app_user_id: Some(data.claims.sub),
        identifier: Some(data.claims.identifier),
        roles: data.claims.roles,
        permissions,
        expires_at: Some(data.claims.exp),
    }))
}

// PUBLIC: introspect an API key (model 3). Looks the key up by its prefix, verifies
// the secret against the stored hash, checks expiry, and stamps last_used_at.
pub async fn verify_app_key(
    State(state): State<AppState>,
    Path(baas_id): Path<Uuid>,
    Json(payload): Json<VerifyKeyRequest>,
) -> Result<Json<VerifyKeyResponse>, AppError> {
    let invalid = Json(VerifyKeyResponse { valid: false, expired: false, name: None });

    let Some((prefix, secret)) = payload.key.split_once('.') else {
        return Ok(invalid);
    };

    let key = sqlx::query!(
        "SELECT id, name, key_hash, expires_at FROM app_api_keys WHERE baas_id = $1 AND key_prefix = $2",
        baas_id, prefix
    )
    .fetch_optional(&state.pool)
    .await?;

    let Some(key) = key else {
        return Ok(invalid);
    };

    if !crate::utils::crypto::verify_password(secret, &key.key_hash)? {
        return Ok(invalid);
    }

    if let Some(exp) = key.expires_at {
        if exp < chrono::Utc::now() {
            return Ok(Json(VerifyKeyResponse { valid: false, expired: true, name: Some(key.name) }));
        }
    }

    let _ = sqlx::query!(
        "UPDATE app_api_keys SET last_used_at = now() WHERE id = $1",
        key.id
    )
    .execute(&state.pool)
    .await;

    Ok(Json(VerifyKeyResponse { valid: true, expired: false, name: Some(key.name) }))
}

// Integration info for the dashboard: the app id, its (generated) signing secret,
// and the absolute auth endpoint URLs — everything a developer needs to wire up
// local verification.
pub async fn get_auth_integration(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    headers: HeaderMap,
    Path(baas_id): Path<Uuid>,
) -> Result<Json<AuthIntegrationResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app = sqlx::query!(
        "SELECT project_id FROM baas_services WHERE id = $1 AND workspace_id = $2",
        baas_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application not found in this workspace.".to_string()))?;

    let secret = crate::utils::app_auth::get_or_create_secret(
        &state.pool, baas_id, ws_id, app.project_id,
    ).await?;

    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:8000");
    let proto = if host.contains("localhost") || host.contains("127.0.0.1") || host.contains("192.168.") {
        "http"
    } else {
        "https"
    };
    let api_base_url = format!("{}://{}/api/v1", proto, host);

    // Publish only the minimal pair into the project pool (idempotent): the service
    // id; the secret is already published by get_or_create. The API URL + alias vars
    // (HERMES_AUTH_API_URL / HERMES_BAAS_URL / HERMES_APP_ID) were intentionally dropped.
    let _ = crate::utils::app_auth::publish_baas_var(
        &state.pool, ws_id, app.project_id, baas_id, "HERMES_AUTH_APP_ID", &baas_id.to_string(), false,
    ).await;

    Ok(Json(AuthIntegrationResponse {
        baas_id,
        api_base_url: api_base_url.clone(),
        auth_secret_env_key: crate::utils::app_auth::AUTH_SECRET_ENV_KEY.to_string(),
        auth_secret: secret,
        register_endpoint: format!("{}/baas/{}/auth/register", api_base_url, baas_id),
        login_endpoint: format!("{}/baas/{}/auth/login", api_base_url, baas_id),
        refresh_endpoint: format!("{}/baas/{}/auth/refresh", api_base_url, baas_id),
        logout_endpoint: format!("{}/baas/{}/auth/logout", api_base_url, baas_id),
        verify_token_endpoint: format!("{}/baas/{}/auth/verify-token", api_base_url, baas_id),
        verify_key_endpoint: format!("{}/baas/{}/auth/verify-key", api_base_url, baas_id),
    }))
}

/// POST /apps/:id/auth/rotate-secret — generate a fresh BaaS signing secret,
/// republish it to the project pool, and return it. Invalidates every end-user JWT
/// signed with the previous secret (they must re-login); apps pick up the new value
/// on their next reload (no auto-reload).
pub async fn rotate_auth_secret(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(baas_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app = sqlx::query!(
        "SELECT project_id FROM baas_services WHERE id = $1 AND workspace_id = $2",
        baas_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application not found in this workspace.".to_string()))?;

    let secret = crate::utils::app_auth::rotate_secret(
        &state.pool, baas_id, ws_id, app.project_id,
    ).await?;

    Ok(Json(serde_json::json!({
        "auth_secret": secret,
        "auth_secret_env_key": crate::utils::app_auth::AUTH_SECRET_ENV_KEY,
    })))
}
// ─── Standalone BaaS service CRUD (project resource, no app required) ──────────

/// POST /baas — create a standalone BaaS auth service in a project. Generates and
/// publishes its signing secret to the project env pool immediately.
pub async fn create_baas_service(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<CreateBaasServiceRequest>,
) -> Result<(StatusCode, Json<BaasServiceResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Validation("A name is required.".to_string()));
    }

    let belongs = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
    )
    .bind(payload.project_id)
    .bind(ws_id)
    .fetch_one(&state.pool)
    .await?;
    if !belongs {
        return Err(AppError::NotFound("Project not found in this workspace.".to_string()));
    }

    let baas_id = crate::utils::app_auth::create_baas_service(&state.pool, ws_id, payload.project_id, &name).await?;

    let svc = sqlx::query_as::<_, BaasService>("SELECT * FROM baas_services WHERE id = $1")
        .bind(baas_id)
        .fetch_one(&state.pool)
        .await?;

    Ok((StatusCode::CREATED, Json(svc.into())))
}

/// GET /projects/:project_id/baas — list the auth services in a project.
pub async fn list_project_baas_services(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<BaasServiceResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let services = sqlx::query_as::<_, BaasService>(
        "SELECT * FROM baas_services WHERE project_id = $1 AND workspace_id = $2 ORDER BY created_at DESC",
    )
    .bind(project_id)
    .bind(ws_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(services.into_iter().map(Into::into).collect()))
}

/// GET /baas/:id — a single auth service.
pub async fn get_baas_service(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(baas_id): Path<Uuid>,
) -> Result<Json<BaasServiceResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let svc = sqlx::query_as::<_, BaasService>(
        "SELECT * FROM baas_services WHERE id = $1 AND workspace_id = $2",
    )
    .bind(baas_id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Auth service not found.".to_string()))?;

    Ok(Json(svc.into()))
}

/// DELETE /baas/:id — delete an auth service. Removes its published pool vars
/// (reloading any linked apps) and cascades its users/roles/api-keys/tokens.
pub async fn delete_baas_service(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(baas_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM baas_services WHERE id = $1 AND workspace_id = $2)",
    )
    .bind(baas_id)
    .bind(ws_id)
    .fetch_one(&state.pool)
    .await?;
    if !exists {
        return Err(AppError::NotFound("Auth service not found in this workspace.".to_string()));
    }

    // Unpublish the secret/app-id/api-url pool vars and reload any apps linking them.
    let linked = crate::utils::app_env::unpublish_project_env(&state.pool, "baas_auth", baas_id).await;
    for inst in linked {
        crate::controllers::env_variable_controller::hot_reload_if_running(&state.pool, inst);
    }

    sqlx::query("DELETE FROM baas_services WHERE id = $1 AND workspace_id = $2")
        .bind(baas_id)
        .bind(ws_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
