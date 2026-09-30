use crate::{db::DbPool, db_query, errors::AppResult, models::user::User};

pub async fn find_by_id(pool: &DbPool, id: &str) -> AppResult<Option<User>> {
    let user = db_query!(pool, |p| {
        sqlx::query_as::<_, User>(
            "SELECT id, username, email, password_hash, role, \
             CAST(is_active AS BOOLEAN) as is_active, \
             CAST(created_at AS TEXT) as created_at, \
             CAST(updated_at AS TEXT) as updated_at \
             FROM users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(p)
        .await?
    });
    Ok(user)
}

pub async fn find_by_username(pool: &DbPool, username: &str) -> AppResult<Option<User>> {
    let user = db_query!(pool, |p| {
        sqlx::query_as::<_, User>(
            "SELECT id, username, email, password_hash, role, \
             CAST(is_active AS BOOLEAN) as is_active, \
             CAST(created_at AS TEXT) as created_at, \
             CAST(updated_at AS TEXT) as updated_at \
             FROM users WHERE username = $1 AND is_active = TRUE",
        )
        .bind(username)
        .fetch_optional(p)
        .await?
    });
    Ok(user)
}

pub async fn create(
    pool: &DbPool,
    id: &str,
    username: &str,
    email: &str,
    password_hash: &str,
) -> AppResult<User> {
    db_query!(pool, |p| {
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash, role) VALUES ($1, $2, $3, $4, 'user')"
        )
        .bind(id).bind(username).bind(email).bind(password_hash)
        .execute(p)
        .await?;
    });
    Ok(find_by_id(pool, id).await?.expect("Just inserted"))
}

pub async fn list_all(pool: &DbPool) -> AppResult<Vec<User>> {
    let users = db_query!(pool, |p| {
        sqlx::query_as::<_, User>(
            "SELECT id, username, email, password_hash, role, \
             CAST(is_active AS BOOLEAN) as is_active, \
             CAST(created_at AS TEXT) as created_at, \
             CAST(updated_at AS TEXT) as updated_at \
             FROM users ORDER BY created_at DESC, id DESC",
        )
        .fetch_all(p)
        .await?
    });
    Ok(users)
}

/// Paginated user listing with deterministic order (`id` tiebreaker guards
/// equal `created_at` values). `exclude_admins` hides admin accounts
/// (moderator view). Returns `(rows, total)`.
pub async fn list_paginated(
    pool: &DbPool,
    page: i64,
    per_page: i64,
    ascending: bool,
    exclude_admins: bool,
) -> AppResult<(Vec<User>, i64)> {
    let offset = (page - 1) * per_page;
    let direction = if ascending { "ASC" } else { "DESC" };
    let filter = if exclude_admins {
        "WHERE role != 'admin'"
    } else {
        ""
    };
    // SAFETY: `direction`/`filter` are static literals derived from bools.
    let data_sql = format!(
        "SELECT id, username, email, password_hash, role, \
         CAST(is_active AS BOOLEAN) as is_active, \
         CAST(created_at AS TEXT) as created_at, \
         CAST(updated_at AS TEXT) as updated_at \
         FROM users {filter} ORDER BY created_at {direction}, id {direction} \
         LIMIT $1 OFFSET $2"
    );
    let count_sql = format!("SELECT COUNT(*) FROM users {filter}");
    let (rows, total) = db_query!(pool, |p| {
        let total = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.as_str()))
            .fetch_one(p)
            .await?;
        let rows = sqlx::query_as::<_, User>(sqlx::AssertSqlSafe(data_sql.as_str()))
            .bind(per_page)
            .bind(offset)
            .fetch_all(p)
            .await?;
        (rows, total)
    });
    Ok((rows, total))
}

