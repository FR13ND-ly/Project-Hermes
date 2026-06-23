use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignRoleRequest {
    /// The end user's per-app unique identifier.
    pub identifier: String,
    pub role: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveRoleRequest {
    pub app_user_id: Uuid,
    pub role: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUserWithRolesResponse {
    pub app_user_id: Uuid,
    pub identifier: String,
    pub status: String,
    pub last_login: Option<DateTime<Utc>>,
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUserRegisterRequest {
    /// Whatever the app uses as a unique handle (email, username, phone, …). Opaque.
    pub identifier: String,
    pub password: String,
    /// Extra claims to embed in the access token. Only honoured for server-to-server
    /// calls that prove they are the app backend via the `X-Hermes-Auth-Secret`
    /// header; ignored otherwise. Reserved claim names are stripped.
    #[serde(default)]
    pub additional_claims: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUserLoginRequest {
    pub identifier: String,
    pub password: String,
    #[serde(default)]
    pub additional_claims: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoutRequest {
    pub refresh_token: String,
}

/// Access + refresh token pair, OAuth2-style.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUserAuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    /// Access-token lifetime in seconds.
    pub expires_in: i64,
    pub app_user_id: Uuid,
    pub identifier: String,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserStatusRequest {
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPasswordRequest {
    /// The new plaintext password. `newPasswordHash` accepted as a legacy alias.
    #[serde(alias = "newPasswordHash")]
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAuthConfigRequest {
    pub auth_roles_config: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiKeyResponse {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub raw_key: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyResponse {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyTokenRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyTokenResponse {
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_user_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyKeyRequest {
    pub key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyKeyResponse {
    pub valid: bool,
    pub expired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthIntegrationResponse {
    pub baas_id: Uuid,
    pub api_base_url: String,
    pub auth_secret_env_key: String,
    pub auth_secret: String,
    pub register_endpoint: String,
    pub login_endpoint: String,
    pub refresh_endpoint: String,
    pub logout_endpoint: String,
    pub verify_token_endpoint: String,
    pub verify_key_endpoint: String,
}
