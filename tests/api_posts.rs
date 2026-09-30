//! Integration tests: posts lifecycle, visibility, search, and RBAC.

mod common;

use axum::http::StatusCode;
use common::{make_admin, make_post, make_published_post, new_app, register_user, uid, TestClient};

#[tokio::test]
async fn create_post_requires_auth() {
    let mut c = TestClient::new(new_app().await);
    let (status, _) = c
        .post(
            "/api/v1/posts",
            None,
            serde_json::json!({"title": "Nope", "content": "no auth"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn create_and_fetch_post_by_slug_and_id() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let (post_id, slug) = make_published_post(
        &mut c,
        &token,
        &admin_token,
        &format!("Hello World {}", uid()),
    )
    .await;

    for key in [&slug, &post_id] {
        let (status, body) = c.get(&format!("/api/v1/posts/{key}"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["id"], post_id);
        assert_eq!(body["slug"], slug);
    }

    let (status, _) = c.get("/api/v1/posts/does-not-exist", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_post_rejects_invalid_input() {
    let mut c = TestClient::new(new_app().await);
    let (token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let (status, _) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": "", "content": "x"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn duplicate_titles_get_unique_slugs() {
    let mut c = TestClient::new(new_app().await);
    let (token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let title = format!("Same Title {}", uid());
    let (_, slug_a) = make_post(&mut c, &token, &title).await;
    let (_, slug_b) = make_post(&mut c, &token, &title).await;
    assert_ne!(slug_a, slug_b);
}

#[tokio::test]
async fn draft_hidden_publicly_but_visible_to_owner() {
    let mut c = TestClient::new(new_app().await);
    let (token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let (other_token, _, _) = register_user(&mut c, &format!("other_{}", uid())).await;

    let title = format!("Draft {}", uid());
    let (status, body) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": title, "content": "secret", "published": false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let slug = body["slug"].as_str().unwrap().to_owned();
    assert_eq!(body["published"], false);

    // Anonymous and unrelated users see 404 (no existence leak).
    let (status, _) = c.get(&format!("/api/v1/posts/{slug}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = c
        .get(&format!("/api/v1/posts/{slug}"), Some(&other_token))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Owner can preview the draft.
    let (status, body) = c.get(&format!("/api/v1/posts/{slug}"), Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["published"], false);
}

#[tokio::test]
async fn update_and_delete_enforce_ownership() {
    let mut c = TestClient::new(new_app().await);
    let (token, _, _) = register_user(&mut c, &format!("owner_{}", uid())).await;
    let (attacker_token, _, _) = register_user(&mut c, &format!("attacker_{}", uid())).await;
    let (post_id, _) = make_post(&mut c, &token, &format!("Owned {}", uid())).await;

    let (status, _) = c
        .put(
            &format!("/api/v1/posts/{post_id}"),
            Some(&attacker_token),
            serde_json::json!({"title": "Hacked"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = c
        .delete(&format!("/api/v1/posts/{post_id}"), Some(&attacker_token))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = c
        .put(
            &format!("/api/v1/posts/{post_id}"),
            Some(&token),
            serde_json::json!({"title": "Updated Title"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["title"], "Updated Title");
    assert_eq!(body["slug"], "updated-title");

    let (status, _) = c
        .delete(&format!("/api/v1/posts/{post_id}"), Some(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = c.get(&format!("/api/v1/posts/{post_id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn moderator_role_allows_edit_without_relogin() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (owner_token, _, _) = register_user(&mut c, &format!("owner_{}", uid())).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    let (post_id, _) = make_post(&mut c, &owner_token, &format!("Post {}", uid())).await;

    // Promote with the *same* token the moderator already holds.
    let (status, _) = c
        .put(
            &format!("/api/v1/admin/users/{mod_id}/role"),
            Some(&admin_token),
            serde_json::json!({"role": "moderator"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = c
        .put(
            &format!("/api/v1/posts/{post_id}"),
            Some(&mod_token),
            serde_json::json!({"title": "Moderated Title"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["title"], "Moderated Title");
}

#[tokio::test]
async fn retitle_slug_collision_is_deduped() {
    let mut c = TestClient::new(new_app().await);
    let (token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let title_a = format!("Collision A {}", uid());
    let title_b = format!("Collision B {}", uid());
    let (_, slug_a) = make_post(&mut c, &token, &title_a).await;
    let (post_b_id, _) = make_post(&mut c, &token, &title_b).await;

    let (status, body) = c
        .put(
            &format!("/api/v1/posts/{post_b_id}"),
            Some(&token),
            serde_json::json!({"title": title_a}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_ne!(body["slug"], slug_a);
    assert!(body["slug"].as_str().unwrap().starts_with("collision-a-"));
}

#[tokio::test]
async fn list_supports_search_pagination_and_filters() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (token, _, username) = register_user(&mut c, &format!("author_{}", uid())).await;
    let tag = uid();
    for i in 0..5 {
        make_published_post(
            &mut c,
            &token,
            &admin_token,
            &format!("Tokio Deep Dive {tag} {i}"),
        )
        .await;
    }
    make_published_post(
        &mut c,
        &token,
        &admin_token,
        &format!("Unrelated Cooking {tag}"),
    )
    .await;

    // Search is case-insensitive.
    for term in ["tokio", "TOKIO", "ToKiO"] {
        let (status, body) = c.get(&format!("/api/v1/posts?search={term}"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["total"].as_i64().unwrap() >= 5, "{body}");
    }

    // Pagination.
    let (status, body) = c.get("/api/v1/posts?per_page=2&page=2", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["posts"].as_array().unwrap().len(), 2);
    assert_eq!(body["page"], 2);
    assert_eq!(body["per_page"], 2);

    // Author filter.
    let (status, body) = c
        .get(&format!("/api/v1/posts?author={username}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["total"].as_i64().unwrap() >= 6, "{body}");

    // Drafts are excluded from anonymous lists even with ?published=false
    // (visibility is viewer-scoped, the query param is ignored).
    let (status, _) = c
        .post(
            "/api/v1/posts",
            Some(&token),
            serde_json::json!({"title": format!("Hidden {}", uid()), "content": "x", "published": false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, all_body) = c.get("/api/v1/posts?published=false", None).await;
    let (status2, pub_body) = c.get("/api/v1/posts", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(status2, StatusCode::OK);
    assert_eq!(
        all_body["total"].as_i64().unwrap(),
        pub_body["total"].as_i64().unwrap(),
        "anon must not see drafts even with ?published=false: {all_body} vs {pub_body}"
    );
    // Owner sees own draft in list, moderator/admin see all.
    let (status, own_body) = c.get("/api/v1/posts?published=false", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        own_body["total"].as_i64().unwrap() > pub_body["total"].as_i64().unwrap(),
        "{own_body} vs {pub_body}"
    );
    let (status, mod_body) = c.get("/api/v1/posts", Some(&admin_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        mod_body["total"].as_i64().unwrap() >= own_body["total"].as_i64().unwrap(),
        "{mod_body} vs {own_body}"
    );
}
