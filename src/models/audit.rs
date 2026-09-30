use serde::Serialize;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AuditEntry {
    pub id: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub detail: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct AuditActor {
    pub id: String,
    pub username: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditResponse {
    pub id: String,
    pub actor: AuditActor,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub detail: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct AuditListResponse {
    pub entries: Vec<AuditResponse>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

#[derive(Debug, serde::Deserialize)]
pub struct AuditQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    /// `desc` (default, newest first) or `asc`.
    pub order: Option<String>,
    /// Admin-only filter; ignored for moderators (forced to self).
    pub actor: Option<String>,
    /// Exact action filter, e.g. `post.delete`.
    pub action: Option<String>,
}
