//! Integration tests: tags CRUD and role-gated moderation.

mod common;

use axum::http::StatusCode;
use common::{make_admin, make_post, new_app, register_user, uid, TestClient};

async fn make_tag(c: &mut TestClient, token: &str, name: &str) -> (String, String) {
    let (status, body) = c
        .post(
            "/api/v1/tags",
            Some(token),
            serde_json::json!({"name": name}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (
        body["id"].as_str().unwrap().to_owned(),
        body["slug"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test]
async fn create_tag_requires_moderator_role() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (user_token, user_id, _) = register_user(&mut c, &format!("user_{}", uid())).await;

    // Anonymous -> 401, plain user -> 403.
    let (status, _) = c
        .post("/api/v1/tags", None, serde_json::json!({"name": "rust"}))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = c
        .post(
            "/api/v1/tags",
            Some(&user_token),
            serde_json::json!({"name": "rust"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Promote -> 200.
    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{user_id}/role"),
            Some(&admin_token),
            serde_json::json!({"role": "moderator"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let name = format!("rust_{}", uid());
    let (status, body) = c
        .post(
            "/api/v1/tags",
            Some(&user_token),
            serde_json::json!({"name": name}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Duplicate name conflicts.
    let (status, _) = c
        .post(
            "/api/v1/tags",
            Some(&user_token),
            serde_json::json!({"name": name}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn get_tag_by_id_and_slug() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (tag_id, slug) = make_tag(&mut c, &admin_token, &format!("async_{}", uid())).await;

    for key in [&tag_id, &slug] {
        let (status, body) = c.get(&format!("/api/v1/tags/{key}"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["id"], tag_id);
    }

    let (status, _) = c.get("/api/v1/tags/no-such-tag", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_and_delete_tag_rbac() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (user_token, user_id, _) = register_user(&mut c, &format!("user_{}", uid())).await;
    let (tag_id, _) = make_tag(&mut c, &admin_token, &format!("web_{}", uid())).await;

    // Plain user cannot update...
    let (status, _) = c
        .put(
            &format!("/api/v1/tags/{tag_id}"),
            Some(&user_token),
            serde_json::json!({"name": "hacked"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // ...moderator can.
    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{user_id}/role"),
            Some(&admin_token),
            serde_json::json!({"role": "moderator"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = c
        .put(
            &format!("/api/v1/tags/{tag_id}"),
            Some(&user_token),
            serde_json::json!({"name": "web-frameworks"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["slug"], "web-frameworks");

    // Moderator cannot delete (admin-only)...
    let (status, _) = c
        .delete(&format!("/api/v1/tags/{tag_id}"), Some(&user_token))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // ...admin can.
    let (status, _) = c
        .delete(&format!("/api/v1/tags/{tag_id}"), Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = c.get(&format!("/api/v1/tags/{tag_id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_tags_reports_post_counts_and_filters_posts() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (user_token, _, _) = register_user(&mut c, &format!("writer_{}", uid())).await;

    let tag_name = format!("tokio_{}", uid());
    let (tag_id, tag_slug) = make_tag(&mut c, &admin_token, &tag_name).await;

    // Attach the tag to a post via tag_ids (created as draft, then published).
    let title = format!("Tagged {}", uid());
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&user_token),
            serde_json::json!({
                "title": title,
                "content": "with tags",
                "published": true,
                "tag_ids": [tag_id],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["published"], false,
        "regular users always create drafts"
    );
    let draft_id = body["id"].as_str().unwrap().to_owned();
    let (status, _) = c
        .post(
            &format!("/api/v1/posts/{draft_id}/publish"),
            Some(&admin_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = c.get("/api/v1/tags", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entry = body
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == tag_id)
        .expect("tag should be listed");
    assert!(entry["post_count"].as_i64().unwrap() >= 1, "{entry}");

    // Post list filtered by tag slug finds the post.
    let (status, body) = c.get(&format!("/api/v1/posts?tag={tag_slug}"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["total"].as_i64().unwrap() >= 1, "{body}");

    // make_post helper path still works alongside tags.
    let _ = make_post(&mut c, &user_token, &format!("Plain {}", uid())).await;
}
