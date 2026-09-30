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

    #[error("Invalid username or password")]
    InvalidCredentials,

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

    #[error("Too many requests. Please try again later.")]
    TooManyRequests,

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
            AppError::InvalidCredentials => (StatusCode::UNAUTHORIZED, self.to_string()),
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
            AppError::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, self.to_string()),
        };

        let mut res = (
            status,
            Json(json!({ "error": { "status": status.as_u16(), "message": message } })),
        )
            .into_response();
        if status == StatusCode::TOO_MANY_REQUESTS {
            // Static hint; per-endpoint buckets refill within seconds-to-minutes.
            if let Ok(v) = "60".parse() {
                res.headers_mut().insert("retry-after", v);
            }
        }
        res
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn status_of(err: AppError) -> StatusCode {
        err.into_response().status()
    }

    #[test]
    fn error_status_codes() {
        assert_eq!(status_of(AppError::Unauthorized), StatusCode::UNAUTHORIZED);
        assert_eq!(status_of(AppError::Forbidden), StatusCode::FORBIDDEN);
        assert_eq!(
            status_of(AppError::NotFound("gone".into())),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status_of(AppError::BadRequest("bad".into())),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status_of(AppError::Conflict("dup".into())),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_of(AppError::Validation("bad field".into())),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            status_of(AppError::TooManyRequests),
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    #[test]
    fn error_body_shape() {
        let resp = AppError::NotFound("Post not found".into()).into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        // Body is JSON: {"error": {"status": 404, "message": ...}} — shape is
        // asserted at the HTTP level in the integration tests.
    }

    #[test]
    fn anyhow_becomes_internal_server_error() {
        let err: AppError = anyhow::anyhow!("boom").into();
        assert_eq!(status_of(err), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
