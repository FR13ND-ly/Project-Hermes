use axum::{
    body::Body,
    http::{Request, header},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use uuid::Uuid;
use tracing::{info, warn, error, Instrument};

tokio::task_local! {
    /// The current request's correlation id, scoped for the whole handler chain.
    /// `AppError::into_response` reads this so the error body's `request_id`
    /// matches the access log and the `x-request-id` response header.
    pub static REQUEST_ID: String;
}

/// Best-effort current request id (falls back to a fresh uuid outside a request).
pub fn current_request_id() -> String {
    REQUEST_ID
        .try_with(|id| id.clone())
        .unwrap_or_else(|_| Uuid::new_v4().to_string())
}

pub async fn telemetry_logger(
    req: Request<Body>,
    next: Next,
) -> Response {
    let start_time = Instant::now();

    let request_id = req.headers()
        .get("x-request-id")
        .and_then(|val| val.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let method = req.method().clone();
    let uri = req.uri().clone();
    // Log only the PATH, never the query string — SSE/EventSource pass auth tokens
    // as query params and must not leak into access logs.
    let path = uri.path().to_string();

    // Health/metrics probes are high-frequency and uninteresting; skip their access
    // logs and metrics to keep signal clean. They still flow through normally.
    let is_probe = path == "/health" || path == "/metrics";

    let client_ip = req.headers()
        .get("x-forwarded-for")
        .and_then(|val| val.to_str().ok())
        .and_then(|s| s.split(',').next())
        .or_else(|| {
            req.headers()
                .get("x-real-ip")
                .and_then(|val| val.to_str().ok())
        })
        .unwrap_or("unknown")
        .to_string();

    // Continue any upstream W3C trace (traceparent header) so cross-service
    // traces stitch instead of starting a fresh root here.
    let parent_cx = crate::utils::otel::extract_context(req.headers());

    // One span per request so every log emitted deeper in the stack carries
    // request_id automatically (correlation across layers). `trace_id` is filled
    // in below once the span is parented, so log lines link to the trace.
    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %method,
        path = %path,
        trace_id = tracing::field::Empty,
    );

    // Parent the span to the extracted context and surface its trace id on the
    // span (log↔trace correlation). Inert when OTLP tracing is disabled.
    {
        use opentelemetry::trace::TraceContextExt;
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        span.set_parent(parent_cx);
        let trace_id = span.context().span().span_context().trace_id();
        if trace_id != opentelemetry::trace::TraceId::INVALID {
            span.record("trace_id", tracing::field::display(trace_id));
        }
    }

    if !is_probe {
        let _enter = span.enter();
        info!(client_ip = %client_ip, "Request started");
    }

    let rid_for_scope = request_id.clone();
    let mut response = REQUEST_ID
        .scope(rid_for_scope, async move { next.run(req).await })
        .instrument(span.clone())
        .await;

    if let Ok(val) = header::HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", val);
    }

    let duration = start_time.elapsed();
    let duration_ms = duration.as_secs_f64() * 1000.0;
    let status = response.status();

    if !is_probe {
        // RED metrics for the app itself.
        metrics::counter!(
            "http_requests_total",
            "method" => method.to_string(),
            "status" => status.as_u16().to_string(),
        )
        .increment(1);
        metrics::histogram!(
            "http_request_duration_seconds",
            "method" => method.to_string(),
        )
        .record(duration.as_secs_f64());

        let _enter = span.enter();
        if status.is_server_error() {
            error!(client_ip = %client_ip, status = status.as_u16(), duration_ms, "Request failed (Server Error)");
        } else if status.is_client_error() {
            warn!(client_ip = %client_ip, status = status.as_u16(), duration_ms, "Request warning (Client Error)");
        } else {
            info!(status = status.as_u16(), duration_ms, "Request finished");
        }
    }

    response
}
