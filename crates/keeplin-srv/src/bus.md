# `bus.rs` — cross-instance coordination (issue #45)

Self-contained companion for `crates/keeplin-srv/src/bus.rs`. It documents **every code
block of the source file, in source order** — a reader with only this file must be able to
understand `bus.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `bus.rs` carries exactly one marker comment of the
form `// md:<Header> > … > <Block header>`, whose path is the header chain of the section
documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview` at
the top of the file.

```rust
use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgListener;
use uuid::Uuid;

use crate::state::AppState;
```

**What it does** — The cross-instance coordination bus (issue #45), which lets
keeplin-srv run as **more than one replica**. The collaborative channel and the device
relay keep their live state in process memory (per-note sessions, per-user fan-out
channels), so without coordination two users connected to different replicas would not
see each other. This module bridges the replicas over Postgres `LISTEN/NOTIFY` — the
database they already share; no extra infrastructure.

Three notification channels (constants below):

- `collab_op`, payload `"<event_seq>:<origin_instance>"` — a collaborative op batch was
  applied; the ops row lives in the `collab_events` outbox table. Each instance loads
  the row and delivers it to its local subscribers — except the origin instance, which
  already broadcast it locally.
- `collab_presence`, payload `"<note_id>:<origin_instance>"` — a note's presence
  changed; every instance except the origin rebuilds the merged presence list for its
  local subscribers.
- `sync_batch`, payload `"<user_id>:<origin_instance>"` — a relay batch landed for a
  user; sibling instances wake that user's local devices to re-scan the journal.

Why an **outbox** for ops: `NOTIFY` payloads are capped (~8 KB) and an op batch can be
larger, so ops are written to `collab_events` and only the row `seq` is notified; a
sibling reads the row. The `seq` (a `BIGSERIAL`) doubles as the server sequence stamped
on fan-out.

**Dependencies** — `sqlx::postgres::PgListener` (external): the dedicated LISTEN
connection. `tokio` (external): task spawn + sleep. `uuid`, `tracing` (external).
Internal: `crate::state::AppState` (`state.rs`) — for the pool, `instance_id`, store
and hub; `crate::collab::{deliver_event, deliver_presence}` (`collab.rs`) and
`state.hub.wake_user` (`sync.rs`) as delivery targets;
`state.store.get_collab_event` (`store.rs`).

**Used by** — `main.rs` calls `bus::spawn(state)` once at boot. The NOTIFY side (the
producers) lives in `store.rs` (`notify` / the append+outbox writes invoked from
`collab.rs` and `sync.rs`).

**Repeated context** — Bus invariants: (1) all cross-instance delivery rides Postgres
`LISTEN/NOTIFY` — no other broker may be introduced; (2) every payload carries the
origin `instance_id` (minted per process in `AppState::new`), and an instance must
ignore its own events because **the origin always delivers locally and synchronously**
— which is also why a single-instance deployment (and the test suite) works without the
bus running; (3) the bus is **at-least-once and wake-only**: consumers re-read durable
state from the database, so a missed NOTIFY may delay but never lose data. Ordering
correctness does not depend on the bus: op batches serialise at the database via the
per-note advisory lock (`pg_advisory_xact_lock`, `store::lock_note_order`), and
presence rows are keyed `(note_id, instance_id, conn_id)` with heartbeats plus a
maintenance-loop sweep for rows a crashed instance left behind.

---

## Channel constants

**Identification** — logical section: the three channel-name constants; marker
`// md:Channel constants`.

```rust
pub const CH_COLLAB_OP: &str = "collab_op";
pub const CH_COLLAB_PRESENCE: &str = "collab_presence";
pub const CH_SYNC_BATCH: &str = "sync_batch";
```

**What it does** — The Postgres NOTIFY channel names, defined once so listener and
notifiers cannot drift. Payload formats per channel are described in *Overview*.

**Dependencies** — none.

**Used by** — `run` (this file) subscribes to all three; `store.rs` notifies on them
(`pg_notify`) from the collab apply path, the presence writes, and the relay append
path.

**Repeated context** — Channel names are part of the deployment's shared-database
contract: replicas of different builds must agree on them, so renaming one is a
breaking cross-instance change.

---

## fn spawn

**Identification** — public function; marker `// md:fn spawn`.

```rust
pub fn spawn(state: Arc<AppState>)
```

**What it does** — Spawns the singleton listener task for this process: an endless
`tokio` task that calls `run` and, when `run` returns an error (the LISTEN connection
dropped), logs a warning and retries after a 1-second backoff — so a transient database
blip does not permanently sever cross-instance delivery. Never returns; never panics.

**Dependencies** — `tokio::spawn`, `tokio::time::sleep` (external); `run` (this file);
`tracing::warn!` (external).

**Used by** — `main.rs`, once at boot, after building `AppState`. Not used by tests
(single-instance behaviour needs no bus — the origin delivers locally).

**Repeated context** — At-least-once/wake-only (see *Overview*): missing notifications
while reconnecting is safe because consumers re-read durable state; the cost is delay,
not loss.

