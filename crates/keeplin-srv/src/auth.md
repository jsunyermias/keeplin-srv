# `auth.rs` — passwords, tokens, and the auth middleware

Self-contained companion for `crates/keeplin-srv/src/auth.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able to
understand `auth.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `auth.rs` carries exactly one marker comment of the
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
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    body::Body,
    extract::{FromRequestParts, State},
    http::{request::Parts, Request},
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{error::AppError, state::AppState};
```

**What it does** — The single authentication surface of the server: Argon2 password
hashing/verification, HS256 JWT device-token mint/verify, the axum middleware guarding
every protected REST route, and the `AuthedUser` extractor handlers take as an
argument. It also enforces **token revocation**: a token is only accepted while its
device row still exists and belongs to the claimed user.

**Dependencies** — `argon2` (external): Argon2id hashing with `OsRng` salts.
`jsonwebtoken` (external): HS256 encode/decode with expiry validation. `axum`
(external): middleware + extractor machinery. `serde`, `uuid`, `chrono`, `tracing`,
`async_trait` (external). Internal: `crate::error::AppError` (`error.rs`),
`crate::state::AppState` (`state.rs`), `state.store.get_device` (`store.rs`).

**Used by** — `http.rs` (mounts `auth_mw` on the protected router; calls
`hash_password`/`verify_password`/`dummy_password_hash`/`create_token` from the
account handlers; every protected handler takes `AuthedUser`), `sync.rs` and
`collab.rs` (both WebSocket handshakes call `verify_token` + their own device check),
`tests/integration.rs` (asserts the revocation and lockout behaviours end to end).

**Repeated context** — Identity model: the **device** is the unit of authentication
and concurrency. `POST /api/login` creates a `user_devices` row and mints a JWT
carrying both `sub` (user id) and `device_id`; a user with two devices has two tokens.
The device id is what a device signs collaborative edits with (vv actor) and what the
relay uses for echo suppression and delivery cursors. Tokens travel in the
`Authorization: Bearer` header (preferred) or the WebSocket query string (fallback);
TLS is terminated at a reverse proxy. Default TTL: `TOKEN_TTL_DAYS` (365) — which is
exactly why revocation-by-device-deletion must be checked on **every** authenticated
surface, not just at mint time.

---

## Claims

**Identification** — struct; marker `// md:Claims`.

**Code** — complete and verbatim:

```rust
// md:Claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub device_id: Uuid,
    pub email: String,
    pub exp: usize,
}
```

**What it does** — The JWT payload: `sub` is the **user id** (standard JWT subject
claim), `device_id` the device the token was minted for, `email` the account email at
mint time (informational — not re-validated), `exp` the Unix-seconds expiry that
`jsonwebtoken`'s default `Validation` enforces on decode.

**Dependencies** — `uuid`, `serde` derives (external).

**Used by** — `create_token` (encodes it) and `verify_token` (decodes it), both this
file. Never leaves this module; the rest of the crate sees `AuthedUser`.

**Repeated context** — JWTs are signed (HS256, `JWT_SECRET`) but not encrypted:
claims are attacker-readable, so nothing secret goes in them. Signature validity is
deliberately **insufficient** for acceptance — the device row must still exist
(revocation, see `fn auth_mw`).

---

## AuthedUser

**Identification** — struct; marker `// md:AuthedUser`.

**Code** — complete and verbatim:

```rust
// md:AuthedUser
#[derive(Debug, Clone)]
pub struct AuthedUser {
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub email: String,
}
```

**What it does** — The authenticated caller, as handlers consume it: the verified
user id, device id, and email from the token. Constructed by `verify_token`; inserted
into request extensions by `auth_mw`; extracted by handlers via the
`FromRequestParts` impl (last block), so a handler simply declares
`user: AuthedUser` as a parameter.

**Dependencies** — `uuid` (external).

**Used by** — every protected handler in `http.rs` (~30 uses); `sync.rs::authenticate`
and `collab.rs::handler` build one from `verify_token` for their WebSocket sessions.

**Repeated context** — `user_id` scopes every query (all durable data is per-user;
sharing grants cross-user access explicitly via capability rows); `device_id` is the
concurrency actor (vv components, relay cursor identity).

---

## fn hash_password

**Identification** — public function; marker `// md:fn hash_password`.

**Code** — complete and verbatim:

```rust
// md:fn hash_password
pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("password hash failed: {}", e)))?;
    Ok(hash.to_string())
}
```

**What it does** — Hashes a password with Argon2id (library defaults) and a fresh
random salt from `OsRng`, returning the PHC-format string (`$argon2id$v=…$…`) that is
stored in `users.password_hash`. Failure (allocation/parameter errors — practically
never) maps to `AppError::Internal`.

**Dependencies** — `argon2`, `OsRng`/`SaltString` (external); `AppError` (`error.rs`).

**Used by** — `http.rs`: `register`, `change_password`, `reset_password`;
`dummy_password_hash` (this file).

**Repeated context** — Passwords are **only ever stored as Argon2id hashes** — never
plaintext, never reversible. The random salt means equal passwords produce different
hashes.

