# `src/reencrypt.rs` — one-off at-rest re-encrypt pass

## Purpose

Rewrites rows that predate `AT_REST_KEY` from plaintext to the `enc:v1:` encrypted form.
`src/crypto.rs` reads both forms, so enabling the key on a live database is safe — but nothing
ever migrated the old rows until this pass. It is library code (so tests can drive it against
a `#[sqlx::test]` database); the thin `src/bin/reencrypt.rs` binary wraps it for operators.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `Options` | struct | `dry_run: bool` (report, write nothing) + `batch_size: i64` (rows per transaction; default 500) |
| `TableStats` | struct | per-column counters: `scanned` (plaintext rows seen), `rewritten`, `skipped_concurrent` (guard failed — row changed mid-pass) |
| `Stats` | struct | `notes_title` + `lines_content`, one `TableStats` each |

## Public API

| Function | Description |
|----------|-------------|
| `run(pool, cipher, opts) -> Result<Stats, AppError>` | Re-encrypts `notes.title` then `lines.content`. **Errors if the cipher is disabled** — a keyless run would report success while doing nothing. |

## The pass — batching, resumability, live-server safety

Per column, a keyset-paginated loop:

```sql
SELECT id, <col> AS value FROM <table>
WHERE <col> NOT LIKE 'enc:v1:%' AND id > $last ORDER BY id LIMIT $batch
```

then (unless `dry_run`) one transaction per batch of guarded updates:

```sql
UPDATE <table> SET <col> = $encrypted WHERE id = $id AND <col> = $plaintext_we_read
```

Invariants that make this an operator-safe tool:

- **Idempotent**: encrypted rows never match `NOT LIKE 'enc:v1:%'`; a completed pass re-run is
  a no-op (the tag literal is `crypto::ENC_PREFIX` — single source of truth).
- **Bounded transactions**: at most `batch_size` rows per commit; never a whole-table lock.
- **Resumable**: committed batches survive an interruption; the next run selects only what is
  left. The keyset (`id > last`) additionally guarantees forward progress within a run even if
  every update in a batch is skipped.
- **Live-server safe**: the `AND <col> = <plaintext>` guard means a row rewritten concurrently
  by the running server (which holds the same key and encrypts all new writes) is skipped,
  never clobbered — counted as `skipped_concurrent`.
- **`dry_run` writes nothing**: no `UPDATE` statement is even issued.
- Progress is logged per batch via `tracing` (table, column, cumulative counters).

`table`/`column` names are interpolated into the SQL but come only from the two hard-coded call
sites in `run` — never from input.

## Design notes

- Library-module + thin-binary split so `tests/reencrypt.rs` can exercise the real pass under
  `#[sqlx::test]` without spawning a subprocess.
- Optimistic per-row guard instead of `SELECT … FOR UPDATE`: the pass must not hold locks that
  stall a live server; losing a race is fine because the winner's write is already encrypted.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `run()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `reencrypt_column()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `Options` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.default()` — defined here (EXTRACTED; file-local)
- `TableStats` — defined here (EXTRACTED; file-local)
- `Stats` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/crypto.rs` — at-rest encryption of note titles and line content (EXTRACTED: references×2; e.g. `Cipher`)
- `crates/keeplin-srv/src/error.rs` — the API error type (EXTRACTED: imports_from×1, references×2; e.g. `AppError`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/bin/reencrypt.rs` — `keeplin-reencrypt` CLI wrapper (EXTRACTED: references×1; e.g. `parse_args()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Idempotent: rows already tagged `enc:v1:` are never selected; a completed pass re-run is a no-op.
- Bounded transactions (`batch_size` rows per commit) — never a whole-table transaction or lock.
- Live-server safe: every UPDATE is guarded by `AND <col> = <the plaintext read>`; losing a race skips the row (the server's concurrent write is already encrypted).
- `dry_run` issues no UPDATE at all.
- Refuses to run with a disabled cipher — a keyless 'success' would be a silent misfire.

## Related files

- `src/crypto.rs` — the `Cipher` and the `enc:v1:` (`ENC_PREFIX`) tag format this pass targets.
- `src/bin/reencrypt.rs` — the operator-facing CLI wrapper.
- `tests/reencrypt.rs` — end-to-end tests (seed plaintext via a keyless server, re-encrypt, verify).
- `RUNBOOK.md` ("Key rotation & re-encryption") — when and how operators run this.
