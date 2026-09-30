use crate::{
    auth::middleware::{try_auth_from_headers, AuthUser},
    config::Config,
    db::DbPool,
    errors::{AppError, AppResult},
    models::{post::*, user::Role},
    repositories::{post_repo, user_repo},
    spam::SpamState,
    validators::{slugify, validate_request},
};
use axum::{
    extract::{Extension, Path, Query, State},
    http::HeaderMap,
    Json,
};

pub async fn create_post(
    State(pool): State<DbPool>,
    State(config): State<Config>,
    State(spam): State<SpamState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<CreatePostRequest>,
) -> AppResult<Json<PostResponse>> {
    validate_request(&req)?;
    crate::spam::check_post_write(
        &pool,
        &spam,
        &config,
        &auth_user,
        &req.title,
        &req.content,
        req.excerpt.as_deref(),
    )
    .await?;
    let id = uuid::Uuid::new_v4().to_string();
    let slug = slugify(&req.title);
    let final_slug = if post_repo::slug_exists(&pool, &slug).await? {
        format!("{}-{}", slug, &id[..8])
    } else {
        slug
    };

    // Users always create drafts; moderators/admins may publish in one shot.
    let published = req.published && auth_user.role.has_permission(&Role::Moderator);

    post_repo::create(
        &pool,
        &id,
        &req.title,
        &final_slug,
        &req.content,
        req.excerpt.as_deref(),
        published,
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
    State(config): State<Config>,
    headers: HeaderMap,
    Query(q): Query<PostQuery>,
) -> AppResult<Json<PostListResponse>> {
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

    // Viewer-scoped visibility: anon -> published only, user -> published + own,
    // moderator/admin -> everything. `?published=` is ignored (kept for compat).
    let viewer = try_auth_from_headers(&pool, &config.jwt_secret, &headers).await;
    let can_see_all = viewer
        .as_ref()
        .is_some_and(|u| u.role.has_permission(&Role::Moderator));
    let viewer_id = viewer.as_ref().map(|u| u.id.as_str());

    let (rows, total) = post_repo::list(
        &pool,
        viewer_id,
        can_see_all,
        q.author.as_deref(),
        q.tag.as_deref(),
        q.search.as_deref(),
        page,
        per_page,
        ascending,
    )
    .await?;

    let mut posts = Vec::with_capacity(rows.len());
    let post_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let tags_map = post_repo::get_tags_for_posts(&pool, &post_ids).await?;
    for row in rows {
        let tags = tags_map.get(&row.id).cloned().unwrap_or_default();
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
    State(config): State<Config>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<PostResponse>> {
    let post = post_repo::find_by_id_or_slug(&pool, &key)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;

    if post.published {
        return build_post_response_from_post(&pool, &post).await;
    }

    // Draft: require owner or moderator/admin (optional auth, 404-masked).
    let viewer = try_auth_from_headers(&pool, &config.jwt_secret, &headers).await;
    match viewer {
        Some(u) if u.id == post.author_id || u.role.has_permission(&Role::Moderator) => {
            build_post_response_from_post(&pool, &post).await
        }
        _ => Err(AppError::NotFound("Post not found".into())),
    }
}

async fn ensure_can_moderate_post(
    pool: &DbPool,
    viewer: &AuthUser,
    post_author_id: &str,
) -> AppResult<()> {
    // Moderators cannot touch admin-owned posts; admins can touch everything.
    if viewer.role == Role::Moderator {
        if let Some(author) = user_repo::find_by_id(pool, post_author_id).await? {
            if Role::from_str(&author.role) == Role::Admin {
                return Err(AppError::Forbidden);
            }
        }
    }
    Ok(())
}

pub async fn update_post(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(key): Path<String>,
    Json(req): Json<UpdatePostRequest>,
) -> AppResult<Json<PostResponse>> {
    validate_request(&req)?;
    let post = post_repo::find_by_id_or_slug(&pool, &key)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;
    let post_id = post.id.clone();

    if post.author_id != auth_user.id && !auth_user.role.has_permission(&Role::Moderator) {
        return Err(AppError::Forbidden);
    }
    ensure_can_moderate_post(&pool, &auth_user, &post.author_id).await?;

    let title = req.title.unwrap_or(post.title);
    let content = req.content.unwrap_or(post.content);
    let excerpt = req.excerpt.or(post.excerpt);
    let mut new_slug = slugify(&title);
    // Avoid UNIQUE violation when retitling to an existing slug.
    if new_slug != post.slug && post_repo::slug_exists_excluding(&pool, &new_slug, &post_id).await?
    {
        new_slug = format!("{}-{}", new_slug, &post_id[..8.min(post_id.len())]);
    }

    post_repo::update_content(
        &pool,
        &post_id,
        &title,
        &new_slug,
        &content,
        excerpt.as_deref(),
    )
    .await?;

    if let Some(tag_ids) = req.tag_ids {
        post_repo::clear_tags(&pool, &post_id).await?;
        for tid in &tag_ids {
            post_repo::attach_tag(&pool, &post_id, tid).await?;
        }
    }

    build_post_response(&pool, &post_id).await
}

pub async fn delete_post(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(key): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let post = post_repo::find_by_id_or_slug(&pool, &key)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;

    if post.author_id != auth_user.id && !auth_user.role.has_permission(&Role::Moderator) {
        return Err(AppError::Forbidden);
    }
    ensure_can_moderate_post(&pool, &auth_user, &post.author_id).await?;

    post_repo::delete(&pool, &post.id).await?;
    Ok(Json(serde_json::json!({"message": "Post deleted"})))
}

pub async fn publish_post(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(key): Path<String>,
) -> AppResult<Json<PostResponse>> {
    if !auth_user.role.has_permission(&Role::Moderator) {
        return Err(AppError::Forbidden);
    }
    let post = post_repo::find_by_id_or_slug(&pool, &key)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;
    ensure_can_moderate_post(&pool, &auth_user, &post.author_id).await?;
    post_repo::set_published(&pool, &post.id, true).await?;
    build_post_response(&pool, &post.id).await
}

pub async fn unpublish_post(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(key): Path<String>,
) -> AppResult<Json<PostResponse>> {
    if !auth_user.role.has_permission(&Role::Moderator) {
        return Err(AppError::Forbidden);
    }
    let post = post_repo::find_by_id_or_slug(&pool, &key)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;
    ensure_can_moderate_post(&pool, &auth_user, &post.author_id).await?;
    post_repo::set_published(&pool, &post.id, false).await?;
    build_post_response(&pool, &post.id).await
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
