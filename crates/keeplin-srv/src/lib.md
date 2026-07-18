# `lib.rs` — keeplin-srv library root

Self-contained companion for `crates/keeplin-srv/src/lib.rs`. It documents **every code
block of the source file, in source order** — a reader with only this file must be able to
understand `lib.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `lib.rs` carries exactly one marker comment of the
form `// md:<Header> > … > <Block header>`, whose path is the header chain of the section
documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — the file's single block: the crate's module declarations. Marker
`// md:Overview` at the top of the file.

```rust
pub mod auth;
pub mod bus;
pub mod collab;
pub mod config;
pub mod crypto;
pub mod error;
pub mod http;
pub mod mail;
pub mod permissions;
pub mod protocol;
pub mod ratelimit;
pub mod reencrypt;
pub mod state;
pub mod store;
pub mod sync;
```

**What it does** — Declares the public modules of the `keeplin-srv` crate, one per source
file, and nothing else: no logic, no re-exports, no attributes. Making the crate a library
(with `main.rs` as a thin binary on top) is what lets the integration tests in `tests/`
build the full server — `http::router(state)` + `state::AppState::new(config, pool)` —
against a throwaway database without touching the binary. The absence of re-exports is
deliberate: every import in the codebase names its origin module
(`keeplin_srv::store::Store`), so a reader always sees where a type comes from.

The module map, with each module's one-line role:

| Module | Role |
|--------|------|
| `auth` | Argon2 password hashing, JWT device-token mint/verify, the auth middleware with its device-revocation check, the `AuthedUser` extractor |
| `bus` | cross-instance coordination over Postgres `LISTEN/NOTIFY` (multi-replica collab/relay fan-out, issue #45) |
| `collab` | the collaborative line-editing session engine behind `GET /api/ws` |
| `config` | `Config`, loaded from environment variables at startup |
| `crypto` | optional at-rest AES-256-GCM encryption of `notes.title` / `lines.content` (`AT_REST_KEY`, issue keeplin#110) |
| `error` | `AppError`, the crate-wide error enum, and its HTTP status/JSON mapping |
| `http` | the axum router and every REST handler (`/api/*`), plus `PROTOCOL_VERSION` |
| `mail` | delegated email delivery via the operator's mail webhook (issue #49; the server never speaks SMTP) |
| `permissions` | capability bitsets (`read`/`write`/`share_read`/`share_write`/`manage`) and note/notebook access resolution |
| `protocol` | the JSON wire types of the collaborative channel |
| `ratelimit` | per-IP token-bucket rate limiter and its middleware |
| `reencrypt` | the one-off pass migrating pre-key plaintext rows to `enc:v1:` ciphertext |
| `state` | `AppState`, the shared context every handler holds |
| `store` | the single PostgreSQL data-access layer — all SQL lives there |
| `sync` | the device sync relay behind `GET /api/sync` (keeplin-core `DbBackend` wire protocol) |

**Dependencies** — none: the file references only its own submodules.

**Used by** — everything. `main.rs` and `src/bin/reencrypt.rs` consume the crate as
`keeplin_srv::…`; every integration test under `tests/` does the same; within the crate,
sibling modules reach each other through these declarations (`crate::store::Store`, …).

**Repeated context** — Project convention: **every `.rs` has a companion `.md`**
(same path, `.rs` → `.md`), enforced by CI (`scripts/check-docs.sh`); a new module means a
new entry here, a new source file, and a new companion. The companion system is LAYER 2 of
the repo's navigation model; the Graphify graph (`graphify-out/graph.json`) is LAYER 1.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- (no symbols extracted for this file — it contributes only its file node) (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

Every code block of `lib.rs`, in source order, each documented above (five points) and
carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | module declarations (`pub mod …` ×15) | `// md:Overview` | Overview |
