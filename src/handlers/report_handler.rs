//! User reports + moderation queue.
//!
//! Any authenticated user may flag a post, comment, or account. Moderators
//! and admins triage the queue (`open` by default). Report creation is
//! throttled per user like other writes.

use crate::{
    auth::middleware::{require_role, AuthUser},
    config::Config,
    db::DbPool,
    errors::{AppError, AppResult},
    models::{report::*, user::Role},
    repositories::{audit_repo, comment_repo, post_repo, report_repo, user_repo},
    spam::SpamState,
    validators::validate_request,
};
use axum::{
    extract::{Extension, Path, Query, State},
    Json,
};
use std::time::Duration;

pub async fn create_report(
    State(pool): State<DbPool>,
    State(config): State<Config>,
    State(spam): State<SpamState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<CreateReportRequest>,
) -> AppResult<Json<ReportResponse>> {
    validate_request(&req)?;
    let kind = req.target_type.to_lowercase();
    if kind != "post" && kind != "comment" && kind != "user" {
        return Err(AppError::BadRequest(
            "Invalid target_type (expected 'post', 'comment' or 'user')".into(),
        ));
    }

    // Resolve + normalize the target (canonical ids; existence check).
    let (target_id, post_id) = match kind.as_str() {
        "post" => {
            let post = post_repo::find_by_id_or_slug(&pool, &req.target_id)
                .await?
                .ok_or_else(|| AppError::NotFound("Target post not found".into()))?;
            (post.id, None)
        }
        "comment" => {
            let post_key = req.post_id.as_deref().ok_or_else(|| {
                AppError::BadRequest("post_id is required for comment reports".into())
            })?;
            let post = post_repo::find_by_id_or_slug(&pool, post_key)
                .await?
                .ok_or_else(|| AppError::NotFound("Post not found".into()))?;
            let comment = comment_repo::find_by_id(&pool, &req.target_id, &post.id)
                .await?
                .ok_or_else(|| AppError::NotFound("Target comment not found".into()))?;
            (comment.id, Some(post.id))
        }
        _ => {
            let user = user_repo::find_by_id(&pool, &req.target_id)
                .await?
                .ok_or_else(|| AppError::NotFound("Target user not found".into()))?;
            (user.id, None)
        }
    };

    if report_repo::find_open_duplicate(&pool, &auth_user.id, &kind, &target_id)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "You already have an open report on this target".into(),
        ));
    }

    if !crate::spam::is_trusted(&pool, &config, &auth_user).await?
        && !spam.check_rate(
            &auth_user.id,
            "report",
            config.report_rate_per_hour,
            Duration::from_secs(3600),
        )
    {
        return Err(AppError::TooManyRequests);
    }

    let id = uuid::Uuid::new_v4().to_string();
    report_repo::create(
        &pool,
        &id,
        &auth_user.id,
        &kind,
        &target_id,
        post_id.as_deref(),
        &req.reason,
    )
    .await?;
    let report = report_repo::find_by_id(&pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound("Report not found".into()))?;
    Ok(Json(report))
}

pub async fn list_reports(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Query(q): Query<ReportQuery>,
) -> AppResult<Json<ReportListResponse>> {
    require_role(&auth_user, Role::Moderator)?;
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(20).clamp(1, 100);
    let ascending = match q.order.as_deref().map(str::to_lowercase).as_deref() {
        None | Some("desc") => false,
        Some("asc") => true,
        Some(_) => {
            return Err(AppError::BadRequest(
                "Invalid order (expected 'asc' or 'desc')".into(),
            ));
        }
    };
    // Queue semantics: default to open reports.
    let status = match q.status.as_deref().map(str::to_lowercase).as_deref() {
        None | Some("open") => Some("open"),
        Some("dismissed") => Some("dismissed"),
        Some("actioned") => Some("actioned"),
        Some("all") => None,
        Some(_) => {
            return Err(AppError::BadRequest(
                "Invalid status (expected 'open', 'dismissed', 'actioned' or 'all')".into(),
            ));
        }
    };
    let status_owned = status.map(str::to_owned);
    let (reports, total) =
        report_repo::list(&pool, status_owned.as_deref(), page, per_page, ascending).await?;
    Ok(Json(ReportListResponse {
        reports,
        total,
        page,
        per_page,
    }))
}

async fn transition_report(
    pool: &DbPool,
    auth_user: &AuthUser,
    id: &str,
    status: &str,
    action: &str,
) -> AppResult<Json<ReportResponse>> {
    require_role(auth_user, Role::Moderator)?;
    let report = report_repo::find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound("Report not found".into()))?;
    // Idempotent: re-dismissing/acting is a no-op success.
    if report.status != status {
        report_repo::set_status(pool, id, status).await?;
    }
    audit_repo::record(pool, &auth_user.id, action, "report", id, None).await;
    let report = report_repo::find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound("Report not found".into()))?;
    Ok(Json(report))
}

pub async fn dismiss_report(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> AppResult<Json<ReportResponse>> {
    transition_report(&pool, &auth_user, &id, "dismissed", "report.dismiss").await
}

pub async fn action_report(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> AppResult<Json<ReportResponse>> {
    transition_report(&pool, &auth_user, &id, "actioned", "report.action").await
}
