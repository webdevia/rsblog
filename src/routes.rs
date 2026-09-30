use axum::{
    handler::Handler, // Necessary to call .layer() directly on handlers
    middleware,
    routing::{get, post, put, Router},
};

use crate::{
    auth::middleware::auth_middleware,
    config::Config,
    db::DbPool,
    handlers::*,
    rate_limiter::RateLimiterState,
    security::{cors_layer, security_headers_middleware},
    spam::SpamState,
};

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub config: Config,
    pub spam: SpamState,
}

impl axum::extract::FromRef<AppState> for DbPool {
    fn from_ref(s: &AppState) -> Self {
        s.pool.clone()
    }
}

impl axum::extract::FromRef<AppState> for Config {
    fn from_ref(s: &AppState) -> Self {
        s.config.clone()
    }
}

impl axum::extract::FromRef<AppState> for SpamState {
    fn from_ref(s: &AppState) -> Self {
        s.spam.clone()
    }
}

pub fn create_router(state: AppState) -> Router {
    let auth_layer = middleware::from_fn_with_state(state.clone(), auth_middleware);

    if state.config.cors_origins.is_empty() {
        tracing::warn!(
            "CORS_ORIGINS is unset: reflecting any origin. Set it to a comma-separated \
             allowlist (e.g. CORS_ORIGINS=https://app.example) to lock down browser access."
        );
    } else {
        tracing::info!("CORS allowlist: {}", state.config.cors_origins.join(", "));
    }
    let cors = cors_layer(&state.config.cors_origins);
    let hardening = middleware::from_fn(security_headers_middleware);

    // Tight bucket for auth endpoints: slows mass-registration and
    // credential-stuffing per IP without affecting normal traffic.
    let auth_limit = RateLimiterState::new(
        state.config.auth_rate_limit_rps,
        state.config.auth_rate_limit_burst,
    );
    let auth_limit_layer =
        middleware::from_fn_with_state(auth_limit, crate::rate_limiter::rate_limit_middleware);
    let auth_routes = Router::new()
        .route("/auth/register", post(auth_handler::register))
        .route("/auth/login", post(auth_handler::login))
        .route_layer(auth_limit_layer);

    let routes = Router::new()
        // --- Health (Public, no auth, no rate-limit bypass) ---
        .route("/health", get(health))
        .merge(auth_routes)
        // --- Profile (Authenticated) ---
        .route(
            "/users/me",
            get(user_handler::get_me.layer(auth_layer.clone())),
        )
        // --- Posts ---
        .route(
            "/posts",
            get(post_handler::list_posts).post(post_handler::create_post.layer(auth_layer.clone())),
        )
        .route(
            "/posts/{id}",
            get(post_handler::get_post)
                .put(post_handler::update_post.layer(auth_layer.clone()))
                .delete(post_handler::delete_post.layer(auth_layer.clone())),
        )
        .route(
            "/posts/{id}/publish",
            post(post_handler::publish_post.layer(auth_layer.clone())),
        )
        .route(
            "/posts/{id}/unpublish",
            post(post_handler::unpublish_post.layer(auth_layer.clone())),
        )
        // --- Comments ---
        .route(
            "/posts/{post_id}/comments",
            get(comment_handler::get_comments_tree)
                .post(comment_handler::create_comment.layer(auth_layer.clone())),
        )
        .route(
            "/posts/{post_id}/comments/{comment_id}",
            put(comment_handler::update_comment)
                .delete(comment_handler::delete_comment)
                .layer(auth_layer.clone()), // Applies authentication to both PUT and DELETE
        )
        // --- Tags ---
        .route(
            "/tags",
            get(tag_handler::list_tags).post(tag_handler::create_tag.layer(auth_layer.clone())),
        )
        .route(
            "/tags/{id}",
            get(tag_handler::get_tag)
                .put(tag_handler::update_tag.layer(auth_layer.clone()))
                .delete(tag_handler::delete_tag.layer(auth_layer.clone())),
        )
        // --- Admin Console (Authenticated / Moderator+) ---
        .route(
            "/admin/users",
            get(user_handler::list_users.layer(auth_layer.clone())),
        )
        .route(
            "/admin/users/{id}",
            get(user_handler::get_user.layer(auth_layer.clone()))
                .delete(user_handler::delete_user.layer(auth_layer.clone())),
        )
        .route(
            "/admin/users/{id}/role",
            put(user_handler::update_user_role.layer(auth_layer.clone())),
        )
        .route(
            "/admin/users/{id}/deactivate",
            post(user_handler::deactivate_user.layer(auth_layer.clone())),
        )
        .route(
            "/admin/users/{id}/activate",
            post(user_handler::activate_user.layer(auth_layer.clone())),
        );

    // Top-level health for load balancers (outside /api/v1), plus versioned one.
    // Security headers + CORS apply to all of them (they are the app's policy,
    // not deployment configuration).
    Router::new()
        .route("/health", get(health))
        .nest("/api/v1", routes)
        .layer(hardening)
        .layer(cors)
        .with_state(state)
}

async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
