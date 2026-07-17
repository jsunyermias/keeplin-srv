# `auth.rs` — passwords, tokens, and the auth middleware

## Purpose

The single authentication surface: Argon2 password hashing/verification, JWT device-token
mint/verify, the axum middleware that guards protected REST routes, and the `AuthedUser`
extractor handlers use. It also enforces **token revocation** — a token is only accepted while
its device still exists.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `Claims` | struct | JWT payload: `sub` (user id), `device_id`, `email`, `exp` |
| `AuthedUser` | struct | authenticated caller (`user_id`, `device_id`, `email`); an axum extractor |

## Public API

| Function | Description |
|----------|-------------|
| `hash_password(pw) -> String` | Argon2id hash with a fresh random salt |
| `verify_password(pw, hash) -> bool` | constant-time-ish verify via Argon2 |
| `create_token(user_id, device_id, email, secret, ttl_days) -> String` | mint an HS256 JWT expiring in `ttl_days` |
| `verify_token(token, secret) -> AuthedUser` | decode + validate signature/expiry; maps failure to `InvalidToken` |
| `auth_mw(state, req, next)` | middleware: extract Bearer token, verify it, **check the device still exists and belongs to the user**, insert `AuthedUser` |

## Token revocation

Deleting a device (`DELETE /api/devices/:id`) must invalidate its token immediately, not only
at `exp`. The middleware therefore does more than verify the signature: after `verify_token`
it calls `store.get_device(claims.device_id)` and rejects (`InvalidToken`) unless the device
row still exists and its `user_id` matches the claim. Both WebSocket handshakes
(`collab.rs`, `sync.rs`) perform the same check on connect.

## Design notes

- The token carries `device_id` because the **device** is the concurrency actor: collaborative
  ops are signed with it, and the relay uses it as each device's identity. A user with two
  devices has two tokens.
- `AuthedUser` is inserted into request extensions by the middleware and pulled out by the
  `FromRequestParts` impl, so handlers just take `user: AuthedUser` as an argument.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `AuthedUser` — defined here (EXTRACTED; 30 cross-file edge(s))
- `auth_mw()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `hash_password()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `verify_password()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `create_token()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `verify_token()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `Claims` — defined here (EXTRACTED; file-local)
- `dummy_password_hash()` — defined here (EXTRACTED; file-local)
- `.from_request_parts()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/error.rs` — the API error type (EXTRACTED: references×4; e.g. `AppError`)
- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×1; e.g. `AppState`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: calls×1; e.g. `.encode()`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: references×30; e.g. `change_password()`, `create_device()`, `create_note()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Every authenticated surface (REST, `/api/sync`, `/api/ws`) re-checks the device row, so deleting a device revokes its token immediately — JWT validity alone is never enough.
- Passwords are only ever stored as Argon2id hashes; login timing is equalised for unknown emails to prevent account enumeration.
- JWTs are signed with `JWT_SECRET`; the token carries `user_id` + `device_id`, and the device id is the relay identity (echo suppression, delivery cursor).

## Related files

- `http.md` — mounts `auth_mw` on the protected routes and issues tokens in `login`/`create_device`.
- `store.md` — `get_device` / `delete_device` back the revocation check.
- `SECURITY` note: tokens travel in the `Authorization` header (preferred) or the WS query
  string; terminate TLS at a proxy.
