use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGitCredentialRequest {
    pub provider: String,        // 'github' | 'gitlab'
    pub host: Option<String>,    // defaults per provider
    pub label: String,
    pub token: String,
    pub skip_tls_verify: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCredentialResponse {
    pub id: Uuid,
    pub provider: String,
    pub host: String,
    pub label: String,
    pub username: Option<String>,
    pub created_at: DateTime<Utc>,
    pub skip_tls_verify: bool,
}

/// Normalized repo entry returned by the provider's repo listing.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRepoResponse {
    /// owner/name (GitHub) or namespace/path (GitLab) — used as the repo ref everywhere.
    pub full_path: String,
    pub name: String,
    pub private: bool,
    pub default_branch: Option<String>,
    pub html_url: Option<String>,
}
