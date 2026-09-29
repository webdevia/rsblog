use crate::{
    auth::middleware::AuthUser,
    db::DbPool,
    errors::{AppError, AppResult},
    models::{post::*, user::Role},
    repositories::post_repo,
    validators::{slugify, validate_request},
};
use axum::{
    extract::{Extension, Path, Query, State},
    Json,
};

pub async fn create_post(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<CreatePostRequest>,
) -> AppResult<Json<PostResponse>> {
    validate_request(&req)?;
    let id = uuid::Uuid::new_v4().to_string();
    let slug = slugify(&req.title);
    let final_slug = if post_repo::slug_exists(&pool, &slug).await? {
        format!("{}-{}", slug, &id[..8])
    } else {
        slug
    };

    post_repo::create(
        &pool,
        &id,
        &req.title,
        &final_slug,
        &req.content,
        req.excerpt.as_deref(),
        req.published,
        &auth_user.id,
    )
    .await?;

    for tag_id in &req.tag_ids {
        post_repo::attach_tag(&pool, &id, tag_id).await?;
    }

    build_post_response(&pool, &id).await
}

pub async fn list_posts(
    State(pool): State<DbPool>,
    Query(q): Query<PostQuery>,
) -> AppResult<Json<PostListResponse>> {
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(20).clamp(1, 100);
    let published_only = q.published.unwrap_or(true);

    let (rows, total) = post_repo::list(
        &pool,
        published_only,
        q.author.as_deref(),
        q.tag.as_deref(),
        q.search.as_deref(),
        page,
        per_page,
    )
    .await?;

    let mut posts = Vec::with_capacity(rows.len());
    for row in rows {
        let tags = post_repo::get_tags(&pool, &row.id).await?;
        posts.push(PostSummary {
            id: row.id,
            title: row.title,
            slug: row.slug,
            excerpt: row.excerpt,
            published: row.published,
            author: PostAuthor {
                id: row.author_id,
                username: row.author_username,
            },
            tags,
            comment_count: row.comment_count,
            created_at: row.created_at,
        });
    }

    Ok(Json(PostListResponse {
        posts,
        total,
        page,
        per_page,
    }))
}

pub async fn get_post(
    State(pool): State<DbPool>,
    Path(id): Path<String>, // Unified matching parameter
) -> AppResult<Json<PostResponse>> {
    // Looks up via slug (for friendly URLs)
    let post = post_repo::find_by_slug(&pool, &id, true)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;
    build_post_response_from_post(&pool, &post).await
}

pub async fn update_post(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePostRequest>,
) -> AppResult<Json<PostResponse>> {
    validate_request(&req)?;
    let post = post_repo::find_by_id(&pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;

    if post.author_id != auth_user.id && !auth_user.role.has_permission(&Role::Moderator) {
        return Err(AppError::Forbidden);
    }

    let title = req.title.unwrap_or(post.title);
    let content = req.content.unwrap_or(post.content);
    let excerpt = req.excerpt.or(post.excerpt);
    let published = req.published.unwrap_or(post.published);
    let new_slug = slugify(&title);

    post_repo::update(
        &pool,
        &id,
        &title,
        &new_slug,
        &content,
        excerpt.as_deref(),
        published,
    )
    .await?;

    if let Some(tag_ids) = req.tag_ids {
        post_repo::clear_tags(&pool, &id).await?;
        for tid in &tag_ids {
            post_repo::attach_tag(&pool, &id, tid).await?;
        }
    }

    build_post_response(&pool, &id).await
}

pub async fn delete_post(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let post = post_repo::find_by_id(&pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;

    if post.author_id != auth_user.id && !auth_user.role.has_permission(&Role::Moderator) {
        return Err(AppError::Forbidden);
    }

    post_repo::delete(&pool, &id).await?;
    Ok(Json(serde_json::json!({"message": "Post deleted"})))
}

// --- Helper Functions ---

async fn build_post_response(pool: &DbPool, id: &str) -> AppResult<Json<PostResponse>> {
    let post = post_repo::find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;
    build_post_response_from_post(pool, &post).await
}

async fn build_post_response_from_post(
    pool: &DbPool,
    post: &Post,
) -> AppResult<Json<PostResponse>> {
    let author = post_repo::get_author(pool, &post.author_id).await?;
    let tags = post_repo::get_tags(pool, &post.id).await?;
    let comment_count = post_repo::comment_count(pool, &post.id).await?;

    Ok(Json(PostResponse {
        id: post.id.clone(),
        title: post.title.clone(),
        slug: post.slug.clone(),
        content: post.content.clone(),
        excerpt: post.excerpt.clone(),
        published: post.published,
        author,
        tags,
        comment_count,
        created_at: post.created_at.clone(),
        updated_at: post.updated_at.clone(),
    }))
}
