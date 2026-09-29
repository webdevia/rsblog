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
