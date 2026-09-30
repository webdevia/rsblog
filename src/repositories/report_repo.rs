use crate::{db::DbPool, db_query, errors::AppResult, models::report::*};

pub async fn create(
    pool: &DbPool,
    id: &str,
    reporter_id: &str,
    target_type: &str,
    target_id: &str,
    post_id: Option<&str>,
    reason: &str,
) -> AppResult<()> {
    db_query!(pool, |p| {
        sqlx::query(
            "INSERT INTO reports (id, reporter_id, target_type, target_id, post_id, reason) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(reporter_id)
        .bind(target_type)
        .bind(target_id)
        .bind(post_id)
        .bind(reason)
        .execute(p)
        .await?;
    });
    Ok(())
}

#[derive(sqlx::FromRow)]
struct ReportRow {
    id: String,
    reporter_id: String,
    reporter_username: Option<String>,
    target_type: String,
    target_id: String,
    post_id: Option<String>,
    reason: String,
    status: String,
    created_at: String,
}

fn to_response(r: ReportRow) -> ReportResponse {
    ReportResponse {
        id: r.id,
        reporter: crate::models::report::ReportAuthor {
            id: r.reporter_id,
            username: r.reporter_username,
        },
        target_type: r.target_type,
        target_id: r.target_id,
        post_id: r.post_id,
        reason: r.reason,
        status: r.status,
        created_at: r.created_at,
    }
}

const ROW_SQL: &str = "SELECT r.id, r.reporter_id, u.username as reporter_username, \
     r.target_type, r.target_id, r.post_id, r.reason, r.status, \
     CAST(r.created_at AS TEXT) as created_at \
     FROM reports r LEFT JOIN users u ON r.reporter_id = u.id";

pub async fn find_by_id(pool: &DbPool, id: &str) -> AppResult<Option<ReportResponse>> {
    // SAFETY: static fragments only; id is bound.
    let sql = format!("{ROW_SQL} WHERE r.id = $1");
    let row = db_query!(pool, |p| {
        sqlx::query_as::<_, ReportRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(p)
            .await?
    });
    Ok(row.map(to_response))
}

/// Existing *open* report by this reporter on the same target, if any.
pub async fn find_open_duplicate(
    pool: &DbPool,
    reporter_id: &str,
    target_type: &str,
    target_id: &str,
) -> AppResult<Option<ReportResponse>> {
    // SAFETY: static fragments only; values are bound.
    let sql = format!(
        "{ROW_SQL} WHERE r.reporter_id = $1 AND r.target_type = $2 \
         AND r.target_id = $3 AND r.status = 'open' LIMIT 1"
    );
    let row = db_query!(pool, |p| {
        sqlx::query_as::<_, ReportRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(reporter_id)
            .bind(target_type)
            .bind(target_id)
            .fetch_optional(p)
            .await?
    });
    Ok(row.map(to_response))
}

pub async fn set_status(pool: &DbPool, id: &str, status: &str) -> AppResult<u64> {
    let rows = db_query!(pool, |p| {
        sqlx::query("UPDATE reports SET status = $1 WHERE id = $2")
            .bind(status)
            .bind(id)
            .execute(p)
            .await?
            .rows_affected()
    });
    Ok(rows)
}

/// Paginated queue with `id` tiebreaker. `status` filters exact-match;
/// `None` returns every status.
pub async fn list(
    pool: &DbPool,
    status: Option<&str>,
    page: i64,
    per_page: i64,
    ascending: bool,
) -> AppResult<(Vec<ReportResponse>, i64)> {
    let offset = (page - 1) * per_page;
    let direction = if ascending { "ASC" } else { "DESC" };
    let mut conditions = Vec::new();
    let mut params: Vec<String> = Vec::new();
    if let Some(s) = status {
        conditions.push("r.status = $1".to_string());
        params.push(s.to_string());
    }
    let where_clause = if conditions.is_empty() {
        "TRUE".to_string()
    } else {
        conditions.join(" AND ")
    };

    // SAFETY: fragments are static; direction derives from a bool.
    let limit_idx = params.len() + 1;
    let offset_idx = params.len() + 2;
    let count_sql = format!("SELECT COUNT(*) FROM reports r WHERE {where_clause}");
    let data_sql = format!(
        "{ROW_SQL} WHERE {where_clause} \
         ORDER BY r.created_at {direction}, r.id {direction} \
         LIMIT ${limit_idx} OFFSET ${offset_idx}"
    );

    let (rows, total) = db_query!(pool, |p| {
        let mut cq = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.as_str()));
        for v in &params {
            cq = cq.bind(v);
        }
        let total = cq.fetch_one(p).await?;

        let mut dq = sqlx::query_as::<_, ReportRow>(sqlx::AssertSqlSafe(data_sql.as_str()));
        for v in &params {
            dq = dq.bind(v);
        }
        dq = dq.bind(per_page).bind(offset);
        let rows: Vec<ReportRow> = dq.fetch_all(p).await?;
        (rows, total)
    });

    Ok((rows.into_iter().map(to_response).collect(), total))
}
