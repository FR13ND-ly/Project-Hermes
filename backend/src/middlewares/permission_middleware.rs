use axum::{
    extract::State,
    http::Request,
    middleware::Next,
    response::Response,
    Extension,
};
use crate::app_state::AppState;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;

#[derive(Debug, Clone)]
pub struct RequiredPermission(pub &'static str);

pub async fn check_permission(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Extension(RequiredPermission(permission)): Extension<RequiredPermission>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    if claims.is_super_admin {
        return Ok(next.run(request).await);
    }

    let has_perm = sqlx::query_scalar!(
        "SELECT EXISTS (
            SELECT 1 
            FROM workspace_members wm
            JOIN role_permissions rp ON wm.role_id = rp.role_id
            JOIN permissions p ON rp.permission_id = p.id
            WHERE wm.workspace_id = $1 
              AND wm.user_id = $2 
              AND p.name = $3
        ) as \"has_perm!\"",
        ws_id,
        claims.sub,
        permission
    )
    .fetch_one(&state.pool)
    .await?;

    if !has_perm {
        return Err(AppError::Permission(format!(
            "Permission '{}' required for this workspace.",
            permission
        )));
    }

    Ok(next.run(request).await)
}