---

## fn verify_password

**Identification** — public function; marker `// md:fn verify_password`.

**Code** — complete and verbatim:

```rust
// md:fn verify_password
pub fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(format!("invalid password hash: {}", e)))?;
    let argon2 = Argon2::default();
    Ok(argon2.verify_password(password.as_bytes(), &parsed).is_ok())
}
```

**What it does** — Verifies a candidate password against a stored PHC hash string.
Returns `Ok(false)` on mismatch (not an error — the caller decides the response); an
unparsable stored hash is `AppError::Internal` (data corruption, not user error).
Argon2 verification is constant-time-ish with respect to the password content.

**Dependencies** — `argon2::PasswordHash`/`PasswordVerifier` (external); `AppError`
(`error.rs`).

**Used by** — `http.rs`: `login` (real hash, and the dummy hash for absent users),
`change_password`, `delete_account` (both re-verify the current password before the
sensitive action).

**Repeated context** — Sensitive account mutations re-verify the password even on an
authenticated request, so a stolen token alone cannot change credentials or delete
the account.

---

## fn dummy_password_hash

**Identification** — public function; marker `// md:fn dummy_password_hash`.

**Code** — complete and verbatim:

```rust
// md:fn dummy_password_hash
pub fn dummy_password_hash() -> &'static str {
    use std::sync::OnceLock;
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        hash_password("timing-equalizer-not-a-real-password")
            .expect("hashing a fixed dummy password never fails")
    })
}
```

