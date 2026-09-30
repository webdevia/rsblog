use crate::{
    auth::middleware::{require_role, AuthUser},
    db::DbPool,
    errors::{AppError, AppResult},
    models::user::{Role, UpdateUserRoleRequest, UserResponse},
    repositories::user_repo,
};
use axum::{
    extract::{Extension, Path, State},
    Json,
};

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
) -> AppResult<Json<Vec<UserResponse>>> {
    require_role(&auth_user, Role::Admin)?;
    let users: Vec<UserResponse> = user_repo::list_all(&pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(users))
}

pub async fn update_user_role(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateUserRoleRequest>,
) -> AppResult<Json<UserResponse>> {
    require_role(&auth_user, Role::Admin)?;
    if auth_user.id == user_id {
        return Err(AppError::BadRequest("Cannot change your own role".into()));
    }
    let valid = ["admin", "moderator", "user"];
    if !valid.contains(&req.role.as_str()) {
        return Err(AppError::BadRequest("Invalid role".into()));
    }
    if user_repo::update_role(&pool, &user_id, &req.role).await? == 0 {
        return Err(AppError::NotFound("User not found".into()));
    }
    let user = user_repo::find_by_id(&pool, &user_id)
        .await?
        .ok_or(AppError::NotFound("User not found".into()))?;
    Ok(Json(user.into()))
}

pub async fn deactivate_user(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(user_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    require_role(&auth_user, Role::Admin)?;
    if auth_user.id == user_id {
        return Err(AppError::BadRequest("Cannot deactivate yourself".into()));
    }
    if user_repo::deactivate(&pool, &user_id).await? == 0 {
        return Err(AppError::NotFound("User not found".into()));
    }
    Ok(Json(serde_json::json!({"message": "User deactivated"})))
}

pub async fn activate_user(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(user_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    require_role(&auth_user, Role::Admin)?;
    if auth_user.id == user_id {
        return Err(AppError::BadRequest("Cannot activate yourself".into()));
    }
    if user_repo::activate(&pool, &user_id).await? == 0 {
        return Err(AppError::NotFound("User not found".into()));
    }
    Ok(Json(serde_json::json!({"message": "User activated"})))
}
