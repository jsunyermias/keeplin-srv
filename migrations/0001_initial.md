# `0001_initial.sql` — accounts, devices, and the relay journal

## Purpose

The first schema migration, applied at startup by `sqlx::migrate!`. Creates the account and
device tables and the durable journal that backs the device sync relay (`/api/sync`). Run once
per database, in order, before the server accepts requests.

## What it defines

| Table | Purpose |
|-------|---------|
| `users` | one row per account (`email` unique, `password_hash`) |
| `user_devices` | one row per device login; the device id is the relay's per-connection identity |
| `changes` | the relay journal: every `Change` batch received, in arrival order, payload as opaque `JSONB` |
| `device_cursors` | per-device delivery watermark (`last_seq`) |

Indexes: `idx_user_devices_user`, `idx_changes_user_seq`.

## Notes & gotchas

- **Forward-only**: sqlx migrations have no `DOWN`. Correct a mistake with a new corrective
  migration, not a rollback.
- `changes.(batch_id, batch_index)` is `UNIQUE` — this is what dedupes a client's batch retry
  after a reconnect (the second insert is a no-op).
- `changes.payload` is deliberately opaque `JSONB`: the relay never parses keeplin-core's
  `Change` enum, so the client's model can evolve without a server migration.
- A device with no `device_cursors` row starts at `0` and receives the full journal on first
  connect.

## Related files

- `../crates/keeplin-srv/src/sync.md` — the relay that reads/writes these tables.
- `../crates/keeplin-srv/src/store.md` — the queries.
- `0002_collab.md` — the collaborative note model added next.
