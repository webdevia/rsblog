use crate::{db::DbPool, db_query, errors::AppResult, models::post::*};

#[allow(clippy::too_many_arguments)]
pub async fn create(
    pool: &DbPool,
    id: &str,
    title: &str,
    slug: &str,
    content: &str,
    excerpt: Option<&str>,
    published: bool,
    author_id: &str,
) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query(
            "INSERT INTO posts (id, title, slug, content, excerpt, published, author_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(title)
        .bind(slug)
        .bind(content)
        .bind(excerpt)
        .bind(published)
        .bind(author_id)
        .execute(p)
        .await?;
    });
    Ok(())
}

pub async fn find_by_id(pool: &DbPool, id: &str) -> AppResult<Option<Post>> {
    let post = db_query!(pool, |p| {
        sqlx::query_as::<_, Post>(
            "SELECT id, title, slug, content, excerpt, \
             CAST(published AS BOOLEAN) as published, author_id, \
             CAST(created_at AS TEXT) as created_at, \
             CAST(updated_at AS TEXT) as updated_at \
             FROM posts WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(p)
        .await?
    });
    Ok(post)
}

#[allow(dead_code)]
pub async fn find_by_slug(
    pool: &DbPool,
    slug: &str,
    published_only: bool,
) -> AppResult<Option<Post>> {
    let sql = if published_only {
        "SELECT id, title, slug, content, excerpt, \
         CAST(published AS BOOLEAN) as published, author_id, \
         CAST(created_at AS TEXT) as created_at, \
         CAST(updated_at AS TEXT) as updated_at \
         FROM posts WHERE slug = $1 AND published = TRUE"
    } else {
        "SELECT id, title, slug, content, excerpt, \
         CAST(published AS BOOLEAN) as published, author_id, \
         CAST(created_at AS TEXT) as created_at, \
         CAST(updated_at AS TEXT) as updated_at \
         FROM posts WHERE slug = $1"
    };
    let post = db_query!(pool, |p| {
        sqlx::query_as::<_, Post>(sql)
            .bind(slug)
            .fetch_optional(p)
            .await?
    });
    Ok(post)
}

pub async fn slug_exists(pool: &DbPool, slug: &str) -> AppResult<bool> {
    let count = db_query!(pool, |p| {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM posts WHERE slug = $1")
            .bind(slug)
            .fetch_one(p)
            .await?
    });
    Ok(count > 0)
}

pub async fn slug_exists_excluding(pool: &DbPool, slug: &str, exclude_id: &str) -> AppResult<bool> {
    let count = db_query!(pool, |p| {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM posts WHERE slug = $1 AND id != $2")
            .bind(slug)
            .bind(exclude_id)
            .fetch_one(p)
            .await?
    });
    Ok(count > 0)
}

/// Lookup by UUID or slug (friendly URLs). Tries ID first, then slug.
pub async fn find_by_id_or_slug(pool: &DbPool, key: &str) -> AppResult<Option<Post>> {
    if let Some(post) = find_by_id(pool, key).await? {
        return Ok(Some(post));
    }
    let post = db_query!(pool, |p| {
        sqlx::query_as::<_, Post>(
            "SELECT id, title, slug, content, excerpt, \
             CAST(published AS BOOLEAN) as published, author_id, \
             CAST(created_at AS TEXT) as created_at, \
             CAST(updated_at AS TEXT) as updated_at \
             FROM posts WHERE slug = $1",
        )
        .bind(key)
        .fetch_optional(p)
        .await?
    });
    Ok(post)
}

pub async fn update_content(
    pool: &DbPool,
    id: &str,
    title: &str,
    slug: &str,
    content: &str,
    excerpt: Option<&str>,
) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query(
            "UPDATE posts SET title=$1, slug=$2, content=$3, excerpt=$4, \
             updated_at=CURRENT_TIMESTAMP WHERE id=$5",
        )
        .bind(title)
        .bind(slug)
        .bind(content)
        .bind(excerpt)
        .bind(id)
        .execute(p)
        .await?;
    });
    Ok(())
}

