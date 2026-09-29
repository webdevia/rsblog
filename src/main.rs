// Compile-time check: at least one DB backend must be enabled
#[cfg(not(any(feature = "sqlite", feature = "postgres")))]
compile_error!(
    "At least one database backend must be enabled. \
     Use: --features sqlite  or  --features postgres  or  --features all-databases"
);

mod auth;
mod config;
mod db;
mod errors;
mod handlers;
mod models;
mod rate_limiter;
mod repositories;
mod routes;
mod validators; // Custom rate limiter module

use std::net::SocketAddr;
use tower_http::{
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    config::{Config, DatabaseBackend},
    db::DbPool,
    rate_limiter::{rate_limit_middleware, RateLimiterState},
    repositories::user_repo,
    routes::{create_router, AppState},
};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "blog_api=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    print_build_info();

    let config = Config::from_env();
    let pool = DbPool::init(&config).await;
    seed_admin(&pool).await;

    let state = AppState {
        pool,
        config: config.clone(),
    };

    // Rate limiting: 60 requests per minute (avg 1 req/sec) with burst capacity of 60
    let rate_limiter_state = RateLimiterState::new(1, 60);

    let app = create_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024))
        // Apply our custom IP rate limiter middleware
        .layer(axum::middleware::from_fn_with_state(
            rate_limiter_state,
            rate_limit_middleware,
        ));

    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse().unwrap();
    tracing::info!("🚀 Server running on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

fn print_build_info() {
    let backends = DatabaseBackend::available_backends().join(", ");
    tracing::info!("╔══════════════════════════════════════════════════╗");
    tracing::info!("║  Blog API                                         ║");
    tracing::info!("║  Version:  {:<38} ║", env!("CARGO_PKG_VERSION"));
    tracing::info!("║  Backends: {:<38} ║", backends);
    tracing::info!("╚══════════════════════════════════════════════════╝");
}

async fn seed_admin(pool: &DbPool) {
    if user_repo::count_admins(pool).await.unwrap_or(0) == 0 {
        let hash = auth::password::hash_password("Admin@123456").unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        user_repo::create_admin(pool, &id, &hash).await.ok();
        tracing::info!("✅ Admin seeded (username: admin, password: Admin@123456)");
    }
}
