use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use serde_json::json;
use std::{net::IpAddr, num::NonZeroU32, sync::Arc};

// Use DefaultKeyedRateLimiter which correctly maps keys (IpAddr) to rate limit states using DashMap
type IpLimiter = DefaultKeyedRateLimiter<IpAddr>;

#[derive(Clone)]
pub struct RateLimiterState {
    limiter: Arc<IpLimiter>,
}

impl RateLimiterState {
    pub fn new(requests_per_second: u32, burst: u32) -> Self {
        let quota = Quota::per_second(NonZeroU32::new(requests_per_second).unwrap())
            .allow_burst(NonZeroU32::new(burst).unwrap());
        Self {
            limiter: Arc::new(RateLimiter::keyed(quota)),
        }
    }
}

/// Resolve the client IP for logging and rate limiting.
///
/// When running behind the bundled Caddy proxy, Caddy *overwrites*
/// `X-Real-IP` with the peer address it actually sees, so it cannot be
/// spoofed through the proxy — provided the api port is not directly
/// reachable (the compose file binds it to localhost for this reason).
/// Direct connections fall back to the socket address.
pub(crate) fn client_ip(request: &Request, addr: &std::net::SocketAddr) -> IpAddr {
    request
        .headers()
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| addr.ip())
}

pub async fn rate_limit_middleware(
    State(state): State<RateLimiterState>,
    request: Request,
    next: Next,
) -> Response {
    // `ConnectInfo` is present under a real server; absent in unit/oneshot
    // tests, where we fall back to the proxy header or an unspecified address
    // (fail open — buckets are per-IP, and tests use generous limits).
    let ip = match request
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
    {
        Some(connect) => client_ip(&request, &connect.0),
        None => request
            .headers()
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)),
    };

    if state.limiter.check_key(&ip).is_ok() {
        next.run(request).await
    } else {
        let status = StatusCode::TOO_MANY_REQUESTS;
        let body = json!({
            "error": {
                "status": status.as_u16(),
                "message": "Too many requests. Please try again later."
            }
        });
        let mut res = (status, Json(body)).into_response();
        if let Ok(v) = "60".parse() {
            res.headers_mut().insert("retry-after", v);
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn burst_capacity_is_enforced_per_ip() {
        let state = RateLimiterState::new(1, 2);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        assert!(state.limiter.check_key(&ip).is_ok());
        assert!(state.limiter.check_key(&ip).is_ok());
        // Burst exhausted: the immediate third request is rejected.
        assert!(state.limiter.check_key(&ip).is_err());
    }

    #[test]
    fn client_ip_prefers_proxy_header_with_socket_fallback() {
        use axum::body::Body;

        let addr: std::net::SocketAddr = "10.9.9.9:1234".parse().unwrap();
        let expected: IpAddr = "203.0.113.7".parse().unwrap();
        let with_header = Request::builder()
            .header("x-real-ip", "203.0.113.7")
            .body(Body::empty())
            .unwrap();
        assert_eq!(client_ip(&with_header, &addr), expected);

        // Missing or garbage header falls back to the peer address.
        let bare = Request::builder().body(Body::empty()).unwrap();
        assert_eq!(client_ip(&bare, &addr), addr.ip());
        let garbage = Request::builder()
            .header("x-real-ip", "not-an-ip")
            .body(Body::empty())
            .unwrap();
        assert_eq!(client_ip(&garbage, &addr), addr.ip());
    }

    #[test]
    fn limits_are_tracked_per_ip_independently() {
        let state = RateLimiterState::new(1, 1);
        let a = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let b = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        assert!(state.limiter.check_key(&a).is_ok());
        assert!(state.limiter.check_key(&a).is_err());
        // A different IP still has its own quota.
        assert!(state.limiter.check_key(&b).is_ok());
    }
}
