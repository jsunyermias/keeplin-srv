# `sync.rs` — the device sync relay

## Purpose

Implements `GET /api/sync`, the server side of keeplin-core's `DbBackend` wire protocol. It
relays each device's `Change` batches to the user's **other** devices and journals every batch
so a device that was offline is caught up on reconnect. This channel carries the
non-collaborative entities (notebooks, tags, resources); notes go through `/api/ws` instead.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `SyncHub` | struct | per-user broadcast channels for live fan-out; lives in `AppState` |
| `FanoutBatch` | struct | one persisted batch, pre-serialised, tagged with its origin device |

## The wire protocol

Exactly what `DbBackend::connect_ws` / `send_changes` / `receive_changes` speak:

1. Client connects and sends the handshake `{"type":"auth","token":"<jwt>"}` (the token also
   works in the `Authorization` header).
2. Client pushes `{"type":"changes","batch_id":…,"device_id":…,"changes":[Change…]}`.
3. Server delivers `{"type":"changes","changes":[Change…]}` — first the **backlog** the device
   has not seen, then live batches from the user's other devices. The sender is never echoed.

`Change` payloads are stored and forwarded without the relay needing to understand them, so client
model evolution never needs a server change. On top of that pass-through the relay also
**materialises** the entities the server owns (see below); anything it does not model stays opaque.

## Materialisation (`materialize`)

After a batch is journaled and before fan-out, `materialize` parses each `Change` and, for the
entities the server is the source of truth for — notebooks, tags, note↔tag associations and resource
metadata — resolves it by version vector against the stored row and upserts it (via `store`). It
reuses keeplin-core's `note_log::resolve`, so the server converges to the **same winner** every
client does. This is what lets a wiped device rehydrate from REST and lets the journal be pruned
safely. Notes are excluded (they are materialised by `/api/ws`); a `ResourceCreate` still carrying a
binary has it stored to `resource_blobs` (backward compatibility). Failures are logged, not fatal.

## Delivery mechanism

- Every accepted batch is **persisted to the `changes` journal before fan-out**
  (`store.append_changes`), deduped by `(batch_id, batch_index)` so a client retry after a
  reconnect creates no duplicate rows.
- Each device has a durable **delivery cursor** (`device_cursors.last_seq`) that only advances
  after a successful send. On connect the backlog is streamed in chunks from the cursor; live
  batches then arrive through the per-user broadcast channel.
- If a live receiver lags the broadcast channel, it falls back to a journal re-scan from its
  cursor. Because `apply_change` on the client is idempotent, the relay prefers **duplicate
  delivery over loss** — a reconnecting device may re-receive a batch, which is safe.

## Design notes

- Per-user isolation: fan-out channels are keyed by user id, so a batch never crosses accounts.
- The handshake re-checks that the token's device still exists (revocation), mirroring the REST
  middleware.
- Journal growth is bounded by `CHANGES_RETENTION_DAYS` pruning (only rows delivered to every
  device of the user), run by the maintenance loop.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `authenticate()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `SyncHub` — defined here (EXTRACTED; 1 cross-file edge(s))
- `handler()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `run_connection()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `deliver_backlog()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `relay_loop()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `handle_incoming()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `materialize()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `FanoutBatch` — defined here (EXTRACTED; file-local)
- `FanoutMsg` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×7; e.g. `AppState`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: references×1; e.g. `UserDevice`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×1; e.g. `AppState`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Relay payloads are opaque: the server never interprets keeplin-core's `Change` beyond the envelope needed for journaling and materialisation — client model changes must not require server changes.
- `(batch_id, batch_index)` dedupes client retries; a re-sent batch must be a no-op.
- A device never receives its own changes back (echo suppression by origin device id).
- Journal append and materialisation must both happen for every accepted change; the materialised tables — not the journal — are the source of truth.

## Related files

- `store.md` — `append_changes`, cursors, pruning, and the `upsert_*`/`delete_*` materialisation methods.
- `../../../migrations/0004_domain_entities.md` — the tables `materialize` writes into.
- `collab.md` — the sibling WebSocket surface for notes.
- `keeplin/keeplin-core/src/storage/db.md` — the client end of this protocol.

## Keepalive

The relay loop pings each connection every `PING_INTERVAL`; a failed write surfaces a dead peer promptly and the pings keep NAT/proxy paths open (issue #35).
