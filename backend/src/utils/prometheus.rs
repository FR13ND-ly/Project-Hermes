use std::time::Duration;
use chrono::Utc;
use reqwest::Client;
use serde_json::Value;
use crate::utils::error::AppError;

/// Run an instant PromQL query and return the first scalar result, or `None` if
/// Prometheus is unreachable or the query has no data. Mirrors the dual-path
/// access of [`get_historical_metrics`] (direct cluster URL, then k8s API proxy).
pub async fn query_instant(promql: &str) -> Option<f64> {
    fn parse_scalar(json: &Value) -> Option<f64> {
        let result = json.get("data")?.get("result")?.as_array()?;
        let first = result.first()?;
        let value = first.get("value")?.as_array()?;
        value.get(1)?.as_str()?.parse::<f64>().ok()
    }

    // 1. Direct query to Prometheus (in-cluster).
    let prometheus_url = std::env::var("HERMES_PROMETHEUS_URL")
        .unwrap_or_else(|_| "http://prometheus-k8s.monitoring.svc:9090".to_string());
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    if let Ok(res) = client
        .get(format!("{}/api/v1/query", prometheus_url))
        .query(&[("query", promql)])
        .send()
        .await
    {
        if res.status().is_success() {
            if let Ok(json) = res.json::<Value>().await {
                if let Some(v) = parse_scalar(&json) {
                    return Some(v);
                }
            }
        }
    }

    // 2. Fallback via the Kubernetes API server proxy (external/dev usage).
    if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
        let encoded = percent_encoding::utf8_percent_encode(promql, percent_encoding::NON_ALPHANUMERIC).to_string();
        let proxy_url = format!(
            "/api/v1/namespaces/monitoring/services/prometheus-k8s:9090/proxy/api/v1/query?query={}",
            encoded
        );
        if let Ok(request) = axum::http::Request::get(&proxy_url).body(vec![]) {
            if let Ok(json) = k8s_client.request::<Value>(request).await {
                if let Some(v) = parse_scalar(&json) {
                    return Some(v);
                }
            }
        }
    }

    None
}

