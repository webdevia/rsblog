use crate::{
    auth::middleware::{try_auth_from_headers, AuthUser},
    config::Config,
    db::DbPool,
    errors::{AppError, AppResult},
    models::{comment::*, user::Role},
    repositories::{comment_repo, post_repo, user_repo},
    spam::SpamState,
    validators::validate_request,
};
use axum::{
    extract::{Extension, Path, Query, State},
    http::HeaderMap,
    Json,
};
use std::collections::HashMap;

pub async fn create_comment(
    State(pool): State<DbPool>,
    State(config): State<Config>,
    State(spam): State<SpamState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(post_key): Path<String>,
    Json(req): Json<CreateCommentRequest>,
) -> AppResult<Json<CommentTreeNode>> {
    validate_request(&req)?;
    crate::spam::check_comment_write(&pool, &spam, &config, &auth_user, &req.content).await?;

    let post = post_repo::find_by_id_or_slug(&pool, &post_key)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;
    if !post.published {
        return Err(AppError::NotFound("Post not found".into()));
    }
    let post_id = post.id;

    let id = uuid::Uuid::new_v4().to_string();
    let mut path = String::new();
    let mut depth = 0;

    if let Some(ref pid) = req.parent_id {
        let parent = comment_repo::find_by_id(&pool, pid, &post_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Parent comment not found".into()))?;
        if parent.depth >= 10 {
            return Err(AppError::BadRequest("Max nesting depth reached".into()));
        }
        path = if parent.path.is_empty() {
            parent.id.clone()
        } else {
            format!("{}/{}", parent.path, parent.id)
        };
        depth = parent.depth + 1;
    }

    comment_repo::create(
        &pool,
        &id,
        &req.content,
        &post_id,
        &auth_user.id,
        req.parent_id.as_deref(),
        &path,
        depth,
    )
    .await?;

    let c = comment_repo::find_flat(&pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound("Comment not found".into()))?;

    Ok(Json(CommentTreeNode {
        id: c.id,
        content: c.content,
        author: CommentAuthor {
            id: c.author_id,
            username: c.author_username,
        },
        parent_id: c.parent_id,
        depth: c.depth,
        is_deleted: c.is_deleted,
        created_at: c.created_at,
        children: vec![],
    }))
}

pub async fn get_comments_tree(
    State(pool): State<DbPool>,
    State(config): State<Config>,
    headers: HeaderMap,
    Path(post_key): Path<String>,
) -> AppResult<Json<Vec<CommentTreeNode>>> {
    let post = post_repo::find_by_id_or_slug(&pool, &post_key)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;
    // Mirror post visibility: drafts visible only to owner/moderator/admin.
    if !post.published {
        let viewer = try_auth_from_headers(&pool, &config.jwt_secret, &headers).await;
        match viewer {
            Some(u) if u.id == post.author_id || u.role.has_permission(&Role::Moderator) => {}
            _ => return Err(AppError::NotFound("Post not found".into())),
        }
    }
    let flat = comment_repo::list_for_post(&pool, &post.id).await?;
    Ok(Json(build_comment_tree(flat)))
}

async fn ensure_can_moderate_comment(
    pool: &DbPool,
    viewer: &AuthUser,
    comment_author_id: &str,
) -> AppResult<()> {
    if viewer.role == Role::Moderator {
        if let Some(author) = user_repo::find_by_id(pool, comment_author_id).await? {
            if Role::from_str(&author.role) == Role::Admin {
                return Err(AppError::Forbidden);
            }
        }
    }
    Ok(())
}

/// Shared ownership + mod-guard check for comment mutations.
async fn authorize_comment_mutation(
    pool: &DbPool,
    viewer: &AuthUser,
    comment_author_id: &str,
) -> AppResult<()> {
    if comment_author_id != viewer.id && !viewer.role.has_permission(&Role::Moderator) {
        return Err(AppError::Forbidden);
    }
    ensure_can_moderate_comment(pool, viewer, comment_author_id).await
}

async fn apply_comment_delete(pool: &DbPool, comment_id: &str) -> AppResult<()> {
    if comment_repo::has_children(pool, comment_id).await? {
        comment_repo::soft_delete(pool, comment_id).await?;
    } else {
        comment_repo::hard_delete(pool, comment_id).await?;
    }
    Ok(())
}

pub async fn update_comment(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path((post_key, comment_id)): Path<(String, String)>,
    Json(req): Json<UpdateCommentRequest>,
) -> AppResult<Json<serde_json::Value>> {
    validate_request(&req)?;
    let post = post_repo::find_by_id_or_slug(&pool, &post_key)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;
    let c = comment_repo::find_by_id(&pool, &comment_id, &post.id)
        .await?
        .ok_or_else(|| AppError::NotFound("Comment not found".into()))?;
    authorize_comment_mutation(&pool, &auth_user, &c.author_id).await?;
    comment_repo::update_content(&pool, &comment_id, &req.content).await?;
    Ok(Json(serde_json::json!({"message": "Comment updated"})))
}

pub async fn delete_comment(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path((post_key, comment_id)): Path<(String, String)>,
) -> AppResult<Json<serde_json::Value>> {
    let post = post_repo::find_by_id_or_slug(&pool, &post_key)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;
    let c = comment_repo::find_by_id(&pool, &comment_id, &post.id)
        .await?
        .ok_or_else(|| AppError::NotFound("Comment not found".into()))?;
    authorize_comment_mutation(&pool, &auth_user, &c.author_id).await?;
    apply_comment_delete(&pool, &comment_id).await?;
    crate::repositories::audit_repo::record(
        &pool,
        &auth_user.id,
        "comment.delete",
        "comment",
        &comment_id,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({"message": "Comment deleted"})))
}

