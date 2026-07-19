# `config.rs` — runtime configuration

Self-contained companion for `crates/keeplin-srv/src/config.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand `config.rs` without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `config.rs` carries exactly one marker comment of
the form `// md:<Header> > … > <Block header>`, whose path is the header chain of the
section documenting it here (starting below the file title). Grep the marker text to
jump code → doc; grep the section's block name (or the marker path) in the `.rs` to
jump doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level notion only: this file has no imports; its first code
block is `struct Config` itself. No separate `// md:Overview` marker exists — the file
starts at `// md:Config`.

**What it does** — Defines `Config`, the process settings read **once** from
environment variables at startup (`Config::from_env()`), plus the `JWT_SECRET`
strength gate. Sensitive values (`DATABASE_URL`, `JWT_SECRET`) have **no** shippable
code defaults — the server refuses to start without real ones; every other knob has a
backward-compatible default, so a fresh deployment runs with only those two set.

**Dependencies** — `std::env` only (plus `tracing` for the dev-insecure warning).

**Used by** — `main.rs` (`Config::from_env` at boot), `bin/reencrypt.rs` (same
config, same `.env`), `state.rs` (`AppState.config`), and every integration test's
`test_config()` helper (which builds a `Config` literal instead of reading the
environment).

**Repeated context** — Configuration conventions of the crate: **`0` disables** every
optional limit/retention knob; booleans parse from `true`/`false`; all knobs are
read exactly once at boot (no hot reload); `.env.example` mirrors these keys as a
copy-paste starting point. Rotating `JWT_SECRET` invalidates every issued device
token (all devices must log in again).

---

## Config

**Identification** — struct; marker `// md:Config`.

**Code** — complete and verbatim:

```rust
// md:Config
#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub jwt_secret: String,
    pub token_ttl_days: i64,
    pub retention_days: u64,
    pub lines_gc_days: u64,
    pub resource_purge_days: u64,
    pub db_max_connections: u32,
    pub db_acquire_timeout_secs: u64,
    pub db_idle_timeout_secs: u64,
    pub db_max_lifetime_secs: u64,
    pub rate_limit_per_min: u32,
    pub shutdown_grace_secs: u64,
    pub log_json: bool,
    pub max_upload_bytes: usize,
    pub max_note_body_bytes: usize,
    pub max_user_storage_bytes: i64,
    pub max_notes_per_user: i64,
    pub registration_enabled: bool,
    pub at_rest_key: Option<String>,
    pub mail_webhook_url: Option<String>,
    pub mail_webhook_token: Option<String>,
    pub email_token_ttl_secs: u64,
    pub email_verification_required: bool,
    pub login_max_failures: i32,
    pub login_lockout_secs: u64,
    pub history_since_access: bool,
}
```

**What it does** — The full settings surface. Env var → field → default → meaning:

