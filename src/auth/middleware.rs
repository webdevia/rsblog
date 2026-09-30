use crate::{
    auth::jwt::{verify_token, Claims},
    db::DbPool,
    errors::{AppError, AppResult},
    models::user::Role,
    repositories::user_repo,
    routes::AppState,
};
use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, HeaderMap},
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
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let claims = extract_claims(&req, &state.config.jwt_secret)?;
    // Revalidate against DB: revokes deactivated/deleted users and picks up fresh role.
    let user = user_repo::find_by_id(&state.pool, &claims.sub)
        .await?
        .ok_or(AppError::Unauthorized)?;
    if !user.is_active {
        return Err(AppError::Unauthorized);
    }
    req.extensions_mut().insert(AuthUser {
        id: user.id,
        username: user.username,
        role: Role::from_str(&user.role),
    });
    Ok(next.run(req).await)
}

#[allow(dead_code)]
pub async fn optional_auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let authed: Option<AuthUser> = match extract_claims(&req, &state.config.jwt_secret) {
        Ok(claims) => match user_repo::find_by_id(&state.pool, &claims.sub).await {
            Ok(Some(user)) if user.is_active => Some(AuthUser {
                id: user.id.clone(),
                username: user.username.clone(),
                role: Role::from_str(&user.role),
            }),
            _ => None,
        },
        Err(_) => None,
    };
    req.extensions_mut().insert(authed);
    Ok(next.run(req).await)
}

fn extract_claims(req: &Request, jwt_secret: &str) -> AppResult<Claims> {
    let header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized)?;
    Ok(verify_token(token, jwt_secret)?.claims)
}

/// Optional auth for public endpoints with viewer-scoped visibility.
/// Returns `None` for anonymous / invalid / deactivated tokens (never errors).
pub async fn try_auth_from_headers(
    pool: &DbPool,
    jwt_secret: &str,
    headers: &HeaderMap,
) -> Option<AuthUser> {
    use crate::auth::jwt::verify_token;
    let header = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = header.strip_prefix("Bearer ")?;
    let claims = verify_token(token, jwt_secret).ok()?.claims;
    // `find_by_id` returns `AppResult<Option<User>>`; treat DB errors as anonymous.
    let user = user_repo::find_by_id(pool, &claims.sub).await.ok()??;
    if !user.is_active {
        return None;
    }
    Some(AuthUser {
        id: user.id.clone(),
        username: user.username.clone(),
        role: Role::from_str(&user.role),
    })
}

pub fn require_role(user: &AuthUser, required: Role) -> AppResult<()> {
    if !user.role.has_permission(&required) {
        return Err(AppError::Forbidden);
    }
    Ok(())
}
