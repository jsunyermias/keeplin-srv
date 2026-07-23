# `http.rs` — the REST router and handlers

Self-contained companion for `crates/keeplin-srv/src/http.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able to
understand `http.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `http.rs` carries exactly one marker comment of the
form `// md:<Header> > … > <Block header>`, whose path is the header chain of the section
documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**. Small DTO structs
use a compressed layout of the same five points.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::header,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{self, AuthedUser},
    error::AppError,
    permissions::{resolve_note_access, resolve_notebook_access, Capabilities},
    state::AppState,
    store::{Note, NoteShare, NotebookShare, PageCursor, User, UserDevice},
};
```

**What it does** — Builds the axum `Router` and implements every REST/JSON handler:
accounts and devices, email flows, notes CRUD with materialised bodies, sharing and
ownership transfer (notes and notebooks), history, import/export, the read side of the
server-materialised domain entities (notebooks/tags/resources), resource binaries, and
the operational endpoints (`/health`, `/ready`, `/version`, `/api/metrics`). Also owns
the protocol-compatibility constants (`PROTOCOL_VERSION`, `CAPABILITIES`).

**Dependencies** — `axum`, `serde`, `serde_json`, `uuid`, `chrono`, `tracing`
(external). Internal: `auth.rs` (middleware, tokens, hashing), `error.rs` (`AppError`),
`permissions.rs` (access resolution + `Capabilities`), `state.rs` (`AppState`),
`store.rs` (every query + the row/cursor types), `mail.rs` (flow mail), `collab.rs` /
`sync.rs` (the two WebSocket handlers mounted here), `ratelimit.rs` (the middleware).

**Used by** — `main.rs` serves `router(state)`; every integration test spawns the same
router in-process. `collab.rs` reuses nothing from here (the reverse edge in the graph
is via shared store types).

**Repeated context** — Handler conventions repeated throughout this file: authorisation
is resolved **in the handler before any data access** via `resolve_note_access` /
`resolve_notebook_access` (single choke points — no handler rolls its own checks);
errors are `AppError` (uniform `{"error": …}` bodies; internal detail never leaks —
issue #46); every protected handler takes `user: AuthedUser` (inserted by `auth_mw`,
which also enforces device-revocation); note bodies are **derived**, never stored;
deletes are **soft** (tombstones); quotas and size caps are enforced before
allocation/storage.

---

## MAX_PAGE_LIMIT

**Identification** — const; marker `// md:MAX_PAGE_LIMIT`.
`const MAX_PAGE_LIMIT: i64 = 500;`

**Code** — complete and verbatim:

```rust
// md:MAX_PAGE_LIMIT
const MAX_PAGE_LIMIT: i64 = 500;
```

