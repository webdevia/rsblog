use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Comment {
    pub id: String,
    pub content: String,
    pub post_id: String,
    pub author_id: String,
    pub parent_id: Option<String>,
    pub path: String,
    pub depth: i32,
    pub is_deleted: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct CommentTreeNode {
    pub id: String,
    pub content: String,
    pub author: CommentAuthor,
    pub parent_id: Option<String>,
    pub depth: i32,
    pub is_deleted: bool,
    pub created_at: String,
    pub children: Vec<CommentTreeNode>,
}

#[derive(Debug, Serialize, Clone)]
pub struct CommentAuthor {
    pub id: String,
    pub username: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CommentFlat {
    pub id: String,
    pub content: String,
    pub post_id: String,
    pub author_id: String,
    pub author_username: String,
    pub parent_id: Option<String>,
    pub path: String,
    pub depth: i32,
    pub is_deleted: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct CreateCommentRequest {
    #[validate(length(min = 1, max = 5000))]
    pub content: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateCommentRequest {
    #[validate(length(min = 1, max = 5000))]
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct CommentPostRef {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub published: bool,
}

#[derive(Debug, Serialize)]
pub struct CommentListItem {
    pub id: String,
    pub content: String,
    pub author: CommentAuthor,
    pub post: CommentPostRef,
    pub parent_id: Option<String>,
    pub depth: i32,
    pub is_deleted: bool,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct CommentDetailResponse {
    pub item: CommentListItem,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct CommentListResponse {
    pub comments: Vec<CommentListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

#[derive(Debug, Deserialize)]
pub struct CommentListQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    /// `desc` (default, newest first) or `asc`.
    pub order: Option<String>,
    /// Filter by author username.
    pub author: Option<String>,
    /// Filter by post id or slug.
    pub post: Option<String>,
    /// Case-insensitive substring of content.
    pub search: Option<String>,
    /// `true` = only soft-deleted, `false` = only live; default hides deleted.
    pub deleted: Option<bool>,
}
