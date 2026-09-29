//! In-app HTTP hardening: security response headers and CORS policy.
//!
//! These ship inside `create_router` (not the deployment), so every response
//! — including the ones exercised by the integration tests — carries them.
//! TLS termination itself intentionally stays out of the app: doing ACME
//! issuance/renewal inside the binary is operationally worse than a tiny
//! edge proxy, and HSTS below is therefore *not* emitted (it is ignored
//! over plain HTTP anyway).

use axum::{
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use tower_http::cors::{Any, CorsLayer};

/// Applied to every response. JSON API: no content sniffing, no framing,
/// strict referrer, locked-down powerful features, no resource sharing.
pub fn apply_security_headers(headers: &mut HeaderMap) {
    // (name, value) pairs; names are parsed at startup-time cost once per
    // response, which is negligible next to handler + DB work.
    const HEADERS: &[(&str, &str)] = &[
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "strict-origin-when-cross-origin"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=(), payment=()",
        ),
        (
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
        ),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
    ];
    for (name, value) in HEADERS {
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            headers.insert(n, v);
        }
    }
}

pub async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    apply_security_headers(res.headers_mut());
    res
}

/// Parse `CORS_ORIGINS` (`https://a.example, https://b.example`).
/// Empty/unset means "reflect any origin" (previous behaviour).
pub fn parse_cors_origins(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Build the CORS layer. Panics on invalid origins (fail fast at startup,
/// same policy as the rest of `Config::from_env`).
pub fn cors_layer(allowed_origins: &[String]) -> CorsLayer {
    // Keep these in sync with the previous permissive setup; only the
    // origin is tightened when CORS_ORIGINS is set.
    let methods = [
        axum::http::Method::GET,
        axum::http::Method::POST,
        axum::http::Method::PUT,
        axum::http::Method::DELETE,
        axum::http::Method::OPTIONS,
    ];
    let headers = [
        axum::http::header::CONTENT_TYPE,
        axum::http::header::AUTHORIZATION,
    ];
    let base = CorsLayer::new()
        .allow_methods(methods)
        .allow_headers(headers)
        .max_age(std::time::Duration::from_secs(3600));
    if allowed_origins.is_empty() {
        base.allow_origin(Any)
    } else {
        let origins: Vec<HeaderValue> = allowed_origins
            .iter()
            .map(|o| {
                o.parse::<HeaderValue>()
                    .unwrap_or_else(|_| panic!("CORS_ORIGINS contains invalid origin: '{o}'"))
            })
            .collect();
        base.allow_origin(origins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_headers_cover_owasp_basics() {
        let mut map = HeaderMap::new();
        apply_security_headers(&mut map);
        assert_eq!(map["x-content-type-options"], "nosniff");
        assert_eq!(map["x-frame-options"], "DENY");
        assert_eq!(map["referrer-policy"], "strict-origin-when-cross-origin");
        assert!(map.contains_key("content-security-policy"));
        assert!(map.contains_key("permissions-policy"));
        assert!(map.contains_key("cross-origin-opener-policy"));
        assert!(map.contains_key("cross-origin-resource-policy"));
        // Deliberately absent: HSTS is meaningless without TLS termination.
        assert!(!map.contains_key("strict-transport-security"));
    }

    #[test]
    fn cors_origins_parsing() {
        assert!(parse_cors_origins(None).is_empty());
        assert!(parse_cors_origins(Some("")).is_empty());
        assert!(parse_cors_origins(Some("   ")).is_empty());
        assert_eq!(
            parse_cors_origins(Some("https://a.example,https://b.example")),
            ["https://a.example", "https://b.example"]
        );
        assert_eq!(
            parse_cors_origins(Some(" https://a.example , , https://b.example ")),
            ["https://a.example", "https://b.example"]
        );
    }

    #[test]
    #[should_panic(expected = "CORS_ORIGINS contains invalid origin")]
    fn cors_layer_rejects_garbage_origin() {
        let _ = cors_layer(&["not a valid \n origin".to_owned()]);
    }
}