// --- Flat top-level management (no per-post fetching) ---

fn to_list_item(row: crate::repositories::comment_repo::CommentListRow) -> CommentListItem {
    CommentListItem {
        id: row.id,
        content: row.content,
        author: CommentAuthor {
            id: row.author_id,
            username: row.author_username,
        },
        post: CommentPostRef {
            id: row.post_id,
            slug: row.post_slug,
            title: row.post_title,
            published: row.post_published,
        },
        parent_id: row.parent_id,
        depth: row.depth,
        is_deleted: row.is_deleted,
        created_at: row.created_at,
    }
}

pub async fn list_comments(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Query(q): Query<CommentListQuery>,
) -> AppResult<Json<CommentListResponse>> {
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
    let can_see_all = auth_user.role.has_permission(&Role::Moderator);
    // Moderators/admins see everything; plain users see only their own.
    let viewer_id = if can_see_all {
        None
    } else {
        Some(auth_user.id.as_str())
    };
    let (rows, total) = comment_repo::list_scoped(
        &pool,
        viewer_id,
        can_see_all,
        q.author.as_deref(),
        q.post.as_deref(),
        q.search.as_deref(),
        q.deleted,
        page,
        per_page,
        ascending,
    )
    .await?;
    Ok(Json(CommentListResponse {
        comments: rows.into_iter().map(to_list_item).collect(),
        total,
        page,
        per_page,
    }))
}

/// Single flat view with post context.
/// Published-post comments: any authenticated viewer. Draft-post comments
/// follow the tree rule (post author or moderator+, else `404`) — mirroring
/// `list_comments`, which hides other authors' drafts from plain users.
pub async fn get_comment_flat(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(comment_id): Path<String>,
) -> AppResult<Json<CommentDetailResponse>> {
    let row = comment_repo::find_anywhere(&pool, &comment_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Comment not found".into()))?;
    if !row.post_published
        && auth_user.id != row.post_author_id
        && !auth_user.role.has_permission(&Role::Moderator)
    {
        return Err(AppError::NotFound("Comment not found".into()));
    }
    let updated_at = row.updated_at.clone();
    Ok(Json(CommentDetailResponse {
        item: to_list_item(row),
        updated_at,
    }))
}

pub async fn update_comment_flat(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(comment_id): Path<String>,
    Json(req): Json<UpdateCommentRequest>,
) -> AppResult<Json<serde_json::Value>> {
    validate_request(&req)?;
    let row = comment_repo::find_anywhere(&pool, &comment_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Comment not found".into()))?;
    authorize_comment_mutation(&pool, &auth_user, &row.author_id).await?;
    comment_repo::update_content(&pool, &comment_id, &req.content).await?;
    Ok(Json(serde_json::json!({"message": "Comment updated"})))
}

pub async fn delete_comment_flat(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(comment_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let row = comment_repo::find_anywhere(&pool, &comment_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Comment not found".into()))?;
    authorize_comment_mutation(&pool, &auth_user, &row.author_id).await?;
    apply_comment_delete(&pool, &comment_id).await?;
    crate::repositories::audit_repo::record(
        &pool,
        &auth_user.id,
        "comment.delete",
        "comment",
        &comment_id,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({"message": "Comment deleted"})))
}

fn build_comment_tree(mut flat: Vec<CommentFlat>) -> Vec<CommentTreeNode> {
    // Deterministic chronological order (DB already sorts, but enforce for children).
    flat.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

    let mut nodes: HashMap<String, CommentTreeNode> = HashMap::new();
    let mut root_ids: Vec<String> = Vec::new();
    let mut children_map: HashMap<String, Vec<String>> = HashMap::new();

    for c in &flat {
        nodes.insert(
            c.id.clone(),
            CommentTreeNode {
                id: c.id.clone(),
                content: c.content.clone(),
                author: CommentAuthor {
                    id: c.author_id.clone(),
                    username: c.author_username.clone(),
                },
                parent_id: c.parent_id.clone(),
                depth: c.depth,
                is_deleted: c.is_deleted,
                created_at: c.created_at.clone(),
                children: vec![],
            },
        );
        match &c.parent_id {
            None => root_ids.push(c.id.clone()),
            Some(pid) => {
                children_map
                    .entry(pid.clone())
                    .or_default()
                    .push(c.id.clone());
            }
        }
    }

    fn build(
        id: &str,
        nodes: &mut HashMap<String, CommentTreeNode>,
        cm: &HashMap<String, Vec<String>>,
    ) -> CommentTreeNode {
        let mut node = nodes.remove(id).unwrap();
        if let Some(child_ids) = cm.get(id) {
            for cid in child_ids {
                if nodes.contains_key(cid) {
                    node.children.push(build(cid, nodes, cm));
                }
            }
        }
        node
    }

    // Orphans (parent missing, e.g. hard-deleted out of order) become roots
    // instead of disappearing.
    let mut roots: Vec<String> = root_ids
        .into_iter()
        .filter(|id| nodes.contains_key(id))
        .collect();
    let orphaned: Vec<String> = nodes
        .keys()
        .filter(|id| !roots.contains(id) && !children_map.values().any(|v| v.contains(id)))
        .cloned()
        .collect();
    roots.extend(orphaned);

    #[allow(clippy::filter_map_bool_then)]
    roots
        .iter()
        .filter_map(|id| {
            nodes
                .contains_key(id)
                .then(|| build(id, &mut nodes, &children_map))
        })
        .collect()
}
