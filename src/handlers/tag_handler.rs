use crate::{
    auth::middleware::{require_role, AuthUser},
    db::DbPool,
    errors::{AppError, AppResult},
    models::{tag::*, user::Role},
    repositories::tag_repo,
    validators::{slugify, validate_request},
};
use axum::{
    extract::{Extension, Path, State},
    Json,
};

pub async fn create_tag(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<CreateTagRequest>,
) -> AppResult<Json<Tag>> {
    require_role(&auth_user, Role::Moderator)?;
    validate_request(&req)?;
    let id = uuid::Uuid::new_v4().to_string();
    let slug = slugify(&req.name);
    let tag = tag_repo::create(&pool, &id, &req.name, &slug).await?;
    Ok(Json(tag))
}

pub async fn list_tags(State(pool): State<DbPool>) -> AppResult<Json<Vec<TagWithCount>>> {
    Ok(Json(tag_repo::list_with_counts(&pool).await?))
}

pub async fn get_tag(State(pool): State<DbPool>, Path(id): Path<String>) -> AppResult<Json<Tag>> {
    // Try looking up by slug first
    if let Some(tag) = tag_repo::find_by_slug(&pool, &id).await? {
        Ok(Json(tag))
    } else {
        // Fallback to checking by ID
        let tag = tag_repo::find_by_id(&pool, &id)
            .await
            .map_err(|_| AppError::NotFound("Tag not found".into()))?;
        Ok(Json(tag))
    }
}

pub async fn update_tag(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTagRequest>,
) -> AppResult<Json<Tag>> {
    require_role(&auth_user, Role::Moderator)?;
    validate_request(&req)?;
    let slug = slugify(&req.name);
    if tag_repo::update(&pool, &id, &req.name, &slug).await? == 0 {
        return Err(AppError::NotFound("Tag not found".into()));
    }
    Ok(Json(tag_repo::find_by_id(&pool, &id).await?))
}

pub async fn delete_tag(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    require_role(&auth_user, Role::Admin)?;
    if tag_repo::delete(&pool, &id).await? == 0 {
        return Err(AppError::NotFound("Tag not found".into()));
    }
    Ok(Json(serde_json::json!({"message": "Tag deleted"})))
}
