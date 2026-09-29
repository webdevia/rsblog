use crate::{
    auth::middleware::AuthUser,
    db::DbPool,
    errors::{AppError, AppResult},
    models::{comment::*, user::Role},
    repositories::{comment_repo, post_repo},
    validators::validate_request,
};
use axum::{
    extract::{Extension, Path, State},
    Json,
};
use std::collections::HashMap;

pub async fn create_comment(
    State(pool): State<DbPool>,
    Extension(auth_user): Extension<AuthUser>,
    Path(post_key): Path<String>,
    Json(req): Json<CreateCommentRequest>,
) -> AppResult<Json<CommentTreeNode>> {
    validate_request(&req)?;

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
    Path(post_key): Path<String>,
) -> AppResult<Json<Vec<CommentTreeNode>>> {
    let post = post_repo::find_by_id_or_slug(&pool, &post_key)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;
    let flat = comment_repo::list_for_post(&pool, &post.id).await?;
    Ok(Json(build_comment_tree(flat)))
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
    if c.author_id != auth_user.id && !auth_user.role.has_permission(&Role::Moderator) {
        return Err(AppError::Forbidden);
    }
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
    if c.author_id != auth_user.id && !auth_user.role.has_permission(&Role::Moderator) {
        return Err(AppError::Forbidden);
    }
    if comment_repo::has_children(&pool, &comment_id).await? {
        comment_repo::soft_delete(&pool, &comment_id).await?;
    } else {
        comment_repo::hard_delete(&pool, &comment_id).await?;
    }
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
