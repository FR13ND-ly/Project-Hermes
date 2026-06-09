use axum::{
    extract::State,
    http::Request,
    middleware::Next,
    response::Response,
};
use crate::app_state::AppState;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;

pub async fn enforce_workspace_developer(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    if claims.is_super_admin {
        return Ok(next.run(request).await);
    }

    let member_record = sqlx::query!(
        "SELECT r.name as \"user_role!\" 
         FROM workspace_members wm 
         JOIN roles r ON wm.role_id = r.id 
         WHERE wm.workspace_id = $1 AND wm.user_id = $2",
        ws_id,
        claims.sub
    )
    .fetch_optional(&state.pool)
    .await?;

    match member_record {
        Some(row) if row.user_role == "owner" || row.user_role == "admin" || row.user_role == "developer" => {
            Ok(next.run(request).await)
        }
        _ => Err(AppError::Permission("Developer privileges for this workspace required.".to_string())),
    }
}