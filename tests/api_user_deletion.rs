//! Integration tests: narrowed moderator user-management matrix +
//! soft/hard user deletion.
//!
//! Rules under test:
//! - moderators: list/view users (admins masked), promote user->moderator,
//!   ban/unban user-role accounts. Everything else 403/404.
//! - deletion (soft/hard) is admin-only; self 400; unknown 404.

mod common;

use axum::http::StatusCode;
use common::{
    make_admin, make_moderator, make_published_post, new_app, register_user, uid, TestClient,
};

async fn make_comment(c: &mut TestClient, token: &str, post_id: &str, content: &str) -> String {
    let (status, body) = c
        .post(
            &format!("/api/v1/posts/{post_id}/comments"),
            Some(token),
            serde_json::json!({"content": content}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn moderator_promote_only_user_to_moderator() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    make_moderator(&mut c, &admin_token, &mod_id).await;
    let (_, user_id, _) = register_user(&mut c, &format!("user_{}", uid())).await;
    let (_, mod2_id, _) = register_user(&mut c, &format!("mod2_{}", uid())).await;
    make_moderator(&mut c, &admin_token, &mod2_id).await;

    // Promote user -> moderator: allowed.
    let (status, body) = c
        .put(
            &format!("/api/v1/admin/users/{user_id}/role"),
            Some(&mod_token),
            serde_json::json!({"role": "moderator"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["role"], "moderator");

    // Demote moderator -> user: forbidden.
    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{mod2_id}/role"),
            Some(&mod_token),
            serde_json::json!({"role": "user"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Grant admin: forbidden.
    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{user_id}/role"),
            Some(&mod_token),
            serde_json::json!({"role": "admin"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // No-op promote (already moderator) still passes the promote-only rule.
    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{mod2_id}/role"),
            Some(&mod_token),
            serde_json::json!({"role": "moderator"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn moderator_role_actions_on_admins_forbidden() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, admin_id) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    make_moderator(&mut c, &admin_token, &mod_id).await;

    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{admin_id}/role"),
            Some(&mod_token),
            serde_json::json!({"role": "user"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = c
        .get(&format!("/api/v1/admin/users/{admin_id}"), Some(&mod_token))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    for mode in ["", "?mode=soft", "?mode=hard"] {
        let (status, _) = c
            .delete(
                &format!("/api/v1/admin/users/{admin_id}{mode}"),
                Some(&mod_token),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "mode={mode}");
    }
}

#[tokio::test]
async fn soft_delete_preserves_content_under_ghost() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (victim_token, victim_id, victim_name) =
        register_user(&mut c, &format!("victim_{}", uid())).await;
    let victim_email = format!("{victim_name}@test.local");

    let (post_id, slug) = make_published_post(
        &mut c,
        &victim_token,
        &admin_token,
        &format!("Victim Post {}", uid()),
    )
    .await;
    // Victim also comments on someone else's post.
    let (other_token, _, _) = register_user(&mut c, &format!("other_{}", uid())).await;
    let (other_post, _) = make_published_post(
        &mut c,
        &other_token,
        &admin_token,
        &format!("Other Post {}", uid()),
    )
    .await;
    make_comment(&mut c, &victim_token, &other_post, "victim comment").await;

    let (status, body) = c
        .delete(
            &format!("/api/v1/admin/users/{victim_id}"),
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mode"], "soft");
    assert_eq!(body["posts_reassigned"], 1);
    assert_eq!(body["comments_reassigned"], 1);

    // Content stays visible; slug intact; author is the ghost.
    let (status, post) = c.get(&format!("/api/v1/posts/{slug}"), None).await;
    assert_eq!(status, StatusCode::OK, "{post}");
    assert_eq!(post["author"]["username"], "[deleted]");
    assert_eq!(post["id"], post_id);

    let (status, tree) = c
        .get(&format!("/api/v1/posts/{other_post}/comments"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    assert_eq!(tree[0]["content"], "victim comment");
    assert_eq!(tree[0]["author"]["username"], "[deleted]");

    // Account is dead: old name/email no longer work.
    let (status, _) = c
        .post(
            "/api/v1/auth/login",
            None,
            serde_json::json!({"username": victim_name, "password": "SecurePassword123"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = c.get("/api/v1/users/me", Some(&victim_token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let _ = victim_email;
}

#[tokio::test]
async fn soft_delete_idempotent_and_ghost_reused() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (_, victim_a, _) = register_user(&mut c, &format!("a_{}", uid())).await;
    let (_, victim_b, _) = register_user(&mut c, &format!("b_{}", uid())).await;

    for vid in [&victim_a, &victim_b] {
        let (status, body) = c
            .delete(&format!("/api/v1/admin/users/{vid}"), Some(&admin_token))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["posts_reassigned"], 0);
    }
    // Second soft-delete of the same row is a harmless no-op.
    let (status, body) = c
        .delete(
            &format!("/api/v1/admin/users/{victim_a}"),
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn hard_delete_purges_content_and_frees_email() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (victim_token, victim_id, victim_name) =
        register_user(&mut c, &format!("victim_{}", uid())).await;
    let victim_email = format!("{victim_name}@test.local");

    let (post_id, _) = make_published_post(
        &mut c,
        &victim_token,
        &admin_token,
        &format!("Gone {}", uid()),
    )
    .await;
    make_comment(&mut c, &victim_token, &post_id, "gone comment").await;

    let (status, body) = c
        .delete(
            &format!("/api/v1/admin/users/{victim_id}?mode=hard"),
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mode"], "hard");
    assert_eq!(body["posts_deleted"], 1);
    assert_eq!(body["comments_deleted"], 1);

    // Everything is gone.
    let (status, _) = c
        .get(&format!("/api/v1/posts/{post_id}"), Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = c
        .get(
            &format!("/api/v1/admin/users/{victim_id}"),
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Repeat delete -> 404.
    let (status, _) = c
        .delete(
            &format!("/api/v1/admin/users/{victim_id}?mode=hard"),
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Email/username are reusable.
    let (status, body) = c
        .post(
            "/api/v1/auth/register",
            None,
            serde_json::json!({
                "username": victim_name,
                "email": victim_email,
                "password": "SecurePassword123",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn delete_guards() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, admin_id) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (user_token, user_id, _) = register_user(&mut c, &format!("user_{}", uid())).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    make_moderator(&mut c, &admin_token, &mod_id).await;

    // Auth matrix: delete is admin-only.
    let (status, _) = c
        .delete(&format!("/api/v1/admin/users/{user_id}"), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = c
        .delete(&format!("/api/v1/admin/users/{user_id}"), Some(&user_token))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = c
        .delete(
            &format!("/api/v1/admin/users/{user_id}?mode=hard"),
            Some(&mod_token),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Self-delete and bad mode.
    let (status, _) = c
        .delete(
            &format!("/api/v1/admin/users/{admin_id}"),
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = c
        .delete(
            &format!("/api/v1/admin/users/{user_id}?mode=nuke"),
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Unknown id.
    let (status, _) = c
        .delete("/api/v1/admin/users/no-such-id", Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
