use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Report {
    pub id: String,
    pub reporter_id: String,
    pub target_type: String,
    pub target_id: String,
    pub post_id: Option<String>,
    pub reason: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ReportAuthor {
    pub id: String,
    pub username: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReportResponse {
    pub id: String,
    pub reporter: ReportAuthor,
    pub target_type: String,
    pub target_id: String,
    pub post_id: Option<String>,
    pub reason: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ReportListResponse {
    pub reports: Vec<ReportResponse>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

#[derive(Debug, Deserialize, Validate)]
pub struct CreateReportRequest {
    /// `post` | `comment` | `user`.
    #[validate(length(min = 1, max = 20))]
    pub target_type: String,
    /// Post id/slug, comment id, or user id.
    #[validate(length(min = 1))]
    pub target_id: String,
    /// Required when `target_type` is `comment` (locates the comment).
    pub post_id: Option<String>,
    #[validate(length(min = 1, max = 500))]
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct ReportQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    /// `open` (default) | `dismissed` | `actioned` | `all`.
    pub status: Option<String>,
    /// `desc` (default, newest first) or `asc`.
    pub order: Option<String>,
}
