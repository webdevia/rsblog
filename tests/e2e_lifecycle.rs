//! End-to-end test: boots the real HTTP server (same middleware stack as
//! production: CORS, body limit, tracing) on an ephemeral port and drives the
//! full system lifecycle with an HTTP client, mirroring `test_api.sh`:
//! admin seed -> users -> posts/search -> RBAC isolation -> nested comments
//! tree -> admin moderation.

use blog_api::{
    auth::password::hash_password,
    config::{Config, DatabaseBackend},
    db::DbPool,
    repositories::user_repo,
    routes::{create_router, AppState},
};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tower_http::{cors::CorsLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};

struct E2E {
    client: Client,
    base: String,
    run: String,
}

impl E2E {
    async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
        expect: StatusCode,
    ) -> Value {
        let mut req = self
            .client
            .request(method.parse().unwrap(), format!("{}{path}", self.base));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.expect("request should reach server");
        let status = resp.status();
        let json: Value = resp.json().await.unwrap_or(Value::Null);
        assert_eq!(status, expect, "{method} {path} -> {status}: {json}");
        json
    }

    async fn phase_admin_setup(&self) -> (String, String, String) {
        // Admin was seeded directly in the DB (mirrors production seeding).
        let admin = self
            .call(
                "POST",
                "/auth/login",
                None,
                Some(json!({"username": "admin", "password": "Admin@123456"})),
                StatusCode::OK,
            )
            .await;
        let admin_token = admin["token"].as_str().unwrap().to_owned();

        let rust = self
            .call(
                "POST",
                "/tags",
                Some(&admin_token),
                Some(json!({"name": format!("Rust-{}", self.run)})),
                StatusCode::OK,
            )
            .await;
        let _async_tag = self
            .call(
                "POST",
                "/tags",
                Some(&admin_token),
                Some(json!({"name": format!("Async-{}", self.run)})),
                StatusCode::OK,
            )
            .await;
        (
            admin_token,
            rust["id"].as_str().unwrap().to_owned(),
            rust["slug"].as_str().unwrap().to_owned(),
        )
    }

    async fn phase_user_lifecycle(&self) -> (String, String) {
        let username = format!("user_{}", self.run);
        let reg = self
            .call(
                "POST",
                "/auth/register",
                None,
                Some(json!({
                    "username": username,
                    "email": format!("{username}@tokio.rs"),
                    "password": "SecurePassword123",
                })),
                StatusCode::OK,
            )
            .await;
        let token = reg["token"].as_str().unwrap().to_owned();
        let me = self
            .call("GET", "/users/me", Some(&token), None, StatusCode::OK)
            .await;
        assert_eq!(me["username"], username);
        (token, username)
    }

    async fn phase_posts(
        &self,
        token: &str,
        admin_token: &str,
        tag_id: &str,
        tag_slug: &str,
    ) -> (String, String) {
        let title = format!("Asynchronous Programming in Rust {}", self.run);
        let post = self
            .call(
                "POST",
                "/posts",
                Some(token),
                Some(json!({
                    "title": title,
                    "content": "A high-performance deep dive into Tokio system runtime and safe futures.",
                    "excerpt": "High concurrency paradigms in Rust.",
                    "published": true,
                    "tag_ids": [tag_id],
                })),
                StatusCode::OK,
            )
            .await;
        let post_id = post["id"].as_str().unwrap().to_owned();
        let slug = post["slug"].as_str().unwrap().to_owned();
        // Regular users create drafts; publish via admin before public checks.
        if !post["published"].as_bool().unwrap_or(false) {
            self.call(
                "POST",
                &format!("/posts/{post_id}/publish"),
                Some(admin_token),
                None,
                StatusCode::OK,
            )
            .await;
        }

        self.call("GET", "/posts", None, None, StatusCode::OK).await;
        let search = self
            .call("GET", "/posts?search=Tokio", None, None, StatusCode::OK)
            .await;
        assert!(search["total"].as_i64().unwrap() >= 1);
        let by_tag = self
            .call(
                "GET",
                &format!("/posts?tag={tag_slug}"),
                None,
                None,
                StatusCode::OK,
            )
            .await;
        assert!(by_tag["total"].as_i64().unwrap() >= 1);
        let single = self
            .call("GET", &format!("/posts/{slug}"), None, None, StatusCode::OK)
            .await;
        assert_eq!(single["id"], post_id);
        (post_id, slug)
    }

    async fn phase_security(&self, post_id: &str) -> String {
        let attacker_name = format!("attacker_{}", self.run);
        let reg = self
            .call(
                "POST",
                "/auth/register",
                None,
                Some(json!({
                    "username": attacker_name,
                    "email": format!("{attacker_name}@unsafe.rs"),
                    "password": "HackerInjected123",
                })),
                StatusCode::OK,
            )
            .await;
        let attacker = reg["token"].as_str().unwrap().to_owned();
        self.call(
            "PUT",
            &format!("/posts/{post_id}"),
            Some(&attacker),
            Some(json!({"title": "Hacked Title"})),
            StatusCode::FORBIDDEN,
        )
        .await;
        self.call(
            "DELETE",
            &format!("/posts/{post_id}"),
            Some(&attacker),
            None,
            StatusCode::FORBIDDEN,
        )
        .await;
        attacker
    }

    async fn phase_comments(&self, post_id: &str, author_token: &str, other_token: &str) {
        let root = self
            .call(
                "POST",
                &format!("/posts/{post_id}/comments"),
                Some(other_token),
                Some(json!({"content": "Outstanding technical article!"})),
                StatusCode::OK,
            )
            .await;
        let root_id = root["id"].as_str().unwrap().to_owned();

        let level1 = self
            .call(
                "POST",
                &format!("/posts/{post_id}/comments"),
                Some(author_token),
                Some(json!({"content": "Thank you!", "parent_id": root_id})),
                StatusCode::OK,
            )
            .await;
        let level1_id = level1["id"].as_str().unwrap().to_owned();

        self.call(
            "POST",
            &format!("/posts/{post_id}/comments"),
            Some(other_token),
            Some(json!({"content": "Indeed!", "parent_id": level1_id})),
            StatusCode::OK,
        )
        .await;

        let tree = self
            .call(
                "GET",
                &format!("/posts/{post_id}/comments"),
                None,
                None,
                StatusCode::OK,
            )
            .await;
        assert_eq!(tree.as_array().unwrap().len(), 1);
        assert_eq!(tree[0]["children"].as_array().unwrap().len(), 1);
        assert_eq!(
            tree[0]["children"][0]["children"].as_array().unwrap().len(),
            1
        );

        self.call(
            "DELETE",
            &format!("/posts/{post_id}/comments/{root_id}"),
            Some(other_token),
            None,
            StatusCode::OK,
        )
        .await;
        let tree = self
            .call(
                "GET",
                &format!("/posts/{post_id}/comments"),
                None,
                None,
                StatusCode::OK,
            )
            .await;
        assert_eq!(tree[0]["content"], "[deleted]");
        assert_eq!(tree[0]["is_deleted"], true);
        assert_eq!(tree[0]["children"].as_array().unwrap().len(), 1);
    }

    async fn phase_admin(&self, admin_token: &str) {
        let intern = format!("intern_{}", self.run);
        let reg = self
            .call(
                "POST",
                "/auth/register",
                None,
                Some(json!({
                    "username": intern,
                    "email": format!("{intern}@corp.com"),
                    "password": "Temp12345",
                })),
                StatusCode::OK,
            )
            .await;
        let intern_id = reg["user"]["id"].as_str().unwrap().to_owned();

        self.call(
            "GET",
            "/admin/users",
            Some(admin_token),
            None,
            StatusCode::OK,
        )
        .await;
        self.call(
            "PUT",
            &format!("/admin/users/{intern_id}/role"),
            Some(admin_token),
            Some(json!({"role": "moderator"})),
            StatusCode::OK,
        )
        .await;

        let login = self
            .call(
                "POST",
                "/auth/login",
                None,
                Some(json!({"username": intern, "password": "Temp12345"})),
                StatusCode::OK,
            )
            .await;
        let intern_token = login["token"].as_str().unwrap().to_owned();
        let me = self
            .call(
                "GET",
                "/users/me",
                Some(&intern_token),
                None,
                StatusCode::OK,
            )
            .await;
        assert_eq!(me["role"], "moderator");

        self.call(
            "POST",
            &format!("/admin/users/{intern_id}/deactivate"),
            Some(admin_token),
            None,
            StatusCode::OK,
        )
        .await;
        self.call(
            "POST",
            "/auth/login",
            None,
            Some(json!({"username": intern, "password": "Temp12345"})),
            StatusCode::UNAUTHORIZED,
        )
        .await;
        // The pre-deactivation token is revoked as well.
        self.call(
            "GET",
            "/users/me",
            Some(&intern_token),
            None,
            StatusCode::UNAUTHORIZED,
        )
        .await;
    }
}

