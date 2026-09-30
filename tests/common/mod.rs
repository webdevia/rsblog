//! Shared harness for HTTP-level integration tests.
//!
//! Each test gets a fully isolated app: a unique temp SQLite file, migrated
//! schema, and an `axum::Router` exercised in-process via `oneshot`
//! (no TCP needed). Entity names are unique per call so tests can run in
//! parallel inside one file.

// Each integration binary compiles this module but uses only a subset of it.
#![allow(dead_code)]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use blog_api::{
    auth::password::hash_password,
    config::{Config, DatabaseBackend},
    db::DbPool,
    logging::{wrap_router, AccessLogConfig},
    repositories::user_repo,
    routes::{create_router, AppState},
};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use tower::ServiceExt;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique suffix for entity names (`user_<n>`, ...).
pub fn uid() -> String {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{}_{}", std::process::id(), n)
}

fn test_config(db_path: &str) -> Config {
    Config {
        database_url: format!("sqlite://{db_path}?mode=rwc"),
        database_backend: DatabaseBackend::from_str("sqlite").unwrap(),
        jwt_secret: "integration-test-secret-that-is-long-enough-0123456789".into(),
        jwt_expiration_hours: 24,
        host: "127.0.0.1".into(),
        port: 0,
        admin_username: "admin".into(),
        admin_email: "admin@test.local".into(),
        admin_password: None,
        cors_origins: vec![],
        log_format: "text".into(),
        log_include_query: false,
        slow_request_ms: 0,
        // Anti-spam effectively off by default so existing tests are unaffected.
        rate_limit_rps: 1000,
        rate_limit_burst: 1000,
        auth_rate_limit_rps: 1000,
        auth_rate_limit_burst: 1000,
        comment_rate_per_min: 1000,
        post_rate_per_hour: 1000,
        trusted_account_days: 30,
        trusted_published_count: 5,
        duplicate_window_min: 0,
        max_links_new_user: 1000,
    }
}

pub struct TestApp {
    pub router: Router,
    pub pool: DbPool,
    pub config: Config,
    // Kept so the temp DB file lives until the end of the test.
    _db_path: String,
}

/// Build an isolated app instance backed by a fresh temp SQLite database.
/// Uses the same observability wrap as production (`x-request-id` + access log).
pub async fn new_app() -> TestApp {
    let db_path = fresh_db_path();
    new_app_with_config(test_config(&db_path), db_path).await
}

fn fresh_db_path() -> String {
    std::env::temp_dir()
        .join(format!("rsblog-itest-{}-{}.db", std::process::id(), uid()))
        .to_string_lossy()
        .into_owned()
}

/// Shared builder over an explicit config.
async fn new_app_with_config(config: Config, db_path: String) -> TestApp {
    let pool = DbPool::init(&config).await;
    let router = wrap_router(
        create_router(AppState {
            pool: pool.clone(),
            config: config.clone(),
            spam: blog_api::spam::SpamState::default(),
        }),
        AccessLogConfig {
            include_query: false,
            slow_request_ms: 0,
        },
    );
    TestApp {
        router,
        pool,
        config,
        _db_path: db_path,
    }
}

/// Strict anti-spam app for `api_antispam` tests: low throttles, duplicates
/// and link caps enabled, tight auth bucket.
pub async fn new_strict_app() -> TestApp {
    let db_path = fresh_db_path();
    let mut config = test_config(&db_path);
    config.comment_rate_per_min = 3;
    config.post_rate_per_hour = 3;
    config.duplicate_window_min = 60;
    config.max_links_new_user = 1;
    config.auth_rate_limit_rps = 1;
    config.auth_rate_limit_burst = 5;
    new_app_with_config(config, db_path).await
}

pub struct TestClient {
    app: TestApp,
}

impl TestClient {
    pub fn new(app: TestApp) -> Self {
        Self { app }
    }

    pub fn pool(&self) -> &DbPool {
        &self.app.pool
    }

    pub async fn request(
        &mut self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let (status, _, json) = self.request_with_headers(method, uri, token, body).await;
        (status, json)
    }

