use axum::{
    handler::Handler, // Necessary to call .layer() directly on handlers
    middleware,
    routing::{get, post, put, Router},
};

use crate::{auth::middleware::auth_middleware, config::Config, db::DbPool, handlers::*};

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub config: Config,
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

pub fn create_router(state: AppState) -> Router {
    let auth_layer = middleware::from_fn_with_state(state.clone(), auth_middleware);

    let routes = Router::new()
        // --- Authentication (Public) ---
        .route("/auth/register", post(auth_handler::register))
        .route("/auth/login", post(auth_handler::login))
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
        // --- Admin Console (Authenticated / Admin Only) ---
        .route(
            "/admin/users",
            get(user_handler::list_users.layer(auth_layer.clone())),
        )
        .route(
            "/admin/users/{id}/role",
            put(user_handler::update_user_role.layer(auth_layer.clone())),
        )
        .route(
            "/admin/users/{id}/deactivate",
            post(user_handler::deactivate_user.layer(auth_layer.clone())),
        );

    Router::new().nest("/api/v1", routes).with_state(state)
}