**What it does** — Returns a valid Argon2 hash of a fixed dummy password, computed
once per process (`OnceLock`). `login` verifies the submitted password against it when
the email has **no** account, so a missing account and a wrong password take the same
time — closing the user-enumeration timing side-channel (issue #32). The `expect`
inside never fires: hashing a fixed string cannot fail.

**Dependencies** — `std::sync::OnceLock`; `hash_password` (this file).

**Used by** — `http.rs::login` (the only caller).

**Repeated context** — Anti-enumeration posture (issue #32): login answers a uniform
"invalid credentials" for both unknown email and wrong password, and this function
equalises the timing of the two paths; registration of an existing email is likewise
kept non-revealing at the HTTP layer.

---

## fn create_token

**Identification** — public function; marker `// md:fn create_token`.

**Code** — complete and verbatim:

```rust
// md:fn create_token
pub fn create_token(
    user_id: Uuid,
    device_id: Uuid,
    email: &str,
    secret: &str,
    ttl_days: i64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let claims = Claims {
        sub: user_id,
        device_id,
        email: email.to_string(),
        exp: (chrono::Utc::now() + chrono::Duration::days(ttl_days)).timestamp() as usize,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}
```

**What it does** — Mints an HS256 JWT: builds `Claims` with `exp = now + ttl_days`
and signs with `secret` (`JWT_SECRET`). Returns the compact token string. The error
type is the raw `jsonwebtoken` error (callers map it); in practice encoding a valid
claims struct does not fail.

**Dependencies** — `jsonwebtoken::{encode, Header, EncodingKey}` (external);
`chrono` for the expiry arithmetic; `Claims` (this file).

**Used by** — `http.rs`: `login` (after password verification + device-row creation)
and `create_device` (minting a token for an additional device).

**Repeated context** — `ttl_days` comes from `TOKEN_TTL_DAYS` (default 365). Long
TTL is acceptable **only** because acceptance re-checks the device row on every
request — deleting the device revokes the token immediately regardless of `exp`.

---

## fn verify_token

**Identification** — public function; marker `// md:fn verify_token`.

**Code** — complete and verbatim:

```rust
// md:fn verify_token
pub fn verify_token(token: &str, secret: &str) -> Result<AuthedUser, AppError> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| {
        tracing::debug!(error = %e, "token verification failed");
        AppError::InvalidToken
    })?;

    Ok(AuthedUser {
        user_id: token_data.claims.sub,
        device_id: token_data.claims.device_id,
        email: token_data.claims.email,
    })
}
```

**What it does** — Decodes and validates a JWT (signature + expiry, via
`jsonwebtoken`'s default `Validation`) and repackages the claims as `AuthedUser`.
Any failure is logged at `debug` (the reason is useful to operators, not to clients)
and collapsed to `AppError::InvalidToken` (→ 401) so callers can't distinguish
bad-signature from expired. **Does not check the device row** — that is the callers'
job (`auth_mw` here; `sync.rs::authenticate` and `collab.rs::handler` for the
WebSocket surfaces), because the check needs database access and this function is
deliberately pure.

**Dependencies** — `jsonwebtoken::{decode, DecodingKey, Validation}` (external);
`Claims`, `AuthedUser` (this file); `AppError` (`error.rs`); `tracing`.

**Used by** — `auth_mw` (this file), `sync.rs::authenticate`, `collab.rs::handler`.

**Repeated context** — Signature validity alone is never enough: every authenticated
surface pairs this call with a `get_device` existence+ownership check. Keeping this
function DB-free is what lets the WebSocket handshakes share it.

---

## fn auth_mw

**Identification** — public async function (axum middleware); marker
`// md:fn auth_mw`.

**Code** — complete and verbatim:

```rust
// md:fn auth_mw
pub async fn auth_mw(
    state: State<Arc<AppState>>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let State(state) = state;
    let auth = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());
    let token = auth
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(AppError::MissingToken)?;
    let user = verify_token(token, &state.config.jwt_secret)?;
    match state.store.get_device(user.device_id).await? {
        Some(device) if device.user_id == user.user_id => {}
        _ => return Err(AppError::InvalidToken),
    }
    req.extensions_mut().insert(user);
    Ok(next.run(req).await)
}
```

**What it does** — The guard on every protected REST route. Steps: (1) read the
`authorization` header and strip the `Bearer ` prefix — absent/malformed →
`AppError::MissingToken` (401); (2) `verify_token` — invalid → 401; (3) **revocation
check**: load the claimed device via `store.get_device(device_id)` and require it to
exist *and* belong to the claimed user — otherwise `AppError::InvalidToken`. Deleting
a device therefore revokes its token immediately, long before `exp`; (4) insert the
`AuthedUser` into request extensions and run the inner handler.

**Dependencies** — `verify_token` (this file); `state.store.get_device`
(`store.rs`); `axum` middleware types (external); `AppError` (`error.rs`).

**Used by** — `http.rs::router`, layered (`middleware::from_fn_with_state`) onto the
protected route group — everything except `register`/`login`/`health`/`ready`/
`version`/`metrics` and the email flows.

**Repeated context** — The revocation invariant, restated: **every** authenticated
surface re-checks the device row — this middleware for REST,
`sync.rs::authenticate` for `/api/sync`, `collab.rs::handler` for `/api/ws` (the
`/api/ws` check was the gap fixed for issue #20). The device-ownership comparison
(`device.user_id == user.user_id`) prevents a token whose device id was somehow
re-assigned from crossing user boundaries.

---

## impl FromRequestParts for AuthedUser

**Identification** — trait impl (`#[async_trait]`); marker
`// md:impl FromRequestParts for AuthedUser`. Contains `fn from_request_parts`
(next section).

**Code** — container (one member, `fn from_request_parts`, documented below). The impl
header, `where` clause and the `Rejection` associated type are the container's own body
— complete and verbatim:

```rust
// md:impl FromRequestParts for AuthedUser
#[async_trait::async_trait]
impl<S> FromRequestParts<S> for AuthedUser
where
    S: Send + Sync,
{
    type Rejection = AppError;
```

**What it does** — Makes `AuthedUser` an axum extractor so protected handlers declare
it as a plain parameter. The `Rejection = AppError` associated type means a failed
extraction surfaces as the crate's standard error response; the `where S: Send + Sync`
bound is what axum requires of any state type the extractor is generic over.

**Dependencies** — `axum::extract::FromRequestParts` (external), `async_trait`
(external), `AuthedUser` (this file).

**Used by** — axum's handler machinery for every `http.rs` handler with a
`user: AuthedUser` parameter.

**Repeated context** — none beyond the method's own (below).

### fn from_request_parts

**Identification** — trait method; marker
`// md:impl FromRequestParts for AuthedUser > fn from_request_parts`.

**Code** — complete and verbatim:

```rust
    // md:impl FromRequestParts for AuthedUser > fn from_request_parts
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthedUser>()
            .cloned()
            .ok_or(AppError::MissingToken)
    }
```

**What it does** — Pulls the `AuthedUser` that `auth_mw` inserted into request
extensions and clones it out. If absent — the handler was mounted outside the
middleware, a wiring bug — it rejects with `AppError::MissingToken` (401) rather than
panicking, failing closed.

**Dependencies** — `Parts::extensions` (external axum); `AppError` (`error.rs`).

**Used by** — axum, implicitly, wherever a handler takes `user: AuthedUser`.

**Repeated context** — Fail-closed wiring: a route that forgets the middleware
produces 401s (visible immediately in tests), never unauthenticated access.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

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

## Coverage checklist

Every code block of `auth.rs`, in source order, each documented above (five points) and
carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `struct Claims` | `// md:Claims` | Claims |
| 3 | `struct AuthedUser` | `// md:AuthedUser` | AuthedUser |
| 4 | `fn hash_password` | `// md:fn hash_password` | fn hash_password |
| 5 | `fn verify_password` | `// md:fn verify_password` | fn verify_password |
| 6 | `fn dummy_password_hash` | `// md:fn dummy_password_hash` | fn dummy_password_hash |
| 7 | `fn create_token` | `// md:fn create_token` | fn create_token |
| 8 | `fn verify_token` | `// md:fn verify_token` | fn verify_token |
| 9 | `fn auth_mw` | `// md:fn auth_mw` | fn auth_mw |
| 10 | `impl FromRequestParts for AuthedUser` | `// md:impl FromRequestParts for AuthedUser` | impl FromRequestParts for AuthedUser |
| 11 | `fn from_request_parts` | `// md:impl FromRequestParts for AuthedUser > fn from_request_parts` | impl FromRequestParts for AuthedUser › fn from_request_parts |
