use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsHistoryResponse {
    pub timestamps: Vec<i64>,
    pub values: Vec<f64>,
}
