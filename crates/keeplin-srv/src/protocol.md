# `protocol.rs` — the collaborative wire protocol

Self-contained companion for `crates/keeplin-srv/src/protocol.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand `protocol.rs` without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `protocol.rs` carries exactly one marker comment of
the form `// md:<Header> > … > <Block header>`, whose path is the header chain of the
section documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview` at
the top of the file.

```rust
use chrono::{DateTime, Utc};
use keeplin_core::storage::note_log::VersionVector;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
```

**What it does** — This module defines the JSON message types of the collaborative
editing channel, the real-time line-editing surface served at `GET /api/ws` (JWT in the
`Authorization` header, or `?token=<jwt>` as a fallback for WebSocket clients that cannot
set headers). It is **pure data**: serde-derived structs and enums plus one accessor
(`LineOp::last_writer`); no I/O, no async, no state.

The protocol's model, from the Keeplin server-mode design: the unit of concurrency is the
**line** — every operation carries its own version vector, writer and timestamp — and the
**order of a note's lines is a versioned entity of its own**, updated by `Insert`/`Move`
ops. Messages are JSON text frames using serde internal tagging: a frame looks like
`{"type":"Op","note_id":…,"ops":[{"op":"Insert",…}]}`.

These shapes are a **wire contract** with the client: `keeplin-core/src/collab/protocol.rs`
in the client repo ([jsunyermias/keeplin](https://github.com/jsunyermias/keeplin)) mirrors
them field for field and must stay byte-compatible. A breaking change (renaming a field,
changing a tag, removing a variant) requires bumping `PROTOCOL_VERSION` in
`crates/keeplin-srv/src/http.rs` **and** its mirror `keeplin-core/src/compat.rs` — the
handshake (`GET /version`) only accepts an exact match. Additive evolution (new optional
fields with `#[serde(default)]`) does not require a bump.

**Dependencies** —

- `chrono` (external crate): `DateTime<Utc>` — all protocol timestamps are UTC.
- `uuid` (external crate): `Uuid` — ids for notes, lines, users, devices.
- `serde` (external crate): the `Serialize`/`Deserialize` derives on every type.
- `keeplin_core::storage::note_log::VersionVector` (git dependency on the client repo,
  pinned in `crates/keeplin-srv/Cargo.toml`): a version vector — a map from actor id
  (`String`, here always a **device** id) to a monotonically increasing counter. It is
  the causal clock every op and entity carries; see *Repeated context*.

**Used by** — the whole module is consumed by:

- `crates/keeplin-srv/src/collab.rs` — the collaborative session engine; imports
  `CollabClientMsg`, `CollabServerMsg`, `Cursor`, `LineOp`, `LineSnapshot`,
  `NoteLinesSnapshot`, `PresenceInfo` and is the only in-crate producer/consumer of the
  wire messages.
- `crates/keeplin-srv/src/store.rs` — stores two of these types **opaquely** as JSON:
  `PresenceRow.cursor` holds a serialized `Cursor`, `CollabEvent.ops` holds a serialized
  `Vec<LineOp>` (the cross-instance outbox row).
- `crates/keeplin-srv/tests/collab.rs` — exercises the protocol as raw JSON frames
  (deliberately not importing these types, so the tests break if the wire shape drifts).
- `keeplin-core/src/collab/protocol.rs` (client repo) — the byte-compatible mirror.

**Repeated context** (project conventions this file participates in):

