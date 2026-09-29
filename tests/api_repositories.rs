//! Integration tests: repository helpers directly
//! (batch tag fetch, slug dedup helpers, comment ordering).

mod common;

use blog_api::auth::password::hash_password;
use blog_api::repositories::{comment_repo, post_repo, tag_repo, user_repo};
use common::{new_app, uid};

async fn seed_user(pool: &blog_api::db::DbPool, username: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let hash = hash_password("SecurePassword123").unwrap();
    user_repo::create(
        pool,
        &id,
        username,
        &format!("{username}@test.local"),
        &hash,
    )
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn batch_tag_fetch_matches_individual_lookups() {
    let app = new_app().await;
    let pool = &app.pool;
    let author = seed_user(pool, &format!("author_{}", uid())).await;

    let tag_a = uuid::Uuid::new_v4().to_string();
    let tag_b = uuid::Uuid::new_v4().to_string();
    tag_repo::create(pool, &tag_a, "Alpha", "alpha")
        .await
        .unwrap();
    tag_repo::create(pool, &tag_b, "Beta", "beta")
        .await
        .unwrap();

    let post_1 = uuid::Uuid::new_v4().to_string();
    let post_2 = uuid::Uuid::new_v4().to_string();
    for (pid, title, slug) in [
        (post_1.clone(), "First", "first-post"),
        (post_2.clone(), "Second", "second-post"),
    ] {
        post_repo::create(pool, &pid, title, slug, "content", None, true, &author)
            .await
            .unwrap();
    }
    post_repo::attach_tag(pool, &post_1, &tag_a).await.unwrap();
    post_repo::attach_tag(pool, &post_1, &tag_b).await.unwrap();
    post_repo::attach_tag(pool, &post_2, &tag_b).await.unwrap();

    let batch = post_repo::get_tags_for_posts(pool, &[post_1.clone(), post_2.clone()])
        .await
        .unwrap();
    assert_eq!(batch[&post_1].len(), 2);
    assert_eq!(batch[&post_2].len(), 1);

    // Same result as the per-post lookup used for single-post responses.
    let single = post_repo::get_tags(pool, &post_1).await.unwrap();
    assert_eq!(single.len(), batch[&post_1].len());

    // Empty input returns an empty map without querying.
    let empty = post_repo::get_tags_for_posts(pool, &[]).await.unwrap();
    assert!(empty.is_empty());
}

#[tokio::test]
async fn slug_helpers_detect_collisions() {
    let app = new_app().await;
    let pool = &app.pool;
    let author = seed_user(pool, &format!("author_{}", uid())).await;

    let post_id = uuid::Uuid::new_v4().to_string();
    post_repo::create(
        pool,
        &post_id,
        "Title",
        "taken-slug",
        "c",
        None,
        true,
        &author,
    )
    .await
    .unwrap();

    assert!(post_repo::slug_exists(pool, "taken-slug").await.unwrap());
    assert!(!post_repo::slug_exists(pool, "free-slug").await.unwrap());
    // The post itself is excluded from the collision check on update.
    assert!(
        !post_repo::slug_exists_excluding(pool, "taken-slug", &post_id)
            .await
            .unwrap()
    );
    assert!(
        post_repo::slug_exists_excluding(pool, "taken-slug", "other-id")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn id_or_slug_lookup_finds_both_keys() {
    let app = new_app().await;
    let pool = &app.pool;
    let author = seed_user(pool, &format!("author_{}", uid())).await;

    let post_id = uuid::Uuid::new_v4().to_string();
    post_repo::create(
        pool,
        &post_id,
        "Lookup",
        "lookup-slug",
        "c",
        None,
        true,
        &author,
    )
    .await
    .unwrap();

    assert_eq!(
        post_repo::find_by_id_or_slug(pool, &post_id)
            .await
            .unwrap()
            .unwrap()
            .slug,
        "lookup-slug"
    );
    assert_eq!(
        post_repo::find_by_id_or_slug(pool, "lookup-slug")
            .await
            .unwrap()
            .unwrap()
            .id,
        post_id
    );
    assert!(post_repo::find_by_id_or_slug(pool, "missing")
        .await
        .unwrap()
        .is_none());

    let tag_id = uuid::Uuid::new_v4().to_string();
    tag_repo::create(pool, &tag_id, "Tagged", "tagged")
        .await
        .unwrap();
    assert!(tag_repo::find_by_id_or_slug(pool, &tag_id)
        .await
        .unwrap()
        .is_some());
    assert!(tag_repo::find_by_id_or_slug(pool, "tagged")
        .await
        .unwrap()
        .is_some());
    assert!(tag_repo::find_by_id_or_slug(pool, "missing")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn comment_listing_is_chronological() {
    let app = new_app().await;
    let pool = &app.pool;
    let author = seed_user(pool, &format!("author_{}", uid())).await;
    let post_id = uuid::Uuid::new_v4().to_string();
    post_repo::create(pool, &post_id, "T", "chrono-post", "c", None, true, &author)
        .await
        .unwrap();

    for i in 0..3 {
        comment_repo::create(
            pool,
            &uuid::Uuid::new_v4().to_string(),
            &format!("comment {i}"),
            &post_id,
            &author,
            None,
            "",
            0,
        )
        .await
        .unwrap();
    }

    let flat = comment_repo::list_for_post(pool, &post_id).await.unwrap();
    assert_eq!(flat.len(), 3);
    // Contract: rows come back ordered by (created_at, id). Same-second
    // inserts share a timestamp (SQLite `datetime('now')` has 1s resolution),
    // so assert the ordering invariant rather than insertion order.
    let mut contents: Vec<&str> = flat.iter().map(|c| c.content.as_str()).collect();
    contents.sort_unstable();
    assert_eq!(contents, ["comment 0", "comment 1", "comment 2"]);
    let keys: Vec<(&str, &str)> = flat
        .iter()
        .map(|c| (c.created_at.as_str(), c.id.as_str()))
        .collect();
    assert!(
        keys.windows(2).all(|w| w[0] <= w[1]),
        "rows must be ordered by (created_at, id): {keys:?}"
    );
}
