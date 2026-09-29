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

pub async fn rate_limit_middleware(
    State(state): State<RateLimiterState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let ip = addr.ip();

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
        (status, Json(body)).into_response()
    }
}
