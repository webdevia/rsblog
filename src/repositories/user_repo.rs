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
             FROM users ORDER BY created_at DESC",
        )
        .fetch_all(p)
        .await?
    });
    Ok(users)
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

pub async fn count_admins(pool: &DbPool) -> AppResult<i64> {
    let count = db_query!(pool, |p| {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE role = 'admin'")
            .fetch_one(p)
            .await?
    });
    Ok(count)
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
