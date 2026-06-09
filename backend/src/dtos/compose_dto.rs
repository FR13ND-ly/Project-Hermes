use serde::Deserialize;
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
    pub depends_on: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum EnvironmentMapping {
    List(Vec<String>),
    Map(HashMap<String, String>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportComposeRequest {
    pub project_id: uuid::Uuid,
    pub compose_yaml: String,
}