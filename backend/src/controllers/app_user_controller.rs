use axum::{
    extract::{State, Path, Query},
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::models::app_user_model::AppUser;
use crate::dtos::app_user_dto::{
    AssignRoleRequest, RemoveRoleRequest, AppUserWithRolesResponse,
    AppUserRegisterRequest, AppUserLoginRequest, AppUserAuthResponse
};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;

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


#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct AppUserClaims {
    pub sub: Uuid,
    pub app_id: Uuid,
    pub email: String,
    pub roles: Vec<String>,
    pub exp: i64,
}

pub async fn assign_user_role_to_app(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
    Json(payload): Json<AssignRoleRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM apps WHERE id = $1 AND workspace_id = $2", app_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    let app_user = sqlx::query!("SELECT id FROM app_users WHERE email = $1", payload.email)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("User with this email does not exist in Hermes App Engine.".to_string()))?;

    sqlx::query!(
        "INSERT INTO app_user_roles (id, app_id, app_user_id, role)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (app_id, app_user_id, role) DO NOTHING",
        Uuid::new_v4(), app_id, app_user.id, payload.role.trim().to_lowercase()
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::OK)
}

pub async fn remove_user_role_from_app(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
    Json(payload): Json<RemoveRoleRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM apps WHERE id = $1 AND workspace_id = $2", app_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    sqlx::query!(
        "DELETE FROM app_user_roles WHERE app_id = $1 AND app_user_id = $2 AND role = $3",
        app_id, payload.app_user_id, payload.role.trim().to_lowercase()
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_app_users_with_roles(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
    Query(query): Query<UsersQuery>,
) -> Result<Json<PaginatedUsersResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM apps WHERE id = $1 AND workspace_id = $2", app_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    let search_pattern = query.search.as_ref().map(|s| format!("%{}%", s.trim().to_lowercase()));

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT au.id)
         FROM app_users au
         JOIN app_user_roles aur ON au.id = aur.app_user_id
         WHERE aur.app_id = $1
           AND ($2::text IS NULL OR au.email ILIKE $2 OR au.full_name ILIKE $2)",
    )
    .bind(app_id)
    .bind(search_pattern.clone())
    .fetch_one(&state.pool)
    .await?;

    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(10);
    let offset = (page - 1) * limit;

    let records = sqlx::query!(
        r#"
        SELECT 
            au.id as "user_id!", 
            au.email as "email!", 
            au.full_name as "full_name!", 
            au.status as "status!", 
            au.last_login as "last_login?", 
            array_agg(aur.role) as "roles!"
        FROM app_users au
        JOIN app_user_roles aur ON au.id = aur.app_user_id
        WHERE aur.app_id = $1
          AND ($2::text IS NULL OR au.email ILIKE $2 OR au.full_name ILIKE $2)
        GROUP BY au.id, au.email, au.full_name, au.status, au.last_login
        ORDER BY au.email ASC
        LIMIT $3 OFFSET $4
        "#,
        app_id,
        search_pattern,
        limit,
        offset
    )
    .fetch_all(&state.pool)
    .await?;

    let users = records
        .into_iter()
        .map(|rec| AppUserWithRolesResponse {
            app_user_id: rec.user_id,
            email: rec.email,
            full_name: rec.full_name,
            status: rec.status,
            last_login: rec.last_login.map(|dt| dt.with_timezone(&chrono::Utc)),
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
    Path(app_id): Path<Uuid>,
    Json(payload): Json<AppUserRegisterRequest>,
) -> Result<(StatusCode, Json<AppUserAuthResponse>), AppError> {
    let email_clean = payload.email.trim().to_lowercase();

    let user_exists = sqlx::query!("SELECT id FROM app_users WHERE email = $1", email_clean)
        .fetch_optional(&state.pool)
        .await?;

    let user_id = match user_exists {
        Some(user) => {
            let already_in_app = sqlx::query!(
                "SELECT id FROM app_user_roles WHERE app_id = $1 AND app_user_id = $2",
                app_id, user.id
            )
            .fetch_optional(&state.pool)
            .await?;

            if already_in_app.is_some() {
                return Err(AppError::Validation("User is already registered in this application.".to_string()));
            }
            user.id
        }
        None => {
            let hashed_password = crate::utils::crypto::hash_password(&payload.password_hash)?;
            let new_id = Uuid::new_v4();
            sqlx::query!(
                "INSERT INTO app_users (id, email, password_hash, full_name) VALUES ($1, $2, $3, $4)",
                new_id, email_clean, hashed_password, payload.full_name
            )
            .execute(&state.pool)
            .await?;
            new_id
        }
    };

    let default_role = "user".to_string();
    sqlx::query!(
        "INSERT INTO app_user_roles (id, app_id, app_user_id, role) VALUES ($1, $2, $3, $4)",
        Uuid::new_v4(), app_id, user_id, default_role
    )
    .execute(&state.pool)
    .await?;

    let roles = vec![default_role];
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "secret".to_string());
    let expiration = chrono::Utc::now() + chrono::Duration::days(7);

    let claims = AppUserClaims {
        sub: user_id,
        app_id,
        email: email_clean,
        roles: roles.clone(),
        exp: expiration.timestamp(),
    };

    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(jwt_secret.as_bytes())
    ).map_err(|e| AppError::Fatal(anyhow::anyhow!(e.to_string())))?;

    Ok((
        StatusCode::CREATED,
        Json(AppUserAuthResponse {
            token,
            app_user_id: user_id,
            email: payload.email,
            full_name: payload.full_name,
            roles,
        }),
    ))
}

