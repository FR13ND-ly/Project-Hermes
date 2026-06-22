use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use crate::models::app_model::{AppInstanceType, AppStatus};
use crate::dtos::env_variable_dto::EnvVarInput;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAppRequest {
    pub project_id: Uuid,
    pub name: String,
    pub git_repository: String,
    pub branch_name: Option<String>,
    pub build_command: Option<String>,
    pub start_command: Option<String>,
    pub internal_port: Option<i32>,
    pub external_port: Option<i32>,
    pub git_subpath: Option<String>,
    /// Workspace git credential used to clone + detect this app's repo (None = legacy/SSH/public).
    #[serde(default)]
    pub git_credential_id: Option<Uuid>,
    #[serde(default)]
    pub env_variables: Option<Vec<EnvVarInput>>,
    /// Project-pool env vars to link the new instance to at creation time.
    #[serde(default)]
    pub linked_project_env_ids: Option<Vec<Uuid>>,
    #[serde(default)]
    pub enable_baas: Option<bool>,
    /// Custom in-cluster service/DNS name other apps use to reach this one.
    /// None/empty = auto (hermes-app-<slug>-<branch>).
    #[serde(default)]
    pub network_name: Option<String>,
    /// Publish this app's URL into the project env pool. None or true = publish.
    #[serde(default)]
    pub publish_url: Option<bool>,
    /// Env key for the published URL (uppercased). None = <SLUG>_URL.
    #[serde(default)]
    pub url_env_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBranchRequest {
    pub branch_name: String,
    pub instance_type: AppInstanceType,
    pub internal_port: Option<i32>,
    pub cpu_limit: Option<i32>,
    pub memory_limit_mb: Option<i64>,
    pub external_port: Option<i32>,
    pub replicas_min: Option<i32>,
    pub replicas_max: Option<i32>,
    /// Average CPU % target for the autoscaler (only used when replicas_max > min).
    pub autoscale_cpu_percent: Option<i32>,
    /// Scale to 0 when idle. Defaults to enabled for non-production instances.
    pub auto_sleep_enabled: Option<bool>,
    pub auto_sleep_after_minutes: Option<i32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInstanceResponse {
    pub id: Uuid,
    pub branch_name: String,
    pub instance_type: AppInstanceType,
    pub status: AppStatus,
    pub internal_port: i32,
    pub assigned_domain: Option<String>,
    pub container_name: String,
    /// In-cluster service alias other apps use to reach this one (None = old/auto).
    pub network_alias: Option<String>,
    pub external_port: Option<i32>,
    pub meta_data: serde_json::Value,
    pub cpu_limit: i32,
    pub memory_limit_mb: i64,
    pub replicas_min: i32,
    pub replicas_max: i32,
    pub autoscale_cpu_percent: i32,
    pub auto_sleep_enabled: bool,
    pub auto_sleep_after_minutes: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppDetailedResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub slug: String,
    #[serde(rename = "git_repo_url")]
    pub git_repository: String,
    /// Kubernetes namespace this app's workloads live in (hermes-ws-<workspace_id>).
    pub namespace: String,
    pub instances: Vec<AppInstanceResponse>,
    pub git_subpath: Option<String>,
    pub git_credential_id: Option<Uuid>,
    pub build_command: Option<String>,
    pub start_command: Option<String>,
    #[serde(rename = "created_at")]
    pub created_at: DateTime<Utc>,
    pub enable_baas: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigureServerlessRequest {
    pub enabled: bool,
    pub min_scale: i32,
    pub max_scale: i32,
    pub target_concurrency: i32,
}