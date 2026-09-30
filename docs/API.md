# rsblog — API reference

Base path: `/api/v1`. Health is also served at top-level `/health` (for load balancers).

Auth: `Authorization: Bearer <token>` (JWT, HS256). Roles: `user` / `moderator` / `admin`
(`moderator` permission includes `admin`; `user` passes only user-level checks).
Deactivated users get `401` on every authenticated call (per-request DB revalidation
revokes live tokens); reactivation revives non-expired tokens. Deactivated users'
posts and comments stay visible.

Errors are always JSON:

```json
{ "error": { "status": 404, "message": "Post not found" } }
```

Status mapping: `401` authentication required / bad credentials, `403` insufficient
permissions, `404` not found (also used to mask drafts from unauthorized viewers —
no existence leak), `400` bad request, `409` conflict (e.g. duplicate username/email/tag),
`422` validation error. Unknown `published` field in `PUT /posts/{id}` is rejected
with `422` (`deny_unknown_fields`).

Resources can be addressed by UUID **or** slug: `{id_or_slug}`. Slugs are derived from
the title and deduped (`hello-world`, `hello-world-<id8>` on collision, also on retitle).
Pagination: `?page` (default `1`), `?per_page` (default `20`, max `100`); paginated
list responses are `{ posts|users, total, page, per_page }` (see Posts, Admin).
Sort: newest-first by default with an `id` tiebreaker (stable across pages);
`GET /posts` and `GET /admin/users` accept `?order=asc|desc` (anything else → `400`).
`GET /posts/{id}/comments` returns the full nested tree chronologically
(`created_at, id` — intentionally unpaginated); `GET /tags` returns all tags A–Z
(low cardinality, intentionally unpaginated).