#[tokio::test]
async fn full_system_lifecycle_over_http() {
    // Isolated temp database for this run.
    let db_path = std::env::temp_dir().join(format!(
        "rsblog-e2e-{}-{}.db",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    let db_path = db_path.to_string_lossy().into_owned();
    let config = Config {
        database_url: format!("sqlite://{db_path}?mode=rwc"),
        database_backend: DatabaseBackend::from_str("sqlite").unwrap(),
        jwt_secret: "e2e-test-secret-that-is-long-enough-0123456789".into(),
        jwt_expiration_hours: 24,
        host: "127.0.0.1".into(),
        port: 0,
        admin_username: "admin".into(),
        admin_email: "admin@blog.com".into(),
        admin_password: None,
        cors_origins: vec![],
        log_format: "text".into(),
        log_include_query: false,
        slow_request_ms: 0,
        // Anti-spam effectively off so the lifecycle is unaffected.
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
    };
    let pool = DbPool::init(&config).await;

    // Seed admin exactly like production startup does.
    let admin_hash = hash_password("Admin@123456").unwrap();
    user_repo::create_admin(
        &pool,
        &uuid::Uuid::new_v4().to_string(),
        "admin",
        "admin@blog.com",
        &admin_hash,
    )
    .await
    .unwrap();

    // Same middleware stack as `main()`.
    let app = create_router(AppState {
        pool,
        config: config.clone(),
        spam: blog_api::spam::SpamState::default(),
    })
    .layer(TraceLayer::new_for_http())
    .layer(
        CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
            ]),
    )
    .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let e2e = E2E {
        client: Client::new(),
        base: format!("http://{addr}/api/v1"),
        run: format!("e2e{}", std::process::id() % 100000),
    };

    // Health is reachable before anything else.
    let health: Value = e2e
        .client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["status"], "ok");

    let (admin_token, tag_id, tag_slug) = e2e.phase_admin_setup().await;
    let (user_token, _) = e2e.phase_user_lifecycle().await;
    let (post_id, _slug) = e2e
        .phase_posts(&user_token, &admin_token, &tag_id, &tag_slug)
        .await;
    let attacker_token = e2e.phase_security(&post_id).await;
    e2e.phase_comments(&post_id, &user_token, &attacker_token)
        .await;
    e2e.phase_admin(&admin_token).await;
}
