use crate::{db::DbPool, db_query, errors::AppResult, models::audit::*};

/// Best-effort audit write: callers log a warning on failure rather than
/// failing the moderated action itself.
pub async fn log(
    pool: &DbPool,
    actor_id: &str,
    action: &str,
    target_type: &str,
    target_id: &str,
    detail: Option<&str>,
) -> AppResult<()> {
    let id = uuid::Uuid::new_v4().to_string();
    db_query!(pool, |p| {
        sqlx::query(
            "INSERT INTO audit_log (id, actor_id, action, target_type, target_id, detail) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(actor_id)
        .bind(action)
        .bind(target_type)
        .bind(target_id)
        .bind(detail)
        .execute(p)
        .await?;
    });
    Ok(())
}

/// Best-effort audit write: callers use this so a logging failure never
/// fails the moderated action itself.
pub async fn record(
    pool: &DbPool,
    actor_id: &str,
    action: &str,
    target_type: &str,
    target_id: &str,
    detail: Option<&str>,
) {
    if let Err(e) = log(pool, actor_id, action, target_type, target_id, detail).await {
        tracing::warn!(actor_id, action, target_id, "audit write failed: {e:?}");
    }
}

#[derive(sqlx::FromRow)]
struct AuditRow {
    id: String,
    actor_id: String,
    actor_username: Option<String>,
    action: String,
    target_type: String,
    target_id: String,
    detail: Option<String>,
    created_at: String,
}

/// Paginated audit listing, newest/oldest first with `id` tiebreaker.
/// `actor` / `action` narrow the result; both are exact matches.
#[allow(clippy::too_many_arguments)]
pub async fn list(
    pool: &DbPool,
    actor: Option<&str>,
    action: Option<&str>,
    page: i64,
    per_page: i64,
    ascending: bool,
) -> AppResult<(Vec<AuditResponse>, i64)> {
    let offset = (page - 1) * per_page;
    let direction = if ascending { "ASC" } else { "DESC" };
    let mut conditions = Vec::new();
    let mut params: Vec<String> = Vec::new();
    let mut idx = 0usize;
    if let Some(a) = actor {
        idx += 1;
        conditions.push(format!("a.actor_id = ${idx}"));
        params.push(a.to_string());
    }
    if let Some(a) = action {
        idx += 1;
        conditions.push(format!("a.action = ${idx}"));
        params.push(a.to_string());
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

    // SAFETY: fragments are static; direction/filter derive from bools/params.
    let count_sql = format!("SELECT COUNT(*) FROM audit_log a WHERE {where_clause}");
    let data_sql = format!(
        "SELECT a.id, a.actor_id, u.username as actor_username, a.action, \
         a.target_type, a.target_id, a.detail, \
         CAST(a.created_at AS TEXT) as created_at \
         FROM audit_log a LEFT JOIN users u ON a.actor_id = u.id \
         WHERE {where_clause} \
         ORDER BY a.created_at {direction}, a.id {direction} \
         LIMIT ${limit_idx} OFFSET ${offset_idx}"
    );

    let (rows, total) = db_query!(pool, |p| {
        let mut cq = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.as_str()));
        for v in &params {
            cq = cq.bind(v);
        }
        let total = cq.fetch_one(p).await?;

        let mut dq = sqlx::query_as::<_, AuditRow>(sqlx::AssertSqlSafe(data_sql.as_str()));
        for v in &params {
            dq = dq.bind(v);
        }
        dq = dq.bind(per_page).bind(offset);
        let rows: Vec<AuditRow> = dq.fetch_all(p).await?;
        (rows, total)
    });

    let entries = rows
        .into_iter()
        .map(|r| AuditResponse {
            id: r.id,
            actor: AuditActor {
                id: r.actor_id,
                username: r.actor_username,
            },
            action: r.action,
            target_type: r.target_type,
            target_id: r.target_id,
            detail: r.detail,
            created_at: r.created_at,
        })
        .collect();
    Ok((entries, total))
}
