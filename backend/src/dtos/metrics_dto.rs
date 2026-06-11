use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsHistoryResponse {
    pub timestamps: Vec<i64>,
    pub values: Vec<f64>,
    /// True when the data points are synthetic (Prometheus was unreachable),
    /// so the UI can label them instead of presenting fabricated data as real.
    pub simulated: bool,
}
