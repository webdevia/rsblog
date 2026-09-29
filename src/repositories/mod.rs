pub mod comment_repo;
pub mod post_repo;
pub mod tag_repo;
pub mod user_repo;

/// Dispatches a query expression against the active pool variant.
/// Only the arms for enabled features are compiled.
#[macro_export]
macro_rules! db_query {
    ($pool:expr, |$p:ident| $body:expr) => {
        match $pool {
            #[cfg(feature = "sqlite")]
            $crate::db::DbPool::Sqlite(ref $p) => $body,

            #[cfg(feature = "postgres")]
            $crate::db::DbPool::Postgres(ref $p) => $body,
        }
    };
}
