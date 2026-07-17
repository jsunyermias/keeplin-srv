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

## Related files

- `src/reencrypt.rs` — the actual pass (batching/resume/guard semantics documented there).
- `src/config.rs` — the shared environment configuration this binary loads.
- `RUNBOOK.md` ("Key rotation & re-encryption") — the operational procedure around this tool.
