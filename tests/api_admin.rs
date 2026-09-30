//! Integration tests: admin console — listing, role changes, deactivation.

mod common;

use axum::http::StatusCode;
use common::{make_admin, new_app, register_user, uid, TestClient};

#[tokio::test]
async fn admin_user_listing_is_admin_only() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (user_token, _, username) = register_user(&mut c, &format!("user_{}", uid())).await;

    let (status, _) = c.get("/api/v1/admin/users", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = c.get("/api/v1/admin/users", Some(&user_token)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = c.get("/api/v1/admin/users", Some(&admin_token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["username"].as_str())
        .collect();
    assert!(names.contains(&username.as_str()));
}

#[tokio::test]
async fn role_update_validation_and_self_protection() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, admin_id) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (_, user_id, _) = register_user(&mut c, &format!("user_{}", uid())).await;

    // Invalid role rejected.
    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{user_id}/role"),
            Some(&admin_token),
            serde_json::json!({"role": "superuser"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Unknown user -> 404.
    let (status, _) = c
        .put(
            "/api/v1/admin/users/no-such-user/role",
            Some(&admin_token),
            serde_json::json!({"role": "moderator"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Cannot change your own role.
    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{admin_id}/role"),
            Some(&admin_token),
            serde_json::json!({"role": "user"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Happy path.
    let (status, body) = c
        .put(
            &format!("/api/v1/admin/users/{user_id}/role"),
            Some(&admin_token),
            serde_json::json!({"role": "moderator"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["role"], "moderator");
}

#[tokio::test]
async fn deactivation_locks_account_and_cannot_target_self() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, admin_id) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (_, user_id, username) = register_user(&mut c, &format!("user_{}", uid())).await;

    // Cannot deactivate yourself.
    let (status, _) = c
        .post(
            &format!("/api/v1/admin/users/{admin_id}/deactivate"),
            Some(&admin_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = c
        .post(
            &format!("/api/v1/admin/users/{user_id}/deactivate"),
            Some(&admin_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = c
        .post(
            "/api/v1/auth/login",
            None,
            serde_json::json!({"username": username, "password": "SecurePassword123"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn activate_deactivate_rbac_and_404() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, admin_id) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (user_token, user_id, _) = register_user(&mut c, &format!("user_{}", uid())).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{mod_id}/role"),
            Some(&admin_token),
            serde_json::json!({"role": "moderator"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    for endpoint in ["deactivate", "activate"] {
        let uri = format!("/api/v1/admin/users/{user_id}/{endpoint}");
        let (status, _) = c.post(&uri, None, serde_json::json!({})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{endpoint} anon");
        let (status, _) = c.post(&uri, Some(&user_token), serde_json::json!({})).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{endpoint} user");
        let (status, _) = c.post(&uri, Some(&mod_token), serde_json::json!({})).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{endpoint} mod");
        let (status, _) = c
            .post(
                &format!("/api/v1/admin/users/no-such-id/{endpoint}"),
                Some(&admin_token),
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{endpoint} unknown");
    }

    // Cannot activate yourself.
    let (status, _) = c
        .post(
            &format!("/api/v1/admin/users/{admin_id}/activate"),
            Some(&admin_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn deactivation_revokes_write_tokens_and_reactivate_restores() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (token, user_id, username) = register_user(&mut c, &format!("user_{}", uid())).await;

    // Deactivate twice: idempotent 200.
    for _ in 0..2 {
        let (status, _) = c
            .post(
                &format!("/api/v1/admin/users/{user_id}/deactivate"),
                Some(&admin_token),
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }

    // Old token cannot write.
    let (status, _) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": format!("X {}", uid()), "content": "x"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Reactivate: account works again. Note: JWTs are stateless + DB-revalidated
    // for `is_active`, so a non-expired pre-deactivation token becomes valid
    // again (no versioning). Clients should still re-login to be safe.
    let (status, _) = c
        .post(
            &format!("/api/v1/admin/users/{user_id}/activate"),
            Some(&admin_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = c.get("/api/v1/users/me", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = c
        .post(
            "/api/v1/auth/login",
            None,
            serde_json::json!({"username": username, "password": "SecurePassword123"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let fresh = body["token"].as_str().unwrap().to_owned();
    let (status, _) = c.get("/api/v1/users/me", Some(&fresh)).await;
    assert_eq!(status, StatusCode::OK);
}
