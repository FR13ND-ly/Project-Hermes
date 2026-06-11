use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetEnvRequest {
    pub app_instance_id: Uuid,
    pub key: String,
    pub value: String,
    pub is_secret: Option<bool>,
}

/// A single key/value pair used both for bulk JSON editing and for env
/// provided at app-creation time.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EnvVarInput {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub is_secret: Option<bool>,
}

/// Replace the full set of environment variables for a single instance.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetEnvBulkRequest {
    pub app_instance_id: Uuid,
    pub variables: Vec<EnvVarInput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvResponse {
    pub id: Uuid,
    pub app_instance_id: Uuid,
    pub key: String,
    pub value: Option<String>,
    pub is_secret: bool,
}

/// Create/update a project-pool env var (manual source).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetProjectEnvRequest {
    pub key: String,
    pub value: String,
    pub is_secret: Option<bool>,
}

/// A project-pool env var. When listed in the context of a specific instance,
/// `linked` indicates whether that instance has opted into this var.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEnvResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub key: String,
    pub value: Option<String>,
    pub is_secret: bool,
    pub source: String,
    pub linked: Option<bool>,
}

/// Opt an instance into a project-pool env var.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkProjectEnvRequest {
    pub project_env_id: Uuid,
}

/// Env vars for a single instance, used by the project-level grouped view.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupedInstanceEnv {
    pub instance_id: Uuid,
    pub branch_name: String,
    pub variables: Vec<EnvResponse>,
}

/// All instances (and their env) for a single app, grouped under the app.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupedAppEnv {
    pub app_id: Uuid,
    pub app_name: String,
    pub instances: Vec<GroupedInstanceEnv>,
}