    /// Same as `request` but also returns the response headers (for
    /// asserting `retry-after`, `x-request-id`, ...).
    pub async fn request_with_headers(
        &mut self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let method: Method = method.parse().unwrap();
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let req = if let Some(json) = body {
            builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&json).unwrap()))
                .unwrap()
        } else {
            builder.body(Body::empty()).unwrap()
        };
        let resp = self
            .app
            .router
            .clone()
            .oneshot(req)
            .await
            .expect("router should handle request");
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, headers, json)
    }

    pub async fn get(&mut self, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
        self.request("GET", uri, token, None).await
    }

    pub async fn post(
        &mut self,
        uri: &str,
        token: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        self.request("POST", uri, token, Some(body)).await
    }

    pub async fn put(
        &mut self,
        uri: &str,
        token: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        self.request("PUT", uri, token, Some(body)).await
    }

    pub async fn delete(&mut self, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
        self.request("DELETE", uri, token, None).await
    }
}

/// Register a user; returns `(token, user_id, username)`.
pub async fn register_user(c: &mut TestClient, username: &str) -> (String, String, String) {
    let (status, body) = c
        .post(
            "/api/v1/auth/register",
            None,
            serde_json::json!({
                "username": username,
                "email": format!("{username}@test.local"),
                "password": "SecurePassword123",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "register failed: {body}");
    (
        body["token"].as_str().unwrap().to_owned(),
        body["user"]["id"].as_str().unwrap().to_owned(),
        username.to_owned(),
    )
}

/// Create an admin directly in the DB and log in; returns `(token, user_id)`.
pub async fn make_admin(c: &mut TestClient, username: &str) -> (String, String) {
    let id = uuid::Uuid::new_v4().to_string();
    let hash = hash_password("AdminPassword123").unwrap();
    user_repo::create_admin(
        c.pool(),
        &id,
        username,
        &format!("{username}@test.local"),
        &hash,
    )
    .await
    .unwrap();
    let (status, body) = c
        .post(
            "/api/v1/auth/login",
            None,
            serde_json::json!({"username": username, "password": "AdminPassword123"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "admin login failed: {body}");
    (
        body["token"].as_str().unwrap().to_owned(),
        body["user"]["id"].as_str().unwrap().to_owned(),
    )
}

/// Create a post (may be draft for regular users); returns `(post_id, slug)`.
pub async fn make_post(c: &mut TestClient, token: &str, title: &str) -> (String, String) {
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(token),
            serde_json::json!({
                "title": title,
                "content": format!("Content of {title}"),
                "published": true,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create post failed: {body}");
    (
        body["id"].as_str().unwrap().to_owned(),
        body["slug"].as_str().unwrap().to_owned(),
    )
}

/// Create a draft explicitly; returns `(post_id, slug)`.
pub async fn make_draft(c: &mut TestClient, token: &str, title: &str) -> (String, String) {
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(token),
            serde_json::json!({
                "title": title,
                "content": format!("Content of {title}"),
                "published": false,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create draft failed: {body}");
    assert_eq!(body["published"], false);
    (
        body["id"].as_str().unwrap().to_owned(),
        body["slug"].as_str().unwrap().to_owned(),
    )
}

/// Publish a post via moderator/admin token; asserts success.
pub async fn publish_post(c: &mut TestClient, mod_token: &str, post_id: &str) {
    let (status, body) = c
        .post(
            &format!("/api/v1/posts/{post_id}/publish"),
            Some(mod_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish failed: {body}");
    assert_eq!(body["published"], true);
}

/// Create a post as `author_token` then publish via `mod_token`.
/// Returns `(post_id, slug)` of a published post.
pub async fn make_published_post(
    c: &mut TestClient,
    author_token: &str,
    mod_token: &str,
    title: &str,
) -> (String, String) {
    let (post_id, slug) = make_post(c, author_token, title).await;
    // Regular authors produce drafts; privileged authors may already be published.
    let (status, body) = c
        .get(&format!("/api/v1/posts/{post_id}"), Some(mod_token))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "fetch for publish check failed: {body}"
    );
    if body["published"] == false {
        publish_post(c, mod_token, &post_id).await;
        let (_, fresh) = c.get(&format!("/api/v1/posts/{post_id}"), None).await;
        assert_eq!(fresh["slug"], slug);
    }
    (post_id, slug)
}

/// Promote `user_id` to moderator via `admin_token`; returns fresh role.
pub async fn make_moderator(c: &mut TestClient, admin_token: &str, user_id: &str) {
    let (status, body) = c
        .put(
            &format!("/api/v1/admin/users/{user_id}/role"),
            Some(admin_token),
            serde_json::json!({"role": "moderator"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "promote failed: {body}");
}
