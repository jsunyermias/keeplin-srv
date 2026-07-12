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

## Related files

- `protocol.md` — the message/op types on the wire.
- `store.md` — the line and order rows, and their opaque `vv` columns.
- `permissions.md` — the capability check on `Join`/`Op`.
- `keeplin/keeplin-core/src/collab/` — the client that speaks this protocol.
