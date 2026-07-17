# `bus.rs` — cross-instance coordination (issue #45)

## Purpose

Lets keeplin-srv run as **more than one replica**. The collaborative channel and
the device relay keep their live state in process memory (per-note sessions,
per-user fan-out channels), so without coordination two users on different
replicas would not see each other. `bus.rs` bridges the replicas over Postgres
`LISTEN/NOTIFY` — the database they already share, no new infrastructure.

## How it works

`spawn` starts one background task per process holding a dedicated `PgListener`.
It listens on three channels and dispatches each notification back into the
in-process collab registry / sync hub:

| Channel | Payload | Handler |
|---------|---------|---------|
| `collab_op` | `"<seq>:<origin_instance>"` | Load the `collab_events` row by `seq` and deliver its ops to local subscribers, unless we are the origin (already broadcast locally). |
| `collab_presence` | `"<note_id>:<origin_instance>"` | Rebuild the merged presence list from `collab_presence` and broadcast to local subscribers, unless we are the origin. |
| `sync_batch` | `"<user_id>:<origin_instance>"` | Wake the user's local relay connections to re-scan the journal, unless we are the origin. |

The origin instance always does its own delivery **locally and synchronously**
(so a single-instance deployment — and the test suite — never needs the bus
running), and stamps its `instance_id` on the notification so siblings act and
it skips its own echo.

## Why an outbox for ops

`NOTIFY` payloads are capped (~8 KB) and an op batch can be larger, so the ops
are written to `collab_events` and only the row `seq` is notified; a sibling
reads the row. The `seq` is a `BIGSERIAL`, so it is also the server sequence a
sibling stamps on the op when it fans it out (a connection only ever talks to one
instance, so the delivering instance's own per-session counter is what its
clients see).

## Ordering & correctness

- **No lost updates on the order.** `apply_op` runs the whole batch on the one
  connection that holds a `pg_advisory_xact_lock(note_id)` (see
  `store::lock_note_order` and the `_on(executor)` store variants), so two
  replicas editing the same note's line order serialise at the database. Running
  the batch on the *lock* connection (rather than a separate one) is also what
  keeps it from deadlocking against the bounded pool.
- **Presence self-healing.** Rows are keyed by `(note_id, instance_id, conn_id)`
  and heartbeat-touched; the maintenance loop sweeps rows a crashed instance left
  behind, and an instance clears its own rows on startup.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `spawn()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `run()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `handle_collab_op()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `handle_collab_presence()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `handle_sync_batch()` — defined here (EXTRACTED; 1 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: imports_from×1, references×5; e.g. `AppState`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- All cross-instance delivery (collab ops/presence, relay wakes) rides Postgres LISTEN/NOTIFY — no other broker may be introduced.
- Events are stamped with the origin `instance_id`; an instance must ignore its own bus events (it already applied them locally).
- The bus is at-least-once and wake-only: consumers re-read durable state from the database; a missed NOTIFY may delay but never lose data.

## Related files

- `collab.md` — `deliver_event` / `deliver_presence` are the delivery entrypoints the bus calls.
- `sync.md` — `SyncHub::wake_user` and the `FanoutMsg::Rescan` path.
- `store.md` — the outbox/presence queries, the advisory lock, and the `_on` executor variants.
