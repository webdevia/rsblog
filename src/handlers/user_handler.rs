use crate::{
    auth::middleware::{require_role, AuthUser},
    db::DbPool,
    errors::{AppError, AppResult},
    models::user::{AdminUsersQuery, Role, UpdateUserRoleRequest, UserResponse},
    repositories::user_repo,
};
use axum::{
    extract::{Extension, Path, Query, State},
    Json,
};

#[derive(Debug, serde::Deserialize)]
pub struct DeleteUserQuery {
    /// `soft` (default): anonymize + re-attribute content to ghost author.
    /// `hard`: cascade purge of user + posts + comments.
    pub mode: Option<String>,
}

/// Moderators manage users but never touch admins; admins manage everyone
/// except themselves. Returns the target on success.
async fn resolve_managed_user(
    pool: &DbPool,
    viewer: &AuthUser,
    user_id: &str,
) -> AppResult<crate::models::user::User> {
    require_role(viewer, Role::Moderator)?;
    if viewer.id == user_id {
        return Err(AppError::BadRequest("Cannot target yourself".into()));
    }
    let target = user_repo::find_by_id(pool, user_id)
        .await?
        .ok_or(AppError::NotFound("User not found".into()))?;
    if viewer.role == Role::Moderator && Role::from_str(&target.role) == Role::Admin {
        // Mask admin accounts from moderators (no existence leak).
        return Err(AppError::NotFound("User not found".into()));
    }
    Ok(target)
}

pub async fn get_me(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
) -> AppResult<Json<UserResponse>> {
    let user = user_repo::find_by_id(&pool, &auth_user.id)
        .await?
        .ok_or(AppError::NotFound("User not found".into()))?;
    Ok(Json(user.into()))
}

pub async fn list_users(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Query(q): Query<AdminUsersQuery>,
) -> AppResult<Json<crate::models::user::UserListResponse>> {
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
    let exclude_admins = auth_user.role == Role::Moderator;
    let (rows, total) =
        user_repo::list_paginated(&pool, page, per_page, ascending, exclude_admins).await?;
    // Moderators don't see admin accounts (no roster leak); admins see all.
    // (Enforced in SQL so `total` matches the visible set.)
    let users: Vec<UserResponse> = rows.into_iter().map(Into::into).collect();
    Ok(Json(crate::models::user::UserListResponse {
        users,
        total,
        page,
        per_page,
    }))
}

pub async fn get_user(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(user_id): Path<String>,
) -> AppResult<Json<UserResponse>> {
    let target = resolve_managed_user(&pool, &auth_user, &user_id).await?;
    Ok(Json(target.into()))
}

pub async fn update_user_role(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateUserRoleRequest>,
) -> AppResult<Json<UserResponse>> {
    require_role(&auth_user, Role::Moderator)?;
    if auth_user.id == user_id {
        return Err(AppError::BadRequest("Cannot change your own role".into()));
    }
    let valid = ["admin", "moderator", "user"];
    if !valid.contains(&req.role.as_str()) {
        return Err(AppError::BadRequest("Invalid role".into()));
    }
    let target = user_repo::find_by_id(&pool, &user_id)
        .await?
        .ok_or(AppError::NotFound("User not found".into()))?;
    let current = Role::from_str(&target.role);
    let requested = Role::from_str(&req.role);
    // Moderators may only promote user -> moderator. Demotions, any grant of
    // admin, and anything targeting admins are admin-only.
    if auth_user.role == Role::Moderator && !(current == Role::User && requested == Role::Moderator)
    {
        return Err(AppError::Forbidden);
    }
    if auth_user.role == Role::Moderator && current == Role::Admin {
        return Err(AppError::Forbidden);
    }
    if user_repo::update_role(&pool, &user_id, &req.role).await? == 0 {
        return Err(AppError::NotFound("User not found".into()));
    }
    crate::repositories::audit_repo::record(
        &pool,
        &auth_user.id,
        "user.role",
        "user",
        &user_id,
        Some(&format!("{} -> {}", current.as_str(), req.role)),
    )
    .await;
    let user = user_repo::find_by_id(&pool, &user_id)
        .await?
        .ok_or(AppError::NotFound("User not found".into()))?;
    Ok(Json(user.into()))
}