- **Version vectors**: every mutable collaborative entity (a line, a note's line order)
  carries a `VersionVector` — per-device counters recording the edits it has absorbed.
  An incoming op's vv is compared with the entity's: if the op dominates it wins; if
  neither dominates (true concurrency), the deterministic **last-writer-wins tiebreak**
  `(updated_at, last_writer)` — implemented by keeplin-core's `note_log::resolve` and
  invoked from `collab.rs` — picks a winner. No locks; every replica converges.
- **The device is the concurrency actor**: vv components and `last_writer` are **device
  ids** (from the JWT), never user ids, so two devices of one user never share a vv
  component. `collab.rs::apply_op` rejects an op whose `last_writer` is not the sender's
  authenticated device (`bad_writer`).
- **Idempotency**: re-applying an already-applied op must be a no-op. `collab.rs`
  enforces this via `advances_writer`: an op must advance its writer's own vv component
  past the entity's current one, so replays are ignored.
- **Soft-delete**: deletion is a tombstone (`deleted_at` timestamp), never a row removal,
  so deletes replicate and conflict-resolve like any other edit. Snapshots include
  tombstoned lines for exactly this reason.

---

## Type aliases

Two aliases that name the id spaces used throughout the protocol.

### LineId

**Identification** — type alias; marker `// md:Type aliases > LineId`.

```rust
pub type LineId = Uuid;
```

**What it does** — The identity of one collaborative line. Line ids are minted by the
**client** performing the `Insert` (the server never renames them), are unique per note in
practice (UUIDv4), and persist through the line's whole life including tombstoning.

**Dependencies** — `uuid::Uuid` (external crate).

**Used by** — every line-referencing field in this file: `Cursor.line_id`,
`LineSnapshot.id`, `NoteLinesSnapshot.order`, and the `line_id`/`line_ids`/`after_line_id`
fields of `LineOp`. Outside this file, `crates/keeplin-srv/src/collab.rs` handles line ids
as plain `Uuid` (e.g. `position_after`), so the alias is a documentation device, not an
enforced newtype.

**Repeated context** — Client-minted ids are what make offline/concurrent inserts
mergeable without coordination: no server round-trip is needed to create a line, and the
version-vector machinery (see *Overview → Repeated context*) resolves collisions on
content, not identity.

### UserId

**Identification** — type alias; marker `// md:Type aliases > UserId`.

```rust
pub type UserId = String;
```

**What it does** — A deliberately loose string alias covering **two distinct id spaces**:

- In **presence** messages (`PresenceInfo.user_id`, and the `user_id` of
  `CollabServerMsg::Op`) it is a **user** id — presence is about people.
- In **ops** (`last_writer` and the version-vector keys) it is the **device** id from the
  token — the concurrency actor — so two devices of the same user never share a vv
  component.

It is a `String` (not `Uuid`) because vv keys are strings in keeplin-core's
`VersionVector` and the two spaces must share one type in `LineOp`.

**Dependencies** — none beyond `std` (`String`).

**Used by** — `LineSnapshot.last_writer`, `NoteLinesSnapshot.last_writer`, every
`last_writer` field of `LineOp`, `PresenceInfo.user_id`, and
`CollabServerMsg::Op.user_id`. `crates/keeplin-srv/src/collab.rs::apply_op` compares
`LineOp::last_writer()` against the authenticated device id.

**Repeated context** — The device-as-actor rule: the JWT minted at `POST /api/login`
carries a `device_id` (one `user_devices` row per login), and that id is what a device
signs its edits with. If the *user* id were the vv actor, concurrent edits from a user's
second device would look like replays of the first device's counters and be dropped.
Presence stays user-based because the UI shows people, not devices.

---

## Cursor

**Identification** — struct; marker `// md:Cursor`.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub line_id: LineId,
    pub column: usize,
}
```

**What it does** — A caret position inside a note: which line (`line_id`) and the column
within it (`column`, a character offset owned by the client — the server never interprets
it). Sent by clients in `CollabClientMsg::Cursor` and echoed to everyone inside
`PresenceInfo`. `PartialEq`/`Eq` exist so callers can cheaply detect "cursor unchanged".
The server treats the whole value as opaque: it does not validate that `line_id` exists
or that `column` is in range — stale carets are a display concern for clients.

**Dependencies** — `LineId` (this file); serde derives.

**Used by** —

- `crates/keeplin-srv/src/collab.rs`: `touch_presence` takes `Option<&Cursor>` and
  persists it; `deliver_presence` deserializes stored cursor JSON back into `Cursor`
  when rebuilding a note's merged presence list.
- `crates/keeplin-srv/src/store.rs`: `PresenceRow.cursor` holds it as opaque stored JSON
  (`serde_json::Value`) — the store never depends on its shape.
- `PresenceInfo.cursor` and `CollabClientMsg::Cursor` in this file.
- `crates/keeplin-srv/tests/collab.rs` sends raw `{"type":"Cursor",…}` frames.

**Repeated context** — Presence (who is in a note, where their caret is) is
**user-scoped, ephemeral state**: it lives in a presence table keyed by connection,
is cleared on `Leave`/disconnect, and is not versioned — unlike lines and order, it has
no vv because concurrent cursor updates need no merging (last write is fine).

---

## LineSnapshot

**Identification** — struct; marker `// md:LineSnapshot`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineSnapshot {
    pub id: LineId,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub vv: VersionVector,
    pub last_writer: UserId,
}
```

**What it does** — One line as sent inside snapshots: the **full versioned entity**,
tombstones included (`deleted_at: Some(_)` marks a soft-deleted line that still ships to
clients). `vv` is the line's current version vector and `last_writer` the device id of
its latest winning edit — a client needs both to resolve its own pending ops against the
snapshot. `content` is the line's text (server-side limits on line count/length are
enforced by the engine, not encoded here).

**Dependencies** — `LineId`, `UserId` (this file); `chrono::DateTime<Utc>`;
`keeplin_core::storage::note_log::VersionVector`; serde derives.

**Used by** —

- `crates/keeplin-srv/src/collab.rs::line_snapshot(line: Line) -> LineSnapshot` converts
  the store's `Line` row into this wire shape; `read_snapshot` collects them into
  `NoteLinesSnapshot.lines`.
- `NoteLinesSnapshot` (this file) embeds a `Vec<LineSnapshot>`.

**Repeated context** — **Soft-delete**: Keeplin never hard-deletes replicated entities;
a delete sets `deleted_at` and the tombstone keeps replicating so every device converges
on "deleted" (a device that was offline during the delete must still learn about it).
Tombstones are garbage-collected server-side only after a retention window
(`LINES_GC_DAYS`, hourly maintenance loop) — long after every live device has seen them.

---

## NoteLinesSnapshot

**Identification** — struct; marker `// md:NoteLinesSnapshot`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteLinesSnapshot {
    pub note_id: Uuid,
    pub order: Vec<LineId>,
    pub updated_at: DateTime<Utc>,
    pub vv: VersionVector,
    pub last_writer: UserId,
    pub lines: Vec<LineSnapshot>,
}
```

**What it does** — The complete state a client needs to render a note, sent in
`CollabServerMsg::Welcome`. It bundles the **order entity** — `order` lists ALL
`LineId`s, tombstoned lines included, with the order's own `vv`, `updated_at` and
`last_writer` (the order is a versioned entity in its own right, mutated by
`Insert`/`Move`) — plus every line as a `LineSnapshot`. A (re)connecting client rebuilds
its state entirely from this snapshot instead of replaying an op log: this is why the
server can prune its cross-instance op outbox and keep no infinite history.

**Dependencies** — `LineId`, `UserId`, `LineSnapshot` (this file); `uuid::Uuid`;
`chrono::DateTime<Utc>`; `keeplin_core::storage::note_log::VersionVector`; serde derives.

**Used by** —

- `crates/keeplin-srv/src/collab.rs::read_snapshot(state, note_id)` builds it from the
  store (`get_note_order` + the note's lines) and the `Join` arm of `handle_msg` sends it
  in `CollabServerMsg::Welcome`.
- `CollabServerMsg::Welcome` (this file) is its only wire carrier.

**Repeated context** — **Snapshot-rebuild model**: the collaborative channel offers no
op history. If a client lags, disconnects, or its outbound queue is dropped, the recovery
path is always the same — reconnect, `Join`, receive `Welcome`, rebuild. Every part of
the server may therefore drop collab messages under pressure without correctness loss;
durability lives in PostgreSQL rows (lines + order), not in message delivery. The
`order`-as-entity design means concurrent `Move`s resolve by vv exactly like content
edits, rather than by operational transformation.

---

## LineOp

**Identification** — enum, serde-tagged `op`, PascalCase variants; marker `// md:LineOp`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "PascalCase")]
pub enum LineOp {
    Insert { after_line_id: Option<LineId>, line_id: LineId, content: String,
             vv: VersionVector, last_writer: UserId, updated_at: DateTime<Utc> },
    Update { line_id: LineId, content: String,
             vv: VersionVector, last_writer: UserId, updated_at: DateTime<Utc> },
    Delete { line_id: LineId, deleted_at: DateTime<Utc>,
             vv: VersionVector, last_writer: UserId, updated_at: DateTime<Utc> },
    Move   { line_ids: Vec<LineId>, after_line_id: Option<LineId>,
             vv: VersionVector, last_writer: UserId, updated_at: DateTime<Utc> },
}
```

**What it does** — One line-level operation, the unit of collaborative editing. On the
wire it is tagged `"op"`: `{"op":"Insert","line_id":…,…}`. The four variants:

- `Insert` — create line `line_id` with `content`, positioned after `after_line_id`
  (`None` = insert at the beginning of the note). Mutates **two** entities: the new line
  and the note's order.
- `Update` — replace the content of `line_id`.
- `Delete` — tombstone `line_id` at `deleted_at` (soft-delete; the line row survives).
- `Move` — reposition the contiguous block `line_ids` after `after_line_id`
  (`None` = to the beginning). Mutates the order entity only.

Every variant carries the same resolution triple — `vv` (the op's version vector),
`last_writer` (the authoring **device** id) and `updated_at` (the op's timestamp) — so
each op is independently resolvable against current entity state: the server applies it
if it wins resolution and silently ignores it if it is dominated (`OpOutcome::Ignored`
in `collab.rs`); malformed ops (unknown line, bad writer, anchor missing) are answered
with `CollabServerMsg::Error` without dropping the connection.

**Dependencies** — `LineId`, `UserId` (this file); `chrono::DateTime<Utc>`;
`keeplin_core::storage::note_log::VersionVector`; serde derives (internal tag `op`).

**Used by** —

- `crates/keeplin-srv/src/collab.rs`: `handle_msg` (the `Op` arm) receives batches
  (`Vec<LineOp>`); `apply_op` matches each variant and validates/resolves/persists it;
  `OpOutcome::Applied(LineOp)` carries the applied op back for fan-out;
  `deliver_event` deserializes `Vec<LineOp>` from a stored outbox row to deliver
  cross-instance ops.
- `crates/keeplin-srv/src/store.rs`: `CollabEvent.ops` persists a serialized
  `Vec<LineOp>` in the `collab_events` outbox (issue #45) — stored opaquely as JSON.
- `CollabClientMsg::Op` / `CollabServerMsg::Op` (this file) carry it on the wire.
- `crates/keeplin-srv/tests/collab.rs` builds op frames as raw JSON.

**Repeated context** — The full resolution pipeline every op goes through in
`collab.rs::apply_op`, restated: (1) `last_writer` must equal the sender's authenticated
device id (`bad_writer` otherwise) — clients cannot forge edits in someone else's name;
(2) the op's vv must **advance the writer's own component** past the entity's current
one (`advances_writer`) — replays of already-applied ops fail this and are ignored,
which is what makes application **idempotent**; (3) the op is resolved against the
current entity (line or order) with keeplin-core's `note_log::resolve` — vv dominance
first, then the deterministic `(updated_at, last_writer)` LWW tiebreak for true
concurrency; (4) on a win, the entity stores `merge_vv(current, op.vv)` (pointwise max —
the merged causal frontier) and the op fans out to the note's other subscribers.

---

## impl LineOp

**Identification** — inherent impl block; marker `// md:impl LineOp`. Contains one
method, `fn last_writer` (next section).