pub async fn update_role(pool: &DbPool, id: &str, role: &str) -> AppResult<u64> {
    let rows = db_query!(pool, |p| {
        sqlx::query("UPDATE users SET role = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
            .bind(role)
            .bind(id)
            .execute(p)
            .await?
            .rows_affected()
    });
    Ok(rows)
}

pub async fn deactivate(pool: &DbPool, id: &str) -> AppResult<u64> {
    let rows = db_query!(pool, |p| {
        sqlx::query(
            "UPDATE users SET is_active = FALSE, updated_at = CURRENT_TIMESTAMP WHERE id = $1",
        )
        .bind(id)
        .execute(p)
        .await?
        .rows_affected()
    });
    Ok(rows)
}

pub async fn activate(pool: &DbPool, id: &str) -> AppResult<u64> {
    let rows = db_query!(pool, |p| {
        sqlx::query(
            "UPDATE users SET is_active = TRUE, updated_at = CURRENT_TIMESTAMP WHERE id = $1",
        )
        .bind(id)
        .execute(p)
        .await?
        .rows_affected()
    });
    Ok(rows)
}

pub async fn count_admins(pool: &DbPool) -> AppResult<i64> {
    let count = db_query!(pool, |p| {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE role = 'admin'")
            .fetch_one(p)
            .await?
    });
    Ok(count)
}

/// System ghost author for soft-deleted users' content.
pub const GHOST_USER_ID: &str = "00000000-0000-0000-0000-000000000000";

/// Idempotent ghost-user seeding (inactive, unusable credentials).
pub async fn ensure_ghost_user(pool: &DbPool) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash, role, is_active) \
             VALUES ($1, '[deleted]', 'deleted@deleted.local', 'deleted', 'user', FALSE) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(GHOST_USER_ID)
        .execute(p)
        .await?;
    });
    Ok(())
}

/// Soft delete: deactivate + erase PII (username/email unlinked, password
/// unusable). Content is re-attributed separately by the caller.
pub async fn anonymize(pool: &DbPool, id: &str, suffix: &str) -> AppResult<u64> {
    let rows = db_query!(pool, |p| {
        sqlx::query(
            "UPDATE users SET is_active = FALSE, username = $1, email = $2, \
             password_hash = 'deleted', updated_at = CURRENT_TIMESTAMP WHERE id = $3",
        )
        .bind(format!("deleted_{suffix}"))
        .bind(format!("deleted_{suffix}@deleted.local"))
        .bind(id)
        .execute(p)
        .await?
        .rows_affected()
    });
    Ok(rows)
}

/// Re-attribute a deleted user's posts/comments to the ghost author.
/// Returns `(posts, comments)` counts.
pub async fn reassign_content(pool: &DbPool, from_id: &str) -> AppResult<(u64, u64)> {
    let (posts, comments) = db_query!(pool, |p| {
        let posts = sqlx::query("UPDATE posts SET author_id = $1 WHERE author_id = $2")
            .bind(GHOST_USER_ID)
            .bind(from_id)
            .execute(p)
            .await?
            .rows_affected();
        let comments = sqlx::query("UPDATE comments SET author_id = $1 WHERE author_id = $2")
            .bind(GHOST_USER_ID)
            .bind(from_id)
            .execute(p)
            .await?
            .rows_affected();
        (posts, comments)
    });
    Ok((posts, comments))
}

pub async fn count_posts_by_author(pool: &DbPool, author_id: &str) -> AppResult<i64> {
    let count = db_query!(pool, |p| {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM posts WHERE author_id = $1")
            .bind(author_id)
            .fetch_one(p)
            .await?
    });
    Ok(count)
}

pub async fn count_published_by_author(pool: &DbPool, author_id: &str) -> AppResult<i64> {
    let count = db_query!(pool, |p| {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM posts WHERE author_id = $1 AND published = TRUE",
        )
        .bind(author_id)
        .fetch_one(p)
        .await?
    });
    Ok(count)
}

pub async fn count_comments_by_author(pool: &DbPool, author_id: &str) -> AppResult<i64> {
    let count = db_query!(pool, |p| {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM comments WHERE author_id = $1")
            .bind(author_id)
            .fetch_one(p)
            .await?
    });
    Ok(count)
}

/// Hard delete: removes the row; posts/comments/post_tags cascade via FK.
pub async fn hard_delete(pool: &DbPool, id: &str) -> AppResult<u64> {
    let rows = db_query!(pool, |p| {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(p)
            .await?
            .rows_affected()
    });
    Ok(rows)
}

pub async fn create_admin(
    pool: &DbPool,
    id: &str,
    username: &str,
    email: &str,
    password_hash: &str,
) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash, role) VALUES ($1, $2, $3, $4, 'admin')"
        )
        .bind(id)
        .bind(username)
        .bind(email)
        .bind(password_hash)
        .execute(p)
        .await
        .ok();
    });
    Ok(())
}
