use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct ComposeStack {
    pub services: HashMap<String, ComposeService>,
}

#[derive(Debug, Deserialize)]
pub struct ComposeService {
    pub image: Option<String>,
    pub build: Option<serde_yaml::Value>,
    pub ports: Option<Vec<String>>,
    pub environment: Option<EnvironmentMapping>,
    pub volumes: Option<Vec<String>>,
    pub depends_on: Option<DependsOn>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum EnvironmentMapping {
    List(Vec<String>),
    Map(HashMap<String, String>),
}

/// compose `depends_on` is either a list or a map of { service: {condition} }.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum DependsOn {
    List(Vec<String>),
    Map(HashMap<String, serde_yaml::Value>),
}

impl DependsOn {
    pub fn names(&self) -> Vec<String> {
        match self {
            DependsOn::List(l) => l.clone(),
            DependsOn::Map(m) => m.keys().cloned().collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportComposeRequest {
    pub project_id: uuid::Uuid,
    pub compose_yaml: String,
}

// ---------- Auto-split plan (preview before creating) ----------

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlanEnv {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlanVolume {
    pub service: String,
    pub name: String,
    pub container_path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlanApp {
    pub service: String,
    pub name: String,
    pub image: Option<String>,
    /// build-context subpath in the repo (None for image-only services).
    pub build_path: Option<String>,
    pub internal_port: i32,
    pub external_port: Option<i32>,
    /// Whether Hermes can build it from the repo (has a build context).
    pub buildable: bool,
    pub env: Vec<PlanEnv>,
    pub volumes: Vec<PlanVolume>,
    pub depends_on: Vec<String>,
    /// Default selection state for the preview.
    pub include: bool,
    /// Create + link a private storage bucket for this service.
    #[serde(default)]
    pub enable_storage: bool,
    /// Custom in-cluster service/DNS name (None/empty = auto hermes-app-<slug>-<branch>).
    #[serde(default)]
    pub network_name: Option<String>,
    /// Publish this app's URL into the project env pool. None or true = publish.
    #[serde(default)]
    pub publish_url: Option<bool>,
    /// Env key for the published URL (uppercased). None = <SERVICE>_URL.
    #[serde(default)]
    pub url_env_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlanDatabase {
    pub service: String,
    pub name: String,
    pub db_type: String,
    pub version: String,
    pub internal_port: i32,
    pub include: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ComposePlan {
    pub apps: Vec<PlanApp>,
    pub databases: Vec<PlanDatabase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRequest {
    pub compose_yaml: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPlanRequest {
    pub project_id: uuid::Uuid,
    pub git_repository: Option<String>,
    pub git_credential_id: Option<uuid::Uuid>,
    pub branch_name: Option<String>,
    pub plan: ComposePlan,
}
