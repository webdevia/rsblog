use crate::{db::DbPool, db_query, errors::AppResult, models::comment::*};

pub async fn create(
    pool: &DbPool,
    id: &str,
    content: &str,
    post_id: &str,
    author_id: &str,
    parent_id: Option<&str>,
    path: &str,
    depth: i32,
) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query(
            "INSERT INTO comments (id, content, post_id, author_id, parent_id, path, depth) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(content)
        .bind(post_id)
        .bind(author_id)
        .bind(parent_id)
        .bind(path)
        .bind(depth)
        .execute(p)
        .await?;
    });
    Ok(())
}

pub async fn find_by_id(pool: &DbPool, id: &str, post_id: &str) -> AppResult<Option<Comment>> {
    let c = db_query!(pool, |p| {
        sqlx::query_as::<_, Comment>(
            "SELECT id, content, post_id, author_id, parent_id, path, depth, \
             CAST(is_deleted AS BOOLEAN) as is_deleted, \
             CAST(created_at AS TEXT) as created_at, \
             CAST(updated_at AS TEXT) as updated_at \
             FROM comments WHERE id = $1 AND post_id = $2",
        )
        .bind(id)
        .bind(post_id)
        .fetch_optional(p)
        .await?
    });
    Ok(c)
}

pub async fn find_flat(pool: &DbPool, id: &str) -> AppResult<Option<CommentFlat>> {
    let c = db_query!(pool, |p| {
        sqlx::query_as::<_, CommentFlat>(
            "SELECT c.id, c.content, c.post_id, c.author_id, u.username as author_username, \
             c.parent_id, c.path, c.depth, \
             CAST(c.is_deleted AS BOOLEAN) as is_deleted, \
             CAST(c.created_at AS TEXT) as created_at, \
             CAST(c.updated_at AS TEXT) as updated_at \
             FROM comments c JOIN users u ON c.author_id = u.id WHERE c.id = $1",
        )
        .bind(id)
        .fetch_optional(p)
        .await?
    });
    Ok(c)
}

pub async fn list_for_post(pool: &DbPool, post_id: &str) -> AppResult<Vec<CommentFlat>> {
    let comments = db_query!(pool, |p| {
        sqlx::query_as::<_, CommentFlat>(
            "SELECT c.id, c.content, c.post_id, c.author_id, u.username as author_username, \
             c.parent_id, c.path, c.depth, \
             CAST(c.is_deleted AS BOOLEAN) as is_deleted, \
             CAST(c.created_at AS TEXT) as created_at, \
             CAST(c.updated_at AS TEXT) as updated_at \
             FROM comments c JOIN users u ON c.author_id = u.id \
             WHERE c.post_id = $1 ORDER BY c.path, c.created_at ASC",
        )
        .bind(post_id)
        .fetch_all(p)
        .await?
    });
    Ok(comments)
}

pub async fn update_content(pool: &DbPool, id: &str, content: &str) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query(
            "UPDATE comments SET content = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2",
        )
        .bind(content)
        .bind(id)
        .execute(p)
        .await?;
    });
    Ok(())
}

pub async fn soft_delete(pool: &DbPool, id: &str) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query(
            "UPDATE comments SET is_deleted = TRUE, content = '[deleted]', \
             updated_at = CURRENT_TIMESTAMP WHERE id = $1",
        )
        .bind(id)
        .execute(p)
        .await?;
    });
    Ok(())
}

pub async fn hard_delete(pool: &DbPool, id: &str) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query("DELETE FROM comments WHERE id = $1")
            .bind(id)
            .execute(p)
            .await?;
    });
    Ok(())
}

pub async fn has_children(pool: &DbPool, id: &str) -> AppResult<bool> {
    let count = db_query!(pool, |p| {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM comments WHERE parent_id = $1")
            .bind(id)
            .fetch_one(p)
            .await?
    });
    Ok(count > 0)
}

pub async fn post_exists_published(pool: &DbPool, post_id: &str) -> AppResult<bool> {
    let count = db_query!(pool, |p| {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM posts WHERE id = $1 AND published = TRUE",
        )
        .bind(post_id)
        .fetch_one(p)
        .await?
    });
    Ok(count > 0)
}
