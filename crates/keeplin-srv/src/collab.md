# `collab.rs` — the collaborative session engine

## Purpose

Implements `GET /api/ws`, the real-time line-editing channel (design §7). It is the **broker
and durable source of truth**: it authenticates a connection, tracks per-note live sessions
and presence, validates and resolves each incoming `LineOp` against current state, persists
it, and fans the applied ops out to the note's other subscribers with a monotonic
`server_seq`. Clients rebuild from a `Welcome` snapshot on connect — there is no infinite op
log.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `CollabRegistry` | struct | all live sessions, keyed by note id; lives in `AppState` |
| `CollabSession` | struct | one note's live session: subscribers, presence, an apply lock, a `server_seq` counter |
| `Subscriber` | struct | one connection: user id, display name, cursor, an outbound channel |
| `OpOutcome` | enum | `Applied(op)` / `Ignored` / `Invalid{code,message}` — the result of resolving one op |

## Connection flow

```
handler → authenticate (token from Authorization header or ?token=; device must still exist)
        → run_connection:
            join session, send Welcome snapshot
            select loop:
              client frame → handle_msg (Join / Op / Cursor / Leave / Ack)
              — Op: apply_op per op under the session lock → broadcast applied ops (server_seq++)
            on disconnect: remove subscriber, broadcast presence, drop empty session
```

## Op validation & resolution

`apply_op` is the load-bearing function. For each op it checks, in order:

1. **Writer identity** — `last_writer` must equal the connection's authenticated **device id**
   (clients cannot forge edits in another's name).
2. **Content limits** — no `\n` in a line, ≤ `MAX_LINE_LEN`, ≤ `MAX_LINES_PER_NOTE`.
3. **Existence** — the target line / `after_line_id` must (or must not, for `Insert`) exist.
4. **Version advance** — the op's `vv` must advance the writer's own component past the
   entity's current one (`advances_writer`); a replay fails this and is `Ignored`, which is
   what makes application **idempotent**.
5. **Resolution** — `note_log::resolve(current, incoming)`. `Insert`/`Move` resolve against the
   **order** entity, `Update`/`Delete` against the **line** entity. A dominated op is
   `Ignored`; concurrent ops fall to the deterministic `(timestamp, device_id)` tiebreak. The
   applied op merges its vector into the entity's (`merge_vv`).

Only `Applied` ops are persisted and fanned out. `Invalid` sends the sender an `Error`;
`Ignored` is silent.

## Concurrency discipline

- `CollabSession::apply_lock` (a `Mutex`) serialises op application **and** the join snapshot:
  a joiner reads the snapshot and subscribes under the lock, so no op can slip between the two
  (which would leave it missing from both). Two ops never interleave their read-modify-write.
- **Access is re-resolved on every op batch** (not cached at join), so a share revoked
  mid-session is enforced on the next edit rather than persisting for the life of the
  connection (issue #30).
- Outbound frames per connection funnel through one **bounded** `mpsc` channel
  (`OUTBOUND_CAPACITY`) owned by a single writer task, so the socket has exactly one writer. A
  subscriber whose queue is full (a slow/stalled consumer) is dropped from the session rather
  than buffering without bound (issue #34); it reconnects and rebuilds from a fresh snapshot.
- The writer task also emits periodic **pings** (`PING_INTERVAL`), and the read loop closes the
  connection if no frame — not even a pong — arrives within `ACTIVITY_TIMEOUT`, so a silently
  dropped peer is reaped instead of leaking a subscriber slot (issue #35).
- Sessions are created on demand and dropped when the last subscriber leaves; on a server
  restart clients reconnect and get a fresh snapshot — sessions hold no durable state.

## Design notes

- The **device** is the vv actor (not the user): two devices of the same user must not share a
  version-vector component, or the server would treat the second's concurrent edit as a replay.
  Presence stays user-based; only the vv/`last_writer` are device-scoped.
- `Move` extracts the moved block then reinserts it after the target, guarding against making a
  moved line its own anchor.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `handle_msg()` — defined here (EXTRACTED; 4 cross-file edge(s))
- `touch_presence()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `read_snapshot()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `apply_op()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `clear_presence()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `deliver_event()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `handler()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `line_snapshot()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `winner()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `line_winner()` — defined here (EXTRACTED; 2 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/error.rs` — the API error type (EXTRACTED: references×6; e.g. `AppError`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×2; e.g. `.resolve()`)
- `crates/keeplin-srv/src/permissions.rs` — note capabilities (EXTRACTED: calls×1; e.g. `resolve_note_access()`)
- `crates/keeplin-srv/src/protocol.rs` — collaborative wire types (EXTRACTED: references×7; e.g. `CollabServerMsg`, `Cursor`, `CollabClientMsg`)
- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×10; e.g. `AppState`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: references×4; e.g. `CollabEvent`, `Line`, `NoteOrder`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×1; e.g. `AppState`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- The unit of concurrency is the line; the order of lines is its own versioned entity; resolution is always `note_log::resolve`, never a lock.
- `last_writer` must equal the authenticated device and the vector must advance the writer's component — forged ops are rejected.
- Viewers can join and watch but never write; access is re-resolved against the share tables, not cached from join time.
- Per-note line order updates are serialised across replicas with a Postgres advisory lock; peer instances are reached only via the bus.
- Limits (line length, lines per note, message size) are enforced before persisting.

## Related files

- `protocol.md` — the message/op types on the wire.
- `store.md` — the line and order rows, and their opaque `vv` columns.
- `permissions.md` — the capability check on `Join`/`Op`.
- `keeplin/keeplin-core/src/collab/` — the client that speaks this protocol.
