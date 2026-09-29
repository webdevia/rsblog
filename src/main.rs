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
mod validators;

use std::net::SocketAddr;
use tower_http::{cors::CorsLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};
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
    seed_admin(&pool, &config).await;

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
                .allow_origin(tower_http::cors::Any)
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::PUT,
                    axum::http::Method::DELETE,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ])
                .max_age(std::time::Duration::from_secs(3600)),
        )
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024))
        // Apply our custom IP rate limiter middleware
        .layer(axum::middleware::from_fn_with_state(
            rate_limiter_state,
            rate_limit_middleware,
        ));

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST/PORT combination");
    tracing::info!("🚀 Server running on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("Server error");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("Shutdown signal received, starting graceful shutdown");
}

fn print_build_info() {
    let backends = DatabaseBackend::available_backends().join(", ");
    tracing::info!("╔══════════════════════════════════════════════════╗");
    tracing::info!("║  Blog API                                        ║");
    tracing::info!("║  Version:  {:<38}║", env!("CARGO_PKG_VERSION"));
    tracing::info!("║  Backends: {:<38}║", backends);
    tracing::info!("╚══════════════════════════════════════════════════╝");
}

async fn seed_admin(pool: &DbPool, config: &Config) {
    if user_repo::count_admins(pool).await.unwrap_or(0) == 0 {
        let Some(ref admin_password) = config.admin_password else {
            tracing::info!(
                "No admin exists and ADMIN_PASSWORD is unset — skipping admin seeding. Set ADMIN_PASSWORD (>=12 chars) to create '{}'.",
                config.admin_username
            );
            return;
        };
        match auth::password::hash_password(admin_password) {
            Ok(hash) => {
                let id = uuid::Uuid::new_v4().to_string();
                user_repo::create_admin(
                    pool,
                    &id,
                    &config.admin_username,
                    &config.admin_email,
                    &hash,
                )
                .await
                .ok();
                tracing::info!("✅ Admin '{}' seeded", config.admin_username);
            }
            Err(e) => tracing::error!("Failed to hash admin password: {e}"),
        }
    }
}
