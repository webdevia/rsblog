# Deploying rsblog to the 560 MB VPS

Target: 1 vCPU, 560 MB RAM (~416 MB available), `docker-compose.yml` + Caddy + PostgreSQL.

The golden rule of this setup: **the VPS never compiles anything**.
Rust release builds need 1 GB+ RAM and would OOM (or swap-thrash for an
hour) on this box. Instead the image is built once — on GitHub Actions or
your own machine — pushed to a registry, and the VPS only pulls and runs
the finished ~100 MB image.

## 0. Prerequisites

- A domain, e.g. `blog.example.com`, with an **A record pointing at the VPS IP**.
  Both `:80` and `:443` must be reachable from the internet (Let's Encrypt
  validates over HTTP on port 80).
- The repo pushed to GitHub (so the `docker` workflow can build the image).
- SSH access to the VPS.

## 1. Build the image once (GitHub Actions → GHCR)

1. Push the repo to GitHub. The workflow in `.github/workflows/docker.yml`
   builds on every push to `master` and pushes:
   - `ghcr.io/<owner>/rsblog:latest`
   - `ghcr.io/<owner>/rsblog:sha-<shortsha>`
2. Check the run under **Actions**; it takes ~5–10 min (dependency compile
   is cached between runs via `type=gha`).
3. Make the package pullable: `ghcr.io/<owner>` → package `rsblog` →
   **Package settings → Change visibility → Public**.
   (Alternative: keep it private and `docker login ghcr.io` on the VPS.)

No GitHub? Build on any machine with 2 GB+ RAM instead:

```bash
docker build -t <your-dockerhub-user>/rsblog:latest .
docker push <your-dockerhub-user>/rsblog:latest
```

and use `API_IMAGE=<your-dockerhub-user>/rsblog:latest` below.

## 2. Prepare the VPS (one time)

```bash
# Docker (Debian/Ubuntu)
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER && newgrp docker

mkdir -p ~/rsblog && cd ~/rsblog
```

Copy these files from the repo to `~/rsblog/` (no source code needed):

- `docker-compose.yml`
- `Caddyfile`
- `.env.prod.example` → rename to `.env`, then edit:

```bash
cp .env.prod.example .env
nano .env
```

Fill in, at minimum:

| Variable            | Value                                              |
| ------------------- | -------------------------------------------------- |
| `DOMAIN`            | your domain, e.g. `blog.example.com`               |
| `ACME_EMAIL`        | your email (Let's Encrypt expiry notices)          |
| `POSTGRES_PASSWORD` | `openssl rand -hex 32`                             |
| `JWT_SECRET`        | `openssl rand -hex 48` (≥ 64 chars required)       |
| `ADMIN_PASSWORD`    | strong password, ≥ 12 chars (seeded once)          |
| `CORS_ORIGINS`      | `https://blog.example.com` (or your frontend URL)  |

## 3. First deploy

```bash
cd ~/rsblog
export API_IMAGE=ghcr.io/<owner>/rsblog:latest   # add to ~/.bashrc to persist

docker compose pull            # downloads ~150 MB total, no compiling
docker compose up -d
docker compose ps              # all three services "healthy" within ~1 min
```

Verify, from your own machine:

```bash
curl -sI https://blog.example.com/health
# HTTP/2 200 ... strict-transport-security: max-age=63072000; ...

TOKEN=$(curl -s -X POST https://blog.example.com/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"<ADMIN_PASSWORD>"}' | jq -r .token)
curl -s https://blog.example.com/api/v1/admin/users \
  -H "Authorization: Bearer $TOKEN" | jq .
```

Migrations run automatically at startup and the admin is seeded once from
`ADMIN_*`. Memory check on the box: `docker stats --no-stream` should show
roughly postgres ~120 MB, api ~25 MB, caddy ~20 MB — well under the caps
(224/96/64 MB).

## 4. Updating to a new version

```bash
cd ~/rsblog
export API_IMAGE=ghcr.io/<owner>/rsblog:latest
docker compose pull api
docker compose up -d api
docker compose logs -f --tail=50 api
```

- To pin a version instead of `latest`, use a SHA tag:
  `API_IMAGE=ghcr.io/<owner>/rsblog:sha-abc1234`.
- Rollback is the same procedure with the previous tag
  (`docker compose pull` it first; the old image stays cached locally
  until pruned, so rollback usually works offline).
- DB migrations run forward automatically at startup. Downgrades that need
  a migration rollback are **not** automated — restore from backup instead.

## 5. Backups (do this before you need it)

What matters: `pgdata` (your content), `caddy_data` (certs — losing it
forces re-issuance against Let's Encrypt rate limits), and `.env` (secrets).

```bash
cd ~/rsblog
# Database dump (no downtime):
docker compose exec -T postgres pg_dump -U blog_user blog_db > backup-$(date +%F).sql
# Volumes + env, offsite (example: rsync to your machine):
docker run --rm -v rsblog_pgdata:/pg -v rsblog_caddy_data:/caddy \
  -v $(pwd):/out alpine tar czf /out/volumes-$(date +%F).tgz /pg /caddy
cp .env /safe/place/rsblog.env.$(date +%F)   # secrets — store securely
```

Suggested schedule: weekly dump via cron + copy offsite. Test restores
occasionally — an untested backup is not a backup.

Restore (fresh VPS or disaster):

```bash
# 1. Recreate ~/rsblog with docker-compose.yml, Caddyfile, .env
# 2. Restore volumes, then:
docker compose up -d postgres
cat backup-YYYY-MM-DD.sql | docker compose exec -T postgres \
  psql -U blog_user -d blog_db
docker compose up -d
```

## 6. Troubleshooting

```bash
docker compose ps                 # health status of all three
docker compose logs --tail=100 api | postgres | caddy
docker stats --no-stream          # actual RAM vs the 224/96/64 caps
free -m                           # host-level pressure + swap use
```

| Symptom | Likely cause / fix |
| ------- | ------------------ |
| `api` OOM-killed (exit 137) | login burst hashing past 96 MB; raise api `mem_limit` to `128m` (total caps then 416 MB = exactly avail — watch `docker stats`) |
| Caddy: `tls.obtain` / 500s | DNS not pointing here, or `:80` blocked; `curl -s http://<domain>/.well-known/` must reach Caddy. Check `docker compose logs caddy`. Too many retries → LE rate limit: wait, then fix DNS first |
| `postgres` unhealthy | data volume perms or OOM; `docker compose logs postgres`; never delete `pgdata` without a dump |
| `401` everywhere after deploy | wrong `JWT_SECRET` vs the one that signed existing tokens — tokens signed by the old secret are invalid; users just log in again |
| Site slow, swap growing | something exceeds caps; `docker stats` shows who; typical healthy idle: pg ~120, api ~25, caddy ~20 MB |
| Need a shell in prod | `docker compose exec postgres psql -U blog_user blog_db` (DB port is intentionally unpublished) |

## 7. Files involved

- `docker-compose.yml` — postgres (256→224 MB cap) + api (96 MB) + caddy (64 MB); caps sum 384 MB < 416 MB avail
- `Dockerfile` — multi-stage, postgres-only release build (used by CI, not the VPS)
- `Caddyfile` — `{$DOMAIN}` + `{$ACME_EMAIL}`, auto-TLS, HSTS, `X-Real-IP` forwarding the rate limiter trusts
- `.github/workflows/docker.yml` — CI build → GHCR
- `.env.prod.example` — template for the VPS `.env`