pub async fn set_published(pool: &DbPool, id: &str, published: bool) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query("UPDATE posts SET published=$1, updated_at=CURRENT_TIMESTAMP WHERE id=$2")
            .bind(published)
            .bind(id)
            .execute(p)
            .await?;
    });
    Ok(())
}

#[allow(dead_code)]
pub async fn update(
    pool: &DbPool,
    id: &str,
    title: &str,
    slug: &str,
    content: &str,
    excerpt: Option<&str>,
    published: bool,
) -> AppResult<()> {
    update_content(pool, id, title, slug, content, excerpt).await?;
    set_published(pool, id, published).await
}

pub async fn delete(pool: &DbPool, id: &str) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query("DELETE FROM posts WHERE id = $1")
            .bind(id)
            .execute(p)
            .await?;
    });
    Ok(())
}

pub async fn attach_tag(pool: &DbPool, post_id: &str, tag_id: &str) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query(
            "INSERT INTO post_tags (post_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(post_id)
        .bind(tag_id)
        .execute(p)
        .await?;
    });
    Ok(())
}

pub async fn clear_tags(pool: &DbPool, post_id: &str) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query("DELETE FROM post_tags WHERE post_id = $1")
            .bind(post_id)
            .execute(p)
            .await?;
    });
    Ok(())
}

pub async fn get_tags(pool: &DbPool, post_id: &str) -> AppResult<Vec<TagInfo>> {
    let tags = db_query!(pool, |p| {
        sqlx::query_as::<_, TagInfo>(
            "SELECT t.id, t.name, t.slug FROM tags t \
             JOIN post_tags pt ON t.id = pt.tag_id WHERE pt.post_id = $1",
        )
        .bind(post_id)
        .fetch_all(p)
        .await?
    });
    Ok(tags)
}

/// Batch tag fetch to avoid N+1 in list endpoints.
/// Returns map post_id -> tags.
pub async fn get_tags_for_posts(
    pool: &DbPool,
    post_ids: &[String],
) -> AppResult<std::collections::HashMap<String, Vec<TagInfo>>> {
    use std::collections::HashMap;

    #[derive(sqlx::FromRow)]
    struct PostTagRow {
        post_id: String,
        id: String,
        name: String,
        slug: String,
    }

    if post_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders: Vec<String> = (1..=post_ids.len()).map(|i| format!("${i}")).collect();
    let sql = format!(
        "SELECT pt.post_id as post_id, t.id, t.name, t.slug FROM tags t \
         JOIN post_tags pt ON t.id = pt.tag_id WHERE pt.post_id IN ({})",
        placeholders.join(", ")
    );

    let rows = db_query!(pool, |p| {
        // SAFETY: placeholders are generated `$N`, values are bound.
        let mut q = sqlx::query_as::<_, PostTagRow>(sqlx::AssertSqlSafe(sql.as_str()));
        for id in post_ids {
            q = q.bind(id);
        }
        q.fetch_all(p).await?
    });

    let mut map: HashMap<String, Vec<TagInfo>> = HashMap::new();
    for r in rows {
        map.entry(r.post_id).or_default().push(TagInfo {
            id: r.id,
            name: r.name,
            slug: r.slug,
        });
    }
    Ok(map)
}

pub async fn get_author(pool: &DbPool, author_id: &str) -> AppResult<PostAuthor> {
    let author = db_query!(pool, |p| {
        sqlx::query_as::<_, PostAuthor>("SELECT id, username FROM users WHERE id = $1")
            .bind(author_id)
            .fetch_one(p)
            .await?
    });
    Ok(author)
}

pub async fn comment_count(pool: &DbPool, post_id: &str) -> AppResult<i64> {
    let count = db_query!(pool, |p| {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM comments WHERE post_id = $1 AND is_deleted = FALSE",
        )
        .bind(post_id)
        .fetch_one(p)
        .await?
    });
    Ok(count)
}

