use std::env;

#[derive(Debug, Clone, PartialEq)]
pub enum DatabaseBackend {
    #[cfg(feature = "sqlite")]
    Sqlite,
    #[cfg(feature = "postgres")]
    Postgres,
}

impl DatabaseBackend {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            #[cfg(feature = "sqlite")]
            "sqlite" => Ok(DatabaseBackend::Sqlite),

            #[cfg(feature = "postgres")]
            "postgres" | "postgresql" => Ok(DatabaseBackend::Postgres),

            other => Err(format!(
                "Backend '{other}' is not available. Compiled backends: [{}]",
                Self::available_backends().join(", ")
            )),
        }
    }

    /// Lists backends compiled into this binary
    #[allow(clippy::vec_init_then_push)]
    pub fn available_backends() -> Vec<&'static str> {
        let mut list = Vec::new();
        #[cfg(feature = "sqlite")]
        list.push("sqlite");
        #[cfg(feature = "postgres")]
        list.push("postgres");
        list
    }

    /// Returns the default backend for this build
    /// Priority: if only one is compiled, that's the default; otherwise SQLite.
    pub fn default_backend() -> Self {
        #[cfg(all(feature = "sqlite", not(feature = "postgres")))]
        {
            DatabaseBackend::Sqlite
        }
        #[cfg(all(feature = "postgres", not(feature = "sqlite")))]
        {
            DatabaseBackend::Postgres
        }
        #[cfg(all(feature = "sqlite", feature = "postgres"))]
        {
            DatabaseBackend::Sqlite
        }
    }

    /// Auto-detect backend from URL scheme
    pub fn from_url(url: &str) -> Option<Self> {
        #[cfg(feature = "sqlite")]
        if url.starts_with("sqlite:") {
            return Some(DatabaseBackend::Sqlite);
        }
        #[cfg(feature = "postgres")]
        if url.starts_with("postgres:") || url.starts_with("postgresql:") {
            return Some(DatabaseBackend::Postgres);
        }
        None
    }
}

impl std::fmt::Display for DatabaseBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "sqlite")]
            DatabaseBackend::Sqlite => write!(f, "sqlite"),
            #[cfg(feature = "postgres")]
            DatabaseBackend::Postgres => write!(f, "postgres"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub database_backend: DatabaseBackend,
    pub jwt_secret: String,
    pub jwt_expiration_hours: i64,
    pub host: String,
    pub port: u16,
    pub admin_username: String,
    pub admin_email: String,
    pub admin_password: Option<String>,
    /// Empty = reflect any origin (previous behaviour, logs a warning).
    pub cors_origins: Vec<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");

        // Backend resolution order:
        //   1. Explicit DATABASE_BACKEND env var
        //   2. Auto-detect from DATABASE_URL scheme
        //   3. Default for this build
        let database_backend = if let Ok(explicit) = env::var("DATABASE_BACKEND") {
            DatabaseBackend::from_str(&explicit)
                .unwrap_or_else(|e| panic!("DATABASE_BACKEND error: {e}"))
        } else if let Some(detected) = DatabaseBackend::from_url(&database_url) {
            detected
        } else {
            let default = DatabaseBackend::default_backend();
            tracing::warn!(
                "Could not detect DB backend from URL; using default '{}'",
                default
            );
            default
        };

        tracing::info!(
            "Configured backend: {} | available in this build: [{}]",
            database_backend,
            DatabaseBackend::available_backends().join(", ")
        );

        let jwt_secret = env::var("JWT_SECRET").expect("JWT_SECRET must be set");
        if jwt_secret.len() < 32 {
            panic!("JWT_SECRET must be at least 32 chars (64+ recommended for HS256)");
        }

        let jwt_expiration_hours: i64 = env::var("JWT_EXPIRATION_HOURS")
            .unwrap_or_else(|_| "24".to_string())
            .parse()
            .expect("JWT_EXPIRATION_HOURS must be a number");
        if !(1..=720).contains(&jwt_expiration_hours) {
            panic!("JWT_EXPIRATION_HOURS must be between 1 and 720");
        }

        let admin_password = env::var("ADMIN_PASSWORD").ok().filter(|s| !s.is_empty());
        if let Some(ref pw) = admin_password {
            if pw.len() < 12 {
                panic!("ADMIN_PASSWORD must be at least 12 chars when set");
            }
        }

        Self {
            database_url,
            database_backend,
            jwt_secret,
            jwt_expiration_hours,
            host: env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()
                .expect("PORT must be a number"),
            admin_username: env::var("ADMIN_USERNAME").unwrap_or_else(|_| "admin".to_string()),
            admin_email: env::var("ADMIN_EMAIL").unwrap_or_else(|_| "admin@blog.com".to_string()),
            admin_password,
            cors_origins: crate::security::parse_cors_origins(
                env::var("CORS_ORIGINS").ok().as_deref(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_from_str() {
        #[cfg(feature = "sqlite")]
        assert_eq!(
            DatabaseBackend::from_str("sqlite").unwrap().to_string(),
            "sqlite"
        );
        #[cfg(feature = "postgres")]
        assert_eq!(
            DatabaseBackend::from_str("PostgreSQL").unwrap().to_string(),
            "postgres"
        );
        assert!(DatabaseBackend::from_str("mysql").is_err());
    }

    #[test]
    fn backend_from_url_scheme() {
        #[cfg(feature = "sqlite")]
        assert_eq!(
            DatabaseBackend::from_url("sqlite://blog.db?mode=rwc").map(|b| b.to_string()),
            Some("sqlite".to_string())
        );
        #[cfg(feature = "postgres")]
        {
            assert_eq!(
                DatabaseBackend::from_url("postgres://u:p@localhost/db").map(|b| b.to_string()),
                Some("postgres".to_string())
            );
            assert_eq!(
                DatabaseBackend::from_url("postgresql://u:p@localhost/db").map(|b| b.to_string()),
                Some("postgres".to_string())
            );
        }
        assert_eq!(DatabaseBackend::from_url("mysql://localhost/db"), None);
    }

    #[test]
    fn available_backends_matches_compiled_features() {
        let backends = DatabaseBackend::available_backends();
        #[cfg(feature = "sqlite")]
        assert!(backends.contains(&"sqlite"));
        #[cfg(feature = "postgres")]
        assert!(backends.contains(&"postgres"));
        assert!(!backends.is_empty());
    }
}
