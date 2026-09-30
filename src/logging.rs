//! Request observability: structured access logs + request IDs.
//!
//! One `INFO` line per request (except `/health`, which load balancers and
//! docker healthchecks poll every few seconds):
//!
//! ```text
//! request method=GET path=/api/v1/posts status=200 latency_ms=3
//!         client_ip=127.0.0.1 request_id=<uuid>
//! ```
//!
//! Design notes:
//! - The middleware lives **outside** the rate limiter / body limit / timeout
//!   layers (see `wrap_router` call order in `main.rs`), so `429`/`413`/`408`
//!   rejections are logged too.
//! - `x-request-id` is honored when the client sends a sane value, otherwise a
//!   UUID v4 is generated. It is echoed on every response and included in the
//!   access line, so `errors.rs` failures can be correlated without touching
//!   `AppError`.
//! - Bodies and headers (notably `Authorization`) are never logged. Query
//!   strings are off by default (`LOG_INCLUDE_QUERY=true` opts in) because
//!   `?search=` / `?author=` values are PII with high cardinality. Note the
//!   logged `path` may still contain resource IDs/slugs.

use axum::{
    extract::{ConnectInfo, MatchedPath, Request, State},
    http::HeaderValue,
    middleware::Next,
    response::Response,
    Router,
};
use std::{net::SocketAddr, time::Instant};
use tower_http::request_id::RequestId;

/// Header used for request correlation (incoming honored, always echoed).
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Runtime knobs for the access log (see `Config`; also constructible directly).
#[derive(Debug, Clone)]
pub struct AccessLogConfig {
    /// Append `?query` to the logged path. Default `false`.
    pub include_query: bool,
    /// Emit a `warn!` when a request exceeds this many ms. `0` disables.
    pub slow_request_ms: u64,
}

impl Default for AccessLogConfig {
    fn default() -> Self {
        Self {
            include_query: false,
            slow_request_ms: 1000,
        }
    }
}

/// Initialize the global tracing subscriber.
///
/// `LOG_FORMAT=json` selects JSON output (production default via compose),
/// anything else keeps the human-readable text format (dev default).
/// `RUST_LOG` filtering is unchanged (`blog_api=debug,tower_http=debug` fallback).
pub fn init_subscriber() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "blog_api=debug,tower_http=debug".into());
    let json = std::env::var("LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);
    if json {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }
}

/// Wrap a router with request-ID handling and the access log.
///
/// The middleware assigns the ID (honoring a sane client-sent value),
/// echoes it on every response, and emits the access line. The wrap is
/// applied outermost in `main.rs`, so every response — including
/// outer-layer rejections — carries the header.
pub fn wrap_router(router: Router, config: AccessLogConfig) -> Router {
    router.layer(axum::middleware::from_fn_with_state(
        config,
        access_log_middleware,
    ))
}

fn is_health_path(path: &str) -> bool {
    path == "/health" || path == "/api/v1/health"
}

/// Resolve the ID for this request: honor a sane client-sent value,
/// otherwise generate a UUID v4. Inserts it into request extensions so
/// `PropagateRequestIdLayer` echoes it on the response.
fn resolve_request_id(req: &mut Request) -> String {
    let incoming: Option<String> = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 200 && s.chars().all(|c| c.is_ascii_graphic()))
        .map(str::to_owned);
    if let Some(s) = incoming {
        if let Ok(hv) = HeaderValue::from_str(&s) {
            req.extensions_mut().insert(RequestId::new(hv));
            return s;
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    if let Ok(hv) = HeaderValue::from_str(&id) {
        req.extensions_mut().insert(RequestId::new(hv));
    }
    id
}

async fn access_log_middleware(
    State(config): State<AccessLogConfig>,
    mut req: Request,
    next: Next,
) -> Response {
    let request_id = resolve_request_id(&mut req);
    let method = req.method().clone();
    // Prefer the matched route template (`/posts/{id}`) when routing already
    // ran; fall back to the raw path (outermost placement usually means the
    // latter). Query is opt-in only.
    let mut path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| req.uri().path().to_owned());
    if config.include_query {
        if let Some(q) = req.uri().query() {
            if !q.is_empty() {
                path.push('?');
                path.push_str(q);
            }
        }
    }
    // `ConnectInfo` is present under a real server; absent in unit/oneshot
    // tests, where we fall back to the proxy header or "unknown".
    let client_ip = match req.extensions().get::<ConnectInfo<SocketAddr>>() {
        Some(c) => crate::rate_limiter::client_ip(&req, &c.0).to_string(),
        None => req
            .headers()
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<std::net::IpAddr>().ok())
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
    };

    let start = Instant::now();
    let mut res = next.run(req).await;
    let latency_ms = start.elapsed().as_millis() as u64;
    let status = res.status().as_u16();
    if let Ok(hv) = HeaderValue::from_str(&request_id) {
        res.headers_mut().insert(REQUEST_ID_HEADER, hv);
    }

    if is_health_path(&path) {
        return res;
    }

    if status >= 500 {
        tracing::error!(
            method = %method,
            path = %path,
            status,
            latency_ms,
            client_ip = %client_ip,
            request_id = %request_id,
            "server error"
        );
    } else {
        tracing::info!(
            method = %method,
            path = %path,
            status,
            latency_ms,
            client_ip = %client_ip,
            request_id = %request_id,
            "request"
        );
    }
    if config.slow_request_ms > 0 && latency_ms > config.slow_request_ms {
        tracing::warn!(
            method = %method,
            path = %path,
            status,
            latency_ms,
            client_ip = %client_ip,
            request_id = %request_id,
            slow_threshold_ms = config.slow_request_ms,
            "slow request"
        );
    }

    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_paths_are_excluded() {
        assert!(is_health_path("/health"));
        assert!(is_health_path("/api/v1/health"));
        assert!(!is_health_path("/api/v1/posts"));
        assert!(!is_health_path("/healthz"));
    }
}