#[allow(clippy::too_many_arguments)]
pub async fn list(
    pool: &DbPool,
    viewer_id: Option<&str>,
    can_see_all: bool,
    author: Option<&str>,
    tag: Option<&str>,
    search: Option<&str>,
    page: i64,
    per_page: i64,
) -> AppResult<(Vec<PostRow>, i64)> {
    let offset = (page - 1) * per_page;

    let mut conditions = Vec::new();
    let mut params: Vec<String> = Vec::new();
    let mut idx = 0usize;

    // Visibility scope:
    // - moderator/admin -> no filter (see everything)
    // - authenticated user -> published OR own drafts
    // - anonymous -> published only
    if can_see_all {
        // no visibility condition
    } else if let Some(vid) = viewer_id {
        idx += 1;
        conditions.push(format!("(p.published = TRUE OR p.author_id = ${idx})"));
        params.push(vid.to_string());
    } else {
        conditions.push("p.published = TRUE".to_string());
    }
    if let Some(a) = author {
        idx += 1;
        conditions.push(format!("u.username = ${idx}"));
        params.push(a.to_string());
    }
    if let Some(t) = tag {
        idx += 1;
        conditions.push(format!(
            "p.id IN (SELECT pt.post_id FROM post_tags pt JOIN tags tg ON pt.tag_id = tg.id WHERE tg.slug = ${idx} OR tg.name = ${idx})"
        ));
        params.push(t.to_string());
    }
    if let Some(s) = search {
        let pat = format!("%{}%", s.to_lowercase());
        idx += 1;
        let i1 = idx;
        idx += 1;
        let i2 = idx;
        // Portable case-insensitive search (SQLite LIKE is ASCII-insensitive,
        // Postgres LIKE is sensitive; LOWER() works on both and can use
        // expression indexes. Postgres FTS index remains for future use).
        conditions.push(format!(
            "(LOWER(p.title) LIKE LOWER(${i1}) OR LOWER(p.content) LIKE LOWER(${i2}))"
        ));
        params.push(pat.clone());
        params.push(pat);
    }

    let where_clause = if conditions.is_empty() {
        "TRUE".to_string()
    } else {
        conditions.join(" AND ")
    };

    let count_sql = format!(
        "SELECT COUNT(DISTINCT p.id) FROM posts p JOIN users u ON p.author_id = u.id WHERE {where_clause}"
    );

    idx += 1;
    let limit_idx = idx;
    idx += 1;
    let offset_idx = idx;

    let data_sql = format!(
        "SELECT p.id, p.title, p.slug, p.excerpt, \
         CAST(p.published AS BOOLEAN) as published, \
         p.author_id, u.username as author_username, \
         CAST(p.created_at AS TEXT) as created_at, \
         (SELECT COUNT(*) FROM comments c WHERE c.post_id = p.id AND c.is_deleted = FALSE) as comment_count \
         FROM posts p JOIN users u ON p.author_id = u.id \
         WHERE {where_clause} \
         ORDER BY p.created_at DESC \
         LIMIT ${limit_idx} OFFSET ${offset_idx}"
    );

    let (rows, total) = db_query!(pool, |p| {
        // SAFETY: `count_sql`/`data_sql` are built only from static fragments and
        // positional `$N` placeholders; all user input goes through bound params.
        let mut cq = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.as_str()));
        for v in &params {
            cq = cq.bind(v);
        }
        let total = cq.fetch_one(p).await?;

        let mut dq = sqlx::query_as::<_, PostRow>(sqlx::AssertSqlSafe(data_sql.as_str()));
        for v in &params {
            dq = dq.bind(v);
        }
        dq = dq.bind(per_page).bind(offset);
        let rows = dq.fetch_all(p).await?;

        (rows, total)
    });

    Ok((rows, total))
}