Every response carries an `x-request-id` header (client-sent value honored if sane,
otherwise a UUID v4 is generated) — use it to correlate access-log lines with error
logs. See [Observability](#observability).

## Contents

- [Conventions](#conventions)
- [RBAC matrix](#rbac-matrix)
- [Observability](#observability)
- [Anti-spam](#anti-spam)
- [Health](#health)
- [Auth](#auth)
- [Posts](#posts)
- [Comments](#comments)
- [Tags](#tags)
- [Admin](#admin)
- [Examples](#examples)

## Conventions

- `optional` auth means: no token → anonymous scope; valid token → viewer scope;
  invalid/deactivated token on these endpoints is treated as anonymous (not `401`),
  except where noted.
- `?published=` query param on `GET /posts` is accepted for compatibility but **ignored**:
  visibility is derived from the viewer.
- Validation limits: post title 1–200 chars, post content 1–100,000 chars,
  excerpt ≤ 1,000 chars, comment content 1–5000 chars, tag name 1–50 chars,
  username 3–30 chars, email must be valid, password 8–128 chars.
  Comment nesting max depth 10.
- Comment delete: leaf → hard-delete (row removed); parent → soft-delete
  (`content: "[deleted]"`, `is_deleted: true`, children preserved).

## RBAC matrix

| Scope | anon | user | moderator | admin |
|---|---|---|---|---|
| `GET /posts` | published only | all published + own drafts | all | all |
| `GET /posts/{id}` draft | `404` | owner only (`404` others) | ✅ | ✅ |
| `POST /posts` | `401` | draft only (`published:true` forced to `false`) | `published?` honored | `published?` honored |
| `PUT/DELETE /posts`, `publish/unpublish`, comment `PUT/DELETE` | `401`/`404` | own only | all except admin-owned (`403`) | all |
| `GET /posts/{id}/comments` | published tree only, draft → `404` | same + own drafts | all | all |
| `POST /posts/{id}/comments` | `401` | published posts only (draft → `404`) | same | same |
| `POST/PUT /tags` | `401`/`403` | `403` | ✅ | ✅ |
| `DELETE /tags` | `401`/`403` | `403` | `403` | ✅ |
| `/admin/*` | `401` | `403` | list/view/promote/ban on users; `403` on demote, admin-role grants, admin targets, all deletes | ✅ (not self; not last admin for delete) |

## Observability

One `INFO` access line per request (target `blog_api::logging`):

```text
request method=GET path=/api/v1/posts status=200 latency_ms=3 client_ip=127.0.0.1 request_id=<uuid>
```

- `5xx` responses log at error level, requests slower than `SLOW_REQUEST_MS`
  (default `1000`, `0` disables) log an additional `slow request` warning.
- The logging layer wraps the rate limiter / body limit / timeout layers, so
  `429`/`413`/`408` rejections are logged too.
- `/health` and `/api/v1/health` still return `x-request-id` but emit no access
  line (Docker healthchecks poll every 15s).
- Bodies and headers (never `Authorization`) are not logged. Query strings are
  excluded unless `LOG_INCLUDE_QUERY=true` (`?search=`/`?author=` are PII);
  note the logged path may still contain resource IDs/slugs.
- Knobs: `LOG_FORMAT=text|json` (`text` dev default, `json` in compose),
  `LOG_INCLUDE_QUERY`, `SLOW_REQUEST_MS`, plus `RUST_LOG` filter
  (e.g. `blog_api=info,tower_http=info,sqlx=warn` in production).

```bash
curl -sI localhost:3000/api/v1/posts | grep -i x-request-id
# x-request-id: 550e8400-e29b-41d4-a716-446655440000
curl -s localhost:3000/api/v1/posts -H 'x-request-id: my-id-1' -D - -o /dev/null | grep -i x-request-id
# x-request-id: my-id-1
```

## Anti-spam

Two layers: per-IP rate buckets stop floods, per-user content checks stop spam
accounts. All `429`s carry a `Retry-After: 60` header and the standard error body.

| Layer | Scope | Defaults | Knobs |
|---|---|---|---|
| Global bucket | per IP, all traffic | 1 rps, burst 60 | `RATE_LIMIT_RPS`, `RATE_LIMIT_BURST` |
| Auth bucket | per IP, `/auth/*` | 1 rps, burst 5 | `AUTH_RATE_RPS`, `AUTH_RATE_BURST` |
| Comment throttle | per user | 5/min | `COMMENT_RATE_PER_MIN` (`0` off) |
| Post throttle | per user | 10/hour | `POST_RATE_PER_HOUR` (`0` off) |
| Duplicates | per user, posts+comments | 60 min window | `DUPLICATE_WINDOW_MIN` (`0` off) → `409` |
| Link cap | untrusted users | 3 links/post-or-comment | `MAX_LINKS_NEW_USER` (`0` off) → `422` |

Trusted users skip write throttles and link caps (duplicates still apply):
moderators/admins, accounts older than `TRUSTED_ACCOUNT_DAYS` (default 30), or
authors with ≥ `TRUSTED_PUBLISHED_COUNT` published posts (default 5).
Throttle state is in-memory sliding windows (no migration); restarts reset
counters toward leniency. Checks run after validation and auth, ordered
link-cap → duplicate → rate, so rejected requests don't burn quota.

```bash
curl -s -X POST $BASE/posts/$POST_ID/comments -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d '{"content":"buy now https://x.example"}' -D - | grep -i retry-after
# HTTP/1.1 429 Too Many Requests + retry-after: 60 (when throttled)
```

## Health

### `GET /health`, `GET /api/v1/health` — no auth

```json
{ "status": "ok", "version": "0.1.0" }
```

## Auth

### `POST /api/v1/auth/register` — no auth

Request: `{ "username": "alice", "email": "alice@test.local", "password": "SecurePassword123" }`

Success `200`:

```json
{
  "token": "<jwt>",
  "token_type": "Bearer",
  "user": { "id": "uuid", "username": "alice", "email": "alice@test.local", "role": "user", "is_active": true, "created_at": "..." }
}
```

Errors: `409` username/email taken, `422` invalid input.

### `POST /api/v1/auth/login` — no auth

Request: `{ "username": "alice", "password": "SecurePassword123" }`.
Deactivated users get `401` (same as wrong password — no account-enumeration leak).
Response: same shape as register.

### `GET /api/v1/users/me` — authenticated (any role, must be active)

Returns own `UserResponse`. `401` without/invalid/deactivated token.

## Posts

Post object:

```json
{
  "id": "uuid", "title": "Hello", "slug": "hello", "content": "...",
  "excerpt": null, "published": false,
  "author": { "id": "uuid", "username": "alice" },
  "tags": [{ "id": "uuid", "name": "rust", "slug": "rust" }],
  "comment_count": 2, "created_at": "...", "updated_at": "..."
}
```

List items (`PostSummary`) are the same minus `content`/`updated_at`.

### `GET /api/v1/posts` — optional auth

Query: `?page&per_page&tag&author&search&order` (`tag` matches slug or name,
`author` matches username, `search` case-insensitive substring of title/content,
`order` `desc` default newest-first or `asc` oldest-first).

Visibility (see matrix): anon → `published` only; user → published + own;
moderator/admin → all. Drafts never leak to strangers (`404` on single fetch,
excluded from list).

### `POST /api/v1/posts` — authenticated

Request: `{ "title": "T", "content": "x", "excerpt": "…?", "published": false, "tag_ids": ["uuid"] }`

- `user`: `published` is ignored — always creates a draft (`false`).
- `moderator`/`admin`: `published` is honored — one-shot publish possible.
- Unknown `tag_ids` → `400` (foreign key) or silently ignored per `ON CONFLICT DO NOTHING`
  for the link row; response is the full `PostResponse`.

### `GET /api/v1/posts/{id_or_slug}` — optional auth

Published → anyone. Draft → owner/moderator/admin, everyone else `404`.

### `PUT /api/v1/posts/{id_or_slug}` — owner/moderator/admin

Request: `{ "title"?, "content"?, "excerpt"?, "tag_ids"? }` — **no `published`**
(sending it → `422`). Retitle re-slugifies with dedup (`slug` or `slug-<id8>`).
Replacing `tag_ids` clears and re-attaches. Moderator editing an admin-owned post → `403`.

### `DELETE /api/v1/posts/{id_or_slug}` — owner/moderator/admin

Deletes post (comments cascade per FK). Moderator deleting admin-owned post → `403`.
Success: `{ "message": "Post deleted" }`.

### `POST /api/v1/posts/{id_or_slug}/publish` — moderator/admin

Sets `published=true`, returns full `PostResponse`. `user` → `403`, anon → `401`,
unknown post → `404`, moderator on admin-owned post → `403`. Idempotent.

### `POST /api/v1/posts/{id_or_slug}/unpublish` — moderator/admin

Same as publish with `published=false`. After unpublish, anonymous `GET` returns `404`.

## Comments

Node shape:

```json
{
  "id": "uuid", "content": "hi", "author": { "id": "uuid", "username": "bob" },
  "parent_id": null, "depth": 0, "is_deleted": false,
  "created_at": "...", "children": []
}
```

### `GET /api/v1/posts/{post_id}/comments` — optional auth

Returns the nested tree (chronological `(created_at, id)` order). Visibility mirrors
the post: draft tree requires owner/moderator/admin, else `404`. Published tree is public.

### `POST /api/v1/posts/{post_id}/comments` — authenticated

Request: `{ "content": "hi", "parent_id": "uuid?" }`. Only on **published** posts
(draft/missing → `404`). Unknown `parent_id` → `404`. Depth > 10 → `400`.

### `PUT /api/v1/posts/{post_id}/comments/{comment_id}` — owner/moderator/admin

Request: `{ "content": "edited" }`. Non-owner non-moderator → `403`;
moderator on admin-authored comment → `403`. Success: `{ "message": "Comment updated" }`.

### `DELETE /api/v1/posts/{post_id}/comments/{comment_id}` — owner/moderator/admin

Same guards as edit. Leaf → hard-delete; parent → soft-delete (`[deleted]`).
Success: `{ "message": "Comment deleted" }`.

## Tags

Tag: `{ "id", "name", "slug", "created_at" }`; list adds `post_count`.

### `GET /api/v1/tags`, `GET /api/v1/tags/{id_or_slug}` — public

### `POST /api/v1/tags` — moderator/admin

Request: `{ "name": "rust" }`. `user` → `403`, anon → `401`. Duplicate name → `409`.

### `PUT /api/v1/tags/{id_or_slug}` — moderator/admin

Request: `{ "name": "web-frameworks" }` (re-slugifies). Same guards as create.

### `DELETE /api/v1/tags/{id_or_slug}` — admin only

Moderator → `403`. Success: `{ "message": "Tag deleted" }`.

## Admin

All user endpoints: moderator+ (`401` anon, `403` plain users).
Moderators may list/view (admin accounts masked as `404`), promote
`user→moderator`, and ban/unban `user`-role accounts only. Demotions, `admin`
grants, anything targeting admins, and **all deletions are admin-only**.
Self-targeting → `400`. Unknown ids → `404`. `deactivate`/`activate` are
idempotent (`200` if already in that state).

### `GET /api/v1/admin/users`

Paginated: `?page&per_page&order` (same defaults as posts). Returns
`{ users, total, page, per_page }` including inactive users, newest first.
Moderators see everyone except admins (`total` matches the visible set).

### `GET /api/v1/admin/users/{id}`

Single-user view (inspect before moderating). Moderators get `404` on admin
accounts and unknown ids.

### `PUT /api/v1/admin/users/{id}/role`

Request: `{ "role": "admin|moderator|user" }` (anything else → `400`).
Moderators may only promote `user→moderator`; demotions and `admin` grants →
`403`. Returns updated `UserResponse`.

### `POST /api/v1/admin/users/{id}/deactivate`

Locks the account: login → `401`, live tokens → `401` on next use.
Posts/comments stay visible. Success: `{ "message": "User deactivated" }`.

### `POST /api/v1/admin/users/{id}/activate`

Unlocks the account. Non-expired pre-deactivation tokens become valid again.
Success: `{ "message": "User activated" }`.

### `DELETE /api/v1/admin/users/{id}[?mode=soft|hard]` — admin only

Moderators (and plain users) → `403`. Default `mode=soft`; invalid mode →
`400`; cannot delete yourself or the last admin → `400`.

- `soft`: deactivates, erases PII (`deleted_<id8>` placeholders), and
  re-attributes posts/comments to the system `[deleted]` ghost author.
  Content (incl. slugs/threads) stays visible. Idempotent-ish: repeat call
  succeeds with zero counts.
  ```json
  { "message": "User soft-deleted", "mode": "soft",
    "posts_reassigned": 3, "comments_reassigned": 12 }
  ```
- `hard`: irreversible CASCADE purge of the user row, their posts
  (`post_tags` links go with them) and their comments (nested children via
  `parent_id` cascade). Frees the username/email for re-registration.
  ```json
  { "message": "User hard-deleted", "mode": "hard",
    "posts_deleted": 3, "comments_deleted": 12 }
  ```

## Examples

All examples assume `BASE=localhost:3000/api/v1` and `jq` for parsing.
`$TOKEN` is a user token, `$ADMIN` an admin token (see login below).

### Health

```bash
curl -s localhost:3000/health | jq .
# { "status": "ok", "version": "0.1.0" }

curl -s $BASE/health | jq .
# same payload, versioned path
```

### `POST /auth/register`

```bash
TOKEN=$(curl -s -X POST $BASE/auth/register \
  -H 'Content-Type: application/json' \
  -d '{"username":"alice","email":"alice@test.local","password":"SecurePassword123"}' \
  | tee /dev/stderr | jq -r .token)
# 200 -> { "token": "<jwt>", "token_type": "Bearer",
#          "user": { "id": "uuid", "username": "alice", "role": "user", "is_active": true, ... } }
# 409 username/email taken; 422 invalid input
```

### `POST /auth/login`

```bash
ADMIN=$(curl -s -X POST $BASE/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"Admin@123456"}' | jq -r .token)
# 200 -> same shape as register
# 401 wrong password or deactivated account (no leak which one)
```

### `GET /users/me`

```bash
curl -s $BASE/users/me -H "Authorization: Bearer $TOKEN" | jq .
# 200 -> { "id": "uuid", "username": "alice", "email": "...", "role": "user", "is_active": true, "created_at": "..." }
# 401 without/invalid/deactivated token
```

### `GET /posts` (list)

```bash
# anonymous: published only (?published is ignored)
curl -s "$BASE/posts?search=tokio&per_page=2&page=1" | jq '{total, page, per_page}'
# 200 -> { "posts": [ { "id", "title", "slug", "excerpt", "published", "author": {"id","username"},
#                       "tags": [], "comment_count": 0, "created_at": "..." } ], "total": 5, "page": 1, "per_page": 2 }

# owner sees own drafts too; moderator/admin see all
curl -s "$BASE/posts?author=alice&per_page=100" -H "Authorization: Bearer $TOKEN" | jq .total
curl -s "$BASE/posts?tag=rust" | jq .total   # tag slug or name
```

### `POST /posts` (create)

```bash
# users always create drafts even with "published": true
POST_ID=$(curl -s -X POST $BASE/posts -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"title":"Hello","content":"secret","published":true}' | tee /dev/stderr | jq -r .id)
# 200 -> full PostResponse with "published": false, "slug": "hello", "author": {...}, "tags": [], "comment_count": 0

# moderator/admin may publish in one shot
curl -s -X POST $BASE/posts -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' \
  -d '{"title":"Release notes","content":"...","published":true,"tag_ids":["<tag-uuid>"]}' | jq .published
# -> true
# 401 anon
```

### `GET /posts/{id_or_slug}`

```bash
curl -s $BASE/posts/$POST_ID | jq .published        # 404 while draft (anonymous)
curl -s $BASE/posts/$POST_ID -H "Authorization: Bearer $TOKEN" | jq .published  # false (owner preview)
# moderator/admin also 200 on drafts; strangers 404
curl -s $BASE/posts/hello | jq .id   # slug works the same as uuid once published
```

### `PUT /posts/{id_or_slug}`

```bash
curl -s -X PUT $BASE/posts/$POST_ID -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"title":"Hello v2","tag_ids":[]}' | jq '{title, slug}'
# 200 -> { "title": "Hello v2", "slug": "hello-v2" } (re-slugified, deduped on collision)
# 403 stranger; 403 moderator on admin-owned post; 422 if body contains "published"
```

### `DELETE /posts/{id_or_slug}`

```bash
curl -s -X DELETE $BASE/posts/$POST_ID -H "Authorization: Bearer $TOKEN" | jq .
# 200 -> { "message": "Post deleted" }
# 403 stranger; 403 moderator on admin-owned post; 404 unknown id/slug
```

### `POST /posts/{id}/publish`

```bash
curl -s -X POST $BASE/posts/$POST_ID/publish -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{}' | jq .published
# 200 -> true (full PostResponse); idempotent
# 403 user token; 401 anon; 403 moderator on admin-owned post; 404 unknown
```

### `POST /posts/{id}/unpublish`

```bash
curl -s -X POST $BASE/posts/$POST_ID/unpublish -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{}' | jq .published
# 200 -> false; anonymous GET now 404 again
```

### `GET /posts/{post_id}/comments`

```bash
curl -s $BASE/posts/$POST_ID/comments | jq '. | length'
# 200 -> nested tree; draft post -> 404 unless owner/moderator/admin
# [{ "id", "content", "author": {"id","username"}, "parent_id": null,
#    "depth": 0, "is_deleted": false, "created_at": "...", "children": [...] }]
```

### `POST /posts/{post_id}/comments`

```bash
CID=$(curl -s -X POST $BASE/posts/$POST_ID/comments -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d '{"content":"hi"}' | tee /dev/stderr | jq -r .id)
# 200 -> single node (children: []); draft post -> 404; unknown parent_id -> 404; depth > 10 -> 400

# reply
curl -s -X POST $BASE/posts/$POST_ID/comments -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d "{\"content\":\"reply\",\"parent_id\":\"$CID\"}" | jq .depth
# -> 1
```

### `PUT /posts/{post_id}/comments/{cid}`

```bash
curl -s -X PUT $BASE/posts/$POST_ID/comments/$CID -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d '{"content":"edited"}' | jq .
# 200 -> { "message": "Comment updated" }
# 403 stranger; 403 moderator on admin-authored comment
```

### `DELETE /posts/{post_id}/comments/{cid}`

```bash
curl -s -X DELETE $BASE/posts/$POST_ID/comments/$CID -H "Authorization: Bearer $TOKEN" | jq .
# 200 -> { "message": "Comment deleted" } (leaf: hard-delete; parent: soft-delete -> "[deleted]")
```

### `GET /tags`, `GET /tags/{id_or_slug}`

```bash
curl -s $BASE/tags | jq '.[0]'
# 200 -> [{ "id", "name": "rust", "slug": "rust", "post_count": 3 }]
curl -s $BASE/tags/rust | jq .
# 200 -> { "id", "name", "slug", "created_at": "..." }; 404 unknown
```

### `POST /tags`

```bash
TAG_ID=$(curl -s -X POST $BASE/tags -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{"name":"Rust"}' | tee /dev/stderr | jq -r .id)
# 200 -> { "id", "name": "Rust", "slug": "rust", "created_at": "..." }
# 403 user; 401 anon; 409 duplicate name
```

### `PUT /tags/{id_or_slug}`

```bash
curl -s -X PUT $BASE/tags/$TAG_ID -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{"name":"web-frameworks"}' | jq .slug
# 200 -> "web-frameworks"; same 403/401/404 guards as create
```

### `DELETE /tags/{id_or_slug}`

```bash
curl -s -X DELETE $BASE/tags/$TAG_ID -H "Authorization: Bearer $ADMIN" | jq .
# 200 -> { "message": "Tag deleted" }
# 403 moderator/user; 401 anon
```

### `GET /admin/users`

```bash
curl -s $BASE/admin/users -H "Authorization: Bearer $ADMIN" | jq '.[0]'
# 200 -> [{ "id", "username", "email", "role", "is_active", "created_at" }] (incl. inactive)
# moderators: same minus admin accounts; 401 anon; 403 user
```

### `GET /admin/users/{id}`

```bash
curl -s $BASE/admin/users/$UID -H "Authorization: Bearer $ADMIN" | jq .username
# 200 -> "intern"; moderators get 404 on admin accounts; 404 unknown
```

### `PUT /admin/users/{id}/role`

```bash
UID=$(curl -s -X POST $BASE/auth/register -H 'Content-Type: application/json' \
  -d '{"username":"intern","email":"intern@corp.com","password":"Temp12345"}' | jq -r .user.id)
curl -s -X PUT $BASE/admin/users/$UID/role -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{"role":"moderator"}' | jq .role
# 200 -> "moderator"; 400 bad role or self-target; 404 unknown
# moderators: only user->moderator (demote/grant-admin -> 403)
```

### `POST /admin/users/{id}/deactivate`

```bash
curl -s -X POST $BASE/admin/users/$UID/deactivate -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{}' | jq .
# 200 -> { "message": "User deactivated" } (idempotent)
# login as intern now 401; live tokens 401; posts stay visible
# 400 self-target; 404 unknown
```

### `POST /admin/users/{id}/activate`

```bash
curl -s -X POST $BASE/admin/users/$UID/activate -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{}' | jq .
# 200 -> { "message": "User activated" } (idempotent; non-expired old tokens work again)
```

### `DELETE /admin/users/{id}[?mode=soft|hard]` (admin only)

```bash
curl -s -X DELETE $BASE/admin/users/$UID -H "Authorization: Bearer $ADMIN" | jq .
# 200 -> { "message": "User soft-deleted", "mode": "soft",
#          "posts_reassigned": 1, "comments_reassigned": 2 }
# content stays visible under author "[deleted]"; login now 401
curl -s -X "DELETE $BASE/admin/users/$UID?mode=hard" -H "Authorization: Bearer $ADMIN" | jq .
# 200 -> { "message": "User hard-deleted", "mode": "hard",
#          "posts_deleted": 1, "comments_deleted": 2 }
# posts/comments gone; email reusable; repeat -> 404
# moderators/users -> 403; self/last-admin/bad-mode -> 400
```

### End-to-end publishing flow (all roles)

```bash
# register + draft (users always create drafts)
TOKEN=$(curl -s -X POST $BASE/auth/register \
  -H 'Content-Type: application/json' \
  -d '{"username":"alice","email":"alice@test.local","password":"SecurePassword123"}' | jq -r .token)
POST_ID=$(curl -s -X POST $BASE/posts -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"title":"Hello","content":"secret","published":true}' | jq -r .id)
# -> published:false

# anonymous cannot see the draft
curl -s $BASE/posts/$POST_ID                # 404
curl -s "$BASE/posts?published=false" | jq .total  # drafts excluded

# owner can preview; moderator publishes
curl -s $BASE/posts/$POST_ID -H "Authorization: Bearer $TOKEN" | jq .published  # false
ADMIN=$(curl -s -X POST $BASE/auth/login -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"Admin@123456"}' | jq -r .token)
curl -s -X POST $BASE/posts/$POST_ID/publish -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{}' | jq .published  # true
curl -s $BASE/posts/$POST_ID | jq .published  # true

# comments only on published posts; draft trees mirror post visibility
curl -s -X POST $BASE/posts/$POST_ID/comments -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d '{"content":"hi"}' | jq .id
```
