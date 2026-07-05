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

## Related files

- `http.md` — mounts `auth_mw` on the protected routes and issues tokens in `login`/`create_device`.
- `store.md` — `get_device` / `delete_device` back the revocation check.
- `SECURITY` note: tokens travel in the `Authorization` header (preferred) or the WS query
  string; terminate TLS at a proxy.
