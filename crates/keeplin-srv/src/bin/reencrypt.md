# `src/bin/reencrypt.rs` — `keeplin-reencrypt` CLI wrapper

## Purpose

Operator entry point for the one-off at-rest re-encrypt pass. All real logic lives in
`src/reencrypt.rs` (so tests can drive it in-process); this binary only parses flags, loads the
**same `Config` as the server** (`Config::from_env`, so it reads the same `.env`/environment —
`DATABASE_URL`, `JWT_SECRET`, `AT_REST_KEY`), opens a small pool, runs the pass, and prints a
summary.

## Usage

```text
keeplin-reencrypt [--dry-run] [--batch-size N]
```

- `--dry-run` — count the plaintext rows that would be rewritten; modify nothing.
- `--batch-size N` — rows per transaction (default 500; must be positive).
- `--help` — usage text.

Exit is non-zero on: unknown flag, missing/invalid `AT_REST_KEY` (the pass **refuses** to run
without a key — nothing to encrypt to), unreachable database, or a mid-pass error. A mid-pass
failure is safe: completed batches are committed, and re-running resumes where it stopped.

## Behaviour contract

- Requires `AT_REST_KEY` set and valid; `Config::from_env` also requires `DATABASE_URL` and a
  strong `JWT_SECRET` (or `KEEPLIN_DEV_INSECURE=1`), exactly like the server.
- Pool of 2 connections — the pass is a sequential scan; it must not compete with a live
  server for pool capacity.
- Safe to run while the server is up: new writes are already encrypted and the pass skips rows
  that change under it (see `src/reencrypt.md`).
- Idempotent: run-to-completion twice → the second run reports 0 rows found.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `parse_args()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `main()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/reencrypt.rs` — one-off at-rest re-encrypt pass (EXTRACTED: references×1; e.g. `Options`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Thin wrapper only: all pass logic lives in `src/reencrypt.rs` so tests drive it in-process.
- Loads the same `Config` as the server (same `.env`); requires a valid `AT_REST_KEY` and refuses to run without one.
- Exit code is non-zero on any failure; completed batches stay committed and a re-run resumes safely.

## Related files

- `src/reencrypt.rs` — the actual pass (batching/resume/guard semantics documented there).
- `src/config.rs` — the shared environment configuration this binary loads.
- `RUNBOOK.md` ("Key rotation & re-encryption") — the operational procedure around this tool.