---

## fn run

**Identification** — private async function; marker `// md:fn run`.

```rust
async fn run(state: &Arc<AppState>) -> anyhow::Result<()>
```

**What it does** — One listener session: connects a `PgListener` on the store's pool,
subscribes to the three channels (`listen_all`), logs readiness with this process's
`instance_id`, then loops forever receiving notifications and dispatching by channel
name to `handle_collab_op` / `handle_collab_presence` / `handle_sync_batch` (unknown
channels are ignored). Returns `Err` only when the connection/listen/recv fails —
which `spawn` turns into a reconnect.

**Dependencies** — `PgListener::connect_with`, `listen_all`, `recv` (external sqlx);
`state.store.pool()` (`store.rs`); the three channel constants and three handlers
(this file); `tracing::info!`.

**Used by** — `spawn` (this file) only.

**Repeated context** — The listener holds a **dedicated** connection outside the
bounded pool semantics (PgListener manages its own), so bus liveness does not compete
with request traffic for pool slots.

---

## fn handle_collab_op

**Identification** — private async function; marker `// md:fn handle_collab_op`.

```rust
async fn handle_collab_op(state: &Arc<AppState>, payload: &str)
```

**What it does** — Handles one `collab_op` notification, payload
`"<seq>:<origin_instance>"`. Parses both halves (malformed payloads are silently
dropped — they can only come from a non-keeplin writer); if `origin` is this process's
`instance_id`, returns immediately (the origin already broadcast to its local
subscribers). Otherwise loads the outbox row `collab_events[seq]` via
`store.get_collab_event`:

- `Ok(Some(event))` → `collab::deliver_event(state, event)` fans the ops out to local
  subscribers of that note.
- `Ok(None)` → the row was pruned already; nothing to do — a reconnecting client
  resyncs from a `Welcome` snapshot, so a missed op is recovered by snapshot rebuild.
- `Err` → log a warning; at-least-once semantics mean the client-side snapshot path
  eventually heals.

**Dependencies** — `state.instance_id` (`state.rs`), `store.get_collab_event`
(`store.rs`), `collab::deliver_event` (`collab.rs`), `tracing::warn!`.

**Used by** — `run` (this file) only.

**Repeated context** — **Snapshot-rebuild recovery** (collab convention): the
collaborative channel has no replay log; any missed fan-out is healed when the client
rejoins and receives the full `Welcome` snapshot. That is what makes "pruned row → do
nothing" correct.

---

## fn handle_collab_presence

**Identification** — private async function; marker `// md:fn handle_collab_presence`.

```rust
async fn handle_collab_presence(state: &Arc<AppState>, payload: &str)
```

**What it does** — Handles one `collab_presence` notification, payload
`"<note_id>:<origin_instance>"`. Parses; drops malformed payloads; skips its own
events (origin == `instance_id`); otherwise calls
`collab::deliver_presence(state, note_id)`, which rebuilds the note's **merged**
presence list from the shared presence table (all instances' rows) and broadcasts it
to local subscribers.

**Dependencies** — `state.instance_id` (`state.rs`), `collab::deliver_presence`
(`collab.rs`).

**Used by** — `run` (this file) only.

**Repeated context** — Presence is ephemeral, unversioned state: the server always
sends the **full** current list (receivers replace, never merge), so rebuilding from
the table on every notification is both simple and self-healing.

---

## fn handle_sync_batch

**Identification** — private async function; marker `// md:fn handle_sync_batch`.

```rust
async fn handle_sync_batch(state: &Arc<AppState>, payload: &str)
```

**What it does** — Handles one `sync_batch` notification, payload
`"<user_id>:<origin_instance>"`. Parses; drops malformed payloads; skips its own
batches (the authoring instance already fanned out live); otherwise calls
`state.hub.wake_user(user_id)`, which nudges every local relay connection of that user
to re-scan the journal (`changes` table) from its delivery cursor.

**Dependencies** — `state.instance_id` (`state.rs`), `SyncHub::wake_user` (`sync.rs`).

**Used by** — `run` (this file) only.

**Repeated context** — The relay's durability model: every change batch is a row in
the per-user `changes` journal with per-device delivery cursors (`device_cursors`);
fan-out (local or bus-woken) is only ever an optimisation of "re-scan the journal", so
a missed wake delays delivery but cannot lose it.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

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

## Coverage checklist

Every code block of `bus.rs`, in source order, each documented above (five points) and
carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `CH_COLLAB_OP` / `CH_COLLAB_PRESENCE` / `CH_SYNC_BATCH` | `// md:Channel constants` | Channel constants |
| 3 | `fn spawn` | `// md:fn spawn` | fn spawn |
| 4 | `fn run` | `// md:fn run` | fn run |
| 5 | `fn handle_collab_op` | `// md:fn handle_collab_op` | fn handle_collab_op |
| 6 | `fn handle_collab_presence` | `// md:fn handle_collab_presence` | fn handle_collab_presence |
| 7 | `fn handle_sync_batch` | `// md:fn handle_sync_batch` | fn handle_sync_batch |