async fn resolve_ban_target(
    pool: &DbPool,
    viewer: &AuthUser,
    user_id: &str,
) -> AppResult<crate::models::user::User> {
    let target = resolve_managed_user(pool, viewer, user_id).await?;
    // Moderators may ban/unban only `user`-role accounts.
    if viewer.role == Role::Moderator && Role::from_str(&target.role) != Role::User {
        return Err(AppError::Forbidden);
    }
    Ok(target)
}

pub async fn deactivate_user(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(user_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let target = resolve_ban_target(&pool, &auth_user, &user_id).await?;
    if user_repo::deactivate(&pool, &target.id).await? == 0 {
        return Err(AppError::NotFound("User not found".into()));
    }
    crate::repositories::audit_repo::record(
        &pool,
        &auth_user.id,
        "user.deactivate",
        "user",
        &target.id,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({"message": "User deactivated"})))
}

pub async fn activate_user(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(user_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let target = resolve_ban_target(&pool, &auth_user, &user_id).await?;
    if user_repo::activate(&pool, &target.id).await? == 0 {
        return Err(AppError::NotFound("User not found".into()));
    }
    crate::repositories::audit_repo::record(
        &pool,
        &auth_user.id,
        "user.activate",
        "user",
        &target.id,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({"message": "User activated"})))
}

pub async fn delete_user(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(user_id): Path<String>,
    Query(q): Query<DeleteUserQuery>,
) -> AppResult<Json<serde_json::Value>> {
    // Deletion is admin-only; moderators always get 403 (even for users).
    require_role(&auth_user, Role::Admin)?;
    if auth_user.id == user_id {
        return Err(AppError::BadRequest("Cannot delete yourself".into()));
    }
    let mode = q.mode.as_deref().unwrap_or("soft");
    if mode != "soft" && mode != "hard" {
        return Err(AppError::BadRequest(
            "Invalid mode (expected 'soft' or 'hard')".into(),
        ));
    }
    let target = user_repo::find_by_id(&pool, &user_id)
        .await?
        .ok_or(AppError::NotFound("User not found".into()))?;
    if Role::from_str(&target.role) == Role::Admin && user_repo::count_admins(&pool).await? <= 1 {
        return Err(AppError::BadRequest("Cannot delete the last admin".into()));
    }

    if mode == "hard" {
        let posts = user_repo::count_posts_by_author(&pool, &target.id).await?;
        let comments = user_repo::count_comments_by_author(&pool, &target.id).await?;
        if user_repo::hard_delete(&pool, &target.id).await? == 0 {
            return Err(AppError::NotFound("User not found".into()));
        }
        crate::repositories::audit_repo::record(
            &pool,
            &auth_user.id,
            "user.hard_delete",
            "user",
            &target.id,
            Some(&format!("posts={posts}, comments={comments}")),
        )
        .await;
        return Ok(Json(serde_json::json!({
            "message": "User hard-deleted",
            "mode": "hard",
            "posts_deleted": posts,
            "comments_deleted": comments,
        })));
    }

    user_repo::ensure_ghost_user(&pool).await?;
    let suffix = &target.id[..8.min(target.id.len())];
    if user_repo::anonymize(&pool, &target.id, suffix).await? == 0 {
        return Err(AppError::NotFound("User not found".into()));
    }
    let (posts, comments) = user_repo::reassign_content(&pool, &target.id).await?;
    crate::repositories::audit_repo::record(
        &pool,
        &auth_user.id,
        "user.soft_delete",
        "user",
        &target.id,
        Some(&format!("posts={posts}, comments={comments}")),
    )
    .await;
    Ok(Json(serde_json::json!({
        "message": "User soft-deleted",
        "mode": "soft",
        "posts_reassigned": posts,
        "comments_reassigned": comments,
    })))
}

pub async fn list_audit(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Query(q): Query<crate::models::audit::AuditQuery>,
) -> AppResult<Json<crate::models::audit::AuditListResponse>> {
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
    // Moderators see only their own actions; admins may filter by actor/action.
    let actor = if auth_user.role == Role::Moderator {
        Some(auth_user.id.clone())
    } else {
        q.actor.clone()
    };
    let (entries, total) = crate::repositories::audit_repo::list(
        &pool,
        actor.as_deref(),
        q.action.as_deref(),
        page,
        per_page,
        ascending,
    )
    .await?;
    Ok(Json(crate::models::audit::AuditListResponse {
        entries,
        total,
        page,
        per_page,
    }))
}
