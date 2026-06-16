//! Centralized observability bootstrap: structured logging (text or JSON),
//! optional OpenTelemetry OTLP tracing, and a Prometheus recorder for the app's
//! own RED metrics (request rate / errors / duration), exposed at `/metrics`.
//!
//! Env knobs:
//! - `HERMES_LOG_FORMAT=json`        → JSON logs (default: pretty text for dev)
//! - `OTEL_EXPORTER_OTLP_ENDPOINT`   → enable OTLP/HTTP trace export (e.g. http://otel-collector:4318)
//! - `OTEL_SERVICE_NAME`             → service name in traces (default: hermes-backend)
//! - `RUST_LOG`                      → standard EnvFilter directives

use std::sync::OnceLock;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

static PROM_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// The Prometheus exposition handle, if the recorder was installed. Used by the
/// `/metrics` route to render the current snapshot.
pub fn metrics_handle() -> Option<&'static PrometheusHandle> {
    PROM_HANDLE.get()
}

/// Initialize logging, tracing and metrics. Call once at startup.
pub fn init() {
    // Prometheus recorder for the backend's own metrics (http_requests_total, …).
    match PrometheusBuilder::new().install_recorder() {
        Ok(handle) => {
            let _ = PROM_HANDLE.set(handle);
            // Register HELP/TYPE descriptions for the platform's own metrics.
            crate::utils::metrics::describe();
        }
        Err(e) => eprintln!("WARN: could not install Prometheus recorder: {e}"),
    }

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    let json_logs = std::env::var("HERMES_LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    let fmt_layer = if json_logs {
        tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(false)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer().boxed()
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(build_otel_layer())
        .init();
}

/// Build an OpenTelemetry tracing layer when `OTEL_EXPORTER_OTLP_ENDPOINT` is set,
/// otherwise `None` (the registry treats `None` as a no-op layer).
fn build_otel_layer<S>() -> Option<Box<dyn Layer<S> + Send + Sync>>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a> + Send + Sync,
{
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig as _;

    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .filter(|s| !s.trim().is_empty())?;
    let service_name =
        std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "hermes-backend".to_string());

    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .build()
    {
        Ok(e) => e,
        Err(e) => {
            eprintln!("WARN: OTLP exporter init failed, tracing disabled: {e}");
            return None;
        }
    };

    let resource = opentelemetry_sdk::Resource::new(vec![opentelemetry::KeyValue::new(
        "service.name",
        service_name,
    )]);

    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(resource)
        .build();

    let tracer = provider.tracer("hermes-backend");
    opentelemetry::global::set_tracer_provider(provider);

    // W3C trace-context propagation so traces stitch across services
    // (see crate::utils::otel for the inbound/outbound helpers).
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    Some(tracing_opentelemetry::layer().with_tracer(tracer).boxed())
}
