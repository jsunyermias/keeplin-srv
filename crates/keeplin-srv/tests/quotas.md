# `tests/quotas.rs` — per-user quota enforcement tests

Self-contained companion for `crates/keeplin-srv/tests/quotas.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand the suite without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**What it does** — Tests of the two optional per-user quotas (both `0` = unlimited by
default) plus the registration switch, driven over real HTTP against a real server on
a throwaway `#[sqlx::test]` PostgreSQL database: the note-count cap
(`MAX_NOTES_PER_USER`, enforced at `POST /api/notes`), the total resource-blob
storage cap (`MAX_USER_STORAGE_BYTES`, enforced at `PUT /api/resources/:id/data`),
and `REGISTRATION_ENABLED=false` (issue #21). Quota rejections are
`507 Insufficient Storage`.

**Dependencies** — `keeplin_srv` (`Config`, `router`, `AppState`), keeplin-core
(`DbBackend`, `Resource`, repository/sync traits — the relay is needed to seed
resource metadata), `reqwest`, `sqlx`, `tempfile`, `tokio`, `serde_json`, `uuid`.

**Used by** — `cargo test`; CI.

**Repeated context** — Quotas are enforced **before** storage at the REST write
point; storage accounting measures actual stored bytes (`octet_length`), counts only
**live** blobs, and excludes the resource being overwritten (no double-count) —
`store.rs::user_blob_bytes_excluding`. Resources are per-user, so the storage quota
is naturally per-account.

---

## fn quota_config

**Identification** — helper; marker `// md:fn quota_config`.
`fn quota_config(max_user_storage_bytes: i64, max_notes_per_user: i64) -> Config`.

**What it does** — The suite's `Config` literal with the two quota knobs as the only
variables (everything else standard test posture: open registration, no rate
limit/lockout/key).

**Dependencies** — `Config`. **Used by** — every test.

**Repeated context** — Config literals (never `from_env`) keep the environment out
of test behaviour; a new `Config` field breaks all suites loudly at compile time.

---

## fn spawn

**Identification** — helper; marker `// md:fn spawn`.
`async fn spawn(pool: PgPool, config: Config) -> SocketAddr`.

**What it does** — Boots the real router with the given config on an ephemeral
loopback port (with `ConnectInfo`, required by the rate-limit middleware's
extractor), on a spawned task.

**Dependencies** — `AppState::new`, `router`. **Used by** — every test.

**Repeated context** — none.

---

## fn register

**Identification** — helper; marker `// md:fn register`. REST registration (fixed
password). **Dependencies** — `reqwest`. **Used by** — every quota test.
**Repeated context** — none.

## fn login

**Identification** — helper; marker `// md:fn login`. REST login returning the
device token. **Dependencies** — `reqwest`. **Used by** — every quota test.
**Repeated context** — none.

---

## fn post_note

**Identification** — helper; marker `// md:fn post_note`.
`async fn post_note(addr, token) -> u16` — POST `/api/notes` with a minimal body,
returning the HTTP status code (the tests assert 200 vs 507).

**Dependencies** — `reqwest`. **Used by** — the note-quota tests.

**Repeated context** — none.

---

## fn device

**Identification** — helper; marker `// md:fn device`.
`async fn device(addr, token) -> DbBackend` — a real server-mode relay client on a
leaked temp SQLite file, connected to `ws://…/api/sync`.

**Dependencies** — keeplin-core `DbBackend::new`, `tempfile`. **Used by** —
`seed_resource` callers (storage-quota tests).

**Repeated context** — Resource **metadata** only travels the relay; the blob is
out-of-band — which is exactly why the tests need a relay device to seed metadata
before `PUT`ting bytes.

---

## fn seed_resource

**Identification** — helper; marker `// md:fn seed_resource`.
`async fn seed_resource(dev: &DbBackend) -> Uuid`.

**What it does** — Creates resource metadata with an **empty** blob through the
relay (`create_resource` → `get_changes_since(epoch)` → `send_changes`, then a
short sleep for materialisation) and returns its id — so the test controls the
stored size purely via `put_blob`; the quota measures actual stored bytes, not the
declared `size`.

**Dependencies** — keeplin-core resource/sync APIs. **Used by** — the storage-quota
tests.

**Repeated context** — none.

---

## fn put_blob

**Identification** — helper; marker `// md:fn put_blob`.
`async fn put_blob(addr, token, id, len) -> u16` — PUT `len` bytes to
`/api/resources/:id/data`, returning the status code.

**Dependencies** — `reqwest`. **Used by** — the storage-quota tests.

**Repeated context** — none.

---

## fn registration_can_be_disabled

**Identification** — `#[sqlx::test]`; marker
`// md:fn registration_can_be_disabled`.

**What it does** — With `registration_enabled = false`, `POST /api/register`
answers `403` (issue #21): the open signup endpoint is closed while everything else
still runs.

**Dependencies** — `quota_config`, `spawn`. **Used by** — `cargo test`.

**Repeated context** — Pins the issue #21 switch.

---

## fn note_quota_blocks_creation_past_the_limit

**Identification** — `#[sqlx::test]`; marker
`// md:fn note_quota_blocks_creation_past_the_limit`.

**What it does** — Limit 2: the first two `POST /api/notes` are 200, the third is
**507**.

**Dependencies** — the helpers. **Used by** — `cargo test`.

**Repeated context** — The count is of **live owned** notes
(`count_live_notes_for_user`) — soft-deleted notes don't consume quota.

---

## fn note_quota_disabled_by_default

**Identification** — `#[sqlx::test]`; marker
`// md:fn note_quota_disabled_by_default`.

**What it does** — Limit `0` (the default): five creations all succeed — `0` means
unlimited, the backward-compatible posture.

**Dependencies** — the helpers. **Used by** — `cargo test`.

**Repeated context** — `0`-disables is the crate-wide convention for optional
limits.

---

## fn storage_quota_blocks_upload_over_the_limit

**Identification** — `#[sqlx::test]`; marker
`// md:fn storage_quota_blocks_upload_over_the_limit`.

**What it does** — Limit 100 bytes, two seeded resources A and B: 50 into A → 200;
re-upload 50 into A → 200 (an **overwrite is not double-counted** — measured by its
new size); 60 into B → **507** (50+60 > 100); 40 into B → 200 (50+40 ≤ 100).

**Dependencies** — the helpers. **Used by** — `cargo test`.

**Repeated context** — Pins the `user_blob_bytes_excluding` accounting rule.

---

## fn storage_quota_isolated_per_user

**Identification** — `#[sqlx::test]`; marker
`// md:fn storage_quota_isolated_per_user`.

**What it does** — Two accounts, limit 100 each: A fills its budget (100 → 200);
B still uploads its own 100 → 200 (unaffected); A's next 1-byte upload → **507**.

**Dependencies** — the helpers. **Used by** — `cargo test`.

**Repeated context** — Quota scoping is per-user, like all durable data.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `spawn()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `quota_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `register()` — defined here (EXTRACTED; file-local)
- `login()` — defined here (EXTRACTED; file-local)
- `post_note()` — defined here (EXTRACTED; file-local)
- `device()` — defined here (EXTRACTED; file-local)
- `seed_resource()` — defined here (EXTRACTED; file-local)
- `put_blob()` — defined here (EXTRACTED; file-local)
- `registration_can_be_disabled()` — defined here (EXTRACTED; file-local)
- `note_quota_blocks_creation_past_the_limit()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×2; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports | `// md:Overview` |
| 2 | `fn quota_config` | `// md:fn quota_config` |
| 3 | `fn spawn` | `// md:fn spawn` |
| 4 | `fn register` | `// md:fn register` |
| 5 | `fn login` | `// md:fn login` |
| 6 | `fn post_note` | `// md:fn post_note` |
| 7 | `fn device` | `// md:fn device` |
| 8 | `fn seed_resource` | `// md:fn seed_resource` |
| 9 | `fn put_blob` | `// md:fn put_blob` |
| 10 | `fn registration_can_be_disabled` | `// md:fn registration_can_be_disabled` |
| 11 | `fn note_quota_blocks_creation_past_the_limit` | `// md:fn note_quota_blocks_creation_past_the_limit` |
| 12 | `fn note_quota_disabled_by_default` | `// md:fn note_quota_disabled_by_default` |
| 13 | `fn storage_quota_blocks_upload_over_the_limit` | `// md:fn storage_quota_blocks_upload_over_the_limit` |
| 14 | `fn storage_quota_isolated_per_user` | `// md:fn storage_quota_isolated_per_user` |
