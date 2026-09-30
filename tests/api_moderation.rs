//! Integration tests: publishing workflow, viewer-scoped visibility,
//! and moderator-vs-admin guards.

mod common;

use axum::http::StatusCode;
use common::{
    make_admin, make_draft, make_moderator, make_post, make_published_post, new_app, publish_post,
    register_user, uid, TestClient,
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
async fn user_create_always_draft_even_if_published_true() {
    let mut c = TestClient::new(new_app().await);
    let (token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": format!("T {}", uid()), "content": "x", "published": true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["published"], false);
}

#[tokio::test]
async fn moderator_and_admin_create_published_one_shot() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    make_moderator(&mut c, &admin_token, &mod_id).await;

    for token in [&mod_token, &admin_token] {
        let (status, body) = c
            .post(
                "/api/v1/posts",
                Some(token),
                serde_json::json!({"title": format!("Pub {}", uid()), "content": "x", "published": true}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["published"], true);
        let slug = body["slug"].as_str().unwrap().to_owned();
        let (status, _) = c.get(&format!("/api/v1/posts/{slug}"), None).await;
        assert_eq!(status, StatusCode::OK);
    }

    // And they can also create drafts explicitly.
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&admin_token),
            serde_json::json!({"title": format!("D {}", uid()), "content": "x", "published": false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["published"], false);
}

#[tokio::test]
async fn user_cannot_publish_via_put_or_publish_endpoints() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let (post_id, _) = make_draft(&mut c, &token, &format!("Draft {}", uid())).await;

    // `published` was removed from PUT: unknown field -> 422.
    let (status, _) = c
        .put(
            &format!("/api/v1/posts/{post_id}"),
            Some(&token),
            serde_json::json!({"published": true}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Dedicated endpoints are moderator-only.
    for ep in ["publish", "unpublish"] {
        let (status, _) = c
            .post(
                &format!("/api/v1/posts/{post_id}/{ep}"),
                Some(&token),
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, _) = c
        .post(
            &format!("/api/v1/posts/{post_id}/publish"),
            None,
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Still a draft.
    let (status, _) = c.get(&format!("/api/v1/posts/{post_id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let _ = admin_token;
}

#[tokio::test]
async fn moderator_publish_unpublish_flow() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let (post_id, slug) = make_draft(&mut c, &token, &format!("Flow {}", uid())).await;

    publish_post(&mut c, &admin_token, &post_id).await;
    let (status, body) = c.get(&format!("/api/v1/posts/{slug}"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["published"], true);

    let (status, body) = c
        .post(
            &format!("/api/v1/posts/{post_id}/unpublish"),
            Some(&admin_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["published"], false);
    let (status, _) = c.get(&format!("/api/v1/posts/{slug}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn draft_get_visibility_matrix() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, admin_id) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (owner_token, _, _) = register_user(&mut c, &format!("owner_{}", uid())).await;
    let (other_token, _, _) = register_user(&mut c, &format!("other_{}", uid())).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    make_moderator(&mut c, &admin_token, &mod_id).await;
    let _ = admin_id;

    let (post_id, _) = make_draft(&mut c, &owner_token, &format!("Secret {}", uid())).await;

    let (status, _) = c.get(&format!("/api/v1/posts/{post_id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = c
        .get(&format!("/api/v1/posts/{post_id}"), Some(&other_token))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for token in [&owner_token, &mod_token, &admin_token] {
        let (status, body) = c
            .get(&format!("/api/v1/posts/{post_id}"), Some(token))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

#[tokio::test]
async fn list_visibility_scopes() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (alice, _, _) = register_user(&mut c, &format!("alice_{}", uid())).await;
    let (bob, _, _) = register_user(&mut c, &format!("bob_{}", uid())).await;

    let (alice_pub, _) =
        make_published_post(&mut c, &alice, &admin_token, &format!("APub {}", uid())).await;
    let (alice_draft, _) = make_draft(&mut c, &alice, &format!("ADraft {}", uid())).await;
    let (bob_draft, _) = make_draft(&mut c, &bob, &format!("BDraft {}", uid())).await;

    // Anon sees only published.
    let (status, body) = c.get("/api/v1/posts?per_page=100", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["posts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(ids.contains(&alice_pub.as_str()));
    assert!(!ids.contains(&alice_draft.as_str()));
    assert!(!ids.contains(&bob_draft.as_str()));

    // Alice sees published + own draft, not Bob's.
    let (status, body) = c.get("/api/v1/posts?per_page=100", Some(&alice)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["posts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(ids.contains(&alice_pub.as_str()));
    assert!(ids.contains(&alice_draft.as_str()));
    assert!(!ids.contains(&bob_draft.as_str()));

    // Admin sees everything.
    let (status, body) = c
        .get("/api/v1/posts?per_page=100", Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["posts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(ids.contains(&alice_pub.as_str()));
    assert!(ids.contains(&alice_draft.as_str()));
    assert!(ids.contains(&bob_draft.as_str()));
}

#[tokio::test]
async fn moderator_cannot_touch_admin_posts_but_admin_can_all() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    make_moderator(&mut c, &admin_token, &mod_id).await;

    // Admin creates published post in one shot.
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&admin_token),
            serde_json::json!({"title": format!("Admin {}", uid()), "content": "x", "published": true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let admin_post = body["id"].as_str().unwrap().to_owned();

    for (method, uri, payload) in [
        (
            "PUT",
            format!("/api/v1/posts/{admin_post}"),
            Some(serde_json::json!({"title": "hijack"})),
        ),
        ("DELETE", format!("/api/v1/posts/{admin_post}"), None),
    ] {
        let (status, _) = if method == "PUT" {
            c.put(&uri, Some(&mod_token), payload.unwrap()).await
        } else {
            c.delete(&uri, Some(&mod_token)).await
        };
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    for ep in ["publish", "unpublish"] {
        let (status, _) = c
            .post(
                &format!("/api/v1/posts/{admin_post}/{ep}"),
                Some(&mod_token),
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    // Moderator's own post can be moderated by admin.
    let (mod_post, _) = make_draft(&mut c, &mod_token, &format!("Mod {}", uid())).await;
    publish_post(&mut c, &admin_token, &mod_post).await;
    let (status, _) = c
        .put(
            &format!("/api/v1/posts/{mod_post}"),
            Some(&admin_token),
            serde_json::json!({"title": "Admin edited"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn draft_comments_mirror_post_visibility() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (owner_token, _, _) = register_user(&mut c, &format!("owner_{}", uid())).await;
    let (other_token, _, _) = register_user(&mut c, &format!("other_{}", uid())).await;

    // Draft with no comments yet: tree hidden from strangers.
    let (draft_id, _) = make_draft(&mut c, &owner_token, &format!("D {}", uid())).await;
    let (status, _) = c
        .get(&format!("/api/v1/posts/{draft_id}/comments"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = c
        .get(
            &format!("/api/v1/posts/{draft_id}/comments"),
            Some(&other_token),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for token in [&owner_token, &admin_token] {
        let (status, tree) = c
            .get(&format!("/api/v1/posts/{draft_id}/comments"), Some(token))
            .await;
        assert_eq!(status, StatusCode::OK, "{tree}");
    }

    // Published post comments visible to all.
    let (pub_id, _) =
        make_published_post(&mut c, &owner_token, &admin_token, &format!("P {}", uid())).await;
    make_comment(&mut c, &owner_token, &pub_id, "hello").await;
    let (status, tree) = c
        .get(&format!("/api/v1/posts/{pub_id}/comments"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    assert_eq!(tree.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn moderator_cannot_edit_admin_comment() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    make_moderator(&mut c, &admin_token, &mod_id).await;
    let (user_token, _, _) = register_user(&mut c, &format!("user_{}", uid())).await;

    let (post_id, _) =
        make_published_post(&mut c, &user_token, &admin_token, &format!("P {}", uid())).await;
    let admin_comment = make_comment(&mut c, &admin_token, &post_id, "admin says").await;
    let mod_comment = make_comment(&mut c, &mod_token, &post_id, "mod says").await;

    let (status, _) = c
        .put(
            &format!("/api/v1/posts/{post_id}/comments/{admin_comment}"),
            Some(&mod_token),
            serde_json::json!({"content": "hijack"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = c
        .delete(
            &format!("/api/v1/posts/{post_id}/comments/{admin_comment}"),
            Some(&mod_token),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admin can moderate the moderator's comment.
    let (status, _) = c
        .put(
            &format!("/api/v1/posts/{post_id}/comments/{mod_comment}"),
            Some(&admin_token),
            serde_json::json!({"content": "moderated"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Sanity: make_post helper still creates (draft for users).
    let _ = make_post(&mut c, &user_token, &format!("Extra {}", uid())).await;
}
