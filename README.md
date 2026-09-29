# rsblog — Blog API

A RESTful blog API written in Rust with [Axum](https://github.com/tokio-rs/axum) and [SQLx](https://github.com/launchbadge/sqlx), supporting **SQLite** and **PostgreSQL** backends. Features JWT authentication, role-based access control, posts with tags, nested comments, and an admin console.

## Features

- **Auth**: registration/login with Argon2 password hashing and JWT (HS256) tokens
- **RBAC**: `user` / `moderator` / `admin` roles; deactivated users have live tokens revoked via per-request DB revalidation
- **Posts**: create/list/search, pagination, tag/author filters, drafts (owner-only preview), slug dedup on create and retitle
- **Tags**: CRUD with post counts; looked up by id or slug
- **Comments**: nested tree (materialized path, max depth 10), soft-delete preserves children (`[deleted]`), hard-delete removes leaves
- **Admin**: list users, change roles, deactivate accounts
- **Ops**: health endpoints, graceful shutdown, per-IP rate limiting (1 req/s, burst 60), configurable CORS, 10 MB body limit, 30 s request timeout
- **Hardening (no proxy needed)**: OWASP security headers on every response, CORS allowlist via `CORS_ORIGINS` (TLS termination itself stays out of the app — use an edge proxy for HTTPS)

## Tech stack

| Layer    | Crates                                                     |
| -------- | ---------------------------------------------------------- |
| Web      | `axum 0.8`, `axum-extra 0.12`, `tower 0.5`, `tower-http 0.7`, `tokio 1` |
| Database | `sqlx 0.9` (sqlite / postgres), migrations in `migrations/` |
| Auth     | `jsonwebtoken 11` (`rust_crypto`), `argon2 0.6`            |
| Misc     | `serde`, `validator 0.21`, `governor 0.10`, `chrono`, `uuid`, `tracing` |
| Tests    | `reqwest 0.12` (dev, E2E client)                           |

## Quickstart

Requirements: Rust stable, and `jq` only for `test_api.sh`.

```bash
cp .env.example .env
# Edit .env: set JWT_SECRET (>= 32 chars) and ADMIN_PASSWORD (>= 12 chars)

cargo build
cargo run
# 🚀 Server running on http://127.0.0.1:3000
```

On first start with no admin present, the account from `ADMIN_USERNAME`/`ADMIN_EMAIL`/`ADMIN_PASSWORD` is seeded. If `ADMIN_PASSWORD` is unset, seeding is skipped.

### Database backends

Feature flags select the compiled backend (`default = ["sqlite"]`):

```bash
cargo build                        # SQLite (default)
cargo build --no-default-features --features postgres
cargo build --features all-databases
```

Backend resolution: `DATABASE_BACKEND` env var → URL scheme auto-detect → build default. Migrations run automatically at startup (`migrations/sqlite`, `migrations/postgres`).

## Configuration (`.env`)

| Variable               | Required | Default              | Notes                                    |
| ---------------------- | -------- | -------------------- | ---------------------------------------- |
| `DATABASE_URL`         | yes      | —                    | e.g. `sqlite://blog.db?mode=rwc`         |
| `DATABASE_BACKEND`     | no       | auto-detect          | `sqlite` or `postgres`                   |
| `JWT_SECRET`           | yes      | —                    | min 32 chars (64+ recommended)           |
| `JWT_EXPIRATION_HOURS` | no       | `24`                 | 1–720                                    |
| `HOST` / `PORT`        | no       | `127.0.0.1` / `3000` | bind address                             |
| `ADMIN_USERNAME`       | no       | `admin`              | seeded admin username                    |
| `ADMIN_EMAIL`          | no       | `admin@blog.com`     | seeded admin email                       |
| `ADMIN_PASSWORD`       | no       | —                    | min 12 chars; unset = skip seeding       |
| `CORS_ORIGINS`         | no       | reflect any origin   | comma-separated allowlist, e.g. `https://app.example` |
| `RUST_LOG`             | no       | `blog_api=debug,...` | tracing filter                           |

Never commit `.env` (already git-ignored; only `.env.example` is tracked).

## API reference

Base path: `/api/v1`. Health also at top-level `/health` (for load balancers).

| Method | Path                                        | Auth              | Description                          |
| ------ | ------------------------------------------- | ----------------- | ------------------------------------ |
| GET    | `/health`, `/api/v1/health`                 | no                | `{"status":"ok","version":"..."}`    |
| POST   | `/api/v1/auth/register`                     | no                | `{username, email, password}` → token |
| POST   | `/api/v1/auth/login`                        | no                | `{username, password}` → token       |
| GET    | `/api/v1/users/me`                          | user              | own profile                          |
| GET    | `/api/v1/posts`                             | no                | list; `?page&per_page&tag&author&search&published` |
| POST   | `/api/v1/posts`                             | user              | `{title, content, excerpt?, published?, tag_ids?}` |
| GET    | `/api/v1/posts/{id_or_slug}`                | optional*         | single post (`*`drafts: owner/moderator only) |
| PUT    | `/api/v1/posts/{id_or_slug}`                | owner/moderator   | partial update                       |
| DELETE | `/api/v1/posts/{id_or_slug}`                | owner/moderator   | delete post                          |
| GET    | `/api/v1/posts/{id_or_slug}/comments`       | no                | nested comment tree                  |
| POST   | `/api/v1/posts/{id_or_slug}/comments`       | user              | `{content, parent_id?}` (published posts only) |
| PUT    | `/api/v1/posts/{id_or_slug}/comments/{cid}` | owner/moderator   | edit comment                         |
| DELETE | `/api/v1/posts/{id_or_slug}/comments/{cid}` | owner/moderator   | soft- or hard-delete                 |
| GET    | `/api/v1/tags`                              | no                | tags with post counts                |
| POST   | `/api/v1/tags`                              | moderator         | `{name}`                             |
| GET    | `/api/v1/tags/{id_or_slug}`                 | no                | single tag                           |
| PUT    | `/api/v1/tags/{id_or_slug}`                 | moderator         | rename tag                           |
| DELETE | `/api/v1/tags/{id_or_slug}`                 | admin             | delete tag                           |
| GET    | `/api/v1/admin/users`                       | admin             | list users                           |
| PUT    | `/api/v1/admin/users/{id}/role`             | admin             | `{role: admin\|moderator\|user}`      |
| POST   | `/api/v1/admin/users/{id}/deactivate`       | admin             | lock account (cannot self-target)    |

Auth: `Authorization: Bearer <token>`. Errors are JSON: `{"error": {"status": <code>, "message": "..."}}`.

Quick smoke test:

```bash
TOKEN=$(curl -s -X POST localhost:3000/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"Admin@123456"}' | jq -r .token)
curl -s localhost:3000/api/v1/posts | jq .
```

## Testing

| Level       | Location | How | Count |
| ----------- | -------- | --- | ----- |
| Unit        | `#[cfg(test)]` in `src/` | `cargo test --lib` | 27 |
| Integration | `tests/api_*.rs` + `tests/common/` | `cargo test --test api_auth ...` | 33 |
| E2E         | `tests/e2e_lifecycle.rs` | live server on ephemeral port via `reqwest` | 1 |
| Shell       | `test_api.sh` | needs running server + `jq` | — |

```bash
cargo test                          # all 61 tests (isolated temp SQLite DBs)
cargo test --features all-databases --lib
cargo clippy --all-targets
cargo fmt --all -- --check

# Legacy shell lifecycle test (needs server running):
cargo run & sleep 2 && bash test_api.sh
```

Integration tests exercise the router in-process (`tower::oneshot`) with a fresh migrated temp DB per test, so they run in parallel without interference.

## Production (Docker + PostgreSQL)

`Dockerfile` (multi-stage, postgres-only build, non-root, `/health` check) and `docker-compose.yml` target a tiny VPS (1 vCPU, ~560 MB RAM):

| Service    | Cap    | Tuning highlights |
| ---------- | ------ | ----------------- |
| `postgres` | 224 MB | `postgres:16-alpine`, `shared_buffers=64MB`, `work_mem=2MB`, `max_connections=30` (app pool uses max 20), port not published, 30 s graceful stop |
| `api`      | 96 MB  | binds `0.0.0.0:3000`, port localhost-only, `RUST_LOG` info by default, capped json-file logs |
| `caddy`    | 64 MB  | `caddy:2-alpine` edge proxy: auto-TLS, HSTS, gzip, `:80`/`:443` |

Caps sum to 384 MB, under the ~416 MB available on a 560 MB box (typical use is ~200 MB; the 1 GB swap is only a backstop).

```bash
cp .env.prod.example .env   # fill in DOMAIN, ACME_EMAIL, POSTGRES_PASSWORD, JWT_SECRET, ADMIN_PASSWORD
docker compose up -d --build
docker compose logs -f api
curl https://<your-domain>/health
```

Full procedure (DNS, first deploy, updates, backups, troubleshooting): see **[DEPLOY.md](DEPLOY.md)**. Key point: never compile on the VPS — the image is built by CI (`.github/workflows/docker.yml` → GHCR) and the box only pulls and runs it via `API_IMAGE=ghcr.io/<owner>/rsblog:latest`.

Notes:
- Migrations run automatically at startup; the admin is seeded once from `ADMIN_*`.
- Compose fails fast with a clear message if `POSTGRES_PASSWORD`/`JWT_SECRET` are unset. `DOMAIN` must already point at the VPS or Let's Encrypt issuance fails.
- Caddy stores certs in the `caddy_data` volume — never delete it casually or you'll hit Let's Encrypt rate limits on re-issue.
- The api port is bound to localhost only: all external traffic goes through Caddy (`:443`), which also lets the rate limiter trust `X-Real-IP` (Caddy overwrites it; direct spoofing is impossible from outside).
- Do **not** build the image on the 560 MB box itself (rustc needs 1 GB+ RAM) — build on CI/a bigger machine, push, and pin `image:` in the compose file.

## Project structure

```
src/
  main.rs          # startup, middleware stack, graceful shutdown, admin seeding
  lib.rs           # public module tree (used by tests/)
  routes.rs        # route table + AppState + health handler
  config.rs        # env config + backend resolution/validation
  db.rs            # DbPool (sqlite/postgres) + pool tuning + migrations
  errors.rs        # AppError -> HTTP status mapping
  auth/            # jwt, argon2 passwords, auth middleware (DB-revalidated)
  handlers/        # auth, post, comment, tag, user endpoints
  models/          # request/response shapes + Role
  repositories/    # SQL queries via db_query! dispatch macro
  validators.rs    # request validation + slugify
  rate_limiter.rs  # governor-based per-IP limiting
tests/
  common/          # shared harness (temp DB, oneshot client, fixtures)
  api_*.rs         # per-domain integration suites
  e2e_lifecycle.rs # full HTTP lifecycle over a live server
migrations/{sqlite,postgres}/
test_api.sh        # shell end-to-end lifecycle script
Dockerfile, docker-compose.yml, Caddyfile, .env.prod.example, .dockerignore
```