**What it does** — Hard ceiling on `?limit=` so a client cannot ask for an unbounded
page and defeat pagination (issue #29).

**Dependencies** — none. **Used by** — `ListQuery::resolve`.

**Repeated context** — Pagination (issue #29) is opt-in: omitting `limit` returns every
row (back-compatible); this cap only bounds explicit requests.

---

## ListQuery

**Identification** — struct; marker `// md:ListQuery`.

**Code** — complete and verbatim:

```rust
// md:ListQuery
#[derive(Debug, Deserialize)]
struct ListQuery {
    limit: Option<i64>,
    cursor: Option<String>,
}
```

**What it does** — The query string shared by the paginated list endpoints
(`?limit=&cursor=`). Both optional — omitting `limit` returns every row
(back-compatible with pre-pagination clients).

**Dependencies** — serde. **Used by** — `list_notes`, `list_notebooks`, `list_tags`,
`list_resources`.

**Repeated context** — see *fn paginated* for the full pagination contract.

---

## impl ListQuery

**Identification** — impl block; marker `// md:impl ListQuery`. Contains `fn resolve`.

**Code** — container: members documented as sub-blocks below: fn resolve.

**What it does / Dependencies / Used by / Repeated context** — see `fn resolve`.

### fn resolve

**Identification** — method; marker `// md:impl ListQuery > fn resolve`.
`fn resolve(&self) -> Result<(Option<i64>, Option<PageCursor>), AppError>`.

**Code** — complete and verbatim:

```rust
    // md:impl ListQuery > fn resolve
    fn resolve(&self) -> Result<(Option<i64>, Option<PageCursor>), AppError> {
        let limit = self.limit.map(|l| l.clamp(1, MAX_PAGE_LIMIT));
        let cursor = match self.cursor.as_deref() {
            Some(token) => Some(
                PageCursor::decode(token)
                    .ok_or_else(|| AppError::BadRequest("invalid cursor".into()))?,
            ),
            None => None,
        };
        Ok((limit, cursor))
    }
```

**What it does** — Clamps the requested limit to `[1, MAX_PAGE_LIMIT]` (or `None` for
"all") and decodes the opaque cursor; a malformed cursor is `400 BadRequest`.

**Dependencies** — `PageCursor::decode` (`store.rs`). **Used by** — the four list
handlers.

**Repeated context** — The cursor format (`"<micros>_<uuid>"`) is owned by
`store::PageCursor`; handlers treat it as opaque.

---

## fn paginated

**Identification** — function; marker `// md:fn paginated`.

**Code** — complete and verbatim:

```rust
// md:fn paginated
fn paginated<T: Serialize>(
    items: Vec<T>,
    limit: Option<i64>,
    cursor_of: impl Fn(&T) -> PageCursor,
) -> Response {
    let next = match limit {
        Some(l) if items.len() as i64 >= l => items.last().map(|it| cursor_of(it).encode()),
        _ => None,
    };
    let mut resp = Json(items).into_response();
    if let Some(token) = next {
        if let Ok(value) = token.parse() {
            resp.headers_mut().insert("x-next-cursor", value);
        }
    }
    resp
}
```

**What it does** — Builds a list response: the JSON array (shape unchanged — always a
bare array, so pre-pagination clients keep working) plus an **`X-Next-Cursor`** header
when a full page was returned, so a paging client knows to ask for more. `limit ==
None` (unpaginated) or a short page → no header — the list is complete. The cursor is
derived from the last item via `cursor_of` and drives **keyset** paging on
`(created_at, id)` (or `(updated_at, id)` for notes) in the store, so deep pages stay
cheap and stable under concurrent inserts.

**Dependencies** — `PageCursor` (`store.rs`), axum/serde. **Used by** — the four list
handlers.

**Repeated context** — Pagination contract (issue #29), in full: body always a bare
array; `X-Next-Cursor` present iff more may exist; re-request with `cursor=<value>`;
absence of the header = exhausted; malformed cursor = 400; `limit` capped at 500.

---

## fn router

**Identification** — public function; marker `// md:fn router`.
`pub fn router(state: Arc<AppState>) -> Router`.

**Code** — complete and verbatim:

```rust
// md:fn router
pub fn router(state: Arc<AppState>) -> Router {
    let resource_data = Router::new()
        .route(
            "/api/resources/:id/data",
            get(get_resource_data).put(put_resource_data),
        )
        .layer(DefaultBodyLimit::max(state.config.max_upload_bytes));

    let protected = Router::new()
        .route("/api/metrics", get(metrics))
        .route(
            "/api/devices",
            post(create_device)
                .get(list_devices)
                .delete(delete_all_devices),
        )
        .route("/api/devices/:id", axum::routing::delete(delete_device))
        .route("/api/account/password", post(change_password))
        .route("/api/account", axum::routing::delete(delete_account))
        .route("/api/account/verify/request", post(verify_request))
        .route("/api/notes", post(create_note).get(list_notes))
        .route(
            "/api/notes/:id",
            get(get_note).patch(update_note).delete(delete_note),
        )
        .route("/api/notes/:id/share", post(create_share).get(list_shares))
        .route(
            "/api/notes/:id/share/:user_id",
            axum::routing::delete(delete_share),
        )
        .route("/api/notes/:id/transfer", post(transfer_ownership))
        .route("/api/notes/:id/history", get(note_history))
        .route("/api/notes/:id/export", get(export_note))
        .route("/api/import", post(import_note))
        .route("/api/notebooks", get(list_notebooks))
        .route(
            "/api/notebooks/:id/share",
            post(create_notebook_share).get(list_notebook_shares),
        )
        .route(
            "/api/notebooks/:id/share/:user_id",
            axum::routing::delete(delete_notebook_share),
        )
        .route("/api/notebooks/:id/transfer", post(transfer_notebook))
        .route("/api/notebooks/:id/history", get(notebook_history))
        .route("/api/tags", get(list_tags))
        .route("/api/resources", get(list_resources))
        .route("/api/notes/:id/tags", get(list_note_tags))
        .merge(resource_data)
        .layer(middleware::from_fn_with_state(state.clone(), auth::auth_mw));

    let limited = Router::new()
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .route("/api/account/verify/confirm", post(verify_confirm))
        .route("/api/account/reset/request", post(reset_request))
        .route("/api/account/reset/confirm", post(reset_confirm))
        .merge(protected)
        .route("/api/ws", get(crate::collab::handler))
        .route("/api/sync", get(crate::sync::handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::ratelimit::rate_limit_mw,
        ));

    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/version", get(version))
        .merge(limited)
        .with_state(state)
}
```

**What it does** — Assembles the three-layer router:

```
/health   (get) — liveness (unauthenticated, NOT rate-limited)
/ready    (get) — readiness: DB round-trip, 503 if down (unauthenticated, not limited)
/version  (get) — protocol version + capabilities (unauthenticated, not limited)
── everything below is rate-limited (per-IP, ratelimit.rs) ──
/api/register                   (post)
/api/login                      (post) — returns { token, device_id }
/api/account/verify/confirm     (post) — unauth: the token is the proof
/api/account/reset/request      (post) — unauth by nature
/api/account/reset/confirm      (post) — unauth: the token is the proof
── everything below also requires auth_mw (Bearer token + live device) ──
/api/metrics                    (get)  — aggregate counters (auth required, issue #22)
/api/devices                    (post|get|delete) — add / list / revoke ALL (issue #31)
/api/devices/:id                (delete) — revoke one device
/api/account/password           (post) — change password (needs current)
/api/account                    (delete) — delete account + everything owned
/api/account/verify/request     (post) — (re)send verification mail
/api/notes                      (post|get)
/api/notes/:id                  (get|patch|delete)
/api/notes/:id/share            (post|get);  /api/notes/:id/share/:user_id (delete)
/api/notes/:id/transfer         (post)
/api/notes/:id/history          (get)  — per-entity history (issue #27)
/api/notes/:id/export           (get);  /api/import (post)
/api/notebooks                  (get)  — materialised read side
/api/notebooks/:id/share        (post|get);  …/share/:user_id (delete) — cascades
/api/notebooks/:id/transfer     (post);  /api/notebooks/:id/history (get)
/api/tags                       (get);  /api/resources (get)
/api/notes/:id/tags             (get)
/api/resources/:id/data         (get|put) — raised body limit (MAX_UPLOAD_BYTES)
── WebSocket surfaces (auth inside each handler) ──
/api/ws     (get) — collaborative channel (collab.rs)
/api/sync   (get) — device relay (sync.rs)
```

The resource-data sub-router carries `DefaultBodyLimit::max(max_upload_bytes)`;
JSON routes keep axum's small default. `/api/metrics` sits behind auth (issue #22 —
aggregate counters are operational reconnaissance). The probes and `/version` sit
outside the limiter so orchestrator probes and the protocol handshake are never
throttled.

**Dependencies** — every handler in this file; `auth::auth_mw` (`auth.rs`);
`ratelimit::rate_limit_mw` (`ratelimit.rs`); `collab::handler`, `sync::handler`.

**Used by** — `main.rs`; every test harness (`spawn_server*` helpers).

**Repeated context** — The server must be served with
`into_make_service_with_connect_info::<SocketAddr>()` (the rate limiter keys on the
peer IP); `main.rs` and all test spawns do this.

---

## PROTOCOL_VERSION

**Identification** — public const; marker `// md:PROTOCOL_VERSION`.
`pub const PROTOCOL_VERSION: u32 = 1;`

**Code** — complete and verbatim:

```rust
// md:PROTOCOL_VERSION
pub const PROTOCOL_VERSION: u32 = 1;
```

**What it does** — The wire-protocol version the server speaks. Bump on a **breaking**
change to the relay/collab message shapes so a client can detect an incompatible
server at connect (issues #39/#114). Mirrored by keeplin-core's `src/compat.rs`
(`PROTOCOL_VERSION` + `compatible_with`), enforced client-side at `DbBackend::new` /
`CollabBackend::start`: an incompatible answer fails the client's startup loudly and
no sync is attempted; a missing `/version` (old server) is a client-side warning.
Procedure on bump: bump both constants together, then bump the keeplin-core `rev`
pinned in this repo's `Cargo.toml` and run this test suite — it drives the real client
against this server, so drift fails here, not in production.

**Dependencies** — none. **Used by** — `compatible_with`, `version`, `mod tests`.

**Repeated context** — Additive evolution (new endpoints/fields) goes through
`CAPABILITIES`, not a version bump.

---

## fn compatible_with

**Identification** — public function; marker `// md:fn compatible_with`.
`pub fn compatible_with(client_protocol: u32) -> bool`.

**Code** — complete and verbatim:

```rust
// md:fn compatible_with
pub fn compatible_with(client_protocol: u32) -> bool {
    client_protocol == PROTOCOL_VERSION
}
```

**What it does** — The compatibility rule, defined once per repo and mirrored in
keeplin-core's `compat::compatible_with`: **exact match**. Capabilities cover additive
evolution, so a version bump is reserved for breaking changes.

**Dependencies** — `PROTOCOL_VERSION`. **Used by** — keeplin-core's mirror (the
contract consumer); `mod tests` pins it.

**Repeated context** — none.

---

## CAPABILITIES

**Identification** — const; marker `// md:CAPABILITIES`.

**Code** — complete and verbatim:

```rust
// md:CAPABILITIES
const CAPABILITIES: &[&str] = &[
    "history",
    "history_visibility",
    "resource_purge",
    "readiness",
    "account_management",
    "pagination",
    "email_flows",
];
```

**What it does** — Feature flags a client can probe to branch behaviour instead of
guessing (e.g. skip the history endpoint on a server that lacks it). Additive: new
capabilities are appended, never removed/renamed. Current set: `history`
(`GET /api/{notes,notebooks}/:id/history`), `history_visibility`
(`HISTORY_VISIBILITY` policy, issue #27), `resource_purge` (server-side deleted-blob
purge, issue #24), `readiness` (`GET /ready`, issue #36), `account_management`
(password change + sign-out-everywhere + deletion, issue #31), `pagination`
(`?limit=&cursor=` + `X-Next-Cursor`, issue #29), `email_flows` (verification + reset
via the mail webhook, issue #49; endpoints answer 501 when unconfigured).

**Dependencies** — none. **Used by** — `version`.

**Repeated context** — none.

---

## fn version

**Identification** — handler; marker `// md:fn version`.
`async fn version() -> Json<serde_json::Value>`.

**Code** — complete and verbatim:

```rust
// md:fn version
async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "keeplin-srv",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": PROTOCOL_VERSION,
        "capabilities": CAPABILITIES,
    }))
}
```

**What it does** — `GET /version`: the unauthenticated capability/version handshake —
`{ name, version (crate), protocol_version, capabilities[] }` — so a client negotiates
behaviour without guessing (issues #39/#114). Never rate-limited.

**Dependencies** — `PROTOCOL_VERSION`, `CAPABILITIES`. **Used by** — routed in
`router`; consumed by keeplin-core's handshake.

**Repeated context** — none.

---

## fn health

**Identification** — handler; marker `// md:fn health`.
`async fn health() -> &'static str`.

**Code** — complete and verbatim:

```rust
// md:fn health
async fn health() -> &'static str {
    "ok"
}
```

**What it does** — `GET /health`, liveness: the process is up. Returns the literal
`"ok"`; cheap and dependency-free, so an orchestrator never restarts a healthy process
just because the database blipped. Never rate-limited.

**Dependencies** — none. **Used by** — routed in `router`; orchestrator probes.

**Repeated context** — Liveness vs readiness split (issue #36): `/health` = process
up; `/ready` = can actually serve.

---

## fn ready

**Identification** — handler; marker `// md:fn ready`.
`async fn ready(State(state)) -> impl IntoResponse`.

**Code** — complete and verbatim:

```rust
// md:fn ready
async fn ready(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.store.ping().await {
        Ok(()) => (axum::http::StatusCode::OK, "ready"),
        Err(e) => {
            tracing::warn!(error = %e, "readiness check failed");
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "database unavailable",
            )
        }
    }
}
```

**What it does** — `GET /ready`, readiness: a lightweight database round-trip
(`store.ping`); `200 ready`, or `503 database unavailable` (logged) so a load
balancer stops routing to an instance that would only error (issue #36). Never
rate-limited.

**Dependencies** — `Store::ping` (`store.rs`). **Used by** — routed in `router`;
orchestrator readiness probes; the Docker `HEALTHCHECK`.

**Repeated context** — as `fn health`.

---

## MetricsQuery

**Identification** — struct; marker `// md:MetricsQuery`.
`struct MetricsQuery { format: Option<String> }` — the `?format=` selector for
`metrics`. **Dependencies** serde; **Used by** `metrics`; **Repeated context** none.

**Code** — complete and verbatim:

```rust
// md:MetricsQuery
#[derive(Debug, Deserialize)]
struct MetricsQuery {
    format: Option<String>,
}
```

---

## fn metrics

**Identification** — handler; marker `// md:fn metrics`.

**Code** — complete and verbatim:

```rust
// md:fn metrics
async fn metrics(
    State(state): State<Arc<AppState>>,
    Query(q): Query<MetricsQuery>,
) -> Result<Response, AppError> {
    let (users, notes, lines, tombstones) = state.store.counts().await?;
    let (collab_sessions, collab_connections) = state.collab.stats().await;
    let relay_users = state.hub.live_users().await;

    if q.format.as_deref() == Some("prometheus") {
        let body = format!(
            "# HELP keeplin_users Registered accounts (shared across replicas).\n\
             # TYPE keeplin_users gauge\n\
             keeplin_users {users}\n\
             # HELP keeplin_notes Live notes (shared across replicas).\n\
             # TYPE keeplin_notes gauge\n\
             keeplin_notes {notes}\n\
             # HELP keeplin_lines Live note lines (shared across replicas).\n\
             # TYPE keeplin_lines gauge\n\
             keeplin_lines {lines}\n\
             # HELP keeplin_line_tombstones Soft-deleted lines awaiting GC (shared across replicas).\n\
             # TYPE keeplin_line_tombstones gauge\n\
             keeplin_line_tombstones {tombstones}\n\
             # HELP keeplin_collab_sessions Live collaborative note sessions on this instance.\n\
             # TYPE keeplin_collab_sessions gauge\n\
             keeplin_collab_sessions {collab_sessions}\n\
             # HELP keeplin_collab_connections Live collaborative connections on this instance.\n\
             # TYPE keeplin_collab_connections gauge\n\
             keeplin_collab_connections {collab_connections}\n\
             # HELP keeplin_relay_live_users Users with a live relay connection on this instance.\n\
             # TYPE keeplin_relay_live_users gauge\n\
             keeplin_relay_live_users {relay_users}\n"
        );
        return Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response());
    }

    Ok(Json(serde_json::json!({
        "users": users,
        "notes": notes,
        "lines": lines,
        "line_tombstones": tombstones,
        "collab_sessions": collab_sessions,
        "collab_connections": collab_connections,
        "relay_live_users": relay_users,
    }))
    .into_response())
}
```

**What it does** — `GET /api/metrics` (authenticated — issue #22): aggregate
operational counters, **no per-user data**: `users`, `notes`, `lines`,
`line_tombstones` (row counts from the shared database — identical on every replica)
plus `collab_sessions`, `collab_connections`, `relay_live_users` (**per-instance**
live gauges — in a multi-replica deployment scrape every replica and sum; issue #45).
JSON by default; `?format=prometheus` renders the Prometheus text exposition format
(v0.0.4) so a scraper consumes it directly (configure the scrape job with the bearer
token).

**Dependencies** — `Store::counts`, `CollabRegistry::stats` (`collab.rs`),
`SyncHub::live_users` (`sync.rs`).

**Used by** — routed in `router` (protected group).

**Repeated context** — Metrics moved behind auth in issue #22: deployment size and
live-activity signal are reconnaissance a public service must not hand out
anonymously.

---

## fn normalize_email

**Identification** — function; marker `// md:fn normalize_email`.
`fn normalize_email(email: &str) -> String`.

**Code** — complete and verbatim:

```rust
// md:fn normalize_email
fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}
```

**What it does** — Canonicalises an email for storage and lookup: trim + lowercase,
so `John@X.com`, `john@x.com` and `  john@x.com ` are one account and login is
case-insensitive (issue #43).

**Dependencies** — none. **Used by** — `register`, `login`, `create_share`,
`transfer_ownership`, `resolve_target`, `reset_request`.

**Repeated context** — Every email that reaches the store passes through here — the
`users.email` column only ever holds normalised addresses.

---

## fn is_valid_email

**Identification** — function; marker `// md:fn is_valid_email`.
`fn is_valid_email(email: &str) -> bool`.

**Code** — complete and verbatim:

```rust
// md:fn is_valid_email
fn is_valid_email(email: &str) -> bool {
    let mut parts = email.split('@');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(local), Some(domain), None) => {
            !local.is_empty()
                && domain.len() >= 3
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        }
        _ => false,
    }
}
```

**What it does** — Minimal structural check — exactly one `@`, a non-empty local
part, a dotted domain (≥ 3 chars, not starting/ending with `.`). Deliberately not
RFC-complete: it only rejects input that is obviously not an email so the `email`
column actually holds addresses.

**Dependencies** — none. **Used by** — `register` only (login deliberately does
**not** validate — see `fn login`).

**Repeated context** — none.

---

## RegisterBody

**Identification** — DTO struct; marker `// md:RegisterBody`.
`struct RegisterBody { email, password, display_name: Option<String> }` —
`display_name` is shown to other participants in collaborative sessions; defaults to
the part of the email before the `@`. **Dependencies** serde; **Used by** `register`;
**Repeated context** none.

**Code** — complete and verbatim:

```rust
// md:RegisterBody
#[derive(Debug, Deserialize)]
struct RegisterBody {
    email: String,
    password: String,
    display_name: Option<String>,
}
```

---

## RegisterResponse

**Identification** — DTO struct; marker `// md:RegisterResponse`.
`struct RegisterResponse { user: User }` — the created account row (no token:
registering does not log in). **Dependencies** `User` (`store.rs`); **Used by**
`register`; **Repeated context** none.

**Code** — complete and verbatim:

```rust
// md:RegisterResponse
#[derive(Debug, serde::Serialize)]
struct RegisterResponse {
    user: User,
}
```

---

## fn register

**Identification** — handler; marker `// md:fn register`.

**Code** — complete and verbatim:

```rust
// md:fn register
async fn register(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RegisterBody>,
) -> Result<Json<RegisterResponse>, AppError> {
    if !state.config.registration_enabled {
        return Err(AppError::Forbidden);
    }
    if body.password.len() < 8 {
        return Err(AppError::BadRequest("password too short".into()));
    }
    let email = normalize_email(&body.email);
    if !is_valid_email(&email) {
        return Err(AppError::BadRequest("invalid email".into()));
    }
    let display_name = body
        .display_name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| email.split('@').next().unwrap_or_default().to_string());
    let hash = auth::hash_password(&body.password)?;
    let user = state
        .store
        .create_user(&email, &hash, &display_name)
        .await?;
    if state.mailer.enabled() {
        if let Err(e) = send_flow_mail(&state, &user, crate::mail::MailKind::VerifyEmail).await {
            tracing::error!(error = %e, "verification mail on register failed");
        }
    }
    Ok(Json(RegisterResponse { user }))
}
```

**What it does** — `POST /api/register`. Gate: `REGISTRATION_ENABLED=false` → `403`
(a private/single-tenant deployment closes signups — issue #21). Validation: password
≥ 8 chars (`400`); email normalised then structurally validated (`400`).
`display_name` defaults from the email local part. Hashes the password (Argon2id) and
creates the user — a duplicate email is `409 Conflict` (unique-violation mapping in
the store). If the mailer is configured, kicks off the verification mail
**best-effort**: a webhook hiccup is logged but must not fail the registration — the
user can re-request via `POST /api/account/verify/request`.

**Dependencies** — `normalize_email`, `is_valid_email`, `send_flow_mail` (this
file); `auth::hash_password`; `Store::create_user`; `Mailer::enabled` (`mail.rs`).

**Used by** — routed in `router` (rate-limited, unauthenticated).

**Repeated context** — Anti-enumeration nuance: registration's `409` on a duplicate
email is a known, accepted disclosure (issue #32 documents the trade-off); login is
the surface that stays oracle-free.

---

## LoginBody

**Identification** — DTO struct; marker `// md:LoginBody`.
`struct LoginBody { email, password, device_name }`. **Dependencies** serde;
**Used by** `login`; **Repeated context** none.

**Code** — complete and verbatim:

```rust
// md:LoginBody
#[derive(Debug, Deserialize)]
struct LoginBody {
    email: String,
    password: String,
    device_name: String,
}
```

---

## LoginResponse

**Identification** — DTO struct; marker `// md:LoginResponse`.
`struct LoginResponse { token: String, device_id: Uuid }` — the **device token**:
pasted into keeplin-daemon's `auth_token` config field. One login (one token) per
device — the relay uses the device identity inside the token to know what each device
has already received. **Dependencies** uuid/serde; **Used by** `login`;
**Repeated context** device-as-actor (see `auth.md` context).

**Code** — complete and verbatim:

```rust
// md:LoginResponse
#[derive(Debug, serde::Serialize)]
struct LoginResponse {
    token: String,
    device_id: Uuid,
}
```

---

## fn login

**Identification** — handler; marker `// md:fn login`.

**Code** — complete and verbatim:

```rust
// md:fn login
async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginBody>,
) -> Result<Json<LoginResponse>, AppError> {
    let email = normalize_email(&body.email);

    let lockout_enabled = state.config.login_max_failures > 0;
    if lockout_enabled && state.store.login_locked(&email).await? {
        return Err(AppError::TooManyAttempts);
    }
    let record_failure = || async {
        if lockout_enabled {
            state
                .store
                .record_login_failure(
                    &email,
                    state.config.login_max_failures,
                    state.config.login_lockout_secs,
                )
                .await?;
        }
        Ok::<(), AppError>(())
    };

    let user = match state.store.get_user_by_email(&email).await? {
        Some(user) => user,
        None => {
            let _ = auth::verify_password(&body.password, auth::dummy_password_hash());
            record_failure().await?;
            return Err(AppError::InvalidToken);
        }
    };

    if !auth::verify_password(&body.password, &user.password_hash)? {
        record_failure().await?;
        return Err(AppError::InvalidToken);
    }

    if state.config.email_verification_required && user.email_verified_at.is_none() {
        return Err(AppError::BadRequest("email not verified".into()));
    }

    if lockout_enabled {
        state.store.clear_login_failures(&email).await?;
    }

    let device = state
        .store
        .create_device(user.id, &body.device_name)
        .await?;

    let token = auth::create_token(
        user.id,
        device.id,
        &user.email,
        &state.config.jwt_secret,
        state.config.token_ttl_days,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(LoginResponse {
        token,
        device_id: device.id,
    }))
}
```

**What it does** — `POST /api/login`, in order:

1. Normalise the email (case-insensitive login, issue #43). **No format
   validation** here: an unknown/malformed address must still run the dummy hash
   below, or the format check itself becomes an enumeration oracle (issue #32).
2. **Brute-force lockout** (`LOGIN_MAX_FAILURES > 0`): refuse with `429
   TooManyAttempts` before touching the password once the email has accumulated too
   many recent failures. DB-backed (`login_attempts` table, migration 0011) so it
   holds across replicas, and keyed by the **submitted** email whether or not an
   account exists — the 429 is uniform and reveals nothing.
3. Look up the user. Absent → verify the password against `dummy_password_hash()`
   so a missing account costs the same Argon2 work as a wrong password (timing
   oracle closed, issue #32), record a failure, return `401 InvalidToken` — the
   same error as a wrong password.
4. Verify the password; wrong → record failure, `401`.
5. `EMAIL_VERIFICATION_REQUIRED` and unverified → `400 email not verified` —
   checked only **after** the password succeeded, so it reveals nothing to a caller
   without the credentials (issue #49).
6. Success: clear the email's failure history, create the device row
   (`user_devices`), mint the JWT (`TOKEN_TTL_DAYS`), return `{token, device_id}`.

**Dependencies** — `normalize_email` (this file); `auth::{verify_password,
dummy_password_hash, create_token}`; `Store::{login_locked, record_login_failure,
get_user_by_email, clear_login_failures, create_device}`.

**Used by** — routed in `router` (rate-limited, unauthenticated).

**Repeated context** — Uniform-failure discipline (issue #32): unknown email and
wrong password are indistinguishable in status, body **and** timing. The device row
created here is the anchor of revocation: deleting it kills the token on every
surface.

---

## CreateDeviceBody

**Identification** — DTO struct; marker `// md:CreateDeviceBody`.
`struct CreateDeviceBody { device_name }`. **Used by** `create_device`; otherwise
trivial.

**Code** — complete and verbatim:

```rust
// md:CreateDeviceBody
#[derive(Debug, Deserialize)]
struct CreateDeviceBody {
    device_name: String,
}
```

---

## CreateDeviceResponse

**Identification** — DTO struct; marker `// md:CreateDeviceResponse`.
`struct CreateDeviceResponse { token, device_id, device_name }`. **Used by**
`create_device`; otherwise trivial.

**Code** — complete and verbatim:

```rust
// md:CreateDeviceResponse
#[derive(Debug, serde::Serialize)]
struct CreateDeviceResponse {
    token: String,
    device_id: Uuid,
    device_name: String,
}
```

---

## fn create_device

**Identification** — handler; marker `// md:fn create_device`.

**Code** — complete and verbatim:

```rust
// md:fn create_device
async fn create_device(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<CreateDeviceBody>,
) -> Result<Json<CreateDeviceResponse>, AppError> {
    let device = state
        .store
        .create_device(user.user_id, &body.device_name)
        .await?;
    let token = auth::create_token(
        user.user_id,
        device.id,
        &user.email,
        &state.config.jwt_secret,
        state.config.token_ttl_days,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(CreateDeviceResponse {
        token,
        device_id: device.id,
        device_name: device.device_name,
    }))
}
```

**What it does** — `POST /api/devices` (authenticated): register an additional
device for the caller and return its own token — equivalent to a fresh login without
re-sending the password.

**Dependencies** — `Store::create_device`, `auth::create_token`.
**Used by** — routed in `router`.

**Repeated context** — One token per device; a user with two devices has two tokens
and two relay cursors.

---

## fn delete_device

**Identification** — handler; marker `// md:fn delete_device`.

**Code** — complete and verbatim:

```rust
// md:fn delete_device
async fn delete_device(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.store.delete_device(id, user.user_id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}
```

**What it does** — `DELETE /api/devices/:id`: revoke one of the **caller's** devices
(`404` if it isn't theirs). Its token stops working immediately on REST and on both
WebSocket channels — the revocation checks re-read the device row per
request/handshake.

**Dependencies** — `Store::delete_device`. **Used by** — routed in `router`.

**Repeated context** — The crate-wide revocation invariant (see `auth.md`); pruning
interaction: a deleted device also stops blocking journal pruning (issue #23).

---

## fn list_devices

**Identification** — handler; marker `// md:fn list_devices`.

**Code** — complete and verbatim:

```rust
// md:fn list_devices
async fn list_devices(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<Vec<UserDevice>>, AppError> {
    let devices = state.store.list_devices_by_user(user.user_id).await?;
    Ok(Json(devices))
}
```

**What it does** — `GET /api/devices`: the caller's device rows (ids, names,
`created_at`/`last_seen_at`).

**Dependencies** — `Store::list_devices_by_user`. **Used by** — routed in `router`.

**Repeated context** — none.

---

## fn delete_all_devices

**Identification** — handler; marker `// md:fn delete_all_devices`.

**Code** — complete and verbatim:

```rust
// md:fn delete_all_devices
async fn delete_all_devices(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let removed = state.store.delete_all_devices(user.user_id).await?;
    Ok(Json(serde_json::json!({ "ok": true, "revoked": removed })))
}
```

**What it does** — `DELETE /api/devices`: revoke **all** the caller's devices —
"sign out everywhere" (issue #31). Every token, including the caller's current one,
stops working immediately. Returns the revoked count.

**Dependencies** — `Store::delete_all_devices`. **Used by** — routed in `router`;
also called internally by `reset_confirm`.

**Repeated context** — none.

---

## ChangePasswordBody

**Identification** — DTO struct; marker `// md:ChangePasswordBody`.
`struct ChangePasswordBody { current_password, new_password }`. **Used by**
`change_password`; otherwise trivial.

**Code** — complete and verbatim:

```rust
// md:ChangePasswordBody
#[derive(Debug, Deserialize)]
struct ChangePasswordBody {
    current_password: String,
    new_password: String,
}
```

---

## fn change_password

**Identification** — handler; marker `// md:fn change_password`.

**Code** — complete and verbatim:

```rust
// md:fn change_password
async fn change_password(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<ChangePasswordBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.new_password.len() < 8 {
        return Err(AppError::BadRequest("password too short".into()));
    }
    let stored = state
        .store
        .get_user_by_id(user.user_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if !auth::verify_password(&body.current_password, &stored.password_hash)? {
        return Err(AppError::InvalidToken);
    }
    let hash = auth::hash_password(&body.new_password)?;
    state.store.update_password(user.user_id, &hash).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
```

**What it does** — `POST /api/account/password` (issue #31): min 8-char new
password; re-verifies the **current** password (a stolen token alone cannot rotate
credentials); stores the new Argon2 hash. Existing device tokens remain valid (they
are JWTs) — call `DELETE /api/devices` afterwards to also sign out everywhere.

**Dependencies** — `auth::{verify_password, hash_password}`;
`Store::{get_user_by_id, update_password}`. **Used by** — routed in `router`.

**Repeated context** — Sensitive-action re-authentication is the pattern shared with
`delete_account`.

---

## DeleteAccountBody

**Identification** — DTO struct; marker `// md:DeleteAccountBody`.
`struct DeleteAccountBody { password }`. **Used by** `delete_account`; otherwise
trivial.

**Code** — complete and verbatim:

```rust
// md:DeleteAccountBody
#[derive(Debug, Deserialize)]
struct DeleteAccountBody {
    password: String,
}
```

---

## fn delete_account

**Identification** — handler; marker `// md:fn delete_account`.

**Code** — complete and verbatim:

```rust
// md:fn delete_account
async fn delete_account(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<DeleteAccountBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let stored = state
        .store
        .get_user_by_id(user.user_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if !auth::verify_password(&body.password, &stored.password_hash)? {
        return Err(AppError::InvalidToken);
    }
    state.store.delete_user(user.user_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
```

**What it does** — `DELETE /api/account` (issue #31): re-verifies the password, then
deletes the user row; every owned entity (devices, notes, notebooks, tags,
resources, shares, journal) **cascades away in the database** — irreversible. This
is the one deliberate exception to soft-delete: account deletion is a privacy
action, not a replicated edit.

**Dependencies** — `auth::verify_password`; `Store::{get_user_by_id, delete_user}`.
**Used by** — routed in `router`.

**Repeated context** — none.

---

## fn send_flow_mail

**Identification** — helper; marker `// md:fn send_flow_mail`.

**Code** — complete and verbatim:

```rust
// md:fn send_flow_mail
async fn send_flow_mail(
    state: &AppState,
    user: &User,
    kind: crate::mail::MailKind,
) -> Result<(), AppError> {
    let (token, expires_at) = state
        .store
        .create_email_token(user.id, kind, state.config.email_token_ttl_secs)
        .await?;
    state
        .mailer
        .send(kind, &user.email, &user.display_name, &token, expires_at)
        .await
        .map_err(AppError::Internal)
}
```

**What it does** — Mints a single-use flow token for `user` (the store keeps only
its **hash**, with `EMAIL_TOKEN_TTL_SECS` expiry) and hands the raw token to the
mail webhook (`Mailer::send`); a delivery failure maps to `AppError::Internal`.

**Dependencies** — `Store::create_email_token`; `Mailer::send` (`mail.rs`).
**Used by** — `register`, `verify_request`, `reset_request`.

**Repeated context** — The email-flow token model (issue #49): server stores a
hash, the user proves receipt by presenting the raw token back; kind-scoped
(verify ≠ reset); expired/used tokens are pruned daily by the maintenance loop.

---

## fn verify_request

**Identification** — handler; marker `// md:fn verify_request`.

**Code** — complete and verbatim:

```rust
// md:fn verify_request
async fn verify_request(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.mailer.enabled() {
        return Err(AppError::NotImplemented(
            "mail webhook not configured".into(),
        ));
    }
    let stored = state
        .store
        .get_user_by_id(user.user_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if stored.email_verified_at.is_some() {
        return Ok(Json(
            serde_json::json!({ "ok": true, "already_verified": true }),
        ));
    }
    send_flow_mail(&state, &stored, crate::mail::MailKind::VerifyEmail).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
```

**What it does** — `POST /api/account/verify/request` (authenticated): (re)send the
caller's verification email. `501 NotImplemented` when no mail webhook is
configured (explicit deferral); short-circuits with `already_verified: true` when
the address is already stamped.

**Dependencies** — `Mailer::enabled`, `send_flow_mail`, `Store::get_user_by_id`.
**Used by** — routed in `router`.

**Repeated context** — none.

---

## TokenBody

**Identification** — DTO struct; marker `// md:TokenBody`.
`struct TokenBody { token }`. **Used by** `verify_confirm`; otherwise trivial.

**Code** — complete and verbatim:

```rust
// md:TokenBody
#[derive(Debug, Deserialize)]
struct TokenBody {
    token: String,
}
```

---

## fn verify_confirm

**Identification** — handler; marker `// md:fn verify_confirm`.

**Code** — complete and verbatim:

```rust
// md:fn verify_confirm
async fn verify_confirm(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TokenBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = state
        .store
        .consume_email_token(crate::mail::MailKind::VerifyEmail, &body.token)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid or expired token".into()))?;
    state.store.mark_email_verified(user_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
```

**What it does** — `POST /api/account/verify/confirm` — **unauthenticated**: the
token *is* the proof. Consumes the (kind-scoped, single-use, hashed) token and
stamps `email_verified_at`; unknown/expired/used → `400`.

**Dependencies** — `Store::{consume_email_token, mark_email_verified}`;
`MailKind::VerifyEmail`. **Used by** — routed in `router` (rate-limited group).

**Repeated context** — none.

---

## ResetRequestBody

**Identification** — DTO struct; marker `// md:ResetRequestBody`.
`struct ResetRequestBody { email }`. **Used by** `reset_request`; otherwise trivial.

**Code** — complete and verbatim:

```rust
// md:ResetRequestBody
#[derive(Debug, Deserialize)]
struct ResetRequestBody {
    email: String,
}
```

---

## fn reset_request

**Identification** — handler; marker `// md:fn reset_request`.

**Code** — complete and verbatim:

```rust
// md:fn reset_request
async fn reset_request(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ResetRequestBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.mailer.enabled() {
        return Err(AppError::NotImplemented(
            "mail webhook not configured".into(),
        ));
    }
    let email = normalize_email(&body.email);
    if let Some(user) = state.store.get_user_by_email(&email).await? {
        if let Err(e) = send_flow_mail(&state, &user, crate::mail::MailKind::PasswordReset).await {
            tracing::error!(error = %e, "password reset mail failed");
        }
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}
```

**What it does** — `POST /api/account/reset/request` — unauthenticated by nature.
`501` when the webhook is unconfigured. Otherwise answers a **uniform `200`**
whether or not the account exists (no existence oracle, issue #32); even a webhook
delivery failure is only logged, for the same reason.

**Dependencies** — `Mailer::enabled`, `normalize_email`, `send_flow_mail`,
`Store::get_user_by_email`. **Used by** — routed in `router`.

**Repeated context** — Uniform-response discipline as in `login`.

---

## ResetConfirmBody

**Identification** — DTO struct; marker `// md:ResetConfirmBody`.
`struct ResetConfirmBody { token, new_password }`. **Used by** `reset_confirm`;
otherwise trivial.

**Code** — complete and verbatim:

```rust
// md:ResetConfirmBody
#[derive(Debug, Deserialize)]
struct ResetConfirmBody {
    token: String,
    new_password: String,
}
```

---

## fn reset_confirm

**Identification** — handler; marker `// md:fn reset_confirm`.

**Code** — complete and verbatim:

```rust
// md:fn reset_confirm
async fn reset_confirm(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ResetConfirmBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.new_password.len() < 8 {
        return Err(AppError::BadRequest("password too short".into()));
    }
    let user_id = state
        .store
        .consume_email_token(crate::mail::MailKind::PasswordReset, &body.token)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid or expired token".into()))?;
    let hash = auth::hash_password(&body.new_password)?;
    state.store.update_password(user_id, &hash).await?;
    state.store.delete_all_devices(user_id).await?;
    if let Some(user) = state.store.get_user_by_id(user_id).await? {
        state.store.clear_login_failures(&user.email).await?;
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}
```

**What it does** — `POST /api/account/reset/confirm`: min 8-char password; consume
the reset token (`400` if invalid/expired/used); set the new hash; **revoke every
device** (sign out everywhere — the reset may mean the old credential was
compromised); clear the login-lockout counter.

**Dependencies** — `auth::hash_password`; `Store::{consume_email_token,
update_password, delete_all_devices, get_user_by_id, clear_login_failures}`.
**Used by** — routed in `router`.

**Repeated context** — none.

---

## fn list_notebooks

**Identification** — handler; marker `// md:fn list_notebooks`.

**Code** — complete and verbatim:

```rust
// md:fn list_notebooks
async fn list_notebooks(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let (limit, cursor) = q.resolve()?;
    let items = state
        .store
        .list_notebooks(user.user_id, limit, cursor)
        .await?;
    Ok(paginated(items, limit, |nb| {
        PageCursor::new(nb.created_at, nb.id)
    }))
}
```

**What it does** — `GET /api/notebooks`: the caller's **live** notebooks (the read
side of relay materialisation, for cold rehydration — writes arrive over
`/api/sync`). Paginated (`ListQuery` → `paginated`, keyset on `(created_at, id)`).

**Dependencies** — `ListQuery::resolve`, `paginated` (this file);
`Store::list_notebooks`. **Used by** — routed in `router`.

**Repeated context** — Server-as-source-of-truth: the relay materialises
notebooks/tags/resources into server tables (`sync.rs::materialize`); the client DB
is a cache that rehydrates from these endpoints; soft-deleted rows are excluded
("live").

---

## fn list_tags

**Identification** — handler; marker `// md:fn list_tags`.

**Code** — complete and verbatim:

```rust
// md:fn list_tags
async fn list_tags(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let (limit, cursor) = q.resolve()?;
    let items = state.store.list_tags(user.user_id, limit, cursor).await?;
    Ok(paginated(items, limit, |t| {
        PageCursor::new(t.created_at, t.id)
    }))
}
```

**What it does** — `GET /api/tags`: the caller's live tags, paginated. Same pattern
and context as `list_notebooks`.

**Dependencies** — `Store::list_tags`. **Used by** — routed in `router`.

**Repeated context** — as `list_notebooks`.

---

## ResourceListFilter

**Identification** — query struct; marker `// md:ResourceListFilter`.

**Code** — complete and verbatim:

```rust
// md:ResourceListFilter
#[derive(Debug, Deserialize)]
struct ResourceListFilter {
    #[serde(default)]
    note_id: Option<Uuid>,
}
```

**What it does** — the optional `?note_id=<uuid>` filter for `GET /api/resources`, extracted as
a second `Query` alongside the shared `ListQuery` (which is left untouched). `serde_urlencoded`
ignores fields it doesn't know, so the two query structs coexist.

**Dependencies** — `serde` derive. **Used by** — `list_resources`.

---

## fn list_resources

**Identification** — handler; marker `// md:fn list_resources`.

**Code** — complete and verbatim:

```rust
// md:fn list_resources
async fn list_resources(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Query(q): Query<ListQuery>,
    Query(f): Query<ResourceListFilter>,
) -> Result<Response, AppError> {
    let (limit, cursor) = q.resolve()?;
    let items = match f.note_id {
        Some(note_id) => {
            state
                .store
                .list_resources_for_note(user.user_id, note_id, limit, cursor)
                .await?
        }
        None => {
            state
                .store
                .list_resources(user.user_id, limit, cursor)
                .await?
        }
    };
    Ok(paginated(items, limit, |r| {
        PageCursor::new(r.created_at, r.id)
    }))
}
```

**What it does** — `GET /api/resources`: the caller's live resource **metadata**,
paginated; binaries are fetched separately via `GET /api/resources/:id/data`. With
`?note_id=<uuid>` (issue #125) it returns just that note's attachments via
`list_resources_for_note`; without it, all of the user's resources.

**Dependencies** — `Store::list_resources`, `Store::list_resources_for_note`. **Used by** —
routed in `router`.

**Repeated context** — as `list_notebooks`; blob/metadata split is issue #24's
storage model (metadata tombstones persist; bytes are purgeable).

---

## fn list_note_tags

**Identification** — handler; marker `// md:fn list_note_tags`.

**Code** — complete and verbatim:

```rust
// md:fn list_note_tags
async fn list_note_tags(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(note_id): Path<Uuid>,
) -> Result<Json<Vec<Uuid>>, AppError> {
    Ok(Json(
        state.store.list_note_tag_ids(user.user_id, note_id).await?,
    ))
}
```

**What it does** — `GET /api/notes/:id/tags`: the live tag ids attached to a note
(the materialised `note_tags` associations), scoped to the caller's user id.

**Dependencies** — `Store::list_note_tag_ids`. **Used by** — routed in `router`.

**Repeated context** — none.

---

## fn get_resource_data

**Identification** — handler; marker `// md:fn get_resource_data`.

**Code** — complete and verbatim:

```rust
// md:fn get_resource_data
async fn get_resource_data(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    if !state.store.resource_owned_by(id, user.user_id).await? {
        return Err(AppError::NotFound);
    }
    let data = state
        .store
        .get_resource_blob(id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data))
}
```

**What it does** — `GET /api/resources/:id/data`: ownership check
(`resource_owned_by`, `404` otherwise — not `403`, so existence is not disclosed),
then the blob as `application/octet-stream`. The bytes are opaque (encrypted by the
client); the client already has the real MIME type from the resource metadata.

**Dependencies** — `Store::{resource_owned_by, get_resource_blob}`.
**Used by** — routed in `router` (raised-body-limit sub-router).

**Repeated context** — Resources are per-user (not shareable); hence the owner
check rather than capability resolution.

---

## fn put_resource_data

**Identification** — handler; marker `// md:fn put_resource_data`.

**Code** — complete and verbatim:

```rust
// md:fn put_resource_data
async fn put_resource_data(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.store.resource_owned_by(id, user.user_id).await? {
        return Err(AppError::NotFound);
    }
    let limit = state.config.max_user_storage_bytes;
    if limit > 0 {
        let others = state
            .store
            .user_blob_bytes_excluding(user.user_id, id)
            .await?;
        if others + body.len() as i64 > limit {
            return Err(AppError::QuotaExceeded(format!(
                "storage limit reached ({limit} bytes)"
            )));
        }
    }
    state.store.put_resource_blob(id, &body).await?;
    Ok(Json(serde_json::json!({ "ok": true, "size": body.len() })))
}
```

**What it does** — `PUT /api/resources/:id/data`: upload (or replace) a resource's
binary **out-of-band** — the metadata must already exist for this user (it arrives
over `/api/sync`; `404` otherwise). The raw body is capped by `MAX_UPLOAD_BYTES`
(axum layer → `413`). Storage quota (`MAX_USER_STORAGE_BYTES > 0`): sum every
*other* live blob of the user plus the incoming size — a replacement is measured by
its new size, not double-counted — and refuse with `507 QuotaExceeded` over the
limit. Then store the blob.

**Dependencies** — `Store::{resource_owned_by, user_blob_bytes_excluding,
put_resource_blob}`. **Used by** — routed in `router` (raised-limit sub-router).

**Repeated context** — Quota accounting counts **live** blobs only (issue #24), so
deleting resources actually frees quota.

---

## fn materialize_body

**Identification** — helper; marker `// md:fn materialize_body`.

**Code** — complete and verbatim:

```rust
// md:fn materialize_body
async fn materialize_body(state: &AppState, note_id: Uuid) -> Result<String, AppError> {
    let order = state
        .store
        .get_note_order(note_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let lines = state.store.list_lines(note_id).await?;
    let by_id: std::collections::HashMap<Uuid, _> = lines.into_iter().map(|l| (l.id, l)).collect();
    let live: Vec<&str> = order
        .order
        .iter()
        .filter_map(|id| by_id.get(id))
        .filter(|line| line.deleted_at.is_none())
        .map(|line| line.content.as_str())
        .collect();

    let cap = state.config.max_note_body_bytes;
    if cap > 0 {
        let separators = live.len().saturating_sub(1);
        let total = live.iter().map(|s| s.len()).sum::<usize>() + separators;
        if total > cap {
            return Err(AppError::PayloadTooLarge(format!(
                "note body is {total} bytes, exceeds the {cap}-byte limit"
            )));
        }
    }

    Ok(live.join("\n"))
}
```

**What it does** — Materialises a note's body for non-collaborative reads (design
§3.4): read the order entity and all lines, keep the live (non-tombstoned) lines in
order, and join with `\n`. Before allocating the joined string, **measure** it
(sum of lengths + separators) and refuse with `413 PayloadTooLarge` when over
`MAX_NOTE_BODY_BYTES` (issue #44) — the collab limits permit a ~1 GB note, and the
read path must not build that in memory. `0` disables the cap.

**Dependencies** — `Store::{get_note_order, list_lines}`. **Used by** —
`get_note`, `export_note`.

**Repeated context** — The note **body is never stored** — it is always derived
from the line model; a non-collaborative client sees a flat note while the server
keeps the versioned lines underneath. Note titles/line contents are decrypted by
the store's cipher choke point before reaching here.

---

## NoteResponse

**Identification** — DTO struct; marker `// md:NoteResponse`.
`struct NoteResponse { #[serde(flatten)] note: Note, body: String }` — note metadata
plus the materialised body, flattened into one JSON object. **Used by** `get_note`;
otherwise trivial.

**Code** — complete and verbatim:

```rust
// md:NoteResponse
#[derive(Debug, serde::Serialize)]
struct NoteResponse {
    #[serde(flatten)]
    note: Note,
    body: String,
}
```

---

## CreateNoteBody

**Identification** — DTO struct; marker `// md:CreateNoteBody`.
`struct CreateNoteBody { id: Option<Uuid>, title: String (default "Untitled note") }`
— the optional client-supplied id lets a daemon uploading a local note keep the same
note id on the server (`409` if taken). **Used by** `create_note`; otherwise
trivial.

**Code** — complete and verbatim:

```rust
// md:CreateNoteBody
#[derive(Debug, Deserialize)]
struct CreateNoteBody {
    id: Option<Uuid>,
    #[serde(default = "default_title")]
    title: String,
}
```

---

## fn default_title

**Identification** — serde default fn; marker `// md:fn default_title`.
`fn default_title() -> String` — `"Untitled note"`. **Used by** `CreateNoteBody`;
otherwise trivial.

**Code** — complete and verbatim:

```rust
// md:fn default_title
fn default_title() -> String {
    "Untitled note".into()
}
```

---

## fn create_note

**Identification** — handler; marker `// md:fn create_note`.

**Code** — complete and verbatim:

```rust
// md:fn create_note
async fn create_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<CreateNoteBody>,
) -> Result<Json<Note>, AppError> {
    let limit = state.config.max_notes_per_user;
    if limit > 0 {
        let count = state.store.count_live_notes_for_user(user.user_id).await?;
        if count >= limit {
            return Err(AppError::QuotaExceeded(format!(
                "note limit reached ({limit})"
            )));
        }
    }
    let note = state
        .store
        .create_note(body.id, &body.title, user.user_id)
        .await?;
    Ok(Json(note))
}
```

**What it does** — `POST /api/notes`: quota check first
(`MAX_NOTES_PER_USER > 0` → count live notes, `507` at the limit), then create the
note owned by the caller — in the **inbox** (no notebook) by default, with an empty
line/order model.

**Dependencies** — `Store::{count_live_notes_for_user, create_note}`.
**Used by** — routed in `router`.

**Repeated context** — Quotas are enforced at the REST write point, before any
insert.

---

## fn list_notes

**Identification** — handler; marker `// md:fn list_notes`.

**Code** — complete and verbatim:

```rust
// md:fn list_notes
async fn list_notes(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let (limit, cursor) = q.resolve()?;
    let notes = state
        .store
        .list_notes_for_user(user.user_id, limit, cursor)
        .await?;
    Ok(paginated(notes, limit, |n| {
        PageCursor::new(n.updated_at, n.id)
    }))
}
```

**What it does** — `GET /api/notes`: the caller's owned **and shared** notes
(including the folder-owner rule: notes filed in a notebook the caller owns),
paginated with keyset on `(updated_at, id)`.

**Dependencies** — `Store::list_notes_for_user`, `ListQuery`/`paginated`.
**Used by** — routed in `router`.

**Repeated context** — Visibility here must mirror `resolve_note_access` — the
list shows exactly the notes a `get_note` would allow.

---

## fn get_note

**Identification** — handler; marker `// md:fn get_note`.

**Code** — complete and verbatim:

```rust
// md:fn get_note
async fn get_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<NoteResponse>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_read() {
        return Err(AppError::Forbidden);
    }
    let body = materialize_body(&state, id).await?;
    Ok(Json(NoteResponse { note, body }))
}
```

**What it does** — `GET /api/notes/:id`: load (`404`), resolve access, require
`can_read` (`403`), and return metadata **plus the materialised body** (subject to
the `413` cap in `materialize_body`).

**Dependencies** — `resolve_note_access` (`permissions.rs`),
`materialize_body` (this file), `Store::get_note`. **Used by** — routed in
`router`.

**Repeated context** — Authorise-before-read, always via the resolver.

---

## fn present

**Identification** — serde helper; marker `// md:fn present`.

**Code** — complete and verbatim:

```rust
// md:fn present
fn present<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    T::deserialize(de).map(Some)
}
```

**What it does** — Deserialises a **present** field (even an explicit `null`) as
`Some(value)`, so `PATCH` can distinguish "leave unchanged" (absent) from "clear"
(null) when combined with `#[serde(default, deserialize_with = "present")]` on an
`Option<Option<T>>` field.

**Dependencies** — serde. **Used by** — `UpdateNoteBody`.

**Repeated context** — The tri-state PATCH pattern: `None` = untouched,
`Some(None)` = clear, `Some(Some(v))` = set.

---

## UpdateNoteBody

**Identification** — DTO struct; marker `// md:UpdateNoteBody`.

**Code** — complete and verbatim:

```rust
// md:UpdateNoteBody
#[derive(Debug, Deserialize)]
struct UpdateNoteBody {
    title: Option<String>,
    #[serde(default, deserialize_with = "present")]
    notebook_id: Option<Option<Uuid>>,
    is_todo: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    todo_due: Option<Option<chrono::DateTime<chrono::Utc>>>,
    #[serde(default, deserialize_with = "present")]
    todo_completed: Option<Option<chrono::DateTime<chrono::Utc>>>,
}
```

The patchable note metadata. **Used by** `update_note`; **Repeated context** the
tri-state pattern above.

---

## fn update_note

**Identification** — handler; marker `// md:fn update_note`.

**Code** — complete and verbatim:

```rust
// md:fn update_note
async fn update_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateNoteBody>,
) -> Result<Json<Note>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_write() {
        return Err(AppError::Forbidden);
    }
    let notebook_id = match body.notebook_id {
        Some(Some(nb)) if nb.is_nil() => Some(None),
        other => other,
    };
    let patch = crate::store::NotePatch {
        title: body.title,
        notebook_id,
        is_todo: body.is_todo,
        todo_due: body.todo_due,
        todo_completed: body.todo_completed,
    };
    let moved_into = match &patch.notebook_id {
        Some(Some(nb)) if note.notebook_id != Some(*nb) => Some(*nb),
        _ => None,
    };
    if let Some(nb) = moved_into {
        let nb_access = resolve_notebook_access(&state.store, nb, user.user_id).await?;
        if !nb_access.can_write() {
            return Err(AppError::Forbidden);
        }
    }
    let note = state
        .store
        .update_note_meta(id, &patch)
        .await?
        .ok_or(AppError::NotFound)?;
    if let Some(nb) = moved_into {
        state.store.apply_notebook_shares_to_note(id, nb).await?;
    }
    Ok(Json(note))
}
```

**What it does** — `PATCH /api/notes/:id`: load (`404`); resolve access; require
`can_write` (`403`). **Inbox mapping**: keeplin-core models the inbox as the nil
UUID (`ordering::INBOX_ID`) while this server models it as `NULL` — a
`notebook_id` of the nil UUID is mapped to `Some(None)` (a move *out* of any
notebook, shares untouched) instead of naming a notebook that cannot exist (which
would 404 below). Build the `NotePatch`. If the patch moves the note **into** a
(different, real) notebook: the move adopts that notebook's grants (destructive
cascade) — both disclosing the note to the notebook's members and replacing the
note's own shares — so the mover needs `write` on the **destination notebook**
too (`resolve_notebook_access`; unknown destination → `404`). Moving out (to the
inbox) needs no destination check. Apply the metadata patch; then, for a real
move-in, `apply_notebook_shares_to_note` performs the cascade.

**Dependencies** — `resolve_note_access`/`resolve_notebook_access`
(`permissions.rs`); `Store::{get_note, update_note_meta,
apply_notebook_shares_to_note}`; `NotePatch` (`store.rs`); `present`
(this file).

**Used by** — routed in `router`.

**Repeated context** — The destructive cascade (permissions model): a notebook's
grants are copied over a child note's `note_shares` on share changes and on
move-in; move-to-inbox leaves the note's own shares intact; consent is required on
both sides of a move-in. "Inbox" is the canonical name of the default,
notebook-less location (server representation: `notebook_id IS NULL`).

---

## fn delete_note

**Identification** — handler; marker `// md:fn delete_note`.

**Code** — complete and verbatim:

```rust
// md:fn delete_note
async fn delete_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Note>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_delete() {
        return Err(AppError::Forbidden);
    }
    let note = state
        .store
        .soft_delete_note(id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(note))
}
```

**What it does** — `DELETE /api/notes/:id`: owner-only (design §9.3 —
`access.can_delete()`, i.e. `is_owner`; capability grants never confer deletion).
**Soft-delete**: sets `deleted_at`; the row and its lines remain as tombstones.

**Dependencies** — `resolve_note_access`; `Store::{get_note, soft_delete_note}`.
**Used by** — routed in `router`.

**Repeated context** — Soft-delete discipline: replicated entities are tombstoned
so deletion syncs; physical reclamation happens only via aged GC.

---

## CreateShareBody

**Identification** — DTO struct; marker `// md:CreateShareBody`.
`struct CreateShareBody { user_id: Option<Uuid>, user_email: Option<String>,
capabilities: i32 }` — target by id or email; the capability bitmask to grant
(implied bits expanded server-side; capped to the granter's own). **Used by**
`create_share`, `create_notebook_share`; **Repeated context** the capability model
(`permissions.md` context).

**Code** — complete and verbatim:

```rust
// md:CreateShareBody
#[derive(Debug, Deserialize)]
struct CreateShareBody {
    user_id: Option<Uuid>,
    user_email: Option<String>,
    capabilities: i32,
}
```

---

## fn create_share

**Identification** — handler; marker `// md:fn create_share`.

**Code** — complete and verbatim:

```rust
// md:fn create_share
async fn create_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateShareBody>,
) -> Result<Json<NoteShare>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_share_write() {
        return Err(AppError::Forbidden);
    }
    let requested = Capabilities::from_bits(body.capabilities);
    if requested.bits() == 0 {
        return Err(AppError::BadRequest(
            "capabilities must be non-empty".into(),
        ));
    }
    if requested.bits() & access.caps.bits() != requested.bits() {
        return Err(AppError::Forbidden);
    }
    let target = match (body.user_id, &body.user_email) {
        (Some(user_id), _) => state.store.get_user_by_id(user_id).await?,
        (None, Some(email)) => {
            state
                .store
                .get_user_by_email(&normalize_email(email))
                .await?
        }
        (None, None) => {
            return Err(AppError::BadRequest(
                "user_id or user_email required".into(),
            ))
        }
    }
    .ok_or(AppError::NotFound)?;
    if target.id == note.owner_id {
        return Err(AppError::BadRequest("owner already has access".into()));
    }
    let share = state
        .store
        .create_or_update_share(id, target.id, requested.bits())
        .await?;
    Ok(Json(share))
}
```

**What it does** — `POST /api/notes/:id/share`: load note (`404`); require
`can_share_write` (`403`). Normalise the requested bits
(`Capabilities::from_bits`); empty → `400`. **No privilege escalation**: the
normalised grant must be a subset of the granter's own capabilities (`403`
otherwise). Resolve the target by id or email (`400` if neither; `404` if
unknown); granting to the owner is `400` ("owner already has access" — ownership
is never a share row). Upsert the share.

**Dependencies** — `resolve_note_access`, `Capabilities` (`permissions.rs`);
`normalize_email` (this file); `Store::{get_note, get_user_by_id,
get_user_by_email, create_or_update_share}`.

**Used by** — routed in `router`.

**Repeated context** — Grants are stored normalised (implied bits expanded) and
capped to the granter — the two rules that make the capability lattice sound.

---

## fn list_shares

**Identification** — handler; marker `// md:fn list_shares`.

**Code** — complete and verbatim:

```rust
// md:fn list_shares
async fn list_shares(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<NoteShare>>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.caps.can_share_read() {
        return Err(AppError::Forbidden);
    }
    Ok(Json(state.store.list_shares(id).await?))
}
```

**What it does** — `GET /api/notes/:id/share`: requires `can_share_read`; returns
the note's share rows.

**Dependencies** — `resolve_note_access`; `Store::list_shares`. **Used by** —
routed in `router`.

**Repeated context** — none.

---

## fn delete_share

**Identification** — handler; marker `// md:fn delete_share`.

**Code** — complete and verbatim:

```rust
// md:fn delete_share
async fn delete_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path((note_id, target_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let note = state
        .store
        .get_note(note_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_share_write() && target_id != user.user_id {
        return Err(AppError::Forbidden);
    }
    state.store.delete_share(note_id, target_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
```

**What it does** — `DELETE /api/notes/:id/share/:user_id`: a `share_write` grantee
can revoke anyone; anyone can remove **themselves** (leaving a share); otherwise
`403`.

**Dependencies** — `resolve_note_access`; `Store::delete_share`. **Used by** —
routed in `router`.

**Repeated context** — Live-session note: revocation takes effect on the
collaborative channel at the next op batch (per-op re-resolution, issue #30).

---

## TransferBody

**Identification** — DTO struct; marker `// md:TransferBody`.
`struct TransferBody { user_id: Option<Uuid>, user_email: Option<String> }` — the
new owner, by id or email. **Used by** `transfer_ownership`, `transfer_notebook`;
otherwise trivial.

**Code** — complete and verbatim:

```rust
// md:TransferBody
#[derive(Debug, Deserialize)]
struct TransferBody {
    user_id: Option<Uuid>,
    user_email: Option<String>,
}
```

---

## fn transfer_ownership

**Identification** — handler; marker `// md:fn transfer_ownership`.

**Code** — complete and verbatim:

```rust
// md:fn transfer_ownership
async fn transfer_ownership(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<TransferBody>,
) -> Result<Json<Note>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_transfer_ownership() {
        return Err(AppError::Forbidden);
    }
    let target = match (body.user_id, &body.user_email) {
        (Some(user_id), _) => state.store.get_user_by_id(user_id).await?,
        (None, Some(email)) => {
            state
                .store
                .get_user_by_email(&normalize_email(email))
                .await?
        }
        (None, None) => {
            return Err(AppError::BadRequest(
                "user_id or user_email required".into(),
            ))
        }
    }
    .ok_or(AppError::NotFound)?;
    state.store.delete_share(id, target.id).await?;
    let note = state
        .store
        .set_note_owner(id, target.id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(note))
}
```

**What it does** — `POST /api/notes/:id/transfer` — owner-only
(`can_transfer_ownership`). Resolve the target; drop any share row for the new
owner first (their access becomes ownership, unambiguous), then move `owner_id`.
Ownership is separate from capability grants and survives the transfer — the old
owner keeps **no** implicit access unless separately shared.

**Dependencies** — `resolve_note_access`; `normalize_email`;
`Store::{get_note, get_user_by_id, get_user_by_email, delete_share,
set_note_owner}`.

**Used by** — routed in `router`.

**Repeated context** — Ownership vs capabilities: only `is_owner` confers
delete/transfer; a `manage` grant does not.

---

## fn resolve_target

**Identification** — helper; marker `// md:fn resolve_target`.

**Code** — complete and verbatim:

```rust
// md:fn resolve_target
async fn resolve_target(
    state: &AppState,
    user_id: Option<Uuid>,
    user_email: &Option<String>,
) -> Result<User, AppError> {
    match (user_id, user_email) {
        (Some(uid), _) => state.store.get_user_by_id(uid).await?,
        (None, Some(email)) => {
            state
                .store
                .get_user_by_email(&normalize_email(email))
                .await?
        }
        (None, None) => {
            return Err(AppError::BadRequest(
                "user_id or user_email required".into(),
            ))
        }
    }
    .ok_or(AppError::NotFound)
}
```

**What it does** — Resolves a share/transfer target from `{user_id | user_email}`
to a `User`: id wins if both given; email is normalised; neither → `400`; unknown
→ `404`. (The notebook handlers use this helper; the note handlers predate it and
inline the same logic.)

**Dependencies** — `normalize_email`; `Store::{get_user_by_id,
get_user_by_email}`. **Used by** — `create_notebook_share`, `transfer_notebook`.

**Repeated context** — none.

---

## fn create_notebook_share

**Identification** — handler; marker `// md:fn create_notebook_share`.

**Code** — complete and verbatim:

```rust
// md:fn create_notebook_share
async fn create_notebook_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateShareBody>,
) -> Result<Json<NotebookShare>, AppError> {
    let access = resolve_notebook_access(&state.store, id, user.user_id).await?;
    if !access.can_share_write() {
        return Err(AppError::Forbidden);
    }
    let requested = Capabilities::from_bits(body.capabilities);
    if requested.bits() == 0 {
        return Err(AppError::BadRequest(
            "capabilities must be non-empty".into(),
        ));
    }
    if requested.bits() & access.caps.bits() != requested.bits() {
        return Err(AppError::Forbidden);
    }
    let target = resolve_target(&state, body.user_id, &body.user_email).await?;
    let owner = state
        .store
        .notebook_owner(id)
        .await?
        .ok_or(AppError::NotFound)?;
    if target.id == owner {
        return Err(AppError::BadRequest("owner already has access".into()));
    }
    let share = state
        .store
        .create_or_update_notebook_share(id, target.id, requested.bits())
        .await?;
    Ok(Json(share))
}
```

**What it does** — `POST /api/notebooks/:id/share` (Front B stage 1b): require
`can_share_write` on the notebook; normalise + non-empty + capped-to-granter
capability checks (same as `create_share`); resolve target; granting to the
notebook owner is `400`. The share write **cascades onto the notebook's notes
inside the store call** (`create_or_update_notebook_share` replaces each child
note's `note_shares` with the notebook profile — the destructive cascade).

**Dependencies** — `resolve_notebook_access`, `Capabilities`
(`permissions.rs`); `resolve_target` (this file);
`Store::{notebook_owner, create_or_update_notebook_share}`.

**Used by** — routed in `router`.

**Repeated context** — The destructive cascade is transactional with the share
write (store-side), so notes never hold a stale grant profile.

---

## fn list_notebook_shares

**Identification** — handler; marker `// md:fn list_notebook_shares`.

**Code** — complete and verbatim:

```rust
// md:fn list_notebook_shares
async fn list_notebook_shares(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<NotebookShare>>, AppError> {
    let access = resolve_notebook_access(&state.store, id, user.user_id).await?;
    if !access.caps.can_share_read() {
        return Err(AppError::Forbidden);
    }
    Ok(Json(state.store.list_notebook_shares(id).await?))
}
```

**What it does** — `GET /api/notebooks/:id/share`: requires `can_share_read`;
returns the notebook's share rows.

**Dependencies** — `resolve_notebook_access`; `Store::list_notebook_shares`.
**Used by** — routed in `router`. **Repeated context** — none.

---

## fn delete_notebook_share

**Identification** — handler; marker `// md:fn delete_notebook_share`.

**Code** — complete and verbatim:

```rust
// md:fn delete_notebook_share
async fn delete_notebook_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path((notebook_id, target_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let access = resolve_notebook_access(&state.store, notebook_id, user.user_id).await?;
    if !access.can_share_write() && target_id != user.user_id {
        return Err(AppError::Forbidden);
    }
    state
        .store
        .delete_notebook_share(notebook_id, target_id)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
```

**What it does** — `DELETE /api/notebooks/:id/share/:user_id`: `share_write` or
self-removal; the revocation **re-cascades** to the notebook's notes inside the
store call.

**Dependencies** — `resolve_notebook_access`; `Store::delete_notebook_share`.
**Used by** — routed in `router`; `transfer_notebook` (dropping the new owner's
share). **Repeated context** — as `create_notebook_share`.

---

## fn transfer_notebook

**Identification** — handler; marker `// md:fn transfer_notebook`.

**Code** — complete and verbatim:

```rust
// md:fn transfer_notebook
async fn transfer_notebook(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<TransferBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let access = resolve_notebook_access(&state.store, id, user.user_id).await?;
    if !access.can_transfer_ownership() {
        return Err(AppError::Forbidden);
    }
    let target = resolve_target(&state, body.user_id, &body.user_email).await?;
    state
        .store
        .set_notebook_owner(id, target.id)
        .await?
        .ok_or(AppError::NotFound)?;
    state.store.delete_notebook_share(id, target.id).await?;
    Ok(Json(
        serde_json::json!({ "ok": true, "owner_id": target.id }),
    ))
}
```

**What it does** — `POST /api/notebooks/:id/transfer` — owner-only. Resolve the
target; move `notebooks.user_id` (`404` if the notebook vanished); then drop any
share row the new owner had (which also re-cascades the notebook's grants so
child notes reflect the new profile). Returns `{ ok, owner_id }`.

**Dependencies** — `resolve_notebook_access`; `resolve_target`;
`Store::{set_notebook_owner, delete_notebook_share}`.

**Used by** — routed in `router`.

**Repeated context** — The folder-owner rule (`permissions.rs`): the notebook
owner holds implicit `manage` over child notes, resolved at access time — so a
transfer needs no share rewrite for the new owner's own access.

---

## HistoryQuery

**Identification** — DTO struct; marker `// md:HistoryQuery`.
`struct HistoryQuery { limit: Option<u32> }` — version-count cap. **Used by** the
two history handlers; **Repeated context** defaults in *History limits*.

**Code** — complete and verbatim:

```rust
// md:HistoryQuery
#[derive(Debug, Deserialize)]
struct HistoryQuery {
    limit: Option<u32>,
}
```

---

## History limits

**Identification** — logical section: the two consts; marker `// md:History limits`.

**Code** — complete and verbatim:

```rust
// md:History limits
const HISTORY_DEFAULT_LIMIT: u32 = 100;
const HISTORY_MAX_LIMIT: u32 = 10_000;
```

**What it does** — `?limit=` defaults to 100 and is hard-capped at 10 000 (the
client's revert-scan bound).

**Dependencies** — none. **Used by** — `history_versions`. **Repeated context** —
none.

---

## fn history_versions

**Identification** — helper; marker `// md:fn history_versions`.

**Code** — complete and verbatim:

```rust
// md:fn history_versions
async fn history_versions(
    state: &AppState,
    kind: crate::store::HistoryKind,
    id: Uuid,
    q: &HistoryQuery,
    access_cutoff: Option<chrono::DateTime<chrono::Utc>>,
    user_scope: Option<Uuid>,
) -> Result<Vec<crate::store::EntityVersionRow>, AppError> {
    let limit = q
        .limit
        .filter(|l| *l > 0)
        .unwrap_or(HISTORY_DEFAULT_LIMIT)
        .min(HISTORY_MAX_LIMIT);
    let retention_cutoff = (state.config.retention_days > 0)
        .then(|| chrono::Utc::now() - chrono::Duration::days(state.config.retention_days as i64));
    state
        .store
        .entity_history(
            kind,
            id,
            limit as i64,
            retention_cutoff,
            access_cutoff,
            user_scope,
        )
        .await
}
```

**What it does** — Shared history read (Front D stage 2, issue #27): clamp the
limit; compute the retention bound (`CHANGES_RETENTION_DAYS > 0` → only journal
rows younger than the window, compared on the row's `received_at`); pass both
bounds to `Store::entity_history`. The two bounds are **independent filters**:
`access_cutoff` — `Some(instant)` only when the `access` visibility policy applies
to this caller — is compared against the **payload's own causal timestamp**
(`updated_at`/`deleted_at`), *not* `received_at`, so journal re-delivery (a
reinstalled device re-pushing from epoch, minting fresh `received_at` values)
cannot slip pre-access versions into a collaborator's window. `user_scope` is
`None` for a server-materialised (authorised, possibly shared) entity —
per-entity history across all users — and `Some(caller)` for a relay-only entity
that is private to the account.

**Dependencies** — `Store::entity_history`, `HistoryKind`, `EntityVersionRow`
(`store.rs`); `HistoryQuery` + the limit consts (this file).

**Used by** — `note_history`, `notebook_history`.

**Repeated context** — History model: the server journal (`changes`) is the
durable, cross-device change record; these endpoints expose it as version history
so a fresh device (empty local journal) can still show and revert past versions.
History is **per-entity** — one timeline per note — so every reader with access
sees every collaborator's edits. Snapshots are returned exactly as pushed:
client-encrypted fields stay ciphertext, decrypted client-side. The
payload-timestamp (not `received_at`) comparison for the access cutoff is the
honest-client security boundary documented in `SECURITY.md`.

---

## fn access_cutoff

**Identification** — helper; marker `// md:fn access_cutoff`.

**Code** — complete and verbatim:

```rust
// md:fn access_cutoff
fn access_cutoff(
    state: &AppState,
    access: &crate::permissions::Access,
    share_created_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    if state.config.history_since_access && !access.is_owner {
        share_created_at
    } else {
        None
    }
}
```

**What it does** — The visibility cutoff for a collaborator under
`HISTORY_VISIBILITY=access`: `Some(share.created_at)` when the policy is on
**and** the caller is a non-owner grantee; else `None` (full history — the owner
always sees everything, and the default `creation` policy shows everyone the full
timeline).

**Dependencies** — `Access` (`permissions.rs`); `config.history_since_access`.
**Used by** — `note_history`, `notebook_history`.

**Repeated context** — Issue #27's policy switch, restated: `creation` (default)
= everyone with read access sees the entity's full history; `access` = a
collaborator sees only versions from when they were granted access.

---

## fn note_history

**Identification** — handler; marker `// md:fn note_history`.

**Code** — complete and verbatim:

```rust
// md:fn note_history
async fn note_history(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<HistoryQuery>,
) -> Result<Json<Vec<crate::store::EntityVersionRow>>, AppError> {
    match state.store.get_note(id).await? {
        Some(note) => {
            let access = resolve_note_access(&state.store, &note, user.user_id).await?;
            if !access.can_read() {
                return Err(AppError::Forbidden);
            }
            let share = state.store.get_share(id, user.user_id).await?;
            let cutoff = access_cutoff(&state, &access, share.map(|s| s.created_at));
            Ok(Json(
                history_versions(
                    &state,
                    crate::store::HistoryKind::Note,
                    id,
                    &q,
                    cutoff,
                    None,
                )
                .await?,
            ))
        }
        None => Ok(Json(
            history_versions(
                &state,
                crate::store::HistoryKind::Note,
                id,
                &q,
                None,
                Some(user.user_id),
            )
            .await?,
        )),
    }
}
```

**What it does** — `GET /api/notes/:id/history?limit=` — past versions, newest
first, `[{ timestamp, device_id, entity? }]` with `entity: null` = tombstone. Two
regimes: a **server-materialised** note (a `notes` row exists) → resolve access,
require `can_read`, compute the collaborator cutoff from their share's
`created_at`, and read **per-entity** (`user_scope: None`) — every user with read
access sees every collaborator's edits. A **relay-only** note (no server-side
row, hence no owner/share model) → private to the account: read from the
caller's own journal (`user_scope: Some(caller)`, no cutoff).

**Dependencies** — `resolve_note_access`; `access_cutoff`, `history_versions`
(this file); `Store::{get_note, get_share}`.

**Used by** — routed in `router`.

**Repeated context** — as `history_versions`.

---

## fn notebook_history

**Identification** — handler; marker `// md:fn notebook_history`.

**Code** — complete and verbatim:

```rust
// md:fn notebook_history
async fn notebook_history(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<HistoryQuery>,
) -> Result<Json<Vec<crate::store::EntityVersionRow>>, AppError> {
    if state.store.notebook_owner(id).await?.is_some() {
        let access = resolve_notebook_access(&state.store, id, user.user_id).await?;
        if !access.can_read() {
            return Err(AppError::Forbidden);
        }
        let share = state.store.get_notebook_share(id, user.user_id).await?;
        let cutoff = access_cutoff(&state, &access, share.map(|s| s.created_at));
        Ok(Json(
            history_versions(
                &state,
                crate::store::HistoryKind::Notebook,
                id,
                &q,
                cutoff,
                None,
            )
            .await?,
        ))
    } else {
        Ok(Json(
            history_versions(
                &state,
                crate::store::HistoryKind::Notebook,
                id,
                &q,
                None,
                Some(user.user_id),
            )
            .await?,
        ))
    }
}
```

**What it does** — `GET /api/notebooks/:id/history` — same two regimes keyed on
whether the notebook is materialised (`notebook_owner` row exists): materialised →
notebook access + `can_read` + collaborator cutoff from `notebook_shares`;
otherwise per-user journal read.

**Dependencies** — `resolve_notebook_access`; `access_cutoff`,
`history_versions`; `Store::{notebook_owner, get_notebook_share}`.

**Used by** — routed in `router`.

**Repeated context** — as `history_versions`.

---

## ImportBody

**Identification** — DTO struct; marker `// md:ImportBody`.
`struct ImportBody { title, body }` — a flat note to import. **Used by**
`import_note`; otherwise trivial.

**Code** — complete and verbatim:

```rust
// md:ImportBody
#[derive(Debug, Deserialize)]
struct ImportBody {
    title: String,
    body: String,
}
```

---

## ImportResponse

**Identification** — DTO struct; marker `// md:ImportResponse`.
`struct ImportResponse { note_id, line_count }`. **Used by** `import_note`;
otherwise trivial.

**Code** — complete and verbatim:

```rust
// md:ImportResponse
#[derive(Debug, serde::Serialize)]
struct ImportResponse {
    note_id: Uuid,
    line_count: usize,
}
```

---

## fn import_note

**Identification** — handler; marker `// md:fn import_note`.

**Code** — complete and verbatim:

```rust
// md:fn import_note
async fn import_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<ImportBody>,
) -> Result<Json<ImportResponse>, AppError> {
    let note = state
        .store
        .create_note(None, &body.title, user.user_id)
        .await?;
    let writer = user.device_id.to_string();
    let now = chrono::Utc::now();
    let lines: Vec<&str> = body.body.split('\n').collect();

    let mut order = Vec::with_capacity(lines.len());
    let line_vv = keeplin_core::storage::note_log::VersionVector::from([(writer.clone(), 1u64)]);
    for content in &lines {
        let line_id = Uuid::new_v4();
        state
            .store
            .insert_line(line_id, note.id, content, &line_vv, &writer, now)
            .await?;
        order.push(line_id);
    }
    let order_vv = keeplin_core::storage::note_log::VersionVector::from([(
        writer.clone(),
        lines.len() as u64,
    )]);
    state
        .store
        .set_note_order(note.id, &order, &order_vv, &writer, now)
        .await?;

    Ok(Json(ImportResponse {
        note_id: note.id,
        line_count: lines.len(),
    }))
}
```

**What it does** — `POST /api/import` (design §10): offline → server migration for
one note. Creates the note, splits the flat body on `\n` into one versioned line
per row, and seeds version vectors with the importer's **device** component (the
same actor collaborative ops are signed with): each line gets
`{device: 1}`; the order entity gets `{device: line_count}`. Returns
`{note_id, line_count}`.

**Dependencies** — `Store::{create_note, insert_line, set_note_order}`;
`keeplin_core::…::VersionVector`.

**Used by** — routed in `router`; the test harnesses use it to seed notes.

**Repeated context** — Device-as-actor even on REST: an import is an edit like any
other, so its vv must be attributable and advanceable by later collaborative ops
from the same device.

---

## ExportResponse

**Identification** — DTO struct; marker `// md:ExportResponse`.
`struct ExportResponse { id, title, body }`. **Used by** `export_note`; otherwise
trivial.

**Code** — complete and verbatim:

```rust
// md:ExportResponse
#[derive(Debug, serde::Serialize)]
struct ExportResponse {
    id: Uuid,
    title: String,
    body: String,
}
```

---

## fn export_note

**Identification** — handler; marker `// md:fn export_note`.

**Code** — complete and verbatim:

```rust
// md:fn export_note
async fn export_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ExportResponse>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_read() {
        return Err(AppError::Forbidden);
    }
    let body = materialize_body(&state, id).await?;
    Ok(Json(ExportResponse {
        id: note.id,
        title: note.title,
        body,
    }))
}
```

**What it does** — `GET /api/notes/:id/export` (design §10): server → offline
migration — access-checked (`can_read`), the live lines joined with `\n`
(`materialize_body`, subject to the `413` cap).

**Dependencies** — `resolve_note_access`; `materialize_body`;
`Store::get_note`.

**Used by** — routed in `router`.

**Repeated context** — as `get_note`.

---

## mod tests

**Identification** — `#[cfg(test)]` module; marker `// md:mod tests`. One test,
below.

**Code** — container: members documented as sub-blocks below: fn protocol_compatibility_is_exact_match.

**What it does** — Unit-level pin of the compatibility rule.

**Dependencies** — `super::*`. **Used by** — `cargo test`. **Repeated context** —
none.

### fn protocol_compatibility_is_exact_match

**Identification** — `#[test]`; marker
`// md:mod tests > fn protocol_compatibility_is_exact_match`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn protocol_compatibility_is_exact_match
    #[test]
    fn protocol_compatibility_is_exact_match() {
        assert!(compatible_with(PROTOCOL_VERSION));
        assert!(!compatible_with(PROTOCOL_VERSION + 1));
        assert!(!compatible_with(0));
    }
```

**What it does** — `compatible_with(PROTOCOL_VERSION)` is true; `+1` and `0` are
false — the exact-match rule mirrored in keeplin-core's `compat::compatible_with`.

**Dependencies / Used by** — `compatible_with`; `cargo test`.

**Repeated context** — none.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `router()` — defined here (EXTRACTED; 13 cross-file edge(s))
- `update_note()` — defined here (EXTRACTED; 6 cross-file edge(s))
- `delete_note()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `create_share()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `list_shares()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `transfer_ownership()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `create_notebook_share()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `list_notebook_shares()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `note_history()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `notebook_history()` — defined here (EXTRACTED; 5 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/auth.rs` — passwords, tokens, and the auth middleware (EXTRACTED: references×30; e.g. `AuthedUser`)
- `crates/keeplin-srv/src/error.rs` — the API error type (EXTRACTED: references×41; e.g. `AppError`)
- `crates/keeplin-srv/src/mail.rs` — delegated email delivery (mail webhook) (EXTRACTED: references×1; e.g. `MailKind`)
- `crates/keeplin-srv/src/permissions.rs` — note capabilities (EXTRACTED: calls×15, references×1; e.g. `resolve_note_access()`, `resolve_notebook_access()`, `Access`)
- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×43; e.g. `AppState`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: references×19; e.g. `PageCursor`, `User`, `UserDevice`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/collab.rs` — the collaborative session engine (EXTRACTED: calls×2; e.g. `winner()`, `line_winner()`)
- `crates/keeplin-srv/src/main.rs` — keeplin-srv entry point (EXTRACTED: calls×1; e.g. `main()`)
- `crates/keeplin-srv/tests/collab.rs` — collaborative channel & hardening tests (EXTRACTED: calls×3; e.g. `spawn_instance()`, `spawn_rate_limited()`, `spawn_server_with_state()`)
- `crates/keeplin-srv/tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries (EXTRACTED: calls×1; e.g. `spawn_server()`)
- `crates/keeplin-srv/tests/integration.rs` — device relay tests (real `DbBackend`) (EXTRACTED: calls×3; e.g. `spawn_instance()`, `spawn_server()`, `spawn_server_with_config()`)
- `crates/keeplin-srv/tests/materialize.rs` — domain-entity materialisation tests (EXTRACTED: calls×1; e.g. `spawn_server()`)
- `crates/keeplin-srv/tests/quotas.rs` — per-user quota enforcement tests (EXTRACTED: calls×1; e.g. `spawn()`)
- `crates/keeplin-srv/tests/reencrypt.rs` — re-encrypt pass tests (EXTRACTED: calls×1; e.g. `spawn_server()`)
- `crates/keeplin-srv/tests/soak.rs` — multi-instance collaborative soak/load drill (EXTRACTED: calls×1; e.g. `spawn_instance()`)

## Coverage checklist

Every code block of `http.rs`, in source order, each documented above (five points)
and carrying its marker in the code:

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports | `// md:Overview` |
| 2 | `MAX_PAGE_LIMIT` | `// md:MAX_PAGE_LIMIT` |
| 3 | `struct ListQuery` | `// md:ListQuery` |
| 4 | `impl ListQuery` | `// md:impl ListQuery` |
| 5 | `fn resolve` | `// md:impl ListQuery > fn resolve` |
| 6 | `fn paginated` | `// md:fn paginated` |
| 7 | `fn router` | `// md:fn router` |
| 8 | `PROTOCOL_VERSION` | `// md:PROTOCOL_VERSION` |
| 9 | `fn compatible_with` | `// md:fn compatible_with` |
| 10 | `CAPABILITIES` | `// md:CAPABILITIES` |
| 11 | `fn version` | `// md:fn version` |
| 12 | `fn health` | `// md:fn health` |
| 13 | `fn ready` | `// md:fn ready` |
| 14 | `struct MetricsQuery` | `// md:MetricsQuery` |
| 15 | `fn metrics` | `// md:fn metrics` |
| 16 | `fn normalize_email` | `// md:fn normalize_email` |
| 17 | `fn is_valid_email` | `// md:fn is_valid_email` |
| 18 | `struct RegisterBody` | `// md:RegisterBody` |
| 19 | `struct RegisterResponse` | `// md:RegisterResponse` |
| 20 | `fn register` | `// md:fn register` |
| 21 | `struct LoginBody` | `// md:LoginBody` |
| 22 | `struct LoginResponse` | `// md:LoginResponse` |
| 23 | `fn login` | `// md:fn login` |
| 24 | `struct CreateDeviceBody` | `// md:CreateDeviceBody` |
| 25 | `struct CreateDeviceResponse` | `// md:CreateDeviceResponse` |
| 26 | `fn create_device` | `// md:fn create_device` |
| 27 | `fn delete_device` | `// md:fn delete_device` |
| 28 | `fn list_devices` | `// md:fn list_devices` |
| 29 | `fn delete_all_devices` | `// md:fn delete_all_devices` |
| 30 | `struct ChangePasswordBody` | `// md:ChangePasswordBody` |
| 31 | `fn change_password` | `// md:fn change_password` |
| 32 | `struct DeleteAccountBody` | `// md:DeleteAccountBody` |
| 33 | `fn delete_account` | `// md:fn delete_account` |
| 34 | `fn send_flow_mail` | `// md:fn send_flow_mail` |
| 35 | `fn verify_request` | `// md:fn verify_request` |
| 36 | `struct TokenBody` | `// md:TokenBody` |
| 37 | `fn verify_confirm` | `// md:fn verify_confirm` |
| 38 | `struct ResetRequestBody` | `// md:ResetRequestBody` |
| 39 | `fn reset_request` | `// md:fn reset_request` |
| 40 | `struct ResetConfirmBody` | `// md:ResetConfirmBody` |
| 41 | `fn reset_confirm` | `// md:fn reset_confirm` |
| 42 | `fn list_notebooks` | `// md:fn list_notebooks` |
| 43 | `fn list_tags` | `// md:fn list_tags` |
| 44 | `ResourceListFilter` | `// md:ResourceListFilter` |
| 45 | `fn list_resources` | `// md:fn list_resources` |
| 46 | `fn list_note_tags` | `// md:fn list_note_tags` |
| 47 | `fn get_resource_data` | `// md:fn get_resource_data` |
| 48 | `fn put_resource_data` | `// md:fn put_resource_data` |
| 49 | `fn materialize_body` | `// md:fn materialize_body` |
| 50 | `struct NoteResponse` | `// md:NoteResponse` |
| 51 | `struct CreateNoteBody` | `// md:CreateNoteBody` |
| 52 | `fn default_title` | `// md:fn default_title` |
| 53 | `fn create_note` | `// md:fn create_note` |
| 54 | `fn list_notes` | `// md:fn list_notes` |
| 55 | `fn get_note` | `// md:fn get_note` |
| 56 | `fn present` | `// md:fn present` |
| 57 | `struct UpdateNoteBody` | `// md:UpdateNoteBody` |
| 58 | `fn update_note` | `// md:fn update_note` |
| 59 | `fn delete_note` | `// md:fn delete_note` |
| 60 | `struct CreateShareBody` | `// md:CreateShareBody` |
| 61 | `fn create_share` | `// md:fn create_share` |
| 62 | `fn list_shares` | `// md:fn list_shares` |
| 63 | `fn delete_share` | `// md:fn delete_share` |
| 64 | `struct TransferBody` | `// md:TransferBody` |
| 65 | `fn transfer_ownership` | `// md:fn transfer_ownership` |
| 66 | `fn resolve_target` | `// md:fn resolve_target` |
| 67 | `fn create_notebook_share` | `// md:fn create_notebook_share` |
| 68 | `fn list_notebook_shares` | `// md:fn list_notebook_shares` |
| 69 | `fn delete_notebook_share` | `// md:fn delete_notebook_share` |
| 70 | `fn transfer_notebook` | `// md:fn transfer_notebook` |
| 71 | `struct HistoryQuery` | `// md:HistoryQuery` |
| 72 | `HISTORY_DEFAULT_LIMIT` / `HISTORY_MAX_LIMIT` | `// md:History limits` |
| 73 | `fn history_versions` | `// md:fn history_versions` |
| 74 | `fn access_cutoff` | `// md:fn access_cutoff` |
| 75 | `fn note_history` | `// md:fn note_history` |
| 76 | `fn notebook_history` | `// md:fn notebook_history` |
| 77 | `struct ImportBody` | `// md:ImportBody` |
| 78 | `struct ImportResponse` | `// md:ImportResponse` |
| 79 | `fn import_note` | `// md:fn import_note` |
| 80 | `struct ExportResponse` | `// md:ExportResponse` |
| 81 | `fn export_note` | `// md:fn export_note` |
| 82 | `mod tests` | `// md:mod tests` |
| 83 | `fn protocol_compatibility_is_exact_match` | `// md:mod tests > fn protocol_compatibility_is_exact_match` |
