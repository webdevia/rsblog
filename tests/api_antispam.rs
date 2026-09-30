//! Integration tests: anti-spam — content caps, write throttles, duplicates,
//! link caps, trusted bypass, and the tight auth bucket.
//!
//! Most tests use `new_strict_app` (comment 3/min, post 3/hour, duplicates
//! 60 min, max 1 link, auth burst 10). Content-cap tests use the default app.

mod common;

use axum::http::StatusCode;
use common::{make_admin, new_app, new_strict_app, register_user, uid, TestClient};

#[tokio::test]
async fn post_content_and_excerpt_caps() {
    let mut c = TestClient::new(new_app().await);
    let (token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;

    let huge = "x".repeat(100_001);
    let (status, _) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": "Big", "content": huge}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let long_excerpt = "y".repeat(1001);
    let (status, _) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": "E", "content": "ok", "excerpt": long_excerpt}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Update path enforces the same caps.
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": format!("T {}", uid()), "content": "ok"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let post_id = body["id"].as_str().unwrap().to_owned();
    let (status, _) = c
        .put(
            &format!("/api/v1/posts/{post_id}"),
            Some(&token),
            serde_json::json!({"content": huge}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn comment_flood_returns_429_with_retry_after() {
    let mut c = TestClient::new(new_strict_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (token, _, _) = register_user(&mut c, &format!("spammer_{}", uid())).await;
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&admin_token),
            serde_json::json!({
                "title": format!("Open {}", uid()),
                "content": "discuss",
                "published": true,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let post_id = body["id"].as_str().unwrap().to_owned();

    for i in 0..3 {
        let (status, body) = c
            .post(
                &format!("/api/v1/posts/{post_id}/comments"),
                Some(&token),
                serde_json::json!({"content": format!("comment number {i} {}", uid())}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, body) = c
        .post(
            &format!("/api/v1/posts/{post_id}/comments"),
            Some(&token),
            serde_json::json!({"content": format!("one too many {}", uid())}),
        )
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"]["status"], 429);
}

#[tokio::test]
async fn retry_after_header_present_on_throttle() {
    let mut c = TestClient::new(new_strict_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (token, _, _) = register_user(&mut c, &format!("spammer_{}", uid())).await;
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&admin_token),
            serde_json::json!({
                "title": format!("Open {}", uid()),
                "content": "discuss",
                "published": true,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let post_id = body["id"].as_str().unwrap().to_owned();
    for i in 0..3 {
        let (status, _) = c
            .post(
                &format!("/api/v1/posts/{post_id}/comments"),
                Some(&token),
                serde_json::json!({"content": format!("flood {i} {}", uid())}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, headers, _) = c
        .request_with_headers(
            "POST",
            &format!("/api/v1/posts/{post_id}/comments"),
            Some(&token),
            Some(serde_json::json!({"content": "blocked"})),
        )
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(headers.contains_key("retry-after"), "{headers:?}");
}

#[tokio::test]
async fn post_hourly_cap_per_user() {
    let mut c = TestClient::new(new_strict_app().await);
    let (token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    for i in 0..3 {
        let (status, body) = c
            .post(
                "/api/v1/posts",
                Some(&token),
                serde_json::json!({"title": format!("Cap {} {}", i, uid()), "content": "x"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": format!("Over {}", uid()), "content": "x"}),
        )
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
}

#[tokio::test]
async fn duplicate_content_rejected() {
    let mut c = TestClient::new(new_strict_app().await);
    let (token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let title = format!("Same {}", uid());
    let (status, _) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": title, "content": "identical body"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": title, "content": "identical body"}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // Different content passes.
    let (status, _) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": title, "content": "different body"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn link_cap_for_new_users() {
    let mut c = TestClient::new(new_strict_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (token, _, _) = register_user(&mut c, &format!("newbie_{}", uid())).await;

    // Strict app allows max 1 link: two links rejected.
    let (status, _) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({
                "title": format!("Links {}", uid()),
                "content": "see https://a.example and https://b.example",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // One link passes.
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({
                "title": format!("One link {}", uid()),
                "content": "see https://a.example",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let _ = admin_token;
}

#[tokio::test]
async fn trusted_users_skip_throttles_and_link_caps() {
    let mut c = TestClient::new(new_strict_app().await);
    let (admin_token, admin_id) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (token, user_id, _) = register_user(&mut c, &format!("veteran_{}", uid())).await;

    // Age the account 40 days via SQL (trusted threshold is 30).
    blog_api::db_query!(c.pool(), |p| {
        sqlx::query("UPDATE users SET created_at = datetime('now', '-40 days') WHERE id = $1")
            .bind(&user_id)
            .execute(p)
            .await
            .unwrap();
    });

    // Past the post cap (3/hour) and link cap (1): all pass.
    for i in 0..5 {
        let (status, body) = c
            .post(
                "/api/v1/posts",
                Some(&token),
                serde_json::json!({
                    "title": format!("Veteran {} {}", i, uid()),
                    "content": "a https://a.example b https://b.example c https://c.example",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    // Moderators are trusted by role: flood of links passes.
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{mod_id}/role"),
            Some(&admin_token),
            serde_json::json!({"role": "moderator"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    for i in 0..4 {
        let (status, body) = c
            .post(
                "/api/v1/posts",
                Some(&mod_token),
                serde_json::json!({
                    "title": format!("Mod {} {}", i, uid()),
                    "content": "a https://a.example b https://b.example",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let _ = admin_id;
}

#[tokio::test]
async fn auth_bucket_rejects_registration_flood() {
    let mut c = TestClient::new(new_strict_app().await);
    // Strict auth bucket: burst 5. Invalid registrations fail fast in the
    // handler (no Argon2 work), so rapid attempts deterministically exhaust
    // the bucket without timing flakes.
    for i in 0..5 {
        let (status, _) = c
            .post(
                "/api/v1/auth/register",
                None,
                serde_json::json!({
                    "username": format!("flood{}_{}", i, uid()),
                    "email": "bad-email",
                    "password": "x",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "attempt {i}");
    }
    for i in 0..3 {
        let (status, headers, body) = c
            .request_with_headers(
                "POST",
                "/api/v1/auth/register",
                None,
                Some(serde_json::json!({
                    "username": format!("blocked{}_{}", i, uid()),
                    "email": "bad-email",
                    "password": "x",
                })),
            )
            .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "attempt {i}: {body}");
        assert_eq!(body["error"]["status"], 429);
        assert!(headers.contains_key("retry-after"), "{headers:?}");
    }
}
