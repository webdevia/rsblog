use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Authentication required")]
    Unauthorized,

    #[error("Insufficient permissions")]
    Forbidden,

    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    Conflict(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Internal server error")]
    Internal(#[from] anyhow::Error),

    #[error("Database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::Validation(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg.clone()),
            AppError::Internal(err) => {
                tracing::error!("Internal error: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::Sqlx(err) => {
                tracing::error!("Database error: {:?}", err);
                match err {
                    sqlx::Error::RowNotFound => {
                        (StatusCode::NOT_FOUND, "Resource not found".to_string())
                    }
                    sqlx::Error::Database(db_err) => {
                        let msg = db_err.message();
                        if msg.contains("UNIQUE")
                            || msg.contains("duplicate key")
                            || msg.contains("unique")
                        {
                            (StatusCode::CONFLICT, "Resource already exists".to_string())
                        } else if msg.contains("foreign key")
                            || msg.contains("violates foreign key")
                        {
                            (
                                StatusCode::BAD_REQUEST,
                                "Referenced resource not found".to_string(),
                            )
                        } else {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Internal server error".to_string(),
                            )
                        }
                    }
                    _ => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error".to_string(),
                    ),
                }
            }
            AppError::Jwt(_) => (StatusCode::UNAUTHORIZED, "Invalid token".to_string()),
        };

        (
            status,
            Json(json!({ "error": { "status": status.as_u16(), "message": message } })),
        )
            .into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
