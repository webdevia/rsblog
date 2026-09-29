//! Integration tests: registration, login, and profile endpoints.

mod common;

use axum::http::StatusCode;
use common::{make_admin, new_app, register_user, uid, TestClient};

#[tokio::test]
async fn register_returns_token_and_user() {
    let mut c = TestClient::new(new_app().await);
    let name = format!("alice_{}", uid());
    let (status, body) = c
        .post(
            "/api/v1/auth/register",
            None,
            serde_json::json!({
                "username": name,
                "email": format!("{name}@test.local"),
                "password": "SecurePassword123",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["user"]["username"], name);
    assert_eq!(body["user"]["role"], "user");
    assert!(!body["token"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn register_duplicate_username_conflicts() {
    let mut c = TestClient::new(new_app().await);
    let name = format!("bob_{}", uid());
    register_user(&mut c, &name).await;
    let (status, body) = c
        .post(
            "/api/v1/auth/register",
            None,
            serde_json::json!({
                "username": name,
                "email": "different@test.local",
                "password": "SecurePassword123",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn register_duplicate_email_conflicts() {
    let mut c = TestClient::new(new_app().await);
    let name = format!("carol_{}", uid());
    register_user(&mut c, &name).await;
    let (status, _) = c
        .post(
            "/api/v1/auth/register",
            None,
            serde_json::json!({
                "username": format!("other_{}", uid()),
                "email": format!("{name}@test.local"),
                "password": "SecurePassword123",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn register_rejects_invalid_input() {
    let mut c = TestClient::new(new_app().await);
    for payload in [
        serde_json::json!({"username": "ab", "email": "x@test.local", "password": "SecurePassword123"}),
        serde_json::json!({"username": "validname", "email": "not-an-email", "password": "SecurePassword123"}),
        serde_json::json!({"username": "validname", "email": "x@test.local", "password": "short"}),
    ] {
        let (status, _) = c.post("/api/v1/auth/register", None, payload).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[tokio::test]
async fn login_roundtrip_and_failures() {
    let mut c = TestClient::new(new_app().await);
    let name = format!("dave_{}", uid());
    register_user(&mut c, &name).await;

    let (status, body) = c
        .post(
            "/api/v1/auth/login",
            None,
            serde_json::json!({"username": name, "password": "SecurePassword123"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["username"], name);

    let (status, _) = c
        .post(
            "/api/v1/auth/login",
            None,
            serde_json::json!({"username": name, "password": "WrongPassword!"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = c
        .post(
            "/api/v1/auth/login",
            None,
            serde_json::json!({"username": "nobody-here", "password": "Whatever123"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn users_me_requires_auth() {
    let mut c = TestClient::new(new_app().await);
    let (status, _) = c.get("/api/v1/users/me", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let name = format!("erin_{}", uid());
    let (token, _, _) = register_user(&mut c, &name).await;
    let (status, body) = c.get("/api/v1/users/me", Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["username"], name);
}

#[tokio::test]
async fn users_me_rejects_tampered_token() {
    let mut c = TestClient::new(new_app().await);
    let (status, _) = c.get("/api/v1/users/me", Some("tampered.token.here")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn deactivated_user_token_is_revoked() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let name = format!("mallory_{}", uid());
    let (token, user_id, _) = register_user(&mut c, &name).await;

    // Sanity: token works before deactivation.
    let (status, _) = c.get("/api/v1/users/me", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = c
        .post(
            &format!("/api/v1/admin/users/{user_id}/deactivate"),
            Some(&admin_token),
            serde_json::json!({}),
        )
        .await;
    // Deactivate handler ignores the body; any JSON is fine.
    assert_eq!(status, StatusCode::OK);

    // Existing token must stop working, and login must fail.
    let (status, _) = c.get("/api/v1/users/me", Some(&token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = c
        .post(
            "/api/v1/auth/login",
            None,
            serde_json::json!({"username": name, "password": "SecurePassword123"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
