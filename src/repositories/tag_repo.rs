use crate::{db::DbPool, db_query, errors::AppResult, models::tag::*};

pub async fn create(pool: &DbPool, id: &str, name: &str, slug: &str) -> AppResult<Tag> {
    db_query!(pool, |p| {
        sqlx::query("INSERT INTO tags (id, name, slug) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(name)
            .bind(slug)
            .execute(p)
            .await?;
    });
    find_by_id(pool, id).await
}

pub async fn find_by_id(pool: &DbPool, id: &str) -> AppResult<Tag> {
    let tag =
        db_query!(pool, |p| {
            sqlx::query_as::<_, Tag>(
            "SELECT id, name, slug, CAST(created_at AS TEXT) as created_at FROM tags WHERE id = $1"
        )
        .bind(id).fetch_one(p).await?
        });
    Ok(tag)
}

pub async fn find_by_slug(pool: &DbPool, slug: &str) -> AppResult<Option<Tag>> {
    let tag = db_query!(pool, |p| {
        sqlx::query_as::<_, Tag>(
            "SELECT id, name, slug, CAST(created_at AS TEXT) as created_at FROM tags WHERE slug = $1"
        )
        .bind(slug).fetch_optional(p).await?
    });
    Ok(tag)
}

pub async fn list_with_counts(pool: &DbPool) -> AppResult<Vec<TagWithCount>> {
    let tags = db_query!(pool, |p| {
        sqlx::query_as::<_, TagWithCount>(
            "SELECT t.id, t.name, t.slug, COUNT(pt.post_id) as post_count \
             FROM tags t \
             LEFT JOIN post_tags pt ON t.id = pt.tag_id \
             LEFT JOIN posts p ON pt.post_id = p.id AND p.published = TRUE \
             GROUP BY t.id, t.name, t.slug ORDER BY t.name ASC",
        )
        .fetch_all(p)
        .await?
    });
    Ok(tags)
}

pub async fn update(pool: &DbPool, id: &str, name: &str, slug: &str) -> AppResult<u64> {
    let rows = db_query!(pool, |p| {
        sqlx::query("UPDATE tags SET name = $1, slug = $2 WHERE id = $3")
            .bind(name)
            .bind(slug)
            .bind(id)
            .execute(p)
            .await?
            .rows_affected()
    });
    Ok(rows)
}

pub async fn delete(pool: &DbPool, id: &str) -> AppResult<u64> {
    let rows = db_query!(pool, |p| {
        sqlx::query("DELETE FROM tags WHERE id = $1")
            .bind(id)
            .execute(p)
            .await?
            .rows_affected()
    });
    Ok(rows)
}
