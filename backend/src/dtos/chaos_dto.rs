use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Request to start a chaos experiment on an app instance.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartChaosRequest {
    /// "pod_kill" | "scale_down" | "cpu_stress".
    pub kind: String,
    /// Auto-revert window in seconds (scale_down / cpu_stress). Capped server-side.
    #[serde(default)]
    pub duration_sec: Option<i64>,
    /// pod_kill: kill all pods vs a single one (default: one).
    #[serde(default)]
    pub target_all_pods: Option<bool>,
    /// scale_down: replica count to drop to (default: 0).
    #[serde(default)]
    pub target_replicas: Option<i32>,
    /// cpu_stress: number of busy-loop workers (default: 1).
    #[serde(default)]
    pub cpu_workers: Option<i32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChaosExperimentResponse {
    pub id: Uuid,
    pub kind: String,
    pub status: String,
    pub message: Option<String>,
    pub params: serde_json::Value,
    pub original_replicas: Option<i32>,
    pub started_at: DateTime<Utc>,
    pub revert_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}
