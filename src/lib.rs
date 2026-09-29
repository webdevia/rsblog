// Compile-time assertion: at least one database backend must be enabled
#[cfg(not(any(feature = "sqlite", feature = "postgres")))]
compile_error!(
    "At least one database backend must be enabled. \
     Use --features sqlite and/or --features postgres. \
     Example: cargo build --no-default-features --features postgres"
);