/// Returns `(timestamps, values, simulated)`. `simulated` is true when the data
/// is synthetic because Prometheus could not be reached.
pub async fn get_historical_metrics(
    namespace: &str,
    container_name: &str,
    metric_type: &str,
    range_str: &str,
    engine: &str,
) -> Result<(Vec<i64>, Vec<f64>, bool), AppError> {
    let now = Utc::now();
    let end_time = now.timestamp();
    
    // The rate window MUST scale with the step, or wide ranges sample a tiny slice of
    // each interval and skip the rest (e.g. a 5m rate every 2h on 7d shows ~4% of the
    // data — sparse, noisy, and misleading). Sizing the window ≈ the step makes every
    // point an honest average over its interval, with no gaps.
    let (start_time, step_seconds, rate_window) = match range_str {
        "24h" => (end_time - 86400, 900, "15m"),  // 96 data points
        "7d" => (end_time - 604800, 7200, "2h"),  // 84 data points
        _ => (end_time - 3600, 60, "5m"),         // "1h" -> 60 data points (default)
    };

    // Serverless (Knative) pods run the function in a `user-container` next to a
    // `queue-proxy` sidecar. For per-container metrics (cpu/mem/fs) scope to the
    // function container so the sidecar's usage doesn't inflate the chart. Network
    // counters are pod-level (no per-container series), so they stay unscoped.
    let container_sel = if engine == "serverless" {
        ",container=\"user-container\""
    } else {
        ""
    };

    let query = match metric_type {
        "cpu" => format!(
            "sum(rate(container_cpu_usage_seconds_total{{namespace=\"{}\",pod=~\"{}-.*\"{}}}[5m]))",
            namespace, container_name, container_sel
        ),
        "memory" => format!(
            "sum(container_memory_working_set_bytes{{namespace=\"{}\",pod=~\"{}-.*\"{}}})",
            namespace, container_name, container_sel
        ),
        // Network counters live on the pod's `pod` label (the per-interface
        // metric isn't broken out per container), so we must scope by pod —
        // otherwise this returns the whole namespace's traffic, not this app's.
        "network_rx" => format!(
            "sum(rate(container_network_receive_bytes_total{{namespace=\"{}\",pod=~\"{}-.*\"}}[5m]))",
            namespace, container_name
        ),
        "network_tx" => format!(
            "sum(rate(container_network_transmit_bytes_total{{namespace=\"{}\",pod=~\"{}-.*\"}}[5m]))",
            namespace, container_name
        ),
        "fs_read" => format!(
            "sum(rate(container_fs_reads_bytes_total{{namespace=\"{}\",pod=~\"{}-.*\"{}}}[5m]))",
            namespace, container_name, container_sel
        ),
        "fs_write" => format!(
            "sum(rate(container_fs_writes_bytes_total{{namespace=\"{}\",pod=~\"{}-.*\"{}}}[5m]))",
            namespace, container_name, container_sel
        ),
        "db_size" => format!(
            "sum(container_fs_usage_bytes{{namespace=\"{}\",pod=~\"{}-.*\"{}}})",
            namespace, container_name, container_sel
        ),
        // Engine-specific: each DB type ships a different exporter (see deploy_database).
        "db_connections" => match engine {
            "redis" => format!(
                "sum(redis_connected_clients{{namespace=\"{}\",pod=~\"{}-0\"}})",
                namespace, container_name
            ),
            "mongodb" => format!(
                "sum(mongodb_ss_connections{{conn_type=\"current\",namespace=\"{}\",pod=~\"{}-0\"}})",
                namespace, container_name
            ),
            "mysql" => format!(
                "sum(mysql_global_status_threads_connected{{namespace=\"{}\",pod=~\"{}-0\"}})",
                namespace, container_name
            ),
            _ => format!(
                "sum(pg_stat_database_numbackends{{namespace=\"{}\",pod=~\"{}-0\"}})",
                namespace, container_name
            ),
        },
        "db_cache_hit_rate" => match engine {
            "redis" => format!(
                "sum(rate(redis_keyspace_hits_total{{namespace=\"{}\",pod=~\"{}-0\"}}[5m])) / (sum(rate(redis_keyspace_hits_total{{namespace=\"{}\",pod=~\"{}-0\"}}[5m])) + sum(rate(redis_keyspace_misses_total{{namespace=\"{}\",pod=~\"{}-0\"}}[5m])) + 1) * 100",
                namespace, container_name, namespace, container_name, namespace, container_name
            ),
            _ => format!(
                "sum(rate(pg_stat_database_blks_hit{{namespace=\"{}\",pod=~\"{}-0\"}}[5m])) / (sum(rate(pg_stat_database_blks_hit{{namespace=\"{}\",pod=~\"{}-0\"}}[5m])) + sum(rate(pg_stat_database_blks_read{{namespace=\"{}\",pod=~\"{}-0\"}}[5m])) + 1) * 100",
                namespace, container_name, namespace, container_name, namespace, container_name
            ),
        },
        _ => return Err(AppError::Validation(format!("Unsupported metric type: {}", metric_type))),
    };

    // Apply the range-sized rate window uniformly. Gauge queries (memory, db_size,
    // connections) contain no `[5m]`, so they're unaffected; every rate() query scales.
    let query = query.replace("[5m]", &format!("[{}]", rate_window));

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

    // Prometheus was unreachable. Do NOT fabricate data — return an empty series
    // flagged `simulated = true` so the UI can render an explicit "metrics
    // unavailable" state instead of a plausible-looking but fictional chart.
    let _ = (start_time, end_time, step_seconds);
    Ok((Vec::new(), Vec::new(), true))
}
