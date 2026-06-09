use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::models::user_model::UserStatus;

#[derive(Debug, Deserialize)]
pub struct ProvisionUserRequest {
    pub username: String,
    pub email: String,
    pub is_super_admin: bool,
    pub initial_workspace_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub login_identity: String,
    pub password: String,
    #[serde(skip_deserializing)]
    pub client_ip: Option<String>,
    #[serde(skip_deserializing)]
    pub user_agent: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ActivateAccountRequest {
    pub email: String,
    pub temporary_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct PasswordChangeRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct SwitchWorkspaceRequest {
    pub workspace_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub status: UserStatus,
    pub is_super_admin: bool,
    pub current_workspace_id: Option<Uuid>,
    pub github_username: Option<String>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ProvisionUserResponse {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub temporary_password: String,
    pub is_super_admin: bool,
    pub status: UserStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub user: UserResponse,

}
impl From<crate::models::user_model::User> for UserResponse {
    fn from(user: crate::models::user_model::User) -> Self {
        Self {
            id: user.id,
            username: user.username,
            email: user.email,
            status: user.status,
            is_super_admin: user.is_super_admin,
            current_workspace_id: user.current_workspace_id,
            github_username: user.github_username,
            last_login_at: user.last_login_at,
            created_at: user.created_at,
        }
    }
}