pub async fn login_public_user(
    State(state): State<AppState>,
    Path(app_id): Path<Uuid>,
    Json(payload): Json<AppUserLoginRequest>,
) -> Result<Json<AppUserAuthResponse>, AppError> {
    let email_clean = payload.email.trim().to_lowercase();

    let user = sqlx::query_as::<_, AppUser>("SELECT * FROM app_users WHERE email = $1")
        .bind(&email_clean)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::Auth("Invalid credentials.".to_string()))?;

    let verified = crate::utils::crypto::verify_password(&payload.password_hash, &user.password_hash)?;
    if !verified {
        return Err(AppError::Auth("Invalid credentials.".to_string()));
    }

    if user.status != "active" {
        return Err(AppError::Auth("Contul dumneavoastră este suspendat sau inactiv.".to_string()));
    }

    // Update last_login audit
    let now = chrono::Utc::now();
    sqlx::query!(
        "UPDATE app_users SET last_login = $1 WHERE id = $2",
        now, user.id
    )
    .execute(&state.pool)
    .await?;

    let role_records = sqlx::query!(
        "SELECT role FROM app_user_roles WHERE app_id = $1 AND app_user_id = $2",
        app_id, user.id
    )
    .fetch_all(&state.pool)
    .await?;

    if role_records.is_empty() {
        return Err(AppError::Permission("User does not have access to this application.".to_string()));
    }

    let roles: Vec<String> = role_records.into_iter().map(|r| r.role).collect();
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "secret".to_string());
    let expiration = chrono::Utc::now() + chrono::Duration::days(7);

    let claims = AppUserClaims {
        sub: user.id,
        app_id,
        email: user.email.clone(),
        roles: roles.clone(),
        exp: expiration.timestamp(),
    };

    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(jwt_secret.as_bytes())
    ).map_err(|e| AppError::Fatal(anyhow::anyhow!(e.to_string())))?;

    Ok(Json(AppUserAuthResponse {
        token,
        app_user_id: user.id,
        email: user.email,
        full_name: user.full_name,
        roles,
    }))
}

// Update app user status
pub async fn update_app_user_status(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((app_id, user_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<crate::dtos::app_user_dto::UpdateUserStatusRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM apps WHERE id = $1 AND workspace_id = $2", app_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    let status_clean = payload.status.trim().to_lowercase();
    if status_clean != "active" && status_clean != "suspended" {
        return Err(AppError::Validation("Invalid status. Must be active or suspended.".to_string()));
    }

    sqlx::query!(
        "UPDATE app_users SET status = $1 WHERE id = $2",
        status_clean, user_id
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::OK)
}

// Reset app user password
pub async fn reset_app_user_password(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((app_id, user_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<crate::dtos::app_user_dto::ResetPasswordRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM apps WHERE id = $1 AND workspace_id = $2", app_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    let hashed_password = crate::utils::crypto::hash_password(&payload.new_password_hash)?;

    sqlx::query!(
        "UPDATE app_users SET password_hash = $1 WHERE id = $2",
        hashed_password, user_id
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::OK)
}

// Get JSON auth configurations (roles & permissions)
pub async fn get_app_auth_config(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app = sqlx::query!(
        "SELECT auth_roles_config FROM apps WHERE id = $1 AND workspace_id = $2",
        app_id,
        ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application not found.".to_string()))?;

    Ok(Json(app.auth_roles_config))
}

// Update JSON auth configurations (roles & permissions)
pub async fn update_app_auth_config(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
    Json(payload): Json<crate::dtos::app_user_dto::UpdateAuthConfigRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM apps WHERE id = $1 AND workspace_id = $2", app_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    sqlx::query!(
        "UPDATE apps SET auth_roles_config = $1 WHERE id = $2",
        payload.auth_roles_config,
        app_id
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::OK)
}

// List API keys
pub async fn list_app_api_keys(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
) -> Result<Json<Vec<crate::dtos::app_user_dto::ApiKeyResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM apps WHERE id = $1 AND workspace_id = $2", app_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    let keys = sqlx::query_as::<_, crate::models::app_user_model::AppApiKey>(
        "SELECT id, app_id, name, key_hash, key_prefix, created_at, expires_at, last_used_at FROM app_api_keys WHERE app_id = $1 ORDER BY created_at DESC",
    )
    .bind(app_id)
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
    Path(app_id): Path<Uuid>,
    Json(payload): Json<crate::dtos::app_user_dto::CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<crate::dtos::app_user_dto::CreateApiKeyResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM apps WHERE id = $1 AND workspace_id = $2", app_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

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
        "INSERT INTO app_api_keys (id, app_id, name, key_hash, key_prefix, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        new_id,
        app_id,
        payload.name.trim(),
        key_hash,
        prefix,
        now,
        payload.expires_at
    )
    .execute(&state.pool)
    .await?;

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
    Path((app_id, key_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM apps WHERE id = $1 AND workspace_id = $2", app_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    sqlx::query!(
        "DELETE FROM app_api_keys WHERE id = $1 AND app_id = $2",
        key_id,
        app_id
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}