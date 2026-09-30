//! Integration tests: nested comment tree, soft/hard delete, and RBAC.

mod common;

use axum::http::StatusCode;
use common::{make_admin, make_published_post, new_app, register_user, uid, TestClient};

async fn make_comment(
    c: &mut TestClient,
    token: &str,
    post_id: &str,
    content: &str,
    parent: Option<&str>,
) -> String {
    let mut payload = serde_json::json!({"content": content});
    if let Some(p) = parent {
        payload["parent_id"] = p.into();
    }
    let (status, body) = c
        .post(
            &format!("/api/v1/posts/{post_id}/comments"),
            Some(token),
            payload,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn nested_three_level_tree_structure() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (alice, _, _) = register_user(&mut c, &format!("alice_{}", uid())).await;
    let (bob, _, _) = register_user(&mut c, &format!("bob_{}", uid())).await;
    let (post_id, _) =
        make_published_post(&mut c, &alice, &admin_token, &format!("Post {}", uid())).await;

    let root = make_comment(&mut c, &bob, &post_id, "root", None).await;
    let child = make_comment(&mut c, &alice, &post_id, "child", Some(&root)).await;
    make_comment(&mut c, &bob, &post_id, "grandchild", Some(&child)).await;

    let (status, tree) = c
        .get(&format!("/api/v1/posts/{post_id}/comments"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    assert_eq!(tree.as_array().unwrap().len(), 1);
    let root_node = &tree[0];
    assert_eq!(root_node["id"], root);
    assert_eq!(root_node["depth"], 0);
    assert_eq!(root_node["children"].as_array().unwrap().len(), 1);
    let child_node = &root_node["children"][0];
    assert_eq!(child_node["id"], child);
    assert_eq!(child_node["depth"], 1);
    assert_eq!(child_node["children"].as_array().unwrap().len(), 1);
    assert_eq!(child_node["children"][0]["depth"], 2);
}

#[tokio::test]
async fn soft_delete_parent_preserves_children() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (alice, _, _) = register_user(&mut c, &format!("alice_{}", uid())).await;
    let (bob, _, _) = register_user(&mut c, &format!("bob_{}", uid())).await;
    let (post_id, _) =
        make_published_post(&mut c, &alice, &admin_token, &format!("Post {}", uid())).await;

    let root = make_comment(&mut c, &bob, &post_id, "root", None).await;
    make_comment(&mut c, &alice, &post_id, "child", Some(&root)).await;

    let (status, _) = c
        .delete(
            &format!("/api/v1/posts/{post_id}/comments/{root}"),
            Some(&bob),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, tree) = c
        .get(&format!("/api/v1/posts/{post_id}/comments"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    assert_eq!(tree[0]["content"], "[deleted]");
    assert_eq!(tree[0]["is_deleted"], true);
    assert_eq!(tree[0]["children"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn hard_delete_leaf_removes_it() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (alice, _, _) = register_user(&mut c, &format!("alice_{}", uid())).await;
    let (post_id, _) =
        make_published_post(&mut c, &alice, &admin_token, &format!("Post {}", uid())).await;

    let leaf = make_comment(&mut c, &alice, &post_id, "lonely", None).await;
    let (status, _) = c
        .delete(
            &format!("/api/v1/posts/{post_id}/comments/{leaf}"),
            Some(&alice),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, tree) = c
        .get(&format!("/api/v1/posts/{post_id}/comments"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    assert_eq!(tree.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn comment_rbac_and_validation() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (alice, _, _) = register_user(&mut c, &format!("alice_{}", uid())).await;
    let (bob, _, _) = register_user(&mut c, &format!("bob_{}", uid())).await;
    let (post_id, _) =
        make_published_post(&mut c, &alice, &admin_token, &format!("Post {}", uid())).await;

    // Anonymous cannot comment.
    let (status, _) = c
        .post(
            &format!("/api/v1/posts/{post_id}/comments"),
            None,
            serde_json::json!({"content": "hi"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Empty content rejected.
    let (status, _) = c
        .post(
            &format!("/api/v1/posts/{post_id}/comments"),
            Some(&alice),
            serde_json::json!({"content": ""}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Unknown parent rejected.
    let (status, _) = c
        .post(
            &format!("/api/v1/posts/{post_id}/comments"),
            Some(&alice),
            serde_json::json!({"content": "x", "parent_id": "no-such-id"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Non-owner cannot edit/delete.
    let id = make_comment(&mut c, &alice, &post_id, "mine", None).await;
    let (status, _) = c
        .put(
            &format!("/api/v1/posts/{post_id}/comments/{id}"),
            Some(&bob),
            serde_json::json!({"content": "hijacked"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = c
        .delete(
            &format!("/api/v1/posts/{post_id}/comments/{id}"),
            Some(&bob),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Owner can edit.
    let (status, _) = c
        .put(
            &format!("/api/v1/posts/{post_id}/comments/{id}"),
            Some(&alice),
            serde_json::json!({"content": "edited"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn comments_on_missing_or_draft_post_404() {
    let mut c = TestClient::new(new_app().await);
    let (alice, _, _) = register_user(&mut c, &format!("alice_{}", uid())).await;

    let (status, _) = c
        .post(
            "/api/v1/posts/no-such-post/comments",
            Some(&alice),
            serde_json::json!({"content": "hi"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&alice),
            serde_json::json!({"title": format!("Draft {}", uid()), "content": "x", "published": false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let draft_id = body["id"].as_str().unwrap().to_owned();
    let (status, _) = c
        .post(
            &format!("/api/v1/posts/{draft_id}/comments"),
            Some(&alice),
            serde_json::json!({"content": "hi"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
