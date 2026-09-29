use crate::{
    auth::jwt::verify_token,
    config::Config,
    errors::{AppError, AppResult},
    models::user::Role,
};
use axum::{
    extract::{Request, State},
    http::header::AUTHORIZATION,
    middleware::Next,
    response::Response,
};

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: String,
    #[allow(dead_code)]
    pub username: String,
    pub role: Role,
}

pub async fn auth_middleware(
    State(config): State<Config>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let auth_user = extract_user(&req, &config)?;
    req.extensions_mut().insert(auth_user);
    Ok(next.run(req).await)
}

#[allow(dead_code)]
pub async fn optional_auth_middleware(
    State(config): State<Config>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let user = extract_user(&req, &config).ok();
    req.extensions_mut().insert(user);
    Ok(next.run(req).await)
}

fn extract_user(req: &Request, config: &Config) -> AppResult<AuthUser> {
    let header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized)?;
    let claims = verify_token(token, &config.jwt_secret)?.claims;
    Ok(AuthUser {
        id: claims.sub,
        username: claims.username,
        role: Role::from_str(&claims.role),
    })
}

pub fn require_role(user: &AuthUser, required: Role) -> AppResult<()> {
    if !user.role.has_permission(&required) {
        return Err(AppError::Forbidden);
    }
    Ok(())
}
