# `error.rs` — the API error type

## Purpose

Defines `AppError`, the error every handler and the `Store` return, and its conversion into an
HTTP response. One enum maps the whole server's failure modes onto status codes and a uniform
JSON body `{"error": "..."}`.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `AppError` | enum | the crate-wide error; `impl IntoResponse` so handlers can `?` and return it directly |

## Public API

`AppError` variants and their HTTP status:

| Variant | Status | When |
|---------|--------|------|
| `Database(sqlx::Error)` | 500 (or 404 for `RowNotFound`) | any database failure |
| `MissingToken` / `InvalidToken` | 401 | absent or unverifiable/revoked token |
| `NotFound` | 404 | entity absent |
| `Forbidden` | 403 | role check failed |
| `Conflict` | 409 | unique-constraint violation (e.g. duplicate email or note id) |
| `BadRequest(String)` | 400 | malformed input |
| `QuotaExceeded(String)` | 507 | a per-user quota (storage bytes / note count) would be exceeded |
| `Internal(String)` | 500 | unexpected internal failure |

`#[from] sqlx::Error` lets store methods use `?` and surface DB errors as `AppError::Database`.

## Design notes

- Rate-limit rejections (`429`) are produced directly by the rate-limit middleware, not
  through `AppError`, because they happen before a handler runs.
- The response body shape is identical for every variant so clients parse one error format.

## Graph context

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

**Invariants** (restated on purpose; a change to this file must keep these true)

- Database/internal detail is logged server-side and never returned to clients (information-leak boundary).
- Every `AppError` variant maps to one stable HTTP status; handlers rely on that mapping (e.g. quota → `507`, missing → `404`).

## Related files

- `store.md` / `http.md` — the producers of these errors.
- `ratelimit.md` — the one status (`429`) generated outside this enum.
