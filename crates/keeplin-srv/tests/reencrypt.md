# `tests/reencrypt.rs` — re-encrypt pass tests

Self-contained companion for `crates/keeplin-srv/tests/reencrypt.rs`. It documents
**every code block of the source file, in source order** — a reader with only this file
must be able to understand the suite without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**What it does** — Tests of the one-off at-rest re-encrypt pass
(`keeplin_srv::reencrypt::run`, the engine behind the `keeplin-reencrypt` binary)
against a throwaway `#[sqlx::test]` PostgreSQL database. The seeding deliberately
reproduces the real pre-key scenario: rows are written through a **real server
instance with `AT_REST_KEY` unset** (genuine plaintext at rest), the pass then runs
with a key, and a **second server instance holding the key** must still serve the
original plaintext.

**Dependencies** — `keeplin_srv` (`Config`, `Cipher`, `router`, `reencrypt`,
`AppState`, `crypto::ENC_PREFIX`), `reqwest`, `sqlx`, `base64`, `axum`, `tokio`,
`serde_json`, `uuid`.

**Used by** — `cargo test`; CI.

**Repeated context** — The at-rest model (issue keeplin#110): values are stored
plaintext (pre-key) or `enc:v1:<base64(nonce‖ciphertext)>`; both decrypt, so a mixed
database is healthy and the pass migrates the mix forward. The pass is library code
precisely so this suite can drive it in-process (no subprocess).

---

## fn test_key

**Identification** — helper; marker `// md:fn test_key`. A fixed valid base64
32-byte key (`[9u8; 32]`).

**Dependencies** — `base64`. **Used by** — the keyed steps of the tests.
**Repeated context** — deterministic keys are fine in tests; validity is what
matters.

---

## fn test_config

**Identification** — helper; marker `// md:fn test_config`.
`fn test_config(at_rest_key: Option<String>) -> Config` — the standard test config
with the key as the **only variable**, so a test can spawn keyless and keyed servers
over the same database.

**Dependencies** — `Config`. **Used by** — `spawn_server`.
**Repeated context** — none.

---

## fn spawn_server

**Identification** — helper; marker `// md:fn spawn_server`.
`async fn spawn_server(pool, at_rest_key) -> SocketAddr` — boots the real router
(with `ConnectInfo`) with or without the cipher, on an ephemeral port.

**Dependencies** — `AppState::new`, `router`. **Used by** — the seeding and
serve-back steps. **Repeated context** — `AppState::new` builds the `Cipher` from
the config, so the keyless instance genuinely writes plaintext.

---

## fn seed_note

**Identification** — helper; marker `// md:fn seed_note`.
`async fn seed_note(addr, title, body) -> (String, Uuid)` — register + login +
`POST /api/import` over real HTTP, creating one note with two lines; returns the
token and note id.

**Dependencies** — `reqwest`. **Used by** — the first two tests.
**Repeated context** — import splits the body on `\n` into versioned lines — hence
"two lines" for `"line one\nline two"`.

---

## fn raw_values

**Identification** — helper; marker `// md:fn raw_values`.
`async fn raw_values(pool) -> (Vec<String>, Vec<String>)` — raw
`SELECT title FROM notes` / `SELECT content FROM lines`, each **ordered by the
selected value itself** so the result is deterministic (`lines.id` is a random
UUIDv4 — ordering by id flaked). Asserts are on **stored bytes**, never decrypted
views.

**Dependencies** — sqlx. **Used by** — the first two tests.

**Repeated context** — Raw-column reads are the point: the suite pins the storage
form (`enc:v1:` tag), not API behaviour.

---

## fn reencrypts_pre_key_rows_and_server_still_serves_plaintext

**Identification** — `#[sqlx::test]`; marker
`// md:fn reencrypts_pre_key_rows_and_server_still_serves_plaintext`.

**What it does** — Seed plaintext via a **keyless** server ("Secret title", two
lines) and confirm the raw values are plaintext. Run the pass with the key and
`batch_size: 1` — deliberately forcing multiple batches (1 note + 2 lines) so the
keyset pagination, per-batch transactions and resume loop are exercised rather than
one lucky batch; stats must report 1 title + 2 lines rewritten. Then: every raw
stored value starts with `ENC_PREFIX`; a **keyed** server instance still serves the
original title and body over REST; and a second `run` scans 0 rows (idempotence —
the operator's completion signal).

**Dependencies** — all helpers; `Cipher::from_key`, `reencrypt::{run, Options}`,
`ENC_PREFIX`. **Used by** — `cargo test`.

**Repeated context** — Pins the pass's core contract: idempotent, batched,
mixed-state-safe, and lossless (the keyed server proves decryptability).

---

## fn dry_run_reports_but_does_not_modify

**Identification** — `#[sqlx::test]`; marker
`// md:fn dry_run_reports_but_does_not_modify`.

**What it does** — Seed plaintext; run with `dry_run: true`: stats report 1 title +
2 lines **scanned** and 0 rewritten, and the raw column values are **byte-identical**
before/after — a dry run issues no `UPDATE` at all.

**Dependencies** — the helpers; `reencrypt`. **Used by** — `cargo test`.

**Repeated context** — Pins the `--dry-run` inertness contract.

---

## fn refuses_to_run_without_a_key

**Identification** — `#[sqlx::test]`; marker
`// md:fn refuses_to_run_without_a_key`.

**What it does** — `run` with a disabled cipher (`from_key(None)`) is an `Err`: a
keyless run reporting success would be a silent misfire for an administrative tool.

**Dependencies** — `Cipher`, `reencrypt`. **Used by** — `cargo test`.

**Repeated context** — Fail-fast tooling, same philosophy as the server refusing a
malformed key at startup.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `test_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `spawn_server()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `test_key()` — defined here (EXTRACTED; file-local)
- `seed_note()` — defined here (EXTRACTED; file-local)
- `raw_values()` — defined here (EXTRACTED; file-local)
- `reencrypts_pre_key_rows_and_server_still_serves_plaintext()` — defined here (EXTRACTED; file-local)
- `dry_run_reports_but_does_not_modify()` — defined here (EXTRACTED; file-local)
- `refuses_to_run_without_a_key()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×1; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports | `// md:Overview` |
| 2 | `fn test_key` | `// md:fn test_key` |
| 3 | `fn test_config` | `// md:fn test_config` |
| 4 | `fn spawn_server` | `// md:fn spawn_server` |
| 5 | `fn seed_note` | `// md:fn seed_note` |
| 6 | `fn raw_values` | `// md:fn raw_values` |
| 7 | `fn reencrypts_pre_key_rows_and_server_still_serves_plaintext` | `// md:fn reencrypts_pre_key_rows_and_server_still_serves_plaintext` |
| 8 | `fn dry_run_reports_but_does_not_modify` | `// md:fn dry_run_reports_but_does_not_modify` |
| 9 | `fn refuses_to_run_without_a_key` | `// md:fn refuses_to_run_without_a_key` |
