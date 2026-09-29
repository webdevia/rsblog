//! Integration tests: in-app HTTP hardening (security headers + CORS).

mod common;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use common::{new_app, TestClient};
use tower::ServiceExt;

#[tokio::test]
async fn security_headers_present_on_responses() {
    let mut c = TestClient::new(new_app().await);
    for uri in ["/health", "/api/v1/health", "/api/v1/posts"] {
        let (status, _) = c.get(uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
    }
    // Inspect raw headers (the JSON helper discards them).
    let app = new_app().await;
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let h = resp.headers();
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["x-frame-options"], "DENY");
    assert!(h.contains_key("content-security-policy"));
    assert!(h.contains_key("permissions-policy"));
    assert!(!h.contains_key("strict-transport-security"));
}

#[tokio::test]
async fn cors_preflight_allows_configured_method_and_headers() {
    let app = new_app().await;
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/v1/posts")
                .header(header::ORIGIN, "https://app.example")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // Default (CORS_ORIGINS unset): permissive, reflects the origin.
    assert!(resp.headers().contains_key("access-control-allow-origin"));
}
