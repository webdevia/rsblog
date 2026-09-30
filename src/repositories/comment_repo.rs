use crate::{db::DbPool, db_query, errors::AppResult, models::comment::*};

#[allow(clippy::too_many_arguments)]
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
             WHERE c.post_id = $1 ORDER BY c.created_at ASC, c.id ASC",
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

#[allow(dead_code)]
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

/// Flat cross-post comment row for the top-level management list.
#[derive(sqlx::FromRow)]
pub struct CommentListRow {
    pub id: String,
    pub content: String,
    pub post_id: String,
    pub post_slug: String,
    pub post_title: String,
    pub post_published: bool,
    pub post_author_id: String,
    pub author_id: String,
    pub author_username: String,
    pub parent_id: Option<String>,
    pub depth: i32,
    pub is_deleted: bool,
    pub created_at: String,
    pub updated_at: String,
}

const LIST_COLS: &str = "c.id, c.content, c.post_id, p.slug as post_slug, \
     p.title as post_title, CAST(p.published AS BOOLEAN) as post_published, \
     p.author_id as post_author_id, c.author_id, u.username as author_username, \
     c.parent_id, c.depth, CAST(c.is_deleted AS BOOLEAN) as is_deleted, \
     CAST(c.created_at AS TEXT) as created_at, \
     CAST(c.updated_at AS TEXT) as updated_at";
const LIST_JOINS: &str = "FROM comments c JOIN users u ON c.author_id = u.id \
     JOIN posts p ON c.post_id = p.id";

/// Flat cross-post listing with viewer scoping, filters, and pagination.
/// `viewer_id=None` + `can_see_all=false` = anonymous (sees nothing: the
/// flat list is authenticated-only; handlers reject anon before calling).
/// Deleted filter: `None` = hide soft-deleted, `Some(true)` = only deleted,
/// `Some(false)` = only live.
#[allow(clippy::too_many_arguments)]
pub async fn list_scoped(
    pool: &DbPool,
    viewer_id: Option<&str>,
    can_see_all: bool,
    author: Option<&str>,
    post: Option<&str>,
    search: Option<&str>,
    deleted: Option<bool>,
    page: i64,
    per_page: i64,
    ascending: bool,
) -> AppResult<(Vec<CommentListRow>, i64)> {
    let offset = (page - 1) * per_page;
    let direction = if ascending { "ASC" } else { "DESC" };

    let mut conditions = Vec::new();
    let mut params: Vec<String> = Vec::new();
    let mut idx = 0usize;

    if can_see_all {
        // No visibility condition.
    } else if let Some(vid) = viewer_id {
        // Own comments — but on others' drafts only the post author may look
        // (mirrors the comment-tree visibility rule).
        idx += 1;
        conditions.push(format!(
            "(c.author_id = ${idx} AND (p.published = TRUE OR p.author_id = ${idx}))"
        ));
        params.push(vid.to_string());
    } else {
        conditions.push("FALSE".to_string());
    }
    if let Some(a) = author {
        idx += 1;
        conditions.push(format!("u.username = ${idx}"));
        params.push(a.to_string());
    }
    if let Some(pk) = post {
        idx += 1;
        conditions.push(format!("(p.id = ${idx} OR p.slug = ${idx})"));
        params.push(pk.to_string());
    }
    if let Some(s) = search {
        let pat = format!("%{}%", s.to_lowercase());
        idx += 1;
        let i1 = idx;
        conditions.push(format!("LOWER(c.content) LIKE LOWER(${i1})"));
        params.push(pat);
    }
    match deleted {
        Some(true) => conditions.push("c.is_deleted = TRUE".to_string()),
        None | Some(false) => conditions.push("c.is_deleted = FALSE".to_string()),
    }

    let where_clause = if conditions.is_empty() {
        "TRUE".to_string()
    } else {
        conditions.join(" AND ")
    };

    idx += 1;
    let limit_idx = idx;
    idx += 1;
    let offset_idx = idx;

    // SAFETY: static fragments only; direction derives from a bool.
    let count_sql = format!("SELECT COUNT(*) {LIST_JOINS} WHERE {where_clause}");
    let data_sql = format!(
        "SELECT {LIST_COLS} {LIST_JOINS} WHERE {where_clause} \
         ORDER BY c.created_at {direction}, c.id {direction} \
         LIMIT ${limit_idx} OFFSET ${offset_idx}"
    );

    let (rows, total) = db_query!(pool, |p| {
        let mut cq = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.as_str()));
        for v in &params {
            cq = cq.bind(v);
        }
        let total = cq.fetch_one(p).await?;

        let mut dq = sqlx::query_as::<_, CommentListRow>(sqlx::AssertSqlSafe(data_sql.as_str()));
        for v in &params {
            dq = dq.bind(v);
        }
        dq = dq.bind(per_page).bind(offset);
        let rows = dq.fetch_all(p).await?;
        (rows, total)
    });

    Ok((rows, total))
}

/// Locate a comment by id across all posts, with post + author context
/// (single query; orphans impossible via FK).
pub async fn find_anywhere(pool: &DbPool, id: &str) -> AppResult<Option<CommentListRow>> {
    // SAFETY: static fragments only; id is bound.
    let sql = format!("SELECT {LIST_COLS} {LIST_JOINS} WHERE c.id = $1");
    let row = db_query!(pool, |p| {
        sqlx::query_as::<_, CommentListRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(p)
            .await?
    });
    Ok(row)
}
