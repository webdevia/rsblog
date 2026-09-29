use crate::config::{Config, DatabaseBackend};

/// Unified database pool. Each variant is conditionally compiled.
#[derive(Clone)]
pub enum DbPool {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlx::SqlitePool),

    #[cfg(feature = "postgres")]
    Postgres(sqlx::PgPool),
}

impl DbPool {
    pub async fn init(config: &Config) -> Self {
        match &config.database_backend {
            #[cfg(feature = "sqlite")]
            DatabaseBackend::Sqlite => Self::init_sqlite(&config.database_url).await,

            #[cfg(feature = "postgres")]
            DatabaseBackend::Postgres => Self::init_postgres(&config.database_url).await,
        }
    }

    #[cfg(feature = "sqlite")]
    async fn init_sqlite(url: &str) -> Self {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;

        let opts = SqliteConnectOptions::from_str(url)
            .expect("Invalid SQLite DATABASE_URL")
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(30))
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .min_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect_with(opts)
            .await
            .expect("Failed to create SQLite pool");

        sqlx::migrate!("./migrations/sqlite")
            .run(&pool)
            .await
            .expect("Failed to run SQLite migrations");

        tracing::info!("✅ Connected to SQLite");
        DbPool::Sqlite(pool)
    }

    #[cfg(feature = "postgres")]
    async fn init_postgres(url: &str) -> Self {
        use sqlx::postgres::PgPoolOptions;

        let pool = PgPoolOptions::new()
            .max_connections(20)
            .min_connections(2)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .idle_timeout(std::time::Duration::from_secs(600))
            .connect(url)
            .await
            .expect("Failed to create Postgres pool");

        sqlx::migrate!("./migrations/postgres")
            .run(&pool)
            .await
            .expect("Failed to run Postgres migrations");

        tracing::info!("✅ Connected to PostgreSQL");
        DbPool::Postgres(pool)
    }
}
