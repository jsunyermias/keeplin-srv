# `error.rs` — the API error type

Self-contained companion for `crates/keeplin-srv/src/error.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able to
understand `error.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `error.rs` carries exactly one marker comment of the
form `// md:<Header> > … > <Block header>`, whose path is the header chain of the section
documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview` at
the top of the file.

**Code** — complete and verbatim:

```rust
// md:Overview
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
```

**What it does** — This module defines `AppError`, the error type every handler, the
`Store`, and both WebSocket engines return, and its conversion into an HTTP response.
One enum maps the whole server's failure modes onto status codes and a uniform JSON body
`{"error": "..."}`, so clients parse a single error format everywhere.

**Dependencies** — `axum` (external crate): `IntoResponse`/`Response`/`Json` for the
HTTP conversion. `serde_json` (external): the body literal. `thiserror` (external):
derives `std::error::Error` + `Display` from the `#[error("…")]` attributes.
`sqlx::Error` (external) appears in the `Database` variant. `tracing` (external) logs
server-side detail in `into_response`.

**Used by** — everywhere: `http.rs` (every handler returns `Result<_, AppError>`),
`store.rs` (all data-access methods; ~90 references), `auth.rs`, `permissions.rs`,
`collab.rs`, `sync.rs`, `crypto.rs`, `reencrypt.rs`.

**Repeated context** — Error-surface conventions of the server: handlers use `?`
freely because everything converts into `AppError`; the axum framework then calls
`into_response` on it. The one status produced *outside* this enum is `429` from the
rate-limit middleware (`ratelimit.rs`), because rate rejection happens before a handler
runs. Information-leak boundary (issue #46): internal detail (database messages that can
name tables/columns/constraints) is logged server-side and **never** sent to clients.

---

## AppError

**Identification** — enum deriving `Debug` + `thiserror::Error`; marker
`// md:AppError`.

**Code** — complete and verbatim:

```rust
// md:AppError
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("missing token")]
    MissingToken,

    #[error("invalid token")]
    InvalidToken,

    #[error("not found")]
    NotFound,

    #[error("forbidden")]
    Forbidden,

    #[error("conflict")]
    Conflict,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),

    #[error("payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("too many attempts; try again later")]
    TooManyAttempts,

    #[error("not implemented: {0}")]
    NotImplemented(String),

    #[error("internal error: {0}")]
    Internal(String),
}
```

**What it does** — The crate-wide error. Each variant carries a `#[error("…")]` display
string (kept in the code — attributes are not comments) and maps to one stable HTTP
status (mapping implemented in `fn status`, next sections):

| Variant | Display | Status | When |
|---------|---------|--------|------|
| `Database(sqlx::Error)` | `database error: {0}` | 500 (404 for `RowNotFound`) | any database failure; `#[from]` lets store methods use `?` directly |
| `MissingToken` | `missing token` | 401 | no `Authorization: Bearer` on a protected route |
| `InvalidToken` | `invalid token` | 401 | bad signature/expiry, or the token's device row no longer exists (revocation) |
| `NotFound` | `not found` | 404 | entity absent or not visible to the caller |
| `Forbidden` | `forbidden` | 403 | capability check failed |
| `Conflict` | `conflict` | 409 | unique-constraint violation (e.g. duplicate email or note id) |
| `BadRequest(String)` | `bad request: {0}` | 400 | malformed input |
| `QuotaExceeded(String)` | `quota exceeded: {0}` | 507 | a per-user quota (storage bytes / note count) would be exceeded |
| `PayloadTooLarge(String)` | `payload too large: {0}` | 413 | a body/note over `MAX_UPLOAD_BYTES` / `MAX_NOTE_BODY_BYTES` |
| `TooManyAttempts` | `too many attempts; try again later` | 429 | login lockout (`LOGIN_MAX_FAILURES`) |
| `NotImplemented(String)` | `not implemented: {0}` | 501 | a feature explicitly deferred by configuration (e.g. mail flows without `MAIL_WEBHOOK_URL`) |
| `Internal(String)` | `internal error: {0}` | 500 | unexpected internal failure |

**Dependencies** — `sqlx::Error` (external) in `Database`; `thiserror` derive.

**Used by** — produced across the crate: `store.rs` (maps unique violations to
`Conflict`, lockout to `TooManyAttempts`, and everything sqlx via `#[from]`), `http.rs`
(`BadRequest`, `QuotaExceeded`, `PayloadTooLarge`, `NotImplemented`, `TooManyAttempts`,
`Forbidden`…), `auth.rs` (`MissingToken`/`InvalidToken`/`Internal`), `permissions.rs`
(`NotFound`/`Forbidden`), `collab.rs`, `sync.rs`, `crypto.rs` (`Internal` on
encrypt/decrypt failures), `reencrypt.rs`. Consumed by axum via `IntoResponse`.

**Repeated context** — Handlers rely on the stability of the variant→status mapping
(quota → 507, revoked token → 401, lockout → 429); tests assert those codes. `429` from
the rate limiter is generated in `ratelimit.rs` directly, not through this enum.

---

## impl AppError

**Identification** — inherent impl block; marker `// md:impl AppError`. Contains
`fn status` and `fn client_message` (next sections).

**Code** — container: members documented as sub-blocks below: fn status, fn client_message.

**What it does** — The two private mappings used when converting to a response: variant
→ HTTP status, and variant → client-visible message.

**Dependencies** — `AppError` (this file).

**Used by** — only `into_response` (this file).

**Repeated context** — none beyond the methods' own (below).

### fn status

**Identification** — private method; marker `// md:impl AppError > fn status`.

**Code** — complete and verbatim:

```rust
    // md:impl AppError > fn status
    fn status(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;
        match self {
            AppError::Database(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
            AppError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::MissingToken | AppError::InvalidToken => StatusCode::UNAUTHORIZED,
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::Conflict => StatusCode::CONFLICT,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::QuotaExceeded(_) => StatusCode::INSUFFICIENT_STORAGE,
            AppError::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            AppError::TooManyAttempts => StatusCode::TOO_MANY_REQUESTS,
            AppError::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
```

**What it does** — Total match from variant to status code, exactly as tabulated under
*AppError* above. The one nested case: `Database(sqlx::Error::RowNotFound)` → 404 (a
lookup that found nothing is "not found", not a server failure); any other `Database` →
500.

**Dependencies** — `axum::http::StatusCode` (external), `sqlx::Error` (external).

**Used by** — `into_response` (this file) only; private, no external callers.

**Repeated context** — The status mapping is the contract clients and tests depend on;
changing a variant's status is a breaking API change.

### fn client_message

**Identification** — private method; marker `// md:impl AppError > fn client_message`.

**Code** — complete and verbatim:

```rust
    // md:impl AppError > fn client_message
    fn client_message(&self) -> String {
        match self {
            AppError::Database(_) | AppError::Internal(_) => "internal error".to_string(),
            other => other.to_string(),
        }
    }
```

**What it does** — The message placed in the JSON body. `Database(_)` and `Internal(_)`
collapse to the generic `"internal error"` so their detail — which can name
tables/columns/constraints — is never leaked in a response (issue #46); the full error
is logged server-side by `into_response` instead. Every other variant keeps its
specific, safe `Display` string.

**Dependencies** — `AppError`'s `Display` (thiserror-derived, this file).

**Used by** — `into_response` (this file) only; private, no external callers.

**Repeated context** — Information-leak boundary: internal failure detail is
operator-only (logs), never client-visible. This is the single place that rule is
enforced for response bodies.

---

## impl IntoResponse for AppError

**Identification** — trait impl; marker `// md:impl IntoResponse for AppError`.
Contains `fn into_response` (next section).

**Code** — container: members documented as sub-blocks below: fn into_response.

**What it does** — The bridge into axum: because `AppError: IntoResponse`, every
handler can return `Result<T, AppError>` and use `?`, and axum renders the error.

**Dependencies** — `axum::response::IntoResponse` (external), `AppError` (this file).

**Used by** — axum's routing machinery, implicitly, for every handler in `http.rs` and
the WebSocket upgrade handlers in `collab.rs` / `sync.rs`.

**Repeated context** — none beyond the method's own (below).

### fn into_response

**Identification** — trait method; marker
`// md:impl IntoResponse for AppError > fn into_response`.

**Code** — complete and verbatim:

```rust
    // md:impl IntoResponse for AppError > fn into_response
    fn into_response(self) -> Response {
        let status = self.status();
        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        }
        let body = Json(json!({ "error": self.client_message() }));
        (status, body).into_response()
    }
```

**What it does** — Builds the final HTTP response: takes `self.status()`; if the status
is a server error (5xx), logs the **full** error detail at `error` level for operators
(`tracing::error!(error = %self, "request failed")`); then serialises the body as
`{"error": <client_message()>}` and returns `(status, body)`. Client-caused errors
(4xx) are not logged here — they are normal traffic.

**Dependencies** — `fn status`, `fn client_message` (this file); `axum::Json`,
`serde_json::json!`, `tracing::error!` (external).

**Used by** — axum, whenever a handler or middleware returns `Err(AppError)`.

**Repeated context** — The response body shape `{"error": "..."}` is identical for
every variant so clients parse one error format; 5xx detail goes to logs only
(issue #46).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `AppError` — defined here (EXTRACTED; 150 cross-file edge(s))
- `.status()` — defined here (EXTRACTED; file-local)
- `.client_message()` — defined here (EXTRACTED; file-local)
- `.into_response()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/auth.rs` — passwords, tokens, and the auth middleware (EXTRACTED: references×4; e.g. `hash_password()`, `verify_password()`, `verify_token()`)
- `crates/keeplin-srv/src/collab.rs` — the collaborative session engine (EXTRACTED: references×6; e.g. `touch_presence()`, `clear_presence()`, `handler()`)
- `crates/keeplin-srv/src/crypto.rs` — at-rest encryption of note titles and line content (EXTRACTED: imports_from×1, references×2; e.g. `crypto.rs`, `.encrypt()`, `.decrypt()`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: references×41; e.g. `change_password()`, `create_device()`, `create_note()`)
- `crates/keeplin-srv/src/permissions.rs` — note capabilities (EXTRACTED: references×2; e.g. `resolve_note_access()`, `resolve_notebook_access()`)
- `crates/keeplin-srv/src/reencrypt.rs` — one-off at-rest re-encrypt pass (EXTRACTED: imports_from×1, references×2; e.g. `reencrypt_column()`, `reencrypt.rs`, `run()`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: imports_from×1, references×90; e.g. `cascade_notebook_to_notes_tx()`, `replace_note_shares_from_notebook_tx()`, `store.rs`)

## Coverage checklist

Every code block of `error.rs`, in source order, each documented above (five points) and
carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `enum AppError` | `// md:AppError` | AppError |
| 3 | `impl AppError` | `// md:impl AppError` | impl AppError |
| 4 | `fn status` | `// md:impl AppError > fn status` | impl AppError › fn status |
| 5 | `fn client_message` | `// md:impl AppError > fn client_message` | impl AppError › fn client_message |
| 6 | `impl IntoResponse for AppError` | `// md:impl IntoResponse for AppError` | impl IntoResponse for AppError |
| 7 | `fn into_response` | `// md:impl IntoResponse for AppError > fn into_response` | impl IntoResponse for AppError › fn into_response |
