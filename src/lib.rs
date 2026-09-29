// Compile-time assertion: at least one database backend must be enabled
#[cfg(not(any(feature = "sqlite", feature = "postgres")))]
compile_error!(
    "At least one database backend must be enabled. \
     Use --features sqlite and/or --features postgres. \
     Example: cargo build --no-default-features --features postgres"
);

pub mod auth;
pub mod config;
pub mod db;
pub mod errors;
pub mod handlers;
pub mod models;
pub mod rate_limiter;
pub mod repositories;
pub mod routes;
pub mod security;
pub mod validators;
