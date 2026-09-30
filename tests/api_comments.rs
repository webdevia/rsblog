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

/// Publish a post, comment on it, then unpublish: yields a comment on a draft.
async fn make_draft_comment(
    c: &mut TestClient,
    admin_token: &str,
    author_token: &str,
    commenter_token: &str,
    content: &str,
) -> (String, String) {
    let (post_id, _) =
        make_published_post(c, author_token, admin_token, &format!("Drafty {}", uid())).await;
    let cid = make_comment(c, commenter_token, &post_id, content, None).await;
    let (status, _) = c
        .post(
            &format!("/api/v1/posts/{post_id}/unpublish"),
            Some(admin_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    (post_id, cid)
}

#[tokio::test]
async fn flat_list_scopes_by_role() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (alice, _, alice_name) = register_user(&mut c, &format!("alice_{}", uid())).await;
    let (bob, bob_id, _) = register_user(&mut c, &format!("bob_{}", uid())).await;

    let (pub_id, _) =
        make_published_post(&mut c, &alice, &admin_token, &format!("Pub {}", uid())).await;
    let bob_cid = make_comment(&mut c, &bob, &pub_id, "bob here", None).await;
    let alice_cid = make_comment(&mut c, &alice, &pub_id, "alice here", None).await;
    let (_, draft_cid) =
        make_draft_comment(&mut c, &admin_token, &alice, &bob, "bob on draft").await;

    // Anonymous denied.
    let (status, _) = c.get("/api/v1/comments", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Bob sees only his own comments on visible posts (not alice's, not the draft one).
    let (status, body) = c.get("/api/v1/comments?per_page=100", Some(&bob)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["comments"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["id"].as_str())
        .collect();
    assert!(ids.contains(&bob_cid.as_str()), "{ids:?}");
    assert!(!ids.contains(&alice_cid.as_str()), "{ids:?}");
    assert!(!ids.contains(&draft_cid.as_str()), "{ids:?}");
    // Post context included without fetching posts.
    let node = &body["comments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == bob_cid)
        .unwrap();
    assert_eq!(node["post"]["id"], pub_id);
    assert_eq!(node["author"]["id"], bob_id);
    assert!(!node["author"]["username"].as_str().unwrap().is_empty());

    // Alice (post author) sees her comments, incl. nothing hidden from her here.
    let (status, body) = c.get("/api/v1/comments", Some(&alice)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["total"].as_i64().unwrap() >= 1);

    // Admin sees everything, including the draft-post comment.
    let (status, body) = c
        .get("/api/v1/comments?per_page=100", Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["comments"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["id"].as_str())
        .collect();
    assert!(ids.contains(&bob_cid.as_str()));
    assert!(ids.contains(&alice_cid.as_str()));
    assert!(ids.contains(&draft_cid.as_str()));
    assert_eq!(body["page"], 1);
    let _ = alice_name;
}

#[tokio::test]
async fn flat_list_filters_and_deleted_scope() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (alice, _, alice_name) = register_user(&mut c, &format!("alice_{}", uid())).await;
    let (bob, _, _) = register_user(&mut c, &format!("bob_{}", uid())).await;
    let (pub_id, pub_slug) =
        make_published_post(&mut c, &alice, &admin_token, &format!("Pub {}", uid())).await;

    let root = make_comment(&mut c, &bob, &pub_id, "unique-root-word", None).await;
    make_comment(&mut c, &alice, &pub_id, "child reply", Some(&root)).await;
    // Soft-delete the root (has a child).
    let (status, _) = c
        .delete(
            &format!("/api/v1/posts/{pub_id}/comments/{root}"),
            Some(&bob),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Default hides soft-deleted.
    let (status, body) = c
        .get("/api/v1/comments?per_page=100", Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1);
    // Explicit deleted=true shows only the soft-deleted one.
    let (status, body) = c
        .get("/api/v1/comments?deleted=true", Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1);
    assert_eq!(body["comments"][0]["id"], root);
    assert_eq!(body["comments"][0]["is_deleted"], true);

    // Author filter.
    let (status, body) = c
        .get(
            &format!("/api/v1/comments?author={alice_name}"),
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["total"].as_i64().unwrap() >= 1);
    assert!(body["comments"]
        .as_array()
        .unwrap()
        .iter()
        .all(|x| x["author"]["username"] == alice_name));

    // Post filter by slug.
    let (status, body) = c
        .get(
            &format!("/api/v1/comments?post={pub_slug}"),
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["comments"]
        .as_array()
        .unwrap()
        .iter()
        .all(|x| x["post"]["slug"] == pub_slug));

    // Search filter (on live content; soft-deleted bodies read "[deleted]").
    let (status, body) = c
        .get("/api/v1/comments?search=child", Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1);
    assert_eq!(body["comments"][0]["content"], "child reply");
}

#[tokio::test]
async fn flat_list_pagination_and_order() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (alice, _, _) = register_user(&mut c, &format!("alice_{}", uid())).await;
    let (pub_id, _) =
        make_published_post(&mut c, &alice, &admin_token, &format!("Pub {}", uid())).await;
    for i in 0..3 {
        make_comment(&mut c, &alice, &pub_id, &format!("c{i}"), None).await;
    }

    let (status, p1) = c
        .get("/api/v1/comments?per_page=2&page=1", Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{p1}");
    assert_eq!(p1["comments"].as_array().unwrap().len(), 2);
    let (status, p2) = c
        .get("/api/v1/comments?per_page=2&page=2", Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{p2}");
    assert_eq!(p2["comments"].as_array().unwrap().len(), 1);
    assert_eq!(p2["page"], 2);

    let (status, asc) = c
        .get(
            "/api/v1/comments?per_page=100&order=asc",
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{asc}");
    let (status, desc) = c
        .get("/api/v1/comments?per_page=100", Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::OK);
    let mut asc_ids: Vec<&str> = asc["comments"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["id"].as_str())
        .collect();
    let desc_ids: Vec<&str> = desc["comments"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["id"].as_str())
        .collect();
    asc_ids.reverse();
    assert_eq!(desc_ids, asc_ids);

    let (status, _) = c
        .get("/api/v1/comments?order=sideways", Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn flat_get_visibility() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (alice, _, _) = register_user(&mut c, &format!("alice_{}", uid())).await;
    let (bob, _, _) = register_user(&mut c, &format!("bob_{}", uid())).await;
    let (stranger, _, _) = register_user(&mut c, &format!("stranger_{}", uid())).await;

    let (pub_id, _) =
        make_published_post(&mut c, &alice, &admin_token, &format!("Pub {}", uid())).await;
    let cid = make_comment(&mut c, &bob, &pub_id, "visible", None).await;
    let (_, draft_cid) = make_draft_comment(&mut c, &admin_token, &alice, &bob, "hidden gem").await;

    // Published comment: any authenticated viewer.
    for token in [&bob, &alice, &stranger, &admin_token] {
        let (status, body) = c.get(&format!("/api/v1/comments/{cid}"), Some(token)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["item"]["id"], cid);
        assert_eq!(body["item"]["post"]["id"], pub_id);
    }
    let (status, _) = c.get(&format!("/api/v1/comments/{cid}"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Draft-post comment: post author + mod see it; comment author doesn't.
    let (status, _) = c
        .get(&format!("/api/v1/comments/{draft_cid}"), Some(&alice))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = c
        .get(&format!("/api/v1/comments/{draft_cid}"), Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = c
        .get(&format!("/api/v1/comments/{draft_cid}"), Some(&bob))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = c
        .get(&format!("/api/v1/comments/{draft_cid}"), Some(&stranger))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = c.get("/api/v1/comments/nope", Some(&admin_token)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn flat_update_delete_parity() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, admin_id) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (alice, _, _) = register_user(&mut c, &format!("alice_{}", uid())).await;
    let (bob, _, _) = register_user(&mut c, &format!("bob_{}", uid())).await;
    let (pub_id, _) =
        make_published_post(&mut c, &alice, &admin_token, &format!("Pub {}", uid())).await;
    let cid = make_comment(&mut c, &bob, &pub_id, "mine", None).await;

    // Stranger cannot edit/delete.
    let (status, _) = c
        .put(
            &format!("/api/v1/comments/{cid}"),
            Some(&alice),
            serde_json::json!({"content": "hijack"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = c
        .delete(&format!("/api/v1/comments/{cid}"), Some(&alice))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Owner edits via flat alias.
    let (status, _) = c
        .put(
            &format!("/api/v1/comments/{cid}"),
            Some(&bob),
            serde_json::json!({"content": "edited flat"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, tree) = c
        .get(&format!("/api/v1/posts/{pub_id}/comments"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    assert_eq!(tree[0]["content"], "edited flat");

    // Moderator cannot touch admin's comment via flat alias either.
    let admin_cid = make_comment(&mut c, &admin_token, &pub_id, "admin words", None).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{mod_id}/role"),
            Some(&admin_token),
            serde_json::json!({"role": "moderator"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = c
        .put(
            &format!("/api/v1/comments/{admin_cid}"),
            Some(&mod_token),
            serde_json::json!({"content": "hijack"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Owner deletes via flat alias (leaf -> hard delete).
    let (status, _) = c
        .delete(&format!("/api/v1/comments/{cid}"), Some(&bob))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = c
        .get(&format!("/api/v1/comments/{cid}"), Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let _ = admin_id;
}
