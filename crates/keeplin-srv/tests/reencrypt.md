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

## Related files

- `../src/reencrypt.rs` — the pass under test (idempotence/batching/guard invariants).
- `../src/bin/reencrypt.rs` — the CLI wrapper (not spawned here; logic is tested in-process).
- `../src/crypto.rs` — `Cipher` and the `enc:v1:` tag asserted on.
