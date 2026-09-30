// Compile-time check: at least one DB backend must be enabled
#[cfg(not(any(feature = "sqlite", feature = "postgres")))]
compile_error!(
    "At least one database backend must be enabled. \
     Use: --features sqlite  or  --features postgres  or  --features all-databases"
);

use std::net::SocketAddr;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer};

use blog_api::{
    auth,
    config::{Config, DatabaseBackend},
    db::DbPool,
    logging::{self, AccessLogConfig},
    rate_limiter::{rate_limit_middleware, RateLimiterState},
    repositories::user_repo,
    routes::{create_router, AppState},
};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    logging::init_subscriber();

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

    // NOTE: layers apply inside-out (last `.layer()` is outermost). The
    // observability wrap is applied last so the access log + `x-request-id`
    // cover every response, including 429/413/408 rejections from the
    // deployment layers below it.
    let app = logging::wrap_router(
        create_router(state)
            .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024))
            // Bound total handler time so one slow request can't hold a
            // connection (and a DB pool slot) forever. 408 on expiry.
            .layer(TimeoutLayer::with_status_code(
                axum::http::StatusCode::REQUEST_TIMEOUT,
                std::time::Duration::from_secs(30),
            ))
            // Apply our custom IP rate limiter middleware
            .layer(axum::middleware::from_fn_with_state(
                rate_limiter_state,
                rate_limit_middleware,
            )),
        AccessLogConfig {
            include_query: config.log_include_query,
            slow_request_ms: config.slow_request_ms,
        },
    );

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
