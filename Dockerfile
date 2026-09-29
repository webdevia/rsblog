# syntax=docker/dockerfile:1
# Production image for rsblog (blog-api).
# Builds with the postgres backend only to keep the binary small.
# NOTE: compiling on the 560MB VPS itself is not recommended (rustc needs
# ~1GB+ RAM). Build on CI / a bigger machine and push, or set DOCKER_BUILDKIT=1
# with generous swap. `docker compose up` pulls/uses the built image.

FROM rust:1-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release --no-default-features --features postgres \
    && cp target/release/blog-api /usr/local/bin/blog-api

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /usr/sbin/nologin app
COPY --from=builder /usr/local/bin/blog-api /usr/local/bin/blog-api
USER app
WORKDIR /home/app
EXPOSE 3000
HEALTHCHECK --interval=15s --timeout=5s --retries=3 --start-period=30s \
    CMD curl -fsS http://127.0.0.1:3000/health || exit 1
ENTRYPOINT ["/usr/local/bin/blog-api"]
