# `tests/reencrypt.rs` — re-encrypt pass tests

## What is tested

The one-off at-rest re-encrypt pass (`keeplin_srv::reencrypt::run`, the engine behind the
`keeplin-reencrypt` binary) against a throwaway PostgreSQL database (`#[sqlx::test]`). The
seeding deliberately reproduces the real pre-key scenario: rows are written through a **real
server instance with `AT_REST_KEY` unset** (plaintext at rest), then the pass runs with a key,
then a **second server instance with the key** must still serve the original plaintext.

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `reencrypts_pre_key_rows_and_server_still_serves_plaintext` | seed plaintext via keyless server; run pass with `batch_size: 1` | every raw `notes.title` / `lines.content` value starts with `enc:v1:`; a keyed server serves the original title/body; a second run scans 0 rows (idempotence) |
| `dry_run_reports_but_does_not_modify` | seed plaintext; run with `dry_run: true` | stats report 1 title + 2 lines scanned, 0 rewritten; raw column values are byte-identical before/after |
| `refuses_to_run_without_a_key` | run with a disabled cipher | `Err` — a keyless run must not report success |

`batch_size: 1` in the first test forces multiple batches (1 note + 2 lines), exercising the
keyset pagination, per-batch transactions, and the resume-friendly loop rather than a single
lucky batch.

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `test_config(at_rest_key)` | server `Config` with the key as the only variable |
| `spawn_server(pool, at_rest_key)` | boot the real router on an ephemeral port, with or without the cipher |
| `seed_note` | register + login + `POST /api/import` ("Secret title", two lines) over real HTTP |
| `raw_values` | raw `SELECT title/content` — asserts on stored bytes, not decrypted views |
| `test_key()` | fixed base64 32-byte key |

## Graph context

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

**Invariants** (restated on purpose; a change to this file must keep these true)

- Seeding goes through a REAL keyless server instance (genuine pre-key plaintext), assertions on raw column bytes plus a keyed server's decrypted reads.
- The dry-run test must assert byte-identical rows before/after.
- `batch_size: 1` in the main test intentionally exercises multi-batch pagination/resume behaviour.

## Related files

- `../src/reencrypt.rs` — the pass under test (idempotence/batching/guard invariants).
- `../src/bin/reencrypt.rs` — the CLI wrapper (not spawned here; logic is tested in-process).
- `../src/crypto.rs` — `Cipher` and the `enc:v1:` tag asserted on.
