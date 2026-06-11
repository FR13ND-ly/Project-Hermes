use std::time::Duration;
use chrono::Utc;
use reqwest::Client;
use serde_json::Value;
use crate::utils::error::AppError;

/// Returns `(timestamps, values, simulated)`. `simulated` is true when the data
/// is synthetic because Prometheus could not be reached.
pub async fn get_historical_metrics(
    namespace: &str,
    container_name: &str,
    metric_type: &str,
    range_str: &str,
) -> Result<(Vec<i64>, Vec<f64>, bool), AppError> {
    let now = Utc::now();
    let end_time = now.timestamp();
    
    let (start_time, step_seconds) = match range_str {
        "24h" => (end_time - 86400, 900),  // 96 data points
        "7d" => (end_time - 604800, 7200), // 84 data points
        _ => (end_time - 3600, 60),        // "1h" -> 60 data points (default)
    };

    let query = match metric_type {
        "cpu" => format!(
            "sum(rate(container_cpu_usage_seconds_total{{namespace=\"{}\",pod=~\"{}-.*\"}}[5m]))",
            namespace, container_name
        ),
        "memory" => format!(
            "sum(container_memory_working_set_bytes{{namespace=\"{}\",pod=~\"{}-.*\"}})",
            namespace, container_name
        ),
        "network_rx" => format!(
            "sum(rate(container_network_receive_bytes_total{{namespace=\"{}\"}}[5m]))",
            namespace
        ),
        "network_tx" => format!(
            "sum(rate(container_network_transmit_bytes_total{{namespace=\"{}\"}}[5m]))",
            namespace
        ),
        "fs_read" => format!(
            "sum(rate(container_fs_reads_bytes_total{{namespace=\"{}\",pod=~\"{}-.*\"}}[5m]))",
            namespace, container_name
        ),
        "fs_write" => format!(
            "sum(rate(container_fs_writes_bytes_total{{namespace=\"{}\",pod=~\"{}-.*\"}}[5m]))",
            namespace, container_name
        ),
        "db_size" => format!(
            "sum(container_fs_usage_bytes{{namespace=\"{}\",pod=~\"{}-.*\"}})",
            namespace, container_name
        ),
        "db_connections" => format!(
            "sum(pg_stat_database_numbackends{{namespace=\"{}\",pod=~\"{}-0\"}})",
            namespace, container_name
        ),
        "db_cache_hit_rate" => format!(
            "sum(rate(pg_stat_database_blks_hit{{namespace=\"{}\",pod=~\"{}-0\"}}[5m])) / (sum(rate(pg_stat_database_blks_hit{{namespace=\"{}\",pod=~\"{}-0\"}}[5m])) + sum(rate(pg_stat_database_blks_read{{namespace=\"{}\",pod=~\"{}-0\"}}[5m])) + 1) * 100",
            namespace, container_name, namespace, container_name, namespace, container_name
        ),
        _ => return Err(AppError::Validation(format!("Unsupported metric type: {}", metric_type))),
    };

    let mut got_real_metrics = false;
    let mut timestamps = Vec::new();
    let mut metric_vals = Vec::new();

    // 1. Try direct query to Prometheus (for internal cluster usage)
    let prometheus_url = std::env::var("HERMES_PROMETHEUS_URL")
        .unwrap_or_else(|_| "http://prometheus-k8s.monitoring.svc:9090".to_string());
    
    let request_url = format!("{}/api/v1/query_range", prometheus_url);

    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let response = client
        .get(&request_url)
        .query(&[
            ("query", query.as_str()),
            ("start", &start_time.to_string()),
            ("end", &end_time.to_string()),
            ("step", &format!("{}s", step_seconds)),
        ])
        .send()
        .await;

    if let Ok(res) = response {
        if res.status().is_success() {
            if let Ok(json_body) = res.json::<Value>().await {
                if let Some(data) = json_body.get("data") {
                    if let Some(result) = data.get("result").and_then(|r| r.as_array()) {
                        if let Some(first_result) = result.first() {
                            if let Some(values) = first_result.get("values").and_then(|v| v.as_array()) {
                                for val in values {
                                    if let Some(val_arr) = val.as_array() {
                                        if val_arr.len() == 2 {
                                            let ts = val_arr[0].as_i64().unwrap_or(0);
                                            let val_str = val_arr[1].as_str().unwrap_or("0");
                                            let metric_val = val_str.parse::<f64>().unwrap_or(0.0);
                                            
                                            let adjusted_val = if metric_type == "memory" || metric_type == "db_size" {
                                                metric_val / (1024.0 * 1024.0)
                                            } else if metric_type == "network_rx" || metric_type == "network_tx" || metric_type == "fs_read" || metric_type == "fs_write" {
                                                metric_val / 1024.0
                                            } else {
                                                metric_val
                                            };

                                            timestamps.push(ts);
                                            metric_vals.push(adjusted_val);
                                        }
                                    }
                                }
                                if !timestamps.is_empty() {
                                    got_real_metrics = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Fallback: Query Prometheus via Kubernetes API server proxy (for external development usage)
    if !got_real_metrics {
        if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
            let encoded_query = percent_encoding::utf8_percent_encode(&query, percent_encoding::NON_ALPHANUMERIC).to_string();
            let proxy_url = format!(
                "/api/v1/namespaces/monitoring/services/prometheus-k8s:9090/proxy/api/v1/query_range?query={}&start={}&end={}&step={}s",
                encoded_query,
                start_time,
                end_time,
                step_seconds
            );
            if let Ok(request) = axum::http::Request::get(&proxy_url).body(vec![]) {
                if let Ok(response_val) = k8s_client.request::<Value>(request).await {
                    if let Some(data) = response_val.get("data") {
                        if let Some(result) = data.get("result").and_then(|r| r.as_array()) {
                            if let Some(first_result) = result.first() {
                                if let Some(values) = first_result.get("values").and_then(|v| v.as_array()) {
                                    timestamps.clear();
                                    metric_vals.clear();
                                    for val in values {
                                        if let Some(val_arr) = val.as_array() {
                                            if val_arr.len() == 2 {
                                                let ts = val_arr[0].as_i64().unwrap_or(0);
                                                let val_str = val_arr[1].as_str().unwrap_or("0");
                                                let metric_val = val_str.parse::<f64>().unwrap_or(0.0);
                                                
                                                let adjusted_val = if metric_type == "memory" || metric_type == "db_size" {
                                                    metric_val / (1024.0 * 1024.0)
                                                } else if metric_type == "network_rx" || metric_type == "network_tx" || metric_type == "fs_read" || metric_type == "fs_write" {
                                                    metric_val / 1024.0
                                                } else {
                                                    metric_val
                                                };

                                                timestamps.push(ts);
                                                metric_vals.push(adjusted_val);
                                            }
                                        }
                                    }
                                    if !timestamps.is_empty() {
                                        got_real_metrics = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if got_real_metrics {
        return Ok((timestamps, metric_vals, false));
    }

    // Prometheus simulation fallback for local development or disconnected environments
    let mut timestamps = Vec::new();
    let mut values = Vec::new();
    let mut current_time = start_time;

    let base_value = match metric_type {
        "cpu" => 0.0005,
        "memory" => 18.0,
        "network_rx" => 0.1,
        "network_tx" => 0.05,
        "fs_read" => 0.0,
        "fs_write" => 0.02,
        "db_size" => 45.2 * 1024.0 * 1024.0, // 45.2 MB (base val in bytes, adjusted_val will divide by 1024^2)
        "db_connections" => 8.0,
        "db_cache_hit_rate" => 99.4,
        _ => 1.0,
    };

    let mut i = 0;
    while current_time <= end_time {
        timestamps.push(current_time);
        
        // Generate simulated sine wave with random jitter
        let angle = (i as f64) * 0.1;
        let jitter = (rand::random::<f64>() - 0.5) * (base_value * 0.15);
        let val = base_value + (angle.sin() * (base_value * 0.2)) + jitter;
        
        values.push(val.max(0.0));
        current_time += step_seconds;
        i += 1;
    }

    Ok((timestamps, values, true))
}
