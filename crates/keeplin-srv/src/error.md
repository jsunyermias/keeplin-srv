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

## Related files

- `store.md` / `http.md` — the producers of these errors.
- `ratelimit.md` — the one status (`429`) generated outside this enum.
