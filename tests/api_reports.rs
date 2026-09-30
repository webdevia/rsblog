//! Integration tests: user reports, moderation queue, and audit trail.

mod common;

use axum::http::StatusCode;
use common::{
    make_admin, make_moderator, make_published_post, new_app, new_strict_app, register_user, uid,
    TestClient,
};

async fn report(
    c: &mut TestClient,
    token: Option<&str>,
    payload: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    c.post("/api/v1/reports", token, payload).await
}

#[tokio::test]
async fn report_post_comment_and_user() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (reporter, _, reporter_name) = register_user(&mut c, &format!("rep_{}", uid())).await;
    let (author_token, author_id, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let (post_id, _) = make_published_post(
        &mut c,
        &author_token,
        &admin_token,
        &format!("Flagged {}", uid()),
    )
    .await;

    // Comment to flag.
    let (status, body) = c
        .post(
            &format!("/api/v1/posts/{post_id}/comments"),
            Some(&author_token),
            serde_json::json!({"content": "spammy comment"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let comment_id = body["id"].as_str().unwrap().to_owned();

    for payload in [
        serde_json::json!({"target_type": "post", "target_id": post_id, "reason": "spam post"}),
        serde_json::json!({"target_type": "comment", "target_id": comment_id, "post_id": post_id, "reason": "spam comment"}),
        serde_json::json!({"target_type": "user", "target_id": author_id, "reason": "spammer"}),
    ] {
        let (status, body) = report(&mut c, Some(&reporter), payload).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "open");
        assert_eq!(body["reporter"]["username"], reporter_name);
    }

    // Anonymous cannot report.
    let (status, _) = report(
        &mut c,
        None,
        serde_json::json!({"target_type": "post", "target_id": post_id, "reason": "x"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn report_validation_and_duplicates() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (reporter, _, _) = register_user(&mut c, &format!("rep_{}", uid())).await;
    let (author_token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let (post_id, _) = make_published_post(
        &mut c,
        &author_token,
        &admin_token,
        &format!("Flagged {}", uid()),
    )
    .await;

    // Bad kind.
    let (status, _) = report(
        &mut c,
        Some(&reporter),
        serde_json::json!({"target_type": "tag", "target_id": post_id, "reason": "x"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Missing targets.
    let (status, _) = report(
        &mut c,
        Some(&reporter),
        serde_json::json!({"target_type": "post", "target_id": "nope", "reason": "x"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = report(
        &mut c,
        Some(&reporter),
        serde_json::json!({"target_type": "comment", "target_id": "nope", "reason": "x"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST); // post_id required
    let (status, _) = report(
        &mut c,
        Some(&reporter),
        serde_json::json!({"target_type": "user", "target_id": "nope", "reason": "x"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Empty reason rejected.
    let (status, _) = report(
        &mut c,
        Some(&reporter),
        serde_json::json!({"target_type": "post", "target_id": post_id, "reason": ""}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Duplicate open report by same reporter -> 409; others unaffected.
    let payload =
        serde_json::json!({"target_type": "post", "target_id": post_id, "reason": "spam"});
    let (status, _) = report(&mut c, Some(&reporter), payload.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = report(&mut c, Some(&reporter), payload).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (other, _, _) = register_user(&mut c, &format!("other_{}", uid())).await;
    let (status, _) = report(
        &mut c,
        Some(&other),
        serde_json::json!({"target_type": "post", "target_id": post_id, "reason": "spam too"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn moderation_queue_rbac_filter_and_transitions() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    make_moderator(&mut c, &admin_token, &mod_id).await;
    let (reporter, _, _) = register_user(&mut c, &format!("rep_{}", uid())).await;
    let (user_token, user_id, _) = register_user(&mut c, &format!("spammer_{}", uid())).await;

    let (status, body) = report(
        &mut c,
        Some(&reporter),
        serde_json::json!({"target_type": "user", "target_id": user_id, "reason": "spam account"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let report_id = body["id"].as_str().unwrap().to_owned();

    // Plain users cannot see the queue.
    let (status, _) = c.get("/api/v1/moderation/reports", Some(&user_token)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = c.get("/api/v1/moderation/reports", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Moderator sees the open queue with envelope.
    let (status, body) = c.get("/api/v1/moderation/reports", Some(&mod_token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1);
    assert_eq!(body["reports"][0]["id"], report_id);

    // Dismiss (idempotent) then action.
    let (status, body) = c
        .post(
            &format!("/api/v1/moderation/reports/{report_id}/dismiss"),
            Some(&mod_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "dismissed");
    let (status, _) = c
        .post(
            &format!("/api/v1/moderation/reports/{report_id}/dismiss"),
            Some(&mod_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Dismissed no longer in the default (open) queue, visible via filter.
    let (status, body) = c.get("/api/v1/moderation/reports", Some(&mod_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 0);
    let (status, body) = c
        .get(
            "/api/v1/moderation/reports?status=dismissed",
            Some(&mod_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1);

    let (status, body) = c
        .post(
            &format!("/api/v1/moderation/reports/{report_id}/action"),
            Some(&admin_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "actioned");

    // Unknown report -> 404; bad status filter -> 400.
    let (status, _) = c
        .post(
            "/api/v1/moderation/reports/nope/dismiss",
            Some(&mod_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = c
        .get("/api/v1/moderation/reports?status=bogus", Some(&mod_token))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn report_creation_throttled() {
    let mut c = TestClient::new(new_strict_app().await);
    // Strict app: 3 reports/hour.
    let (reporter, _, _) = register_user(&mut c, &format!("rep_{}", uid())).await;
    let mut targets = Vec::new();
    for i in 0..3 {
        let (_, uid_, _) = register_user(&mut c, &format!("t{i}_{}", uid())).await;
        targets.push(uid_);
    }
    for (i, t) in targets.iter().enumerate() {
        let (status, body) = report(
            &mut c,
            Some(&reporter),
            serde_json::json!({"target_type": "user", "target_id": t, "reason": format!("spam {i}")}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (_, extra, _) = register_user(&mut c, &format!("extra_{}", uid())).await;
    let (status, body) = report(
        &mut c,
        Some(&reporter),
        serde_json::json!({"target_type": "user", "target_id": extra, "reason": "one more"}),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
}

#[tokio::test]
async fn audit_trail_records_moderation_and_scopes_by_role() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (mod_token, mod_id, mod_name) = register_user(&mut c, &format!("mod_{}", uid())).await;
    make_moderator(&mut c, &admin_token, &mod_id).await;
    let (author_token, author_id, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let (user_token, _, _) = register_user(&mut c, &format!("user_{}", uid())).await;

    // Actions that must leave audit entries.
    let (post_id, _) = make_published_post(
        &mut c,
        &author_token,
        &admin_token,
        &format!("Audited {}", uid()),
    )
    .await;
    let (status, _) = c
        .post(
            &format!("/api/v1/posts/{post_id}/unpublish"),
            Some(&mod_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = c
        .post(
            &format!("/api/v1/admin/users/{author_id}/deactivate"),
            Some(&admin_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Plain users cannot read the trail.
    let (status, _) = c.get("/api/v1/admin/audit", Some(&user_token)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admin sees everything, incl. filters.
    let (status, body) = c
        .get("/api/v1/admin/audit?per_page=100", Some(&admin_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let actions: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["action"].as_str())
        .collect();
    assert!(actions.contains(&"post.unpublish"), "{actions:?}");
    assert!(actions.contains(&"user.deactivate"), "{actions:?}");
    assert!(actions.contains(&"user.role"), "{actions:?}"); // the promotion above
    let (status, body) = c
        .get(
            "/api/v1/admin/audit?action=post.unpublish",
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["total"].as_i64().unwrap() >= 1);
    assert!(body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["action"] == "post.unpublish"));

    // Moderators see only their own actions (?actor is forced to self).
    let (status, body) = c
        .get(
            "/api/v1/admin/audit?per_page=100&actor=someone-else",
            Some(&mod_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let actors: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["actor"]["username"].as_str())
        .collect();
    assert!(!actors.is_empty());
    assert!(actors.iter().all(|a| *a == mod_name), "{actors:?}");
}

#[tokio::test]
async fn audit_survives_hard_delete_of_actor() {
    let mut c = TestClient::new(new_app().await);
    let (admin_token, _) = make_admin(&mut c, &format!("root_{}", uid())).await;
    let (mod_token, mod_id, _) = register_user(&mut c, &format!("mod_{}", uid())).await;
    make_moderator(&mut c, &admin_token, &mod_id).await;
    let (author_token, _, _) = register_user(&mut c, &format!("author_{}", uid())).await;
    let (post_id, _) = make_published_post(
        &mut c,
        &author_token,
        &admin_token,
        &format!("Audited {}", uid()),
    )
    .await;
    let (status, _) = c
        .post(
            &format!("/api/v1/posts/{post_id}/unpublish"),
            Some(&mod_token),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Admin hard-deletes the moderator: their audit rows survive, actor nulled.
    let (status, _) = c
        .delete(
            &format!("/api/v1/admin/users/{mod_id}?mode=hard"),
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = c
        .get(
            "/api/v1/admin/audit?action=post.unpublish",
            Some(&admin_token),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["total"].as_i64().unwrap() >= 1);
    assert_eq!(body["entries"][0]["actor"]["id"], mod_id);
    assert!(body["entries"][0]["actor"]["username"].is_null());
}