**What it does** — The only logic in this module: field access across variants. Kept
here (rather than in `collab.rs`) because it is shape knowledge — every variant carries
`last_writer` — not engine policy.

**Dependencies** — `LineOp` (this file).

**Used by** — see `fn last_writer`.

**Repeated context** — none beyond the method's own (below).

### fn last_writer

**Identification** — method; marker `// md:impl LineOp > fn last_writer`.

```rust
pub fn last_writer(&self) -> &str
```

**What it does** — Returns the op's `last_writer` field regardless of variant, as `&str`.
Total (every variant carries the field); no failure mode.

**Dependencies** — `LineOp` (this file).

**Used by** — `crates/keeplin-srv/src/collab.rs::apply_op`, which compares it against the
authenticated device id (`op.last_writer() != device_id.to_string()` → reject with
`bad_writer`) before any resolution work.

**Repeated context** — Writer identity is the first gate of the op pipeline: the JWT's
`device_id` is the only identity a connection may sign edits with (device-as-actor rule,
see *Type aliases → UserId*).

---

## PresenceInfo

**Identification** — struct; marker `// md:PresenceInfo`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceInfo {
    pub user_id: UserId,
    pub display_name: String,
    pub cursor: Option<Cursor>,
}
```

**What it does** — One participant in a note's live session: their **user** id (presence
is user-scoped, not device-scoped), the display name to render, and optionally where
their caret is (`None` until they send a first `CollabClientMsg::Cursor`). A user
connected twice appears once — presence lists are merged per user.

**Dependencies** — `UserId`, `Cursor` (this file); serde derives.

**Used by** —

- `crates/keeplin-srv/src/collab.rs::deliver_presence` rebuilds a note's merged list
  (one `PresenceInfo` per user, across all server instances) from stored
  `PresenceRow`s and broadcasts it.
- `CollabServerMsg::Presence` (this file) carries the full list.

**Repeated context** — Presence is ephemeral and unversioned (no vv): it is not
replicated state to converge on, just a live view, so the server always sends the **full
current list** rather than deltas — receivers replace, never merge. In a multi-instance
deployment the merged list is assembled from a shared Postgres presence table and
sibling instances are nudged over the `collab_presence` LISTEN/NOTIFY channel
(`crates/keeplin-srv/src/bus.rs`).

---

## CollabClientMsg

**Identification** — enum, serde-tagged `type`, PascalCase variants; marker
`// md:CollabClientMsg`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum CollabClientMsg {
    Join   { note_id: Uuid },
    Leave  { note_id: Uuid },
    Op     { note_id: Uuid, ops: Vec<LineOp> },
    Cursor { note_id: Uuid, cursor: Cursor },
    Ack    { server_seq: u64 },
}
```

**What it does** — Every client → server frame. One WebSocket connection can join any
number of notes, so every message (except `Ack`) names the note it targets:

- `Join` — subscribe to a note; on success the server replies with
  `CollabServerMsg::Welcome` (full snapshot) and a `Presence` broadcast. Requires read
  access; rejected with `Error` otherwise.
- `Leave` — unsubscribe; clears the sender's presence in that note.
- `Op` — a batch of `LineOp`s to apply, in order, to one note. Requires write access.
- `Cursor` — the sender's caret moved; triggers a `Presence` broadcast.
- `Ack` — client-side delivery bookkeeping of `server_seq`; the server accepts and
  **ignores** it (kept in the protocol so clients can be symmetric about acking).

Unknown/malformed frames are answered with `CollabServerMsg::Error` (`bad_message`)
rather than closing the connection.

**Dependencies** — `LineOp`, `Cursor` (this file); `uuid::Uuid`; serde derives
(internal tag `type`).

**Used by** —

- `crates/keeplin-srv/src/collab.rs::run_connection` parses each incoming text frame
  (`serde_json::from_str`), and `handle_msg` dispatches on the variant (arms
  `Join`/`Leave`/`Op`/`Cursor`/`Ack`).
- `crates/keeplin-srv/tests/collab.rs` and the client repo's
  `keeplin-core/src/collab/protocol.rs` produce these frames.

**Repeated context** — Access control is capability-based and re-resolved from the
shares tables at `Join` time: a note is accessible to its owner and to users granted a
capability bitset (`read`/`write`/`share_read`/`share_write`/`manage`, higher bits
implying lower) directly or cascaded from its notebook (`permissions.rs`). The wire
protocol itself carries no authentication — identity comes from the JWT presented at
the WebSocket upgrade, never from frame contents.

---

## CollabServerMsg

**Identification** — enum, serde-tagged `type`, PascalCase variants; marker
`// md:CollabServerMsg`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum CollabServerMsg {
    Welcome  { note_id: Uuid, snapshot: NoteLinesSnapshot },
    Op       { server_seq: u64, note_id: Uuid, user_id: UserId, ops: Vec<LineOp> },
    Presence { note_id: Uuid, users: Vec<PresenceInfo> },
    Error    { code: String, message: String },
}
```

**What it does** — Every server → client frame:

- `Welcome` — reply to a successful `Join`: the full current state of the note
  (`NoteLinesSnapshot`); the client rebuilds from it rather than replaying history.
- `Op` — operations from another participant, **already validated, resolved and
  persisted** by the server, with a per-note monotonic `server_seq` and the author's
  **user** id (for attribution in the UI; the vv actor inside the ops remains the
  device). `note_id` is included — a deliberate addition to the original design sketch —
  so one connection can multiplex several notes. The sender of a batch does not receive
  its own ops back.
- `Presence` — the **full** presence list for a note, sent after every
  join/leave/cursor move; receivers replace their list, never merge.
- `Error` — a machine-readable `code` (e.g. `bad_writer`, `bad_message`, `forbidden`,
  `unknown_line`) plus a human-readable `message`. Errors are per-frame; the connection
  stays open.

**Dependencies** — `NoteLinesSnapshot`, `UserId`, `LineOp`, `PresenceInfo` (this file);
`uuid::Uuid`; serde derives (internal tag `type`).

**Used by** — produced exclusively by `crates/keeplin-srv/src/collab.rs`:

- `handle_msg` (`Join` arm) sends `Welcome`; the `Op` arm fans out applied batches;
  `send_error` wraps `Error`.
- `CollabSession::broadcast` serializes any `CollabServerMsg` once and delivers it to
  every local subscriber of the note (optionally skipping the originating connection).
- `deliver_event` / `deliver_presence` re-emit `Op` / `Presence` for batches that
  originated on **other** server instances (cross-instance bus, `bus.rs`).
- Consumed by the client mirror (`keeplin-core/src/collab/protocol.rs`) and asserted as
  raw JSON in `crates/keeplin-srv/tests/collab.rs`.

**Repeated context** — `server_seq` is a per-note monotonic sequence stamped on fan-out;
clients use it to detect gaps. There is **no replay channel**: a client that detects a
gap (or reconnects) recovers via `Join` → `Welcome` snapshot rebuild, which is what lets
the server drop messages to slow consumers without correctness loss. In multi-instance
deployments, applied batches are also written to the `collab_events` outbox
(`store.rs::CollabEvent`) and announced over Postgres `LISTEN/NOTIFY`
(`bus.rs::CH_COLLAB_OP`) so sibling instances deliver them to their local subscribers.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `LineOp` — defined here (EXTRACTED; 2 cross-file edge(s))
- `Cursor` — defined here (EXTRACTED; 1 cross-file edge(s))
- `LineSnapshot` — defined here (EXTRACTED; 1 cross-file edge(s))
- `NoteLinesSnapshot` — defined here (EXTRACTED; 1 cross-file edge(s))
- `CollabClientMsg` — defined here (EXTRACTED; 1 cross-file edge(s))
- `CollabServerMsg` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.last_writer()` — defined here (EXTRACTED; file-local)
- `PresenceInfo` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/collab.rs` — the collaborative session engine (EXTRACTED: references×7; e.g. `.broadcast()`, `touch_presence()`, `handle_msg()`)

## Coverage checklist

Every code block of `protocol.rs`, in source order, each documented above (five points)
and carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `pub type LineId` | `// md:Type aliases > LineId` | Type aliases › LineId |
| 3 | `pub type UserId` | `// md:Type aliases > UserId` | Type aliases › UserId |
| 4 | `struct Cursor` | `// md:Cursor` | Cursor |
| 5 | `struct LineSnapshot` | `// md:LineSnapshot` | LineSnapshot |
| 6 | `struct NoteLinesSnapshot` | `// md:NoteLinesSnapshot` | NoteLinesSnapshot |
| 7 | `enum LineOp` | `// md:LineOp` | LineOp |
| 8 | `impl LineOp` | `// md:impl LineOp` | impl LineOp |
| 9 | `fn last_writer` | `// md:impl LineOp > fn last_writer` | impl LineOp › fn last_writer |
| 10 | `struct PresenceInfo` | `// md:PresenceInfo` | PresenceInfo |
| 11 | `enum CollabClientMsg` | `// md:CollabClientMsg` | CollabClientMsg |
| 12 | `enum CollabServerMsg` | `// md:CollabServerMsg` | CollabServerMsg |