| Env var | Field | Default | Meaning |
|---------|-------|---------|---------|
| `PORT` | `port` | `3000` | HTTP/WS listen port |
| `DATABASE_URL` | `database_url` | — (required) | PostgreSQL connection string; `from_env` panics if unset |
| `JWT_SECRET` | `jwt_secret` | — (required) | HMAC secret signing device tokens; refused if unset/empty/short/placeholder (issue #19) unless `KEEPLIN_DEV_INSECURE=1` |
| `TOKEN_TTL_DAYS` | `token_ttl_days` | `365` | Device-token lifetime. Long on purpose: tokens live in daemon config files; revocation is by device deletion, not expiry |
| `CHANGES_RETENTION_DAYS` | `retention_days` | `0` (off) | Prune relay-journal rows older than N days once every device of the owning user has received them |
| `LINES_GC_DAYS` | `lines_gc_days` | `30` | Compact line tombstones soft-deleted more than N days ago (design §6.4) |
| `RESOURCE_PURGE_DAYS` | `resource_purge_days` | `0` (off) | Reclaim blob bytes of resources soft-deleted > N days ago; metadata tombstone kept (issue #24); mirrors the client's `resource_purge_days` |
| `DB_MAX_CONNECTIONS` | `db_max_connections` | `10` | PostgreSQL pool cap |
| `DB_ACQUIRE_TIMEOUT_SECS` | `db_acquire_timeout_secs` | `10` | Fail fast instead of blocking when the pool is exhausted |
| `DB_IDLE_TIMEOUT_SECS` | `db_idle_timeout_secs` | `600` | Reap idle pooled connections |
| `DB_MAX_LIFETIME_SECS` | `db_max_lifetime_secs` | `1800` | Recycle pooled connections after this age |
| `RATE_LIMIT_PER_MIN` | `rate_limit_per_min` | `0` (off) | Per-client-IP token bucket; behind a proxy leave `0` and limit at the proxy (all requests share the proxy IP) |
| `SHUTDOWN_GRACE_SECS` | `shutdown_grace_secs` | `20` | Drain window before force-exit (bounds long-lived WebSockets) |
| `LOG_JSON` | `log_json` | `false` | JSON logs (one object/line) for aggregation |
| `MAX_UPLOAD_BYTES` | `max_upload_bytes` | `104857600` (100 MiB) | Max resource binary upload (`PUT /api/resources/:id/data`); `413` over it |
| `MAX_NOTE_BODY_BYTES` | `max_note_body_bytes` | `26214400` (25 MiB) | Max materialised note body on the read path (`GET /api/notes/:id`, export); `413` instead of building it in memory (issue #44); `0` disables |
| `MAX_USER_STORAGE_BYTES` | `max_user_storage_bytes` | `0` (off) | Total live resource-blob bytes per user; upload over it → `507` |
| `MAX_NOTES_PER_USER` | `max_notes_per_user` | `0` (off) | Max live notes a user may own; create past it → `507` |
| `REGISTRATION_ENABLED` | `registration_enabled` | `true` | `false` → `POST /api/register` answers `403` (close open signups, issue #21) |
| `AT_REST_KEY` | `at_rest_key` | `None` (off) | Base64 32-byte key for at-rest encryption of `notes.title`/`lines.content` (issue keeplin#110); unset = plaintext (backward compatible) |
| `MAIL_WEBHOOK_URL` | `mail_webhook_url` | `None` (off) | Where email delivery is delegated (issue #49); unset → email flows answer `501` |
| `MAIL_WEBHOOK_TOKEN` | `mail_webhook_token` | `None` | Optional bearer sent on webhook posts |
| `EMAIL_TOKEN_TTL_SECS` | `email_token_ttl_secs` | `3600` | Lifetime of a verification/reset token |
| `EMAIL_VERIFICATION_REQUIRED` | `email_verification_required` | `false` | `true` → login refuses unverified accounts (`403`); only sane with a webhook configured |
| `LOGIN_MAX_FAILURES` | `login_max_failures` | `10` | Failed logins per email before a temporary lockout (DB-backed, holds across replicas); `0` disables |
| `LOGIN_LOCKOUT_SECS` | `login_lockout_secs` | `300` | Lockout duration; also the staleness window (older failures restart the counter) |
| `HISTORY_VISIBILITY` | `history_since_access` | `creation` (`false`) | `access` → `true`: a collaborator sees only versions since they were granted access; owner always sees all (issue #27) |

(`KEEPLIN_DEV_INSECURE` is read by `dev_insecure()`, not stored in the struct.)

**Dependencies** — none (plain data).

**Used by** — `AppState.config` (`state.rs`) and through it every subsystem;
`main.rs` reads the pool/logging/port knobs directly; the eight test files build it
literally via their `test_config()` helpers.

**Repeated context** — `Clone` because `main.rs` clones it into `AppState` while
retaining values for its own wiring. Field-level meanings above are the single
reference table; `.env.example` mirrors it.

---

## fn env_parse

**Identification** — private generic helper; marker `// md:fn env_parse`.

**Code** — complete and verbatim:

```rust
// md:fn env_parse
fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
```

**What it does** — Reads `key` from the environment and parses it; any absence or
parse failure yields `default` silently. Used for every knob with a sane default —
deliberately forgiving, in contrast with the hard-required secrets.

**Dependencies** — `std::env`.

**Used by** — `Config::from_env` (this file) for most fields.

**Repeated context** — Silent fallback is acceptable *only* for tunables whose
default is safe; anything security-relevant goes through the strict paths below.

---

## JWT secret constants

**Identification** — logical section: the two consts; marker
`// md:JWT secret constants`.

**Code** — complete and verbatim:

```rust
// md:JWT secret constants
const DEV_JWT_SECRET: &str = "dev-secret-change-in-production";
const MIN_JWT_SECRET_LEN: usize = 16;
```

**What it does** — `DEV_JWT_SECRET` is the historical dev placeholder; it is public
in the source, so a token signed with it is forgeable by anyone — it must never
authenticate a real deployment. `MIN_JWT_SECRET_LEN` (16 bytes) is the minimum
acceptable secret length; shorter secrets are brute-forceable.

**Dependencies** — none.

**Used by** — `is_weak_secret`, `resolve_jwt_secret`, the unit tests (this file).

**Repeated context** — Issue #19: a guessable signing key lets anyone forge a token
for any user; these constants define the reject list.

---

## fn dev_insecure

**Identification** — private function; marker `// md:fn dev_insecure`.

**Code** — complete and verbatim:

```rust
// md:fn dev_insecure
fn dev_insecure() -> bool {
    std::env::var("KEEPLIN_DEV_INSECURE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}
```

**What it does** — Whether the operator explicitly opted into insecure local-dev
behaviour: `KEEPLIN_DEV_INSECURE` set to `1` or `true` (case-insensitive).

**Dependencies** — `std::env`.

**Used by** — `resolve_jwt_secret` (this file).

**Repeated context** — The escape hatch is explicit and loud (a `warn` log) —
mirroring the daemon's security-issues gate philosophy: insecure modes exist only as
conscious opt-ins.

---

## fn is_weak_secret

**Identification** — private function; marker `// md:fn is_weak_secret`.

**Code** — complete and verbatim:

```rust
// md:fn is_weak_secret
fn is_weak_secret(s: &str) -> bool {
    s.trim().is_empty() || s == DEV_JWT_SECRET || s.len() < MIN_JWT_SECRET_LEN
}
```

**What it does** — A secret that must not authenticate a real deployment: empty (or
whitespace-only), the public dev placeholder, or shorter than `MIN_JWT_SECRET_LEN`.

**Dependencies** — the constants (this file).

**Used by** — `resolve_jwt_secret`, unit tests (this file).

**Repeated context** — none.

---

## fn resolve_jwt_secret

**Identification** — private function; marker `// md:fn resolve_jwt_secret`.

**Code** — complete and verbatim:

```rust
// md:fn resolve_jwt_secret
fn resolve_jwt_secret() -> String {
    let raw = std::env::var("JWT_SECRET").ok();
    match raw {
        Some(s) if !is_weak_secret(&s) => s,
        other => {
            if dev_insecure() {
                tracing::warn!(
                    "KEEPLIN_DEV_INSECURE=1: using an insecure JWT_SECRET — device tokens are \
                     forgeable. NEVER do this in production."
                );
                other
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| DEV_JWT_SECRET.into())
            } else {
                panic!(
                    "JWT_SECRET must be set to a strong random secret of at least \
                     {MIN_JWT_SECRET_LEN} characters (not empty and not the dev placeholder). \
                     Without it, device tokens can be forged. Set JWT_SECRET, or set \
                     KEEPLIN_DEV_INSECURE=1 for local development only."
                );
            }
        }
    }
}
```

**What it does** — Resolves `JWT_SECRET`, refusing to start on a missing, empty,
too-short, or placeholder value (issue #19): a strong secret is returned as-is; a
weak/missing one **panics** with an actionable message — unless
`KEEPLIN_DEV_INSECURE=1`, which downgrades the refusal to a loud warning and falls
back to the provided weak value or, if none, the dev placeholder (local development
only; tokens are forgeable).

**Dependencies** — `dev_insecure`, `is_weak_secret`, `DEV_JWT_SECRET` (this file);
`tracing`.

**Used by** — `Config::from_env` (this file).

**Repeated context** — Fail-fast startup: like `DATABASE_URL`'s `expect` and
`AT_REST_KEY` validation in `main.rs`, a security-critical misconfiguration aborts
the boot rather than degrading silently.

---

## impl Config

**Identification** — inherent impl block; marker `// md:impl Config`. Contains
`fn from_env` (next section).

**Code** — container: members documented as sub-blocks below: fn from_env.

**What it does** — Construction from the environment; the struct has no other
behaviour.

**Dependencies** — `Config` (this file).

**Used by** — see `fn from_env`.

**Repeated context** — none beyond the method's own (below).

### fn from_env

**Identification** — associated function; marker `// md:impl Config > fn from_env`.

**Code** — complete and verbatim:

```rust
    // md:impl Config > fn from_env
    pub fn from_env() -> Self {
        Self {
            port: std::env::var("PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3000),
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            jwt_secret: resolve_jwt_secret(),
            token_ttl_days: std::env::var("TOKEN_TTL_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(365),
            retention_days: std::env::var("CHANGES_RETENTION_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            lines_gc_days: std::env::var("LINES_GC_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30),
            resource_purge_days: env_parse("RESOURCE_PURGE_DAYS", 0),
            db_max_connections: env_parse("DB_MAX_CONNECTIONS", 10),
            db_acquire_timeout_secs: env_parse("DB_ACQUIRE_TIMEOUT_SECS", 10),
            db_idle_timeout_secs: env_parse("DB_IDLE_TIMEOUT_SECS", 600),
            db_max_lifetime_secs: env_parse("DB_MAX_LIFETIME_SECS", 1800),
            rate_limit_per_min: env_parse("RATE_LIMIT_PER_MIN", 0),
            shutdown_grace_secs: env_parse("SHUTDOWN_GRACE_SECS", 20),
            log_json: env_parse("LOG_JSON", false),
            max_upload_bytes: env_parse("MAX_UPLOAD_BYTES", 100 * 1024 * 1024),
            max_note_body_bytes: env_parse("MAX_NOTE_BODY_BYTES", 25 * 1024 * 1024),
            max_user_storage_bytes: env_parse("MAX_USER_STORAGE_BYTES", 0),
            max_notes_per_user: env_parse("MAX_NOTES_PER_USER", 0),
            at_rest_key: std::env::var("AT_REST_KEY")
                .ok()
                .filter(|k| !k.trim().is_empty()),
            mail_webhook_url: std::env::var("MAIL_WEBHOOK_URL")
                .ok()
                .filter(|u| !u.trim().is_empty()),
            mail_webhook_token: std::env::var("MAIL_WEBHOOK_TOKEN")
                .ok()
                .filter(|t| !t.trim().is_empty()),
            email_token_ttl_secs: env_parse("EMAIL_TOKEN_TTL_SECS", 3600),
            email_verification_required: env_parse("EMAIL_VERIFICATION_REQUIRED", false),
            login_max_failures: env_parse("LOGIN_MAX_FAILURES", 10),
            login_lockout_secs: env_parse("LOGIN_LOCKOUT_SECS", 300),
            registration_enabled: env_parse("REGISTRATION_ENABLED", true),
            history_since_access: std::env::var("HISTORY_VISIBILITY")
                .map(|v| v.eq_ignore_ascii_case("access"))
                .unwrap_or(false),
        }
    }
```

**What it does** — Reads every field per the table under *Config*: `DATABASE_URL`
via `expect` (hard requirement), `jwt_secret` via `resolve_jwt_secret` (strength
gate), the optional strings (`AT_REST_KEY`, `MAIL_WEBHOOK_URL`, `MAIL_WEBHOOK_TOKEN`)
filtered so blank means unset, `HISTORY_VISIBILITY` mapped `access` →
`history_since_access = true` (anything else → `false`), and everything else through
`env_parse` with its default. Panics only on the two hard requirements.

**Dependencies** — `env_parse`, `resolve_jwt_secret` (this file); `std::env`.

**Used by** — `main.rs` and `bin/reencrypt.rs` (the two binaries). Tests do **not**
call it — they build `Config` literals so the environment can't leak into test
behaviour.

**Repeated context** — Blank-string filtering on the optional values matches how
operators comment out env vars; `0`-disables is uniform across the numeric knobs.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`. Two
tests, below.

**Code** — container: members documented as sub-blocks below: fn weak_secrets_are_rejected, fn a_strong_secret_is_accepted.

**What it does** — Pins the `JWT_SECRET` strength gate (pure functions only — no
environment mutation, so the tests are parallel-safe).

**Dependencies** — `super::*`.

**Used by** — `cargo test` only.

**Repeated context** — Issue #19's contract in executable form.

### fn weak_secrets_are_rejected

**Identification** — `#[test]`; marker
`// md:mod tests > fn weak_secrets_are_rejected`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn weak_secrets_are_rejected
    #[test]
    fn weak_secrets_are_rejected() {
        assert!(is_weak_secret(""));
        assert!(is_weak_secret("   "));
        assert!(is_weak_secret(DEV_JWT_SECRET));
        assert!(is_weak_secret("short"));
        assert!(is_weak_secret(&"x".repeat(MIN_JWT_SECRET_LEN - 1)));
    }
```

**What it does** — Empty, whitespace-only, the dev placeholder, `"short"`, and a
15-char string are all weak.

**Dependencies / Used by** — `is_weak_secret`; `cargo test`.

**Repeated context** — none.

### fn a_strong_secret_is_accepted

**Identification** — `#[test]`; marker
`// md:mod tests > fn a_strong_secret_is_accepted`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn a_strong_secret_is_accepted
    #[test]
    fn a_strong_secret_is_accepted() {
        assert!(!is_weak_secret(&"x".repeat(MIN_JWT_SECRET_LEN)));
        assert!(!is_weak_secret("a-genuinely-long-random-production-secret"));
    }
```

**What it does** — A 16-char string (the exact minimum) and a long random-looking
secret pass the gate.

**Dependencies / Used by** — `is_weak_secret`; `cargo test`.

**Repeated context** — none.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh
with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `Config` — defined here (EXTRACTED; 11 cross-file edge(s))
- `env_parse()` — defined here (EXTRACTED; file-local)
- `dev_insecure()` — defined here (EXTRACTED; file-local)
- `is_weak_secret()` — defined here (EXTRACTED; file-local)
- `resolve_jwt_secret()` — defined here (EXTRACTED; file-local)
- `.from_env()` — defined here (EXTRACTED; file-local)
- `weak_secrets_are_rejected()` — defined here (EXTRACTED; file-local)
- `a_strong_secret_is_accepted()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×2; e.g. `AppState`, `.new()`)
- `crates/keeplin-srv/tests/collab.rs` — collaborative channel & hardening tests (EXTRACTED: references×1; e.g. `test_config()`)
- `crates/keeplin-srv/tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries (EXTRACTED: references×1; e.g. `test_config()`)
- `crates/keeplin-srv/tests/integration.rs` — device relay tests (real `DbBackend`) (EXTRACTED: references×2; e.g. `spawn_server_with_config()`, `test_config()`)
- `crates/keeplin-srv/tests/materialize.rs` — domain-entity materialisation tests (EXTRACTED: references×1; e.g. `test_config()`)
- `crates/keeplin-srv/tests/quotas.rs` — per-user quota enforcement tests (EXTRACTED: references×2; e.g. `quota_config()`, `spawn()`)
- `crates/keeplin-srv/tests/reencrypt.rs` — re-encrypt pass tests (EXTRACTED: references×1; e.g. `test_config()`)
- `crates/keeplin-srv/tests/soak.rs` — multi-instance collaborative soak/load drill (EXTRACTED: references×1; e.g. `test_config()`)

## Coverage checklist

Every code block of `config.rs`, in source order, each documented above (five points)
and carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | `struct Config` | `// md:Config` | Config |
| 2 | `fn env_parse` | `// md:fn env_parse` | fn env_parse |
| 3 | `DEV_JWT_SECRET` / `MIN_JWT_SECRET_LEN` | `// md:JWT secret constants` | JWT secret constants |
| 4 | `fn dev_insecure` | `// md:fn dev_insecure` | fn dev_insecure |
| 5 | `fn is_weak_secret` | `// md:fn is_weak_secret` | fn is_weak_secret |
| 6 | `fn resolve_jwt_secret` | `// md:fn resolve_jwt_secret` | fn resolve_jwt_secret |
| 7 | `impl Config` | `// md:impl Config` | impl Config |
| 8 | `fn from_env` | `// md:impl Config > fn from_env` | impl Config › fn from_env |
| 9 | `mod tests` | `// md:mod tests` | mod tests |
| 10 | `fn weak_secrets_are_rejected` | `// md:mod tests > fn weak_secrets_are_rejected` | mod tests › fn weak_secrets_are_rejected |
| 11 | `fn a_strong_secret_is_accepted` | `// md:mod tests > fn a_strong_secret_is_accepted` | mod tests › fn a_strong_secret_is_accepted |
