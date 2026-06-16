//! W3C trace-context propagation helpers so distributed traces stitch across
//! service boundaries.
//!
//! `axum::http::HeaderMap` and `reqwest::header::HeaderMap` are both the same
//! `http::HeaderMap` type, so these helpers work for inbound (axum) extraction
//! and outbound (reqwest) injection alike. The global propagator is installed
//! in [`crate::utils::observability`] when OTLP tracing is enabled; with no
//! propagator these are inert (extraction yields an empty context).

use axum::http::HeaderMap;
use opentelemetry::propagation::{Extractor, Injector};

struct HeaderMapExtractor<'a>(&'a HeaderMap);

impl<'a> Extractor for HeaderMapExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

struct HeaderMapInjector<'a>(&'a mut HeaderMap);

impl<'a> Injector for HeaderMapInjector<'a> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(val)) = (
            axum::http::HeaderName::from_bytes(key.as_bytes()),
            axum::http::HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, val);
        }
    }
}

/// Extract the upstream W3C trace context from inbound request headers, so a
/// caller's trace continues into this request instead of starting a new root.
pub fn extract_context(headers: &HeaderMap) -> opentelemetry::Context {
    opentelemetry::global::get_text_map_propagator(|prop| prop.extract(&HeaderMapExtractor(headers)))
}

/// Inject the current span's trace context into outbound request headers so a
/// downstream service can continue the trace. No-op when tracing is disabled.
pub fn inject_current(headers: &mut HeaderMap) {
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|prop| {
        prop.inject_context(&cx, &mut HeaderMapInjector(headers))
    });
}
