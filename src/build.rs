fn main() {
    let sqlite = std::env::var("CARGO_FEATURE_SQLITE").is_ok();
    let postgres = std::env::var("CARGO_FEATURE_POSTGRES").is_ok();

    if !sqlite && !postgres {
        eprintln!();
        eprintln!("╔══════════════════════════════════════════════════════════════╗");
        eprintln!("║  ERROR: No database backend selected!                        ║");
        eprintln!("╠══════════════════════════════════════════════════════════════╣");
        eprintln!("║  Please enable at least one backend:                         ║");
        eprintln!("║                                                              ║");
        eprintln!("║    cargo build --features sqlite                             ║");
        eprintln!("║    cargo build --features postgres                           ║");
        eprintln!("║    cargo build --features all-databases                      ║");
        eprintln!("║                                                              ║");
        eprintln!("║  Or (default = sqlite):                                      ║");
        eprintln!("║    cargo build                                               ║");
        eprintln!("╚══════════════════════════════════════════════════════════════╝");
        eprintln!();
        std::process::exit(1);
    }

    // Show enabled backends
    let mut backends = Vec::new();
    if sqlite {
        backends.push("sqlite");
    }
    if postgres {
        backends.push("postgres");
    }

    println!(
        "cargo:warning=Building with database backends: [{}]",
        backends.join(", ")
    );

    // Re-run if features change
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_SQLITE");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_POSTGRES");
}
