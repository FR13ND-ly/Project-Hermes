use axum::{
    body::Body,
    http::{Request, header},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use uuid::Uuid;
use tracing::{info, warn, error};

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

    let user_agent = req.headers()
        .get(header::USER_AGENT)
        .and_then(|val| val.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    info!(
        request_id = %request_id,
        client_ip = %client_ip,
        method = %method,
        uri = %uri,
        user_agent = %user_agent,
        "Request started"
    );

    let mut response = next.run(req).await;

    if let Ok(val) = header::HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", val);
    }

    let duration = start_time.elapsed();
    let duration_ms = duration.as_secs_f64() * 1000.0;
    let status = response.status();

    if status.is_server_error() {
        error!(
            request_id = %request_id,
            client_ip = %client_ip,
            method = %method,
            uri = %uri,
            status = status.as_u16(),
            duration_ms = duration_ms,
            "Request failed (Server Error)"
        );
    } else if status.is_client_error() {
        warn!(
            request_id = %request_id,
            client_ip = %client_ip,
            method = %method,
            uri = %uri,
            status = status.as_u16(),
            duration_ms = duration_ms,
            "Request warning (Client Error)"
        );
    } else {
        info!(
            request_id = %request_id,
            client_ip = %client_ip,
            method = %method,
            uri = %uri,
            status = status.as_u16(),
            duration_ms = duration_ms,
            "Request finished"
        );
    }

    response
}
