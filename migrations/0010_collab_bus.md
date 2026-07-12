# 0010 — cross-instance collaboration bus (issue #45)

Adds the two tables that let the collaborative channel and the device relay work
across **multiple server replicas**, coordinated over Postgres `LISTEN/NOTIFY`
(no new infrastructure).

## `collab_events` — the op-fan-out outbox

Every applied collaborative op batch is inserted here; the `seq BIGSERIAL` is the
note's fan-out sequence number. A single global sequence is monotonic per note,
so it replaces the old in-process `AtomicU64` per-session counter that collided
once more than one instance served the same note.

Flow: apply → `INSERT … RETURNING seq` → `pg_notify('collab_op', '<seq>:<origin_instance>:<origin_conn>')`.
Every instance's listener loads the row by `seq` and delivers it to its local
subscribers of `note_id`; the origin instance skips `origin_conn` (the author,
which already applied the op optimistically).

The table is a short-lived delivery buffer — the lines and order rows are the
durable source of truth — so it is pruned by `created_at` age in the maintenance
loop (`idx_collab_events_created`).

## `collab_presence` — merged presence

One row per connected subscriber, keyed by `(note_id, instance_id, conn_id)`. On
join/leave/cursor an instance rewrites its own rows and notifies
`collab_presence` with the `note_id`; every instance then rebuilds the merged
presence list for its local subscribers. `updated_at` is heartbeat-touched, so
rows orphaned by a crashed instance are swept by age
(`idx_collab_presence_updated`); `idx_collab_presence_instance` supports the
per-instance cleanup on shutdown/startup.

## Concurrency

The order read-modify-write in `apply_op` now runs under a Postgres advisory lock
keyed by `note_id` (`pg_advisory_xact_lock`), so two instances editing the same
note's line order serialise at the database instead of relying on the (per-process)
in-memory apply lock — closing the lost-update window across replicas.
