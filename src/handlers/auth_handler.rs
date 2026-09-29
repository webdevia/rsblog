use crate::{
    auth::{
        jwt::create_token,
        password::{hash_password, verify_password},
    },
    config::Config,
    db::DbPool,
    errors::{AppError, AppResult},
    models::user::*,
    repositories::user_repo,
    validators::validate_request,
};
use axum::{extract::State, Json};

pub async fn register(
    State(pool): State<DbPool>,
    State(config): State<Config>,
    Json(req): Json<RegisterRequest>,
) -> AppResult<Json<AuthResponse>> {
    validate_request(&req)?;
    let pw_hash = hash_password(&req.password)?;
    let id = uuid::Uuid::new_v4().to_string();

    let user = user_repo::create(&pool, &id, &req.username, &req.email, &pw_hash)
        .await
        .map_err(|e| {
            if let AppError::Sqlx(sqlx::Error::Database(ref db_err)) = e {
                let m = db_err.message();
                if m.contains("UNIQUE") || m.contains("duplicate") || m.contains("unique") {
                    return AppError::Conflict("Username or email already exists".into());
                }
            }
            e
        })?;

    let token = create_token(
        &user.id,
        &user.username,
        &user.role,
        &config.jwt_secret,
        config.jwt_expiration_hours,
    )?;
    Ok(Json(AuthResponse {
        token,
        token_type: "Bearer".into(),
        user: user.into(),
    }))
}

pub async fn login(
    State(pool): State<DbPool>,
    State(config): State<Config>,
    Json(req): Json<LoginRequest>,
) -> AppResult<Json<AuthResponse>> {
    validate_request(&req)?;
    let user = user_repo::find_by_username(&pool, &req.username)
        .await?
        .ok_or(AppError::Unauthorized)?;
    if !verify_password(&req.password, &user.password_hash)? {
        return Err(AppError::Unauthorized);
    }
    let token = create_token(
        &user.id,
        &user.username,
        &user.role,
        &config.jwt_secret,
        config.jwt_expiration_hours,
    )?;
    Ok(Json(AuthResponse {
        token,
        token_type: "Bearer".into(),
        user: user.into(),
    }))
}
