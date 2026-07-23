# `store.rs` — the PostgreSQL data-access layer

Self-contained companion for `crates/keeplin-srv/src/store.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able to
understand `store.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `store.rs` carries exactly one marker comment of the
form `// md:<Header> > … > <Block header>`, whose path is the header chain of the section
documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**. Row-mapping
structs and simple CRUD methods use a compressed layout of the same five points.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use chrono::{DateTime, Utc};
use keeplin_core::storage::note_log::VersionVector;
use serde::Serialize;
use sqlx::{types::Json, Pool, Postgres, Row};
use uuid::Uuid;

use crate::error::AppError;
```

**What it does** — The **single place any SQL lives**. `Store` wraps a `sqlx::PgPool`
(plus the at-rest `Cipher`) and exposes typed async methods for every entity: users
and devices, login lockout, email-flow tokens, the relay journal and delivery cursors,
entity history, retention/GC, the collaborative note model (notes, lines, line order,
shares, notebook shares + the destructive cascade), the cross-instance bus primitives
(NOTIFY, advisory lock, outbox, presence), the domain entities materialised from the
relay, and quotas. Handlers and the two WebSocket engines call `Store`; nothing else
touches the database.

The schema is owned by the forward-only SQL migrations (documented in
`migrations/*.md`): `users`/`user_devices`/`changes`/`device_cursors` (0001);
`notes`/`lines`/`note_line_order`/`note_shares` (0002); note metadata (0003);
`notebooks`/`tags`/`note_tags`/`resources`/`resource_blobs` (0004); capability columns
(0005/0006); `collab_events`/`collab_presence` (0010); `login_attempts` (0011);
`email_tokens` (0012); `keeplin_try_timestamptz` (0013).

**Dependencies** — `sqlx` (Postgres pool, queries, `Json` wrapper), `chrono`, `uuid`,
`serde`, `sha2` (token hashing), `aes_gcm::aead::OsRng` (token randomness),
`keeplin_core` (`VersionVector`, `note_log::resolve`, the `models::*` inputs to
materialisation). Internal: `crate::error::AppError` (every method), `crate::crypto`
(the embedded `Cipher`), `crate::mail::MailKind` (token kinds).

**Used by** — everything: `http.rs` (every handler), `auth.rs` (`get_device`),
`sync.rs` (journal/cursors/materialisation/notify), `collab.rs` (lines/order/
presence/outbox/lock), `bus.rs` (`get_collab_event`, `pool`), `main.rs` (maintenance
queries), `permissions.rs` (share lookups), and the whole test suite.

**Repeated context** — Store-wide invariants: (1) **cipher choke point** — the at-rest
`Cipher` encrypts/decrypts exactly two columns, `notes.title` and `lines.content`,
and only inside this module; (2) **version-vector resolution** — for the
relay-materialised entities the store itself resolves via `incoming_wins` under
`SELECT … FOR UPDATE`; for the collaborative line/order rows the store is
mechanism-free (the `vv` columns are opaque JSONB; `collab.rs` decides winners);
(3) **soft-delete** — replicated entities are tombstoned, never hard-deleted; physical
reclamation happens only in the explicit aged GC/purge passes; (4) journal pruning
only removes rows already delivered to every connected device and older than the
retention window — the materialised tables, not the journal, are the source of truth.

---

## PageCursor

**Identification** — public struct; marker `// md:PageCursor`.

**Code** — complete and verbatim:

```rust
// md:PageCursor
#[derive(Debug, Clone, Copy)]
pub struct PageCursor {
    pub ts: DateTime<Utc>,
    pub id: Uuid,
}
```

**What it does** — The opaque keyset-pagination cursor (issue #29): the ordering
timestamp of the last row a caller received plus its id as tiebreaker. Serialised as
`"<micros>_<uuid>"` — URL-safe and dependency-free — so clients echo it back rather
than parse it. Keyset paging (never `OFFSET`) keeps deep pages flat-cost and stable
under concurrent inserts.

**Dependencies** — chrono/uuid. **Used by** — `http.rs` (`ListQuery::resolve`,
`paginated`), the paginated list methods here.

**Repeated context** — none.

## impl PageCursor

**Identification** — the inherent impl block; marker `// md:impl PageCursor`.

**Code** — container: members documented as sub-blocks below: fn new, fn encode, fn decode.

**What it does** — Keyset-pagination cursor encode/decode helpers.

### fn new

**Identification** — method of `impl PageCursor`; marker `// md:impl PageCursor > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl PageCursor > fn new
    pub fn new(ts: DateTime<Utc>, id: Uuid) -> Self {
        Self { ts, id }
    }
```

**What it does** — Build a keyset cursor from a `(timestamp, id)` pair.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn encode

**Identification** — method of `impl PageCursor`; marker `// md:impl PageCursor > fn encode`.

**Code** — complete and verbatim:

```rust
    // md:impl PageCursor > fn encode
    pub fn encode(&self) -> String {
        format!("{}_{}", self.ts.timestamp_micros(), self.id)
    }
```

**What it does** — Serialise the cursor to an opaque `"<micros>_<uuid>"` token (microsecond timestamp + id) returned as a page's `next_cursor`.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn decode

**Identification** — method of `impl PageCursor`; marker `// md:impl PageCursor > fn decode`.

**Code** — complete and verbatim:

```rust
    // md:impl PageCursor > fn decode
    pub fn decode(token: &str) -> Option<Self> {
        let (micros, id) = token.split_once('_')?;
        let ts = DateTime::from_timestamp_micros(micros.parse().ok()?)?;
        Some(Self {
            ts,
            id: id.parse().ok()?,
        })
    }
```

**What it does** — Parse an opaque page token back into a `PageCursor`; returns `None` on any malformed part (bad split, unparseable micros or uuid) so a tampered or garbage cursor is rejected rather than trusted.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

## fn token_hash

**Identification** — private fn; marker `// md:fn token_hash`.
`fn token_hash(raw: &str) -> String` — SHA-256 hex of an email-flow token — **the only
form ever stored** (a database dump cannot be replayed into a takeover).

**Code** — complete and verbatim:

```rust
// md:fn token_hash
fn token_hash(raw: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    format!("{:x}", hasher.finalize())
}
```

**Dependencies** — `sha2`. **Used by** — `create_email_token`,
`consume_email_token`. **Repeated context** — email-flow token model (issue #49).

---

## fn split_cursor

**Identification** — private fn; marker `// md:fn split_cursor`.
`fn split_cursor(Option<PageCursor>) -> (Option<DateTime<Utc>>, Option<Uuid>)` —
splits an optional cursor into the `(timestamp, id)` binds; `None` maps to
`(NULL, NULL)`, which the `$3 IS NULL` guard in each query turns into "no keyset
filter" (first page).

**Code** — complete and verbatim:

```rust
// md:fn split_cursor
fn split_cursor(cursor: Option<PageCursor>) -> (Option<DateTime<Utc>>, Option<Uuid>) {
    match cursor {
        Some(c) => (Some(c.ts), Some(c.id)),
        None => (None, None),
    }
}
```

**Dependencies** — `PageCursor`. **Used by** — the four paginated list methods.
**Repeated context** — none.

---

## User


**Code** — complete and verbatim:

```rust
// md:User
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub email_verified_at: Option<DateTime<Utc>>,
}
```

Marker `// md:User`. `{ id, email, password_hash (never serialised), display_name,
created_at, email_verified_at }` — an account; `email_verified_at: None` = never
proved ownership (issue #49). **Used by** auth/account handlers, `send_flow_mail`.

## Note


**Code** — complete and verbatim:

```rust
// md:Note
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Note {
    pub id: Uuid,
    pub title: String,
    pub owner_id: Uuid,
    pub notebook_id: Option<Uuid>,
    pub is_todo: bool,
    pub todo_due: Option<DateTime<Utc>>,
    pub todo_completed: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}
```

Marker `// md:Note`. `{ id, title, owner_id, notebook_id, is_todo, todo_due,
todo_completed, created_at, updated_at, deleted_at }` — note **metadata** (the body is
derived from lines). `notebook_id: None` = the inbox. `title` is stored encrypted
when the cipher is enabled; every read path decrypts before returning. **Used by**
`http.rs`, `permissions.rs` (`resolve_note_access` takes `&Note`).

## NotePatch


**Code** — complete and verbatim:

```rust
// md:NotePatch
#[derive(Debug, Default)]
pub struct NotePatch {
    pub title: Option<String>,
    pub notebook_id: Option<Option<Uuid>>,
    pub is_todo: Option<bool>,
    pub todo_due: Option<Option<DateTime<Utc>>>,
    pub todo_completed: Option<Option<DateTime<Utc>>>,
}
```

Marker `// md:NotePatch`. Tri-state partial update: `None` = leave unchanged,
`Some(inner)` = set (so `Some(None)` clears a nullable field). **Used by**
`http.rs::update_note` → `update_note_meta`.

## NOTE_COLS


**Code** — complete and verbatim:

```rust
// md:NOTE_COLS
const NOTE_COLS: &str = "id, title, owner_id, notebook_id, is_todo, todo_due, todo_completed, \
                         created_at, updated_at, deleted_at";
```

Marker `// md:NOTE_COLS`. The shared column list every note query selects/returns —
one definition so the `Note` mapping cannot drift per query.

## NoteShare


**Code** — complete and verbatim:

```rust
// md:NoteShare
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct NoteShare {
    pub note_id: Uuid,
    pub user_id: Uuid,
    pub capabilities: i32,
    pub created_at: DateTime<Utc>,
}
```

Marker `// md:NoteShare`. `{ note_id, user_id, capabilities, created_at }` — one
grant; `capabilities` is the **normalised** bitmask (`permissions::Capabilities`).
`created_at` doubles as the `HISTORY_VISIBILITY=access` window start. **Used by**
share handlers, `resolve_note_access`, `access_cutoff`.

## NotebookShare


**Code** — complete and verbatim:

```rust
// md:NotebookShare
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct NotebookShare {
    pub notebook_id: Uuid,
    pub user_id: Uuid,
    pub capabilities: i32,
    pub created_at: DateTime<Utc>,
}
```

Marker `// md:NotebookShare`. Same shape for notebooks; the **source** rows the
destructive cascade copies onto child notes. **Used by** notebook share handlers,
`resolve_notebook_access`.

## Line


**Code** — complete and verbatim:

```rust
// md:Line
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Line {
    pub id: Uuid,
    pub note_id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub vv: Json<VersionVector>,
    pub last_writer: String,
}
```

Marker `// md:Line`. `{ id, note_id, content, created_at, updated_at, deleted_at,
vv: Json<VersionVector>, last_writer }` — one collaborative line: an independently
versioned entity with soft-delete. `content` stored encrypted when enabled; `vv` is
opaque JSONB (resolution happens in `collab.rs`). **Used by** `collab.rs`
(`apply_op`, snapshots), `http.rs::materialize_body`.

## NoteOrder


**Code** — complete and verbatim:

```rust
// md:NoteOrder
#[derive(Debug, Clone)]
pub struct NoteOrder {
    pub note_id: Uuid,
    pub order: Vec<Uuid>,
    pub updated_at: DateTime<Utc>,
    pub vv: VersionVector,
    pub last_writer: String,
}
```

Marker `// md:NoteOrder`. `{ note_id, order: Vec<Uuid>, updated_at, vv,
last_writer }` — the versioned order of a note's lines (`NoteLines` in the design
doc): its own entity, resolved like a line. **Used by** `collab.rs`, `http.rs`.

## CollabEvent


**Code** — complete and verbatim:

```rust
// md:CollabEvent
#[derive(Debug, Clone)]
pub struct CollabEvent {
    pub seq: i64,
    pub note_id: Uuid,
    pub origin_instance: Uuid,
    pub origin_conn: i64,
    pub user_id: Uuid,
    pub ops: serde_json::Value,
}
```

Marker `// md:CollabEvent`. `{ seq, note_id, origin_instance, origin_conn, user_id,
ops }` — one row of the cross-instance op fan-out outbox (issue #45); `ops` is the
serialised `Vec<LineOp>`, opaque here. **Used by** `bus.rs`,
`collab.rs::deliver_event`.

## PresenceRow


**Code** — complete and verbatim:

```rust
// md:PresenceRow
#[derive(Debug, Clone)]
pub struct PresenceRow {
    pub user_id: Uuid,
    pub display_name: String,
    pub cursor: Option<serde_json::Value>,
}
```

Marker `// md:PresenceRow`. `{ user_id, display_name, cursor }` — one merged
presence entry across instances (issue #45); `cursor` is the opaque
`protocol::Cursor` as stored JSON. **Used by** `collab.rs::deliver_presence`.

## UserDevice


**Code** — complete and verbatim:

```rust
// md:UserDevice
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserDevice {
    pub id: Uuid,
    pub user_id: Uuid,
    pub device_name: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
}
```

Marker `// md:UserDevice`. `{ id, user_id, device_name, created_at, last_seen_at }` —
one device login; the row whose existence *is* token validity (revocation by
deletion). **Used by** `auth.rs`, `sync.rs`, device handlers.

## ChangeRow


**Code** — complete and verbatim:

```rust
// md:ChangeRow
#[derive(Debug, Clone)]
pub struct ChangeRow {
    pub seq: i64,
    pub origin_device_id: Uuid,
    pub payload: serde_json::Value,
}
```

Marker `// md:ChangeRow`. `{ seq, origin_device_id, payload }` — one journal row as
fetched for delivery: sequence, the pushing device (echo suppression), the opaque
`Change` payload. **Used by** `sync.rs::deliver_backlog`.

---

## HistoryKind

**Identification** — enum; marker `// md:HistoryKind`. `Note | Notebook` — which
journaled entity kind a history query targets.

**Code** — complete and verbatim:

```rust
// md:HistoryKind
#[derive(Debug, Clone, Copy)]
pub enum HistoryKind {
    Note,
    Notebook,
}
```

**Used by** — `entity_history`, `http.rs` history handlers.
**Repeated context** — none.

## impl HistoryKind

**Identification** — the inherent impl block; marker `// md:impl HistoryKind`.

**Code** — container: members documented as sub-blocks below: fn snapshot_key, fn upsert_ops, fn delete_ops.

**What it does** — Maps a history entity kind (note / notebook) to the change-journal op-tags and snapshot key its history query matches on.

### fn snapshot_key

**Identification** — method of `impl HistoryKind`; marker `// md:impl HistoryKind > fn snapshot_key`.

**Code** — complete and verbatim:

```rust
    // md:impl HistoryKind > fn snapshot_key
    fn snapshot_key(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Notebook => "notebook",
        }
    }
```

**What it does** — The JSON key an entity's create/update snapshot is stored under in a change payload (`"note"` / `"notebook"`).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn upsert_ops

**Identification** — method of `impl HistoryKind`; marker `// md:impl HistoryKind > fn upsert_ops`.

**Code** — complete and verbatim:

```rust
    // md:impl HistoryKind > fn upsert_ops
    fn upsert_ops(self) -> &'static [&'static str] {
        match self {
            Self::Note => &["note_create", "note_update", "create", "update"],
            Self::Notebook => &["notebook_create", "notebook_update"],
        }
    }
```

**What it does** — The `op` tags that count as a create/update for this entity kind (notes include the v1 aliases `create`/`update`); `entity_history` matches these for the snapshot shape.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn delete_ops

**Identification** — method of `impl HistoryKind`; marker `// md:impl HistoryKind > fn delete_ops`.

**Code** — complete and verbatim:

```rust
    // md:impl HistoryKind > fn delete_ops
    fn delete_ops(self) -> &'static [&'static str] {
        match self {
            Self::Note => &["note_delete", "delete"],
            Self::Notebook => &["notebook_delete"],
        }
    }
```

**What it does** — The `op` tags that count as a delete/tombstone for this entity kind; `entity_history` matches these for the top-level-id tombstone shape.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

## EntityVersionRow

**Identification** — struct; marker `// md:EntityVersionRow`.
`{ timestamp, device_id, entity: Option<Value> }` — one reconstructed version for the
history endpoints: when it was written, by which sync device, and the snapshot exactly
as the device pushed it (opaque — client-encrypted fields stay ciphertext). `entity:
None` = tombstone. **Used by** `entity_history`, `http.rs` history handlers.

**Code** — complete and verbatim:

```rust
// md:EntityVersionRow
#[derive(Debug, Clone, Serialize)]
pub struct EntityVersionRow {
    pub timestamp: DateTime<Utc>,
    pub device_id: String,
    pub entity: Option<serde_json::Value>,
}
```

---

## Notebook

**Identification** — REST row struct; marker `// md:Notebook`. A notebook as served
over REST (metadata only; `vv`/`last_writer` are internal to resolution and not
exposed). **Used by** — `list_notebooks`, `http.rs`. **Repeated context** —
server-as-source-of-truth; client DB is a cache.

**Code** — complete and verbatim:

```rust
// md:Notebook
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Notebook {
    pub id: Uuid,
    pub title: String,
    pub alias: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}
```

## Tag

**Identification** — REST row struct; marker `// md:Tag`. Same shape and context for
tags. **Used by** — `list_tags`, `http.rs`.

**Code** — complete and verbatim:

```rust
// md:Tag
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Tag {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub system: bool,
}
```

`system` (issue #128) is the transport-only internal-function marker mirrored from
`keeplin-core`'s `Tag.system`: the server stores and returns it but never interprets the
(encrypted) tag title nor filters by it. Every `SELECT` mapping into this `FromRow` struct
must include the `system` column, or the row decode fails at runtime.

## ResourceMeta

**Identification** — REST row struct; marker `// md:ResourceMeta`. Resource metadata
as served over REST; excludes the binary payload — fetched separately from
`resource_blobs` via `GET /api/resources/:id/data`. **Used by** — `list_resources`,
`http.rs`.

**Code** — complete and verbatim:

```rust
// md:ResourceMeta
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ResourceMeta {
    pub id: Uuid,
    pub title: String,
    pub mime_type: String,
    pub file_name: String,
    pub size: i64,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}
```

---

## fn incoming_wins

**Identification** — private fn; marker `// md:fn incoming_wins`.

**Code** — complete and verbatim:

```rust
// md:fn incoming_wins
fn incoming_wins(
    local_vv: &VersionVector,
    local_ts: DateTime<Utc>,
    local_writer: &str,
    incoming_vv: &VersionVector,
    incoming_ts: DateTime<Utc>,
    incoming_writer: &str,
) -> bool {
    use keeplin_core::storage::note_log::{resolve, Winner};
    matches!(
        resolve(
            local_vv,
            local_ts,
            local_writer,
            incoming_vv,
            incoming_ts,
            incoming_writer,
        ),
        Winner::Incoming
    )
}
```

**What it does** — Decides whether an incoming versioned write should replace the
stored one by delegating to keeplin-core's exact resolution (`note_log::resolve`:
vv dominance, then the deterministic `(timestamp, device)` tiebreak) — so the server
converges to the **same winner** as every client.

**Dependencies** — keeplin-core `resolve`/`Winner`. **Used by** — the six
materialisation writes (`upsert_notebook`, `delete_notebook`, `upsert_tag`,
`delete_tag`, `upsert_note_tag`, `upsert_resource_meta`, `delete_resource`).

**Repeated context** — Same-resolution-everywhere is the system's convergence
contract; this is its store-side entry point.

---

## Store

**Identification** — struct; marker `// md:Store`.

**Code** — complete and verbatim:

```rust
// md:Store
#[derive(Clone)]
pub struct Store {
    pool: Pool<Postgres>,
    cipher: crate::crypto::Cipher,
}
```

**What it does** — The data-access handle: the bounded pool plus the at-rest cipher
(issue keeplin#110; disabled/passthrough unless `AT_REST_KEY` is set). Cloneable
(pool and cipher are cheap handles).

**Used by** — `AppState.store` and everything through it.
**Repeated context** — cipher choke point (see *Overview*).

---

## impl Store

**Identification** — the inherent impl block; marker `// md:impl Store`.

**Code** — container: members documented as sub-blocks below: fn new, fn with_cipher, fn create_user, fn get_user_by_email, fn get_user_by_id, fn update_password, fn delete_user, fn login_locked, fn record_login_failure, fn clear_login_failures, fn prune_login_attempts, fn create_email_token, fn consume_email_token, fn mark_email_verified, fn prune_email_tokens, fn create_device, fn get_device, fn list_devices_by_user, fn delete_device, fn delete_all_devices, fn touch_device, fn append_changes, fn changes_after, fn entity_history, fn get_cursor, fn advance_cursor, fn prune_delivered_changes, fn purge_deleted_resource_blobs, fn gc_line_tombstones, fn ping, fn counts, fn create_note, fn get_note, fn list_notes_for_user, fn update_note_meta, fn decrypt_note_title, fn soft_delete_note, fn set_note_owner, fn create_or_update_share, fn get_share, fn list_shares, fn delete_share, fn notebook_owner, fn set_notebook_owner, fn get_notebook_share, fn list_notebook_shares, fn create_or_update_notebook_share, fn delete_notebook_share, fn cascade_notebook_to_notes, fn apply_notebook_shares_to_note, fn get_line, fn get_line_on, fn list_lines, fn insert_line, fn insert_line_on, fn update_line, fn update_line_on, fn soft_delete_line, fn soft_delete_line_on, fn get_note_order, fn get_note_order_on, fn set_note_order, fn set_note_order_on, fn pool, fn notify, fn lock_note_order, fn insert_collab_event, fn get_collab_event, fn prune_collab_events, fn upsert_presence, fn delete_presence, fn list_presence, fn touch_instance_presence, fn sweep_presence, fn delete_instance_presence, fn upsert_notebook, fn delete_notebook, fn upsert_tag, fn delete_tag, fn upsert_note_tag, fn upsert_resource_meta, fn delete_resource, fn put_resource_blob, fn get_resource_blob, fn resource_owned_by, fn list_notebooks, fn list_tags, fn list_resources, fn list_note_tag_ids, fn user_blob_bytes_excluding, fn count_live_notes_for_user.

**What it does** — The relay's entire data-access surface. Every method carries its own `// md:impl Store > fn <name>` marker and is documented as a sub-block below, in source order. The methods fall into these regions: Constructors; Users; Login lockout; Email-flow tokens; Devices; Change journal; Delivery cursors; Retention / maintenance / metrics; Notes; Note shares; Notebook ownership & shares; Lines (each with a pool form and an `_on(executor)` form that runs on the connection holding the note's advisory lock); Line order; Cross-instance bus primitives; Domain-entity materialisation (server = source of truth, every write resolved by `incoming_wins` under `SELECT … FOR UPDATE`); Domain-entity reads (cold rehydration); Per-user quotas. All queries run through `self.pool` (or an `_on` executor) and encrypt/decrypt human-readable columns through `self.cipher`.

### fn new

**Identification** — method of `impl Store`; marker `// md:impl Store > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn new
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self {
            pool,
            cipher: crate::crypto::Cipher::from_key(None).expect("null key never fails"),
        }
    }
```

**What it does** — store with encryption **disabled** (plaintext); used by tests and as the default. Production wires a real cipher via `with_cipher`.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn with_cipher

**Identification** — method of `impl Store`; marker `// md:impl Store > fn with_cipher`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn with_cipher
    pub fn with_cipher(pool: Pool<Postgres>, cipher: crate::crypto::Cipher) -> Self {
        Self { pool, cipher }
    }
```

**What it does** — store with a configured at-rest cipher (`AppState::new` calls this).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn create_user

**Identification** — method of `impl Store`; marker `// md:impl Store > fn create_user`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn create_user
    pub async fn create_user(
        &self,
        email: &str,
        password_hash: &str,
        display_name: &str,
    ) -> Result<User, AppError> {
        let user = sqlx::query_as::<_, User>(
            r#"INSERT INTO users (id, email, password_hash, display_name)
               VALUES ($1, $2, $3, $4)
               RETURNING id, email, password_hash, display_name, created_at, email_verified_at"#,
        )
        .bind(Uuid::new_v4())
        .bind(email)
        .bind(password_hash)
        .bind(display_name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.is_unique_violation() => AppError::Conflict,
            _ => AppError::from(e),
        })?;
        Ok(user)
    }
```

**What it does** — INSERT returning the row; unique-violation (duplicate email) → `AppError::Conflict`. Emails arrive already normalised (`http.rs`).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_user_by_email

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_user_by_email`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_user_by_email
    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        let user = sqlx::query_as::<_, User>(
            r#"SELECT id, email, password_hash, display_name, created_at, email_verified_at
               FROM users WHERE email = $1"#,
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(user)
    }
```

**What it does** — straightforward lookups (include `password_hash` for verification; it is never serialised).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_user_by_id

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_user_by_id`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_user_by_id
    pub async fn get_user_by_id(&self, id: Uuid) -> Result<Option<User>, AppError> {
        let user = sqlx::query_as::<_, User>(
            r#"SELECT id, email, password_hash, display_name, created_at, email_verified_at
               FROM users WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(user)
    }
```

**What it does** — straightforward lookups (include `password_hash` for verification; it is never serialised).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn update_password

**Identification** — method of `impl Store`; marker `// md:impl Store > fn update_password`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn update_password
    pub async fn update_password(&self, id: Uuid, password_hash: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
            .bind(id)
            .bind(password_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

**What it does** — replace the Argon2 hash (issue #31).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn delete_user

**Identification** — method of `impl Store`; marker `// md:impl Store > fn delete_user`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn delete_user
    pub async fn delete_user(&self, id: Uuid) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
```

**What it does** — account deletion (issue #31): every FK back to `users` (devices, cursors, journal, notes + lines/order/shares, notebooks, tags, resources + blobs, note_tags) is `ON DELETE CASCADE`, so one statement tears down the whole account. Returns whether the user existed. The deliberate exception to soft-delete (privacy action, not a replicated edit).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn login_locked

**Identification** — method of `impl Store`; marker `// md:impl Store > fn login_locked`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn login_locked
    pub async fn login_locked(&self, email: &str) -> Result<bool, AppError> {
        let locked: Option<bool> = sqlx::query_scalar(
            "SELECT COALESCE(locked_until > now(), false) FROM login_attempts WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(locked.unwrap_or(false))
    }
```

**What it does** — is the email currently locked out? `COALESCE(locked_until > now(), false)`: a row whose lock was never armed has NULL `locked_until`, and `NULL > now()` must read as "not locked".

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn record_login_failure

**Identification** — method of `impl Store`; marker `// md:impl Store > fn record_login_failure`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn record_login_failure
    pub async fn record_login_failure(
        &self,
        email: &str,
        max_failures: i32,
        lockout_secs: u64,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"INSERT INTO login_attempts (email, failed_count, last_failed_at, locked_until)
               VALUES ($1, 1, now(),
                       CASE WHEN 1 >= $2 THEN now() + $3 * interval '1 second' END)
               ON CONFLICT (email) DO UPDATE SET
                   failed_count = CASE
                       WHEN login_attempts.last_failed_at < now() - $3 * interval '1 second' THEN 1
                       ELSE login_attempts.failed_count + 1 END,
                   last_failed_at = now(),
                   locked_until = CASE
                       WHEN (CASE
                           WHEN login_attempts.last_failed_at < now() - $3 * interval '1 second' THEN 1
                           ELSE login_attempts.failed_count + 1 END) >= $2
                       THEN now() + $3 * interval '1 second' END"#,
        )
        .bind(email)
        .bind(max_failures)
        .bind(lockout_secs as f64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

**What it does** — one **atomic upsert** records a failure for the submitted email (whether or not an account exists — uniform, no existence oracle): restarts the counter when the previous failure is older than the lockout window; arms `locked_until` when the counter reaches `max_failures`. Atomicity means concurrent failures across replicas never lose a count (issue #45).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn clear_login_failures

**Identification** — method of `impl Store`; marker `// md:impl Store > fn clear_login_failures`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn clear_login_failures
    pub async fn clear_login_failures(&self, email: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM login_attempts WHERE email = $1")
            .bind(email)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

**What it does** — a successful login (or completed reset) wipes the email's history.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn prune_login_attempts

**Identification** — method of `impl Store`; marker `// md:impl Store > fn prune_login_attempts`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn prune_login_attempts
    pub async fn prune_login_attempts(&self, older_than: DateTime<Utc>) -> Result<u64, AppError> {
        let r = sqlx::query("DELETE FROM login_attempts WHERE last_failed_at < $1")
            .bind(older_than)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }
```

**What it does** — maintenance: drop rows whose last activity predates the cutoff (their lock long expired).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn create_email_token

**Identification** — method of `impl Store`; marker `// md:impl Store > fn create_email_token`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn create_email_token
    pub async fn create_email_token(
        &self,
        user_id: Uuid,
        kind: crate::mail::MailKind,
        ttl_secs: u64,
    ) -> Result<(String, DateTime<Utc>), AppError> {
        use aes_gcm::aead::rand_core::RngCore;
        use base64::Engine as _;
        const MAX_LIVE_EMAIL_TOKENS: i64 = 5;
        let live: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM email_tokens
               WHERE user_id = $1 AND kind = $2 AND used_at IS NULL AND expires_at > now()"#,
        )
        .bind(user_id)
        .bind(kind.as_str())
        .fetch_one(&self.pool)
        .await?;
        if live >= MAX_LIVE_EMAIL_TOKENS {
            return Err(AppError::TooManyAttempts);
        }
        let mut raw = [0u8; 32];
        aes_gcm::aead::OsRng.fill_bytes(&mut raw);
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        let expires_at = Utc::now() + chrono::Duration::seconds(ttl_secs as i64);
        sqlx::query(
            r#"INSERT INTO email_tokens (id, user_id, kind, token_hash, expires_at)
               VALUES ($1, $2, $3, $4, $5)"#,
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(kind.as_str())
        .bind(token_hash(&token))
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok((token, expires_at))
    }
```

**What it does** — mint a single-use token for a kind, valid `ttl_secs`: 32 random bytes (OsRng) → URL-safe base64; the **raw** token is returned once (to hand to the mail webhook); only its SHA-256 (`token_hash`) is stored. Anti mail-bombing: refuses (`429 TooManyAttempts`) once the user already has `MAX_LIVE_EMAIL_TOKENS` (5) unexpired unused tokens of that kind (the reset flow hides even this behind its uniform 200).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn consume_email_token

**Identification** — method of `impl Store`; marker `// md:impl Store > fn consume_email_token`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn consume_email_token
    pub async fn consume_email_token(
        &self,
        kind: crate::mail::MailKind,
        raw_token: &str,
    ) -> Result<Option<Uuid>, AppError> {
        let user_id: Option<Uuid> = sqlx::query_scalar(
            r#"UPDATE email_tokens SET used_at = now()
               WHERE token_hash = $1 AND kind = $2
                 AND used_at IS NULL AND expires_at > now()
               RETURNING user_id"#,
        )
        .bind(token_hash(raw_token))
        .bind(kind.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(user_id)
    }
```

**What it does** — single-use + unexpired, **atomically**: `used_at` is set in the same UPDATE that checks it, so a token racing itself across replicas is safe. Returns the owning user, or `None` for unknown/expired/used.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn mark_email_verified

**Identification** — method of `impl Store`; marker `// md:impl Store > fn mark_email_verified`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn mark_email_verified
    pub async fn mark_email_verified(&self, user_id: Uuid) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

**What it does** — stamp `email_verified_at` (`COALESCE(email_verified_at, now())` — idempotent, keeps the first time).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn prune_email_tokens

**Identification** — method of `impl Store`; marker `// md:impl Store > fn prune_email_tokens`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn prune_email_tokens
    pub async fn prune_email_tokens(&self, older_than: DateTime<Utc>) -> Result<u64, AppError> {
        let r = sqlx::query("DELETE FROM email_tokens WHERE expires_at < $1")
            .bind(older_than)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }
```

**What it does** — maintenance: drop tokens expired before the cutoff.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn create_device

**Identification** — method of `impl Store`; marker `// md:impl Store > fn create_device`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn create_device
    pub async fn create_device(
        &self,
        user_id: Uuid,
        device_name: &str,
    ) -> Result<UserDevice, AppError> {
        let device = sqlx::query_as::<_, UserDevice>(
            r#"INSERT INTO user_devices (id, user_id, device_name)
               VALUES ($1, $2, $3)
               RETURNING id, user_id, device_name, created_at, last_seen_at"#,
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(device_name)
        .fetch_one(&self.pool)
        .await?;
        Ok(device)
    }
```

**What it does** — one row per login; the id goes into the JWT.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_device

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_device`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_device
    pub async fn get_device(&self, id: Uuid) -> Result<Option<UserDevice>, AppError> {
        let device = sqlx::query_as::<_, UserDevice>(
            r#"SELECT id, user_id, device_name, created_at, last_seen_at
               FROM user_devices WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(device)
    }
```

**What it does** — the revocation check's lookup (REST middleware + both WS handshakes).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn list_devices_by_user

**Identification** — method of `impl Store`; marker `// md:impl Store > fn list_devices_by_user`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn list_devices_by_user
    pub async fn list_devices_by_user(&self, user_id: Uuid) -> Result<Vec<UserDevice>, AppError> {
        let devices = sqlx::query_as::<_, UserDevice>(
            r#"SELECT id, user_id, device_name, created_at, last_seen_at
               FROM user_devices WHERE user_id = $1 ORDER BY created_at"#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(devices)
    }
```

**What it does** — the caller's devices, oldest first.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn delete_device

**Identification** — method of `impl Store`; marker `// md:impl Store > fn delete_device`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn delete_device
    pub async fn delete_device(&self, id: Uuid, user_id: Uuid) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM user_devices WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
```

**What it does** — delete one of the user's devices, revoking its token immediately (the auth middleware and both WebSocket handshakes re-check device existence). Returns whether a row was deleted.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn delete_all_devices

**Identification** — method of `impl Store`; marker `// md:impl Store > fn delete_all_devices`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn delete_all_devices
    pub async fn delete_all_devices(&self, user_id: Uuid) -> Result<u64, AppError> {
        let result = sqlx::query("DELETE FROM user_devices WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
```

**What it does** — sign out everywhere (issue #31); returns the count.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn touch_device

**Identification** — method of `impl Store`; marker `// md:impl Store > fn touch_device`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn touch_device
    pub async fn touch_device(&self, device_id: Uuid) -> Result<(), AppError> {
        sqlx::query("UPDATE user_devices SET last_seen_at = now() WHERE id = $1")
            .bind(device_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

**What it does** — stamp `last_seen_at` (relay connect/disconnect).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn append_changes

**Identification** — method of `impl Store`; marker `// md:impl Store > fn append_changes`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn append_changes
    pub async fn append_changes(
        &self,
        user_id: Uuid,
        origin_device_id: Uuid,
        sync_device_id: &str,
        batch_id: Uuid,
        payloads: &[serde_json::Value],
    ) -> Result<Vec<i64>, AppError> {
        let mut tx = self.pool.begin().await?;
        let mut seqs = Vec::with_capacity(payloads.len());
        for (idx, payload) in payloads.iter().enumerate() {
            let row = sqlx::query(
                r#"INSERT INTO changes
                       (user_id, origin_device_id, batch_id, batch_index, sync_device_id, payload)
                   VALUES ($1, $2, $3, $4, $5, $6)
                   ON CONFLICT (user_id, batch_id, batch_index) DO NOTHING
                   RETURNING seq"#,
            )
            .bind(user_id)
            .bind(origin_device_id)
            .bind(batch_id)
            .bind(idx as i32)
            .bind(sync_device_id)
            .bind(payload)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(row) = row {
                seqs.push(row.get::<i64, _>("seq"));
            }
        }
        tx.commit().await?;
        Ok(seqs)
    }
```

**What it does** — append a batch to the user's journal in one transaction: per payload, `INSERT … ON CONFLICT (user_id, batch_id, batch_index) DO NOTHING RETURNING seq`. Duplicate re-sends are silently skipped, so a client retry after a reconnect never creates duplicate rows; dedup is **per user** (issue #26 — a cross-user `batch_id` collision cannot suppress another account's changes). Returns the seqs actually inserted (empty for a pure duplicate → caller skips fan-out).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn changes_after

**Identification** — method of `impl Store`; marker `// md:impl Store > fn changes_after`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn changes_after
    pub async fn changes_after(
        &self,
        user_id: Uuid,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<ChangeRow>, AppError> {
        let rows = sqlx::query(
            r#"SELECT seq, origin_device_id, payload
               FROM changes
               WHERE user_id = $1 AND seq > $2
               ORDER BY seq
               LIMIT $3"#,
        )
        .bind(user_id)
        .bind(after_seq)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ChangeRow {
                seq: r.get("seq"),
                origin_device_id: r.get("origin_device_id"),
                payload: r.get("payload"),
            })
            .collect())
    }
```

**What it does** — up to `limit` rows with `seq > after_seq` in order. Rows from every device are returned (including the caller's own) so the delivery cursor can advance past them; the caller filters out its own before sending.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn entity_history

**Identification** — method of `impl Store`; marker `// md:impl Store > fn entity_history`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn entity_history
    pub async fn entity_history(
        &self,
        kind: HistoryKind,
        entity_id: Uuid,
        limit: i64,
        not_before: Option<DateTime<Utc>>,
        authored_not_before: Option<DateTime<Utc>>,
        user_scope: Option<Uuid>,
    ) -> Result<Vec<EntityVersionRow>, AppError> {
        let upsert_ops: Vec<String> = kind.upsert_ops().iter().map(|s| s.to_string()).collect();
        let delete_ops: Vec<String> = kind.delete_ops().iter().map(|s| s.to_string()).collect();
        let rows = sqlx::query(&format!(
            r#"SELECT payload, sync_device_id, received_at
               FROM changes
               WHERE ((payload->>'op' = ANY($2) AND payload->'{key}'->>'id' = $1)
                   OR (payload->>'op' = ANY($3) AND payload->>'id' = $1))
                 AND ($4::timestamptz IS NULL OR received_at >= $4)
                 AND ($6::uuid IS NULL OR user_id = $6)
                 AND ($7::timestamptz IS NULL OR COALESCE(
                        keeplin_try_timestamptz(
                            CASE WHEN payload->>'op' = ANY($3) THEN payload->>'deleted_at'
                                 ELSE payload->'{key}'->>'updated_at' END),
                        received_at) >= $7)
               ORDER BY seq DESC
               LIMIT $5"#,
            key = kind.snapshot_key(),
        ))
        .bind(entity_id.to_string())
        .bind(&upsert_ops)
        .bind(&delete_ops)
        .bind(not_before)
        .bind(limit)
        .bind(user_scope)
        .bind(authored_not_before)
        .fetch_all(&self.pool)
        .await?;

        let parse_ts =
            |v: &serde_json::Value| -> Option<DateTime<Utc>> { v.as_str()?.parse().ok() };
        Ok(rows
            .into_iter()
            .map(|row| {
                let payload: serde_json::Value = row.get("payload");
                let received_at: DateTime<Utc> = row.get("received_at");
                let op = payload["op"].as_str().unwrap_or_default();
                let (timestamp, entity) = if kind.delete_ops().contains(&op) {
                    (parse_ts(&payload["deleted_at"]), None)
                } else {
                    let snapshot = payload[kind.snapshot_key()].clone();
                    (parse_ts(&snapshot["updated_at"]), Some(snapshot))
                };
                EntityVersionRow {
                    timestamp: timestamp.unwrap_or(received_at),
                    device_id: row.get("sync_device_id"),
                    entity,
                }
            })
            .collect())
    }
```

**What it does** — an entity's past versions, newest first (`seq DESC`) — the server-side counterpart of the client's `HistoryRepository` (the client's local journal holds only its own device's changes; the server journal holds every device's, across every user). History is **per-entity** (issue #27): matched by `op` tag + snapshot id across all users' rows; the HTTP handler authorises read access *before* calling. Two independent lower bounds: `not_before` compares the journal row's `received_at` (retention age); `authored_not_before` compares the **payload's own causal timestamp** — snapshot `updated_at` for create/update, the top-level `deleted_at` for tombstones, via the `keeplin_try_timestamptz` safe cast (migration 0013; one malformed client timestamp degrades to the `received_at` fallback instead of failing every read). It deliberately does **not** use `received_at`: journal re-delivery (a reinstalled client re-pushing from epoch) mints fresh `received_at` values for pre-access content and would leak it to a collaborator under the `access` policy — an honest-client boundary (a forged `updated_at` can still cheat; see `SECURITY.md`). `user_scope`: `None` = across all users (shared, materialised entity); `Some(user)` = that account only (relay-only entity). Returns `EntityVersionRow`s; payloads stay opaque (only the envelope is inspected; snapshots returned verbatim).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_cursor

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_cursor`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_cursor
    pub async fn get_cursor(&self, device_id: Uuid) -> Result<i64, AppError> {
        let row = sqlx::query("SELECT last_seq FROM device_cursors WHERE device_id = $1")
            .bind(device_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<i64, _>("last_seq")).unwrap_or(0))
    }
```

**What it does** — a device's `last_seq` (0 if never connected).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn advance_cursor

**Identification** — method of `impl Store`; marker `// md:impl Store > fn advance_cursor`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn advance_cursor
    pub async fn advance_cursor(&self, device_id: Uuid, seq: i64) -> Result<(), AppError> {
        sqlx::query(
            r#"INSERT INTO device_cursors (device_id, last_seq, updated_at)
               VALUES ($1, $2, now())
               ON CONFLICT (device_id) DO UPDATE
               SET last_seq = GREATEST(device_cursors.last_seq, EXCLUDED.last_seq),
                   updated_at = now()"#,
        )
        .bind(device_id)
        .bind(seq)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

**What it does** — upsert with `GREATEST(existing, new)` so a stale connection racing a newer one can never move the watermark backwards.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn prune_delivered_changes

**Identification** — method of `impl Store`; marker `// md:impl Store > fn prune_delivered_changes`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn prune_delivered_changes
    pub async fn prune_delivered_changes(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, AppError> {
        let result = sqlx::query(
            r#"DELETE FROM changes c
               WHERE c.received_at < $1
                 AND c.seq <= (
                     SELECT COALESCE(MIN(dc.last_seq), 0)
                     FROM user_devices d
                     JOIN device_cursors dc ON dc.device_id = d.id
                     WHERE d.user_id = c.user_id
                 )"#,
        )
        .bind(older_than)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
```

**What it does** — delete journal rows older than the cutoff that every **connected** device of the owning user has passed (`seq <= MIN(last_seq)` over devices **with a cursor row**). A device that was logged in but never connected has no cursor row and no longer blocks pruning forever (issue #23) — safe because a fresh/long-absent device does not replay the journal from 0: it cold-rehydrates materialised entities over REST and rebuilds note state from collab snapshots (pinned by the `materialised_entities_survive_journal_pruning` test). A user with **no** connected devices prunes nothing (`MIN` over no rows → 0 via COALESCE).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn purge_deleted_resource_blobs

**Identification** — method of `impl Store`; marker `// md:impl Store > fn purge_deleted_resource_blobs`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn purge_deleted_resource_blobs
    pub async fn purge_deleted_resource_blobs(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, AppError> {
        let result = sqlx::query(
            r#"DELETE FROM resource_blobs rb
               USING resources r
               WHERE rb.resource_id = r.id
                 AND r.deleted_at IS NOT NULL
                 AND r.deleted_at < $1"#,
        )
        .bind(older_than)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
```

**What it does** — reclaim blob bytes of resources soft-deleted before the cutoff; the **metadata tombstone is kept** (it must keep competing in resolution so the delete converges) — only dead bytes go (issue #24; mirrors the client's `purge_deleted_resources`).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn gc_line_tombstones

**Identification** — method of `impl Store`; marker `// md:impl Store > fn gc_line_tombstones`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn gc_line_tombstones
    pub async fn gc_line_tombstones(&self, older_than: DateTime<Utc>) -> Result<u64, AppError> {
        let rows = sqlx::query(
            r#"DELETE FROM lines
               WHERE deleted_at IS NOT NULL AND deleted_at < $1
               RETURNING id, note_id"#,
        )
        .bind(older_than)
        .fetch_all(&self.pool)
        .await?;
        if rows.is_empty() {
            return Ok(0);
        }
        let mut by_note: std::collections::HashMap<Uuid, Vec<Uuid>> =
            std::collections::HashMap::new();
        for row in &rows {
            by_note
                .entry(row.get("note_id"))
                .or_default()
                .push(row.get("id"));
        }
        for (note_id, dead) in by_note {
            let mut tx = self.pool.begin().await?;
            let existing: Option<(Json<Vec<Uuid>>,)> = sqlx::query_as(
                "SELECT order_json FROM note_line_order WHERE note_id = $1 FOR UPDATE",
            )
            .bind(note_id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some((order_json,)) = existing {
                let kept: Vec<Uuid> = order_json
                    .0
                    .into_iter()
                    .filter(|id| !dead.contains(id))
                    .collect();
                sqlx::query("UPDATE note_line_order SET order_json = $2 WHERE note_id = $1")
                    .bind(note_id)
                    .bind(Json(kept))
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        Ok(rows.len() as u64)
    }
```

**What it does** — compact old line tombstones (design §6.4): delete lines soft-deleted before the cutoff, then per affected note **read-modify-write the order under `SELECT … FOR UPDATE`** so a concurrent collaborative `Insert`/`Move` (which rewrites the whole order) cannot be clobbered (issue #25): the concurrent order UPDATE blocks until this commits and lands on top; a membership drop it did not know about is re-applied by the next GC pass — never a lost edit. Only membership changes; the order's version metadata is untouched (compaction is not an edit).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn ping

**Identification** — method of `impl Store`; marker `// md:impl Store > fn ping`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn ping
    pub async fn ping(&self) -> Result<(), AppError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }
```

**What it does** — `SELECT 1` for the readiness probe (issue #36).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn counts

**Identification** — method of `impl Store`; marker `// md:impl Store > fn counts`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn counts
    pub async fn counts(&self) -> Result<(i64, i64, i64, i64), AppError> {
        let row = sqlx::query(
            r#"SELECT
                 (SELECT count(*) FROM users) AS users,
                 (SELECT count(*) FROM notes WHERE deleted_at IS NULL) AS notes,
                 (SELECT count(*) FROM lines) AS lines,
                 (SELECT count(*) FROM lines WHERE deleted_at IS NOT NULL) AS tombstones"#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok((
            row.get("users"),
            row.get("notes"),
            row.get("lines"),
            row.get("tombstones"),
        ))
    }
```

**What it does** — aggregate `(users, live notes, lines, tombstoned lines)` for `/api/metrics`.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn create_note

**Identification** — method of `impl Store`; marker `// md:impl Store > fn create_note`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn create_note
    pub async fn create_note(
        &self,
        id: Option<Uuid>,
        title: &str,
        owner_id: Uuid,
    ) -> Result<Note, AppError> {
        let mut tx = self.pool.begin().await?;
        let mut note = sqlx::query_as::<_, Note>(&format!(
            "INSERT INTO notes (id, title, owner_id) VALUES ($1, $2, $3) RETURNING {NOTE_COLS}"
        ))
        .bind(id.unwrap_or_else(Uuid::new_v4))
        .bind(self.cipher.encrypt(title)?)
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.is_unique_violation() => AppError::Conflict,
            _ => AppError::from(e),
        })?;
        note.title = title.to_string();
        sqlx::query(
            r#"INSERT INTO note_line_order (note_id, order_json, updated_at, vv, last_writer)
               VALUES ($1, '[]', now(), '{}', $2)"#,
        )
        .bind(note.id)
        .bind(owner_id.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(note)
    }
```

**What it does** — create the note **and its empty versioned line order** in one transaction. `id` may be client-supplied (a daemon keeps its local note id); duplicate → `Conflict`. Title stored via `cipher.encrypt`, returned decrypted.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_note

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_note`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_note
    pub async fn get_note(&self, id: Uuid) -> Result<Option<Note>, AppError> {
        let mut note = sqlx::query_as::<_, Note>(&format!(
            "SELECT {NOTE_COLS} FROM notes WHERE id = $1 AND deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(note) = note.as_mut() {
            note.title = self.cipher.decrypt(&note.title)?;
        }
        Ok(note)
    }
```

**What it does** — live note by id (`deleted_at IS NULL`); title decrypted.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn list_notes_for_user

**Identification** — method of `impl Store`; marker `// md:impl Store > fn list_notes_for_user`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn list_notes_for_user
    pub async fn list_notes_for_user(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
        cursor: Option<PageCursor>,
    ) -> Result<Vec<Note>, AppError> {
        let (cur_ts, cur_id) = split_cursor(cursor);
        let notes = sqlx::query_as::<_, Note>(
            r#"SELECT n.id, n.title, n.owner_id, n.notebook_id, n.is_todo, n.todo_due,
                      n.todo_completed, n.created_at, n.updated_at, n.deleted_at
               FROM notes n
               LEFT JOIN note_shares s ON s.note_id = n.id AND s.user_id = $1
               LEFT JOIN notebooks nb
                      ON nb.id = n.notebook_id AND nb.user_id = $1 AND nb.deleted_at IS NULL
               WHERE n.deleted_at IS NULL
                 AND (n.owner_id = $1 OR s.user_id IS NOT NULL OR nb.id IS NOT NULL)
                 AND ($3::timestamptz IS NULL OR (n.updated_at, n.id) < ($3, $4))
               ORDER BY n.updated_at DESC, n.id DESC
               LIMIT $2"#,
        )
        .bind(user_id)
        .bind(limit.unwrap_or(i64::MAX))
        .bind(cur_ts)
        .bind(cur_id)
        .fetch_all(&self.pool)
        .await?;
        let mut notes = notes;
        for note in notes.iter_mut() {
            note.title = self.cipher.decrypt(&note.title)?;
        }
        Ok(notes)
    }
```

**What it does** — notes visible to the user: owned, shared (`note_shares`), or filed in a notebook they own (the folder-owner rule, mirroring `permissions::resolve_note_access`), newest first; keyset-paginated on `(updated_at, id)`; titles decrypted.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn update_note_meta

**Identification** — method of `impl Store`; marker `// md:impl Store > fn update_note_meta`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn update_note_meta
    pub async fn update_note_meta(
        &self,
        id: Uuid,
        patch: &NotePatch,
    ) -> Result<Option<Note>, AppError> {
        let enc_title = patch
            .title
            .as_deref()
            .map(|t| self.cipher.encrypt(t))
            .transpose()?;
        let note = sqlx::query_as::<_, Note>(&format!(
            r#"UPDATE notes SET
                   title = COALESCE($2, title),
                   notebook_id = CASE WHEN $3 THEN $4 ELSE notebook_id END,
                   is_todo = COALESCE($5, is_todo),
                   todo_due = CASE WHEN $6 THEN $7 ELSE todo_due END,
                   todo_completed = CASE WHEN $8 THEN $9 ELSE todo_completed END,
                   updated_at = now()
               WHERE id = $1 AND deleted_at IS NULL
               RETURNING {NOTE_COLS}"#
        ))
        .bind(id)
        .bind(enc_title.as_deref())
        .bind(patch.notebook_id.is_some())
        .bind(patch.notebook_id.flatten())
        .bind(patch.is_todo)
        .bind(patch.todo_due.is_some())
        .bind(patch.todo_due.flatten())
        .bind(patch.todo_completed.is_some())
        .bind(patch.todo_completed.flatten())
        .fetch_optional(&self.pool)
        .await?;
        self.decrypt_note_title(note)
    }
```

**What it does** — apply a `NotePatch`: `COALESCE`/`CASE` binds so an absent field is untouched while an explicit null clears a nullable column; bumps `updated_at`; title encrypted on the way in.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn decrypt_note_title

**Identification** — method of `impl Store`; marker `// md:impl Store > fn decrypt_note_title`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn decrypt_note_title
    fn decrypt_note_title(&self, note: Option<Note>) -> Result<Option<Note>, AppError> {
        match note {
            Some(mut n) => {
                n.title = self.cipher.decrypt(&n.title)?;
                Ok(Some(n))
            }
            None => Ok(None),
        }
    }
```

**What it does** — private helper decrypting an optional read.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn soft_delete_note

**Identification** — method of `impl Store`; marker `// md:impl Store > fn soft_delete_note`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn soft_delete_note
    pub async fn soft_delete_note(&self, id: Uuid) -> Result<Option<Note>, AppError> {
        let note = sqlx::query_as::<_, Note>(&format!(
            r#"UPDATE notes SET deleted_at = now(), updated_at = now()
               WHERE id = $1 AND deleted_at IS NULL
               RETURNING {NOTE_COLS}"#
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        self.decrypt_note_title(note)
    }
```

**What it does** — tombstone (sets `deleted_at`, bumps `updated_at`).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn set_note_owner

**Identification** — method of `impl Store`; marker `// md:impl Store > fn set_note_owner`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn set_note_owner
    pub async fn set_note_owner(
        &self,
        id: Uuid,
        new_owner: Uuid,
    ) -> Result<Option<Note>, AppError> {
        let note = sqlx::query_as::<_, Note>(&format!(
            r#"UPDATE notes SET owner_id = $2, updated_at = now()
               WHERE id = $1 AND deleted_at IS NULL
               RETURNING {NOTE_COLS}"#
        ))
        .bind(id)
        .bind(new_owner)
        .fetch_optional(&self.pool)
        .await?;
        self.decrypt_note_title(note)
    }
```

**What it does** — ownership transfer (owner-only, enforced at the HTTP layer); ownership is separate from grants.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn create_or_update_share

**Identification** — method of `impl Store`; marker `// md:impl Store > fn create_or_update_share`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn create_or_update_share
    pub async fn create_or_update_share(
        &self,
        note_id: Uuid,
        user_id: Uuid,
        capabilities: i32,
    ) -> Result<NoteShare, AppError> {
        let share = sqlx::query_as::<_, NoteShare>(
            r#"INSERT INTO note_shares (note_id, user_id, capabilities)
               VALUES ($1, $2, $3)
               ON CONFLICT (note_id, user_id) DO UPDATE SET capabilities = EXCLUDED.capabilities
               RETURNING note_id, user_id, capabilities, created_at"#,
        )
        .bind(note_id)
        .bind(user_id)
        .bind(capabilities)
        .fetch_one(&self.pool)
        .await?;
        Ok(share)
    }
```

**What it does** — upsert a grant (bitmask arrives normalised and capped from `http.rs`).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_share

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_share`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_share
    pub async fn get_share(
        &self,
        note_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<NoteShare>, AppError> {
        let share = sqlx::query_as::<_, NoteShare>(
            r#"SELECT note_id, user_id, capabilities, created_at
               FROM note_shares WHERE note_id = $1 AND user_id = $2"#,
        )
        .bind(note_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(share)
    }
```

**What it does** — one grantee's row (also feeds the access-history cutoff).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn list_shares

**Identification** — method of `impl Store`; marker `// md:impl Store > fn list_shares`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn list_shares
    pub async fn list_shares(&self, note_id: Uuid) -> Result<Vec<NoteShare>, AppError> {
        let shares = sqlx::query_as::<_, NoteShare>(
            r#"SELECT note_id, user_id, capabilities, created_at
               FROM note_shares WHERE note_id = $1 ORDER BY created_at"#,
        )
        .bind(note_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(shares)
    }
```

**What it does** — all grants on a note.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn delete_share

**Identification** — method of `impl Store`; marker `// md:impl Store > fn delete_share`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn delete_share
    pub async fn delete_share(&self, note_id: Uuid, user_id: Uuid) -> Result<(), AppError> {
        sqlx::query("DELETE FROM note_shares WHERE note_id = $1 AND user_id = $2")
            .bind(note_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

**What it does** — revoke (or self-remove).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn notebook_owner

**Identification** — method of `impl Store`; marker `// md:impl Store > fn notebook_owner`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn notebook_owner
    pub async fn notebook_owner(&self, notebook_id: Uuid) -> Result<Option<Uuid>, AppError> {
        let owner: Option<(Uuid,)> =
            sqlx::query_as("SELECT user_id FROM notebooks WHERE id = $1 AND deleted_at IS NULL")
                .bind(notebook_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(owner.map(|r| r.0))
    }
```

**What it does** — `notebooks.user_id` of a live notebook, else `None`.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn set_notebook_owner

**Identification** — method of `impl Store`; marker `// md:impl Store > fn set_notebook_owner`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn set_notebook_owner
    pub async fn set_notebook_owner(
        &self,
        notebook_id: Uuid,
        new_owner: Uuid,
    ) -> Result<Option<Uuid>, AppError> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "UPDATE notebooks SET user_id = $2, updated_at = now()
             WHERE id = $1 AND deleted_at IS NULL RETURNING id",
        )
        .bind(notebook_id)
        .bind(new_owner)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }
```

**What it does** — transfer; the caller re-cascades separately.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_notebook_share

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_notebook_share`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_notebook_share
    pub async fn get_notebook_share(
        &self,
        notebook_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<NotebookShare>, AppError> {
        let share = sqlx::query_as::<_, NotebookShare>(
            r#"SELECT notebook_id, user_id, capabilities, created_at
               FROM notebook_shares WHERE notebook_id = $1 AND user_id = $2"#,
        )
        .bind(notebook_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(share)
    }
```

**What it does** — lookups.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn list_notebook_shares

**Identification** — method of `impl Store`; marker `// md:impl Store > fn list_notebook_shares`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn list_notebook_shares
    pub async fn list_notebook_shares(
        &self,
        notebook_id: Uuid,
    ) -> Result<Vec<NotebookShare>, AppError> {
        let shares = sqlx::query_as::<_, NotebookShare>(
            r#"SELECT notebook_id, user_id, capabilities, created_at
               FROM notebook_shares WHERE notebook_id = $1 ORDER BY created_at"#,
        )
        .bind(notebook_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(shares)
    }
```

**What it does** — lookups.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn create_or_update_notebook_share

**Identification** — method of `impl Store`; marker `// md:impl Store > fn create_or_update_notebook_share`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn create_or_update_notebook_share
    pub async fn create_or_update_notebook_share(
        &self,
        notebook_id: Uuid,
        user_id: Uuid,
        capabilities: i32,
    ) -> Result<NotebookShare, AppError> {
        let mut tx = self.pool.begin().await?;
        let share = sqlx::query_as::<_, NotebookShare>(
            r#"INSERT INTO notebook_shares (notebook_id, user_id, capabilities)
               VALUES ($1, $2, $3)
               ON CONFLICT (notebook_id, user_id) DO UPDATE SET capabilities = EXCLUDED.capabilities
               RETURNING notebook_id, user_id, capabilities, created_at"#,
        )
        .bind(notebook_id)
        .bind(user_id)
        .bind(capabilities)
        .fetch_one(&mut *tx)
        .await?;
        cascade_notebook_to_notes_tx(&mut tx, notebook_id).await?;
        tx.commit().await?;
        Ok(share)
    }
```

**What it does** — upsert the grant **and** run the destructive cascade onto every child note, in one transaction.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn delete_notebook_share

**Identification** — method of `impl Store`; marker `// md:impl Store > fn delete_notebook_share`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn delete_notebook_share
    pub async fn delete_notebook_share(
        &self,
        notebook_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM notebook_shares WHERE notebook_id = $1 AND user_id = $2")
            .bind(notebook_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        cascade_notebook_to_notes_tx(&mut tx, notebook_id).await?;
        tx.commit().await?;
        Ok(())
    }
```

**What it does** — revoke + re-cascade, one transaction.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn cascade_notebook_to_notes

**Identification** — method of `impl Store`; marker `// md:impl Store > fn cascade_notebook_to_notes`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn cascade_notebook_to_notes
    pub async fn cascade_notebook_to_notes(&self, notebook_id: Uuid) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        cascade_notebook_to_notes_tx(&mut tx, notebook_id).await?;
        tx.commit().await?;
        Ok(())
    }
```

**What it does** — re-cascade without changing grants (after an ownership transfer).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn apply_notebook_shares_to_note

**Identification** — method of `impl Store`; marker `// md:impl Store > fn apply_notebook_shares_to_note`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn apply_notebook_shares_to_note
    pub async fn apply_notebook_shares_to_note(
        &self,
        note_id: Uuid,
        notebook_id: Uuid,
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        replace_note_shares_from_notebook_tx(&mut tx, note_id, notebook_id).await?;
        tx.commit().await?;
        Ok(())
    }
```

**What it does** — adopt the notebook's grants onto **one** note (the move-into case), destructively.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_line

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_line`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_line
    pub async fn get_line(&self, id: Uuid) -> Result<Option<Line>, AppError> {
        self.get_line_on(&self.pool, id).await
    }
```

**What it does** — lookup; content decrypted.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_line_on

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_line_on`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_line_on
    pub async fn get_line_on<'e, E>(&self, exec: E, id: Uuid) -> Result<Option<Line>, AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let mut line = sqlx::query_as::<_, Line>(
            r#"SELECT id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer
               FROM lines WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(exec)
        .await?;
        if let Some(line) = line.as_mut() {
            line.content = self.cipher.decrypt(&line.content)?;
        }
        Ok(line)
    }
```

**What it does** — lookup; content decrypted.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn list_lines

**Identification** — method of `impl Store`; marker `// md:impl Store > fn list_lines`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn list_lines
    pub async fn list_lines(&self, note_id: Uuid) -> Result<Vec<Line>, AppError> {
        let mut lines = sqlx::query_as::<_, Line>(
            r#"SELECT id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer
               FROM lines WHERE note_id = $1"#,
        )
        .bind(note_id)
        .fetch_all(&self.pool)
        .await?;
        for line in lines.iter_mut() {
            line.content = self.cipher.decrypt(&line.content)?;
        }
        Ok(lines)
    }
```

**What it does** — every line of a note, **tombstones included** (snapshots need them); contents decrypted.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn insert_line

**Identification** — method of `impl Store`; marker `// md:impl Store > fn insert_line`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn insert_line
    pub async fn insert_line(
        &self,
        id: Uuid,
        note_id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Line, AppError> {
        self.insert_line_on(
            &self.pool,
            id,
            note_id,
            content,
            vv,
            last_writer,
            updated_at,
        )
        .await
    }
```

**What it does** — insert with vv/writer/timestamp; content encrypted in, returned decrypted.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn insert_line_on

**Identification** — method of `impl Store`; marker `// md:impl Store > fn insert_line_on`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn insert_line_on
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_line_on<'e, E>(
        &self,
        exec: E,
        id: Uuid,
        note_id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Line, AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let mut line = sqlx::query_as::<_, Line>(
            r#"INSERT INTO lines (id, note_id, content, created_at, updated_at, vv, last_writer)
               VALUES ($1, $2, $3, now(), $4, $5, $6)
               RETURNING id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer"#,
        )
        .bind(id)
        .bind(note_id)
        .bind(self.cipher.encrypt(content)?)
        .bind(updated_at)
        .bind(Json(vv))
        .bind(last_writer)
        .fetch_one(exec)
        .await?;
        line.content = content.to_string();
        Ok(line)
    }
```

**What it does** — insert with vv/writer/timestamp; content encrypted in, returned decrypted.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn update_line

**Identification** — method of `impl Store`; marker `// md:impl Store > fn update_line`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn update_line
    pub async fn update_line(
        &self,
        id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Option<Line>, AppError> {
        self.update_line_on(&self.pool, id, content, vv, last_writer, updated_at)
            .await
    }
```

**What it does** — overwrite content + version metadata (an applied `Update`); **also clears `deleted_at`** — a causally newer edit revives a tombstone, matching keeplin-core's note semantics.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn update_line_on

**Identification** — method of `impl Store`; marker `// md:impl Store > fn update_line_on`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn update_line_on
    pub async fn update_line_on<'e, E>(
        &self,
        exec: E,
        id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Option<Line>, AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let mut line = sqlx::query_as::<_, Line>(
            r#"UPDATE lines
               SET content = $2, vv = $3, last_writer = $4, updated_at = $5, deleted_at = NULL
               WHERE id = $1
               RETURNING id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer"#,
        )
        .bind(id)
        .bind(self.cipher.encrypt(content)?)
        .bind(Json(vv))
        .bind(last_writer)
        .bind(updated_at)
        .fetch_optional(exec)
        .await?;
        if let Some(line) = line.as_mut() {
            line.content = content.to_string();
        }
        Ok(line)
    }
```

**What it does** — overwrite content + version metadata (an applied `Update`); **also clears `deleted_at`** — a causally newer edit revives a tombstone, matching keeplin-core's note semantics.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn soft_delete_line

**Identification** — method of `impl Store`; marker `// md:impl Store > fn soft_delete_line`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn soft_delete_line
    pub async fn soft_delete_line(
        &self,
        id: Uuid,
        deleted_at: DateTime<Utc>,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Option<Line>, AppError> {
        self.soft_delete_line_on(&self.pool, id, deleted_at, vv, last_writer, updated_at)
            .await
    }
```

**What it does** — tombstone (an applied `Delete`); the row stays for convergence and remains in the order until GC.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn soft_delete_line_on

**Identification** — method of `impl Store`; marker `// md:impl Store > fn soft_delete_line_on`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn soft_delete_line_on
    pub async fn soft_delete_line_on<'e, E>(
        &self,
        exec: E,
        id: Uuid,
        deleted_at: DateTime<Utc>,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Option<Line>, AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let mut line = sqlx::query_as::<_, Line>(
            r#"UPDATE lines
               SET deleted_at = $2, vv = $3, last_writer = $4, updated_at = $5
               WHERE id = $1
               RETURNING id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer"#,
        )
        .bind(id)
        .bind(deleted_at)
        .bind(Json(vv))
        .bind(last_writer)
        .bind(updated_at)
        .fetch_optional(exec)
        .await?;
        if let Some(line) = line.as_mut() {
            line.content = self.cipher.decrypt(&line.content)?;
        }
        Ok(line)
    }
```

**What it does** — tombstone (an applied `Delete`); the row stays for convergence and remains in the order until GC.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_note_order

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_note_order`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_note_order
    pub async fn get_note_order(&self, note_id: Uuid) -> Result<Option<NoteOrder>, AppError> {
        self.get_note_order_on(&self.pool, note_id).await
    }
```

**What it does** — the order entity (`order_json`, `vv`, `last_writer`, `updated_at`).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_note_order_on

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_note_order_on`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_note_order_on
    pub async fn get_note_order_on<'e, E>(
        &self,
        exec: E,
        note_id: Uuid,
    ) -> Result<Option<NoteOrder>, AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query(
            r#"SELECT note_id, order_json, updated_at, vv, last_writer
               FROM note_line_order WHERE note_id = $1"#,
        )
        .bind(note_id)
        .fetch_optional(exec)
        .await?;
        Ok(row.map(|r| NoteOrder {
            note_id: r.get("note_id"),
            order: r.get::<Json<Vec<Uuid>>, _>("order_json").0,
            updated_at: r.get("updated_at"),
            vv: r.get::<Json<VersionVector>, _>("vv").0,
            last_writer: r.get("last_writer"),
        }))
    }
```

**What it does** — the order entity (`order_json`, `vv`, `last_writer`, `updated_at`).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn set_note_order

**Identification** — method of `impl Store`; marker `// md:impl Store > fn set_note_order`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn set_note_order
    pub async fn set_note_order(
        &self,
        note_id: Uuid,
        order: &[Uuid],
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.set_note_order_on(&self.pool, note_id, order, vv, last_writer, updated_at)
            .await
    }
```

**What it does** — overwrite the order with its new merged vv (an applied `Insert`/`Move`).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn set_note_order_on

**Identification** — method of `impl Store`; marker `// md:impl Store > fn set_note_order_on`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn set_note_order_on
    pub async fn set_note_order_on<'e, E>(
        &self,
        exec: E,
        note_id: Uuid,
        order: &[Uuid],
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<(), AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            r#"UPDATE note_line_order
               SET order_json = $2, vv = $3, last_writer = $4, updated_at = $5
               WHERE note_id = $1"#,
        )
        .bind(note_id)
        .bind(Json(order))
        .bind(Json(vv))
        .bind(last_writer)
        .bind(updated_at)
        .execute(exec)
        .await?;
        Ok(())
    }
```

**What it does** — overwrite the order with its new merged vv (an applied `Insert`/`Move`).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn pool

**Identification** — method of `impl Store`; marker `// md:impl Store > fn pool`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn pool
    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }
```

**What it does** — the pool, so `bus.rs` can open a dedicated `PgListener`.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn notify

**Identification** — method of `impl Store`; marker `// md:impl Store > fn notify`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn notify
    pub async fn notify(&self, channel: &str, payload: &str) -> Result<(), AppError> {
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(channel)
            .bind(payload)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

**What it does** — `SELECT pg_notify($1, $2)` (the function form takes the payload as a bind; the statement form would need interpolation).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn lock_note_order

**Identification** — method of `impl Store`; marker `// md:impl Store > fn lock_note_order`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn lock_note_order
    pub async fn lock_note_order(
        &self,
        note_id: Uuid,
    ) -> Result<sqlx::Transaction<'static, Postgres>, AppError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(note_id.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
```

**What it does** — open a transaction and take `pg_advisory_xact_lock(hashtextextended(note_id, 0))`; the lock lives until the returned transaction commits (caller's batch end) or drops (error → rollback → release). Serialises a note's order read-modify-write across instances.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn insert_collab_event

**Identification** — method of `impl Store`; marker `// md:impl Store > fn insert_collab_event`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn insert_collab_event
    pub async fn insert_collab_event(
        &self,
        note_id: Uuid,
        origin_instance: Uuid,
        origin_conn: i64,
        user_id: Uuid,
        ops: &serde_json::Value,
    ) -> Result<i64, AppError> {
        let seq: i64 = sqlx::query_scalar(
            r#"INSERT INTO collab_events (note_id, origin_instance, origin_conn, user_id, ops)
               VALUES ($1, $2, $3, $4, $5) RETURNING seq"#,
        )
        .bind(note_id)
        .bind(origin_instance)
        .bind(origin_conn)
        .bind(user_id)
        .bind(Json(ops))
        .fetch_one(&self.pool)
        .await?;
        Ok(seq)
    }
```

**What it does** — append an applied op batch to the `collab_events` outbox, returning its `seq` (the value NOTIFYed to siblings).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_collab_event

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_collab_event`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_collab_event
    pub async fn get_collab_event(&self, seq: i64) -> Result<Option<CollabEvent>, AppError> {
        let row = sqlx::query(
            r#"SELECT note_id, origin_instance, origin_conn, user_id, ops
               FROM collab_events WHERE seq = $1"#,
        )
        .bind(seq)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| CollabEvent {
            seq,
            note_id: r.get("note_id"),
            origin_instance: r.get("origin_instance"),
            origin_conn: r.get("origin_conn"),
            user_id: r.get("user_id"),
            ops: r.get::<Json<serde_json::Value>, _>("ops").0,
        }))
    }
```

**What it does** — load an outbox row for local delivery.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn prune_collab_events

**Identification** — method of `impl Store`; marker `// md:impl Store > fn prune_collab_events`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn prune_collab_events
    pub async fn prune_collab_events(&self, older_than: DateTime<Utc>) -> Result<u64, AppError> {
        let r = sqlx::query("DELETE FROM collab_events WHERE created_at < $1")
            .bind(older_than)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }
```

**What it does** — the outbox is a delivery buffer, not history; aged rows are dropped (maintenance loop, 5-minute TTL).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn upsert_presence

**Identification** — method of `impl Store`; marker `// md:impl Store > fn upsert_presence`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn upsert_presence
    pub async fn upsert_presence(
        &self,
        note_id: Uuid,
        instance_id: Uuid,
        conn_id: i64,
        user_id: Uuid,
        display_name: &str,
        cursor: Option<&serde_json::Value>,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"INSERT INTO collab_presence
                   (note_id, instance_id, conn_id, user_id, display_name, cursor, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, now())
               ON CONFLICT (note_id, instance_id, conn_id)
               DO UPDATE SET cursor = EXCLUDED.cursor,
                             display_name = EXCLUDED.display_name,
                             updated_at = now()"#,
        )
        .bind(note_id)
        .bind(instance_id)
        .bind(conn_id)
        .bind(user_id)
        .bind(display_name)
        .bind(cursor.map(|c| Json(c.clone())))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

**What it does** — record/refresh one connection's presence row, keyed `(note_id, instance_id, conn_id)`.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn delete_presence

**Identification** — method of `impl Store`; marker `// md:impl Store > fn delete_presence`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn delete_presence
    pub async fn delete_presence(
        &self,
        note_id: Uuid,
        instance_id: Uuid,
        conn_id: i64,
    ) -> Result<(), AppError> {
        sqlx::query(
            "DELETE FROM collab_presence WHERE note_id = $1 AND instance_id = $2 AND conn_id = $3",
        )
        .bind(note_id)
        .bind(instance_id)
        .bind(conn_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

**What it does** — remove one connection's row (leave/disconnect).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn list_presence

**Identification** — method of `impl Store`; marker `// md:impl Store > fn list_presence`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn list_presence
    pub async fn list_presence(&self, note_id: Uuid) -> Result<Vec<PresenceRow>, AppError> {
        let rows = sqlx::query(
            r#"SELECT user_id, display_name, cursor
               FROM collab_presence WHERE note_id = $1"#,
        )
        .bind(note_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| PresenceRow {
                user_id: r.get("user_id"),
                display_name: r.get("display_name"),
                cursor: r
                    .get::<Option<Json<serde_json::Value>>, _>("cursor")
                    .map(|c| c.0),
            })
            .collect())
    }
```

**What it does** — all rows for a note across instances (caller merges per user).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn touch_instance_presence

**Identification** — method of `impl Store`; marker `// md:impl Store > fn touch_instance_presence`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn touch_instance_presence
    pub async fn touch_instance_presence(&self, instance_id: Uuid) -> Result<(), AppError> {
        sqlx::query("UPDATE collab_presence SET updated_at = now() WHERE instance_id = $1")
            .bind(instance_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

**What it does** — heartbeat: bump `updated_at` on all this instance's rows so a live instance is never swept.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn sweep_presence

**Identification** — method of `impl Store`; marker `// md:impl Store > fn sweep_presence`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn sweep_presence
    pub async fn sweep_presence(&self, older_than: DateTime<Utc>) -> Result<u64, AppError> {
        let r = sqlx::query("DELETE FROM collab_presence WHERE updated_at < $1")
            .bind(older_than)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }
```

**What it does** — drop rows not heartbeated since the cutoff (crashed instances).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn delete_instance_presence

**Identification** — method of `impl Store`; marker `// md:impl Store > fn delete_instance_presence`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn delete_instance_presence
    pub async fn delete_instance_presence(&self, instance_id: Uuid) -> Result<u64, AppError> {
        let r = sqlx::query("DELETE FROM collab_presence WHERE instance_id = $1")
            .bind(instance_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }
```

**What it does** — remove all of one instance's rows (startup/shutdown cleanup).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn upsert_notebook

**Identification** — method of `impl Store`; marker `// md:impl Store > fn upsert_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn upsert_notebook
    pub async fn upsert_notebook(
        &self,
        user_id: Uuid,
        nb: &keeplin_core::models::Notebook,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        if let Some(row) = sqlx::query(
            "SELECT vv, updated_at, last_writer FROM notebooks WHERE id = $1 FOR UPDATE",
        )
        .bind(nb.id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("updated_at"),
                &row.get::<String, _>("last_writer"),
                &nb.vv,
                nb.updated_at,
                &nb.last_writer,
            ) {
                return Ok(false);
            }
        }
        sqlx::query(
            r#"INSERT INTO notebooks
                   (id, user_id, title, alias, created_at, updated_at, deleted_at, vv, last_writer)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               ON CONFLICT (id) DO UPDATE SET
                   title = EXCLUDED.title, alias = EXCLUDED.alias,
                   updated_at = EXCLUDED.updated_at, deleted_at = EXCLUDED.deleted_at,
                   vv = EXCLUDED.vv, last_writer = EXCLUDED.last_writer"#,
        )
        .bind(nb.id)
        .bind(user_id)
        .bind(&nb.title)
        .bind(&nb.alias)
        .bind(nb.created_at)
        .bind(nb.updated_at)
        .bind(nb.deleted_at)
        .bind(Json(&nb.vv))
        .bind(&nb.last_writer)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
```

**What it does** — create/update if it wins.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn delete_notebook

**Identification** — method of `impl Store`; marker `// md:impl Store > fn delete_notebook`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn delete_notebook
    pub async fn delete_notebook(
        &self,
        user_id: Uuid,
        id: Uuid,
        deleted_at: DateTime<Utc>,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        let existed = if let Some(row) = sqlx::query(
            "SELECT vv, updated_at, last_writer FROM notebooks WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("updated_at"),
                &row.get::<String, _>("last_writer"),
                vv,
                deleted_at,
                last_writer,
            ) {
                return Ok(false);
            }
            true
        } else {
            false
        };
        if existed {
            sqlx::query(
                "UPDATE notebooks SET deleted_at = $2, updated_at = $2, vv = $3, last_writer = $4 WHERE id = $1",
            )
            .bind(id).bind(deleted_at).bind(Json(vv)).bind(last_writer)
            .execute(&mut *tx).await?;
        } else {
            sqlx::query(
                r#"INSERT INTO notebooks (id, user_id, title, created_at, updated_at, deleted_at, vv, last_writer)
                   VALUES ($1, $2, '', $3, $3, $3, $4, $5)
                   ON CONFLICT (id) DO NOTHING"#,
            )
            .bind(id).bind(user_id).bind(deleted_at).bind(Json(vv)).bind(last_writer)
            .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(true)
    }
```

**What it does** — tombstone if it wins; an **unknown** notebook gets a minimal tombstone row so a later stale create/update cannot resurrect it.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn upsert_tag

**Identification** — method of `impl Store`; marker `// md:impl Store > fn upsert_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn upsert_tag
    pub async fn upsert_tag(
        &self,
        user_id: Uuid,
        tag: &keeplin_core::models::Tag,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        if let Some(row) =
            sqlx::query("SELECT vv, updated_at, last_writer FROM tags WHERE id = $1 FOR UPDATE")
                .bind(tag.id)
                .fetch_optional(&mut *tx)
                .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("updated_at"),
                &row.get::<String, _>("last_writer"),
                &tag.vv,
                tag.updated_at,
                &tag.last_writer,
            ) {
                return Ok(false);
            }
        }
        sqlx::query(
            r#"INSERT INTO tags (id, user_id, title, created_at, updated_at, deleted_at, vv, last_writer, system)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               ON CONFLICT (id) DO UPDATE SET
                   title = EXCLUDED.title, updated_at = EXCLUDED.updated_at,
                   deleted_at = EXCLUDED.deleted_at, vv = EXCLUDED.vv,
                   last_writer = EXCLUDED.last_writer, system = EXCLUDED.system"#,
        )
        .bind(tag.id)
        .bind(user_id)
        .bind(&tag.title)
        .bind(tag.created_at)
        .bind(tag.updated_at)
        .bind(tag.deleted_at)
        .bind(Json(&tag.vv))
        .bind(&tag.last_writer)
        .bind(tag.system)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
```

**What it does** — same pattern for tags. `system` (issue #128) is persisted as `$9` and
refreshed on conflict (`system = EXCLUDED.system`), so a `TagUpdate` that flips the flag
converges like any other field. The whole core `Tag` reaches here via `materialize`, so the
flag rides the existing `TagCreate`/`TagUpdate` changes with no new op.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn delete_tag

**Identification** — method of `impl Store`; marker `// md:impl Store > fn delete_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn delete_tag
    pub async fn delete_tag(
        &self,
        user_id: Uuid,
        id: Uuid,
        deleted_at: DateTime<Utc>,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        let existed = if let Some(row) =
            sqlx::query("SELECT vv, updated_at, last_writer FROM tags WHERE id = $1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("updated_at"),
                &row.get::<String, _>("last_writer"),
                vv,
                deleted_at,
                last_writer,
            ) {
                return Ok(false);
            }
            true
        } else {
            false
        };
        if existed {
            sqlx::query(
                "UPDATE tags SET deleted_at = $2, updated_at = $2, vv = $3, last_writer = $4 WHERE id = $1",
            )
            .bind(id).bind(deleted_at).bind(Json(vv)).bind(last_writer)
            .execute(&mut *tx).await?;
        } else {
            sqlx::query(
                r#"INSERT INTO tags (id, user_id, title, created_at, updated_at, deleted_at, vv, last_writer)
                   VALUES ($1, $2, '', $3, $3, $3, $4, $5)
                   ON CONFLICT (id) DO NOTHING"#,
            )
            .bind(id).bind(user_id).bind(deleted_at).bind(Json(vv)).bind(last_writer)
            .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(true)
    }
```

**What it does** — same pattern for tags.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn upsert_note_tag

**Identification** — method of `impl Store`; marker `// md:impl Store > fn upsert_note_tag`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn upsert_note_tag
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_note_tag(
        &self,
        user_id: Uuid,
        note_id: Uuid,
        tag_id: Uuid,
        updated_at: DateTime<Utc>,
        deleted_at: Option<DateTime<Utc>>,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        if let Some(row) = sqlx::query(
            "SELECT vv, updated_at, last_writer FROM note_tags
             WHERE user_id = $1 AND note_id = $2 AND tag_id = $3 FOR UPDATE",
        )
        .bind(user_id)
        .bind(note_id)
        .bind(tag_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("updated_at"),
                &row.get::<String, _>("last_writer"),
                vv,
                updated_at,
                last_writer,
            ) {
                return Ok(false);
            }
        }
        sqlx::query(
            r#"INSERT INTO note_tags (user_id, note_id, tag_id, updated_at, deleted_at, vv, last_writer)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (user_id, note_id, tag_id) DO UPDATE SET
                   updated_at = EXCLUDED.updated_at, deleted_at = EXCLUDED.deleted_at,
                   vv = EXCLUDED.vv, last_writer = EXCLUDED.last_writer"#,
        )
        .bind(user_id)
        .bind(note_id)
        .bind(tag_id)
        .bind(updated_at)
        .bind(deleted_at)
        .bind(Json(vv))
        .bind(last_writer)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
```

**What it does** — the association is itself versioned and soft-deletable: add = `deleted_at NULL`, remove = `deleted_at = updated_at`.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn upsert_resource_meta

**Identification** — method of `impl Store`; marker `// md:impl Store > fn upsert_resource_meta`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn upsert_resource_meta
    pub async fn upsert_resource_meta(
        &self,
        user_id: Uuid,
        r: &keeplin_core::models::Resource,
    ) -> Result<bool, AppError> {
        let incoming_ts = r.deleted_at.unwrap_or(r.created_at);
        let mut tx = self.pool.begin().await?;
        if let Some(row) = sqlx::query(
            "SELECT vv, COALESCE(deleted_at, created_at) AS ts, last_writer
             FROM resources WHERE id = $1 FOR UPDATE",
        )
        .bind(r.id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("ts"),
                &row.get::<String, _>("last_writer"),
                &r.vv,
                incoming_ts,
                &r.last_writer,
            ) {
                return Ok(false);
            }
        }
        sqlx::query(
            r#"INSERT INTO resources
                   (id, user_id, title, mime_type, file_name, size, created_at, deleted_at, vv, last_writer)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (id) DO UPDATE SET
                   title = EXCLUDED.title, mime_type = EXCLUDED.mime_type,
                   file_name = EXCLUDED.file_name, size = EXCLUDED.size,
                   deleted_at = EXCLUDED.deleted_at, vv = EXCLUDED.vv,
                   last_writer = EXCLUDED.last_writer"#,
        )
        .bind(r.id)
        .bind(user_id)
        .bind(&r.title)
        .bind(&r.mime_type)
        .bind(&r.file_name)
        .bind(r.size as i64)
        .bind(r.created_at)
        .bind(r.deleted_at)
        .bind(Json(&r.vv))
        .bind(&r.last_writer)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
```

**What it does** — create if it wins; resolution timestamp is `deleted_at ?? created_at`, matching keeplin-core (resources carry no `updated_at`). The binary is uploaded separately.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn delete_resource

**Identification** — method of `impl Store`; marker `// md:impl Store > fn delete_resource`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn delete_resource
    pub async fn delete_resource(
        &self,
        id: Uuid,
        deleted_at: DateTime<Utc>,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        let Some(row) = sqlx::query(
            "SELECT vv, COALESCE(deleted_at, created_at) AS ts, last_writer
             FROM resources WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(false);
        };
        let lvv = row.get::<Json<VersionVector>, _>("vv").0;
        if !incoming_wins(
            &lvv,
            row.get("ts"),
            &row.get::<String, _>("last_writer"),
            vv,
            deleted_at,
            last_writer,
        ) {
            return Ok(false);
        }
        sqlx::query(
            "UPDATE resources SET deleted_at = $2, vv = $3, last_writer = $4 WHERE id = $1",
        )
        .bind(id)
        .bind(deleted_at)
        .bind(Json(vv))
        .bind(last_writer)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
```

**What it does** — tombstone if it wins; an unknown resource is a no-op (`false`) — a later create arrives with its own vv and resolves normally.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn put_resource_blob

**Identification** — method of `impl Store`; marker `// md:impl Store > fn put_resource_blob`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn put_resource_blob
    pub async fn put_resource_blob(&self, resource_id: Uuid, data: &[u8]) -> Result<(), AppError> {
        sqlx::query(
            r#"INSERT INTO resource_blobs (resource_id, data) VALUES ($1, $2)
               ON CONFLICT (resource_id) DO UPDATE SET data = EXCLUDED.data"#,
        )
        .bind(resource_id)
        .bind(data)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

**What it does** — store/replace the binary (FK requires the metadata).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn get_resource_blob

**Identification** — method of `impl Store`; marker `// md:impl Store > fn get_resource_blob`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn get_resource_blob
    pub async fn get_resource_blob(&self, resource_id: Uuid) -> Result<Option<Vec<u8>>, AppError> {
        let row = sqlx::query("SELECT data FROM resource_blobs WHERE resource_id = $1")
            .bind(resource_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<Vec<u8>, _>("data")))
    }
```

**What it does** — fetch the binary.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn resource_owned_by

**Identification** — method of `impl Store`; marker `// md:impl Store > fn resource_owned_by`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn resource_owned_by
    pub async fn resource_owned_by(
        &self,
        resource_id: Uuid,
        user_id: Uuid,
    ) -> Result<bool, AppError> {
        let row = sqlx::query("SELECT 1 FROM resources WHERE id = $1 AND user_id = $2")
            .bind(resource_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }
```

**What it does** — does a metadata row exist for this user (authorises blob upload/download; resources are per-user, not shareable).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn list_notebooks

**Identification** — method of `impl Store`; marker `// md:impl Store > fn list_notebooks`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn list_notebooks
    pub async fn list_notebooks(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
        cursor: Option<PageCursor>,
    ) -> Result<Vec<Notebook>, AppError> {
        let (cur_ts, cur_id) = split_cursor(cursor);
        Ok(sqlx::query_as::<_, Notebook>(
            "SELECT id, title, alias, created_at, updated_at, deleted_at
             FROM notebooks
             WHERE user_id = $1 AND deleted_at IS NULL
               AND ($3::timestamptz IS NULL OR (created_at, id) > ($3, $4))
             ORDER BY created_at, id
             LIMIT $2",
        )
        .bind(user_id)
        .bind(limit.unwrap_or(i64::MAX))
        .bind(cur_ts)
        .bind(cur_id)
        .fetch_all(&self.pool)
        .await?)
    }
```

**What it does** — the user's live rows, keyset-paginated on `(created_at, id)`.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn list_tags

**Identification** — method of `impl Store`; marker `// md:impl Store > fn list_tags`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn list_tags
    pub async fn list_tags(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
        cursor: Option<PageCursor>,
    ) -> Result<Vec<Tag>, AppError> {
        let (cur_ts, cur_id) = split_cursor(cursor);
        Ok(sqlx::query_as::<_, Tag>(
            "SELECT id, title, created_at, updated_at, deleted_at, system
             FROM tags
             WHERE user_id = $1 AND deleted_at IS NULL
               AND ($3::timestamptz IS NULL OR (created_at, id) > ($3, $4))
             ORDER BY created_at, id
             LIMIT $2",
        )
        .bind(user_id)
        .bind(limit.unwrap_or(i64::MAX))
        .bind(cur_ts)
        .bind(cur_id)
        .fetch_all(&self.pool)
        .await?)
    }
```

**What it does** — the user's live rows, keyset-paginated on `(created_at, id)`.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn list_resources

**Identification** — method of `impl Store`; marker `// md:impl Store > fn list_resources`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn list_resources
    pub async fn list_resources(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
        cursor: Option<PageCursor>,
    ) -> Result<Vec<ResourceMeta>, AppError> {
        let (cur_ts, cur_id) = split_cursor(cursor);
        Ok(sqlx::query_as::<_, ResourceMeta>(
            "SELECT id, title, mime_type, file_name, size, created_at, deleted_at
             FROM resources
             WHERE user_id = $1 AND deleted_at IS NULL
               AND ($3::timestamptz IS NULL OR (created_at, id) > ($3, $4))
             ORDER BY created_at, id
             LIMIT $2",
        )
        .bind(user_id)
        .bind(limit.unwrap_or(i64::MAX))
        .bind(cur_ts)
        .bind(cur_id)
        .fetch_all(&self.pool)
        .await?)
    }
```

**What it does** — the user's live rows, keyset-paginated on `(created_at, id)`.

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn list_note_tag_ids

**Identification** — method of `impl Store`; marker `// md:impl Store > fn list_note_tag_ids`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn list_note_tag_ids
    pub async fn list_note_tag_ids(
        &self,
        user_id: Uuid,
        note_id: Uuid,
    ) -> Result<Vec<Uuid>, AppError> {
        let rows = sqlx::query(
            r#"SELECT nt.tag_id FROM note_tags nt
               JOIN tags t ON t.id = nt.tag_id
               WHERE nt.user_id = $1 AND nt.note_id = $2
                 AND nt.deleted_at IS NULL AND t.deleted_at IS NULL
               ORDER BY nt.updated_at"#,
        )
        .bind(user_id)
        .bind(note_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| r.get::<Uuid, _>("tag_id"))
            .collect())
    }
```

**What it does** — live tag ids on a note (association present and both ends live).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn user_blob_bytes_excluding

**Identification** — method of `impl Store`; marker `// md:impl Store > fn user_blob_bytes_excluding`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn user_blob_bytes_excluding
    pub async fn user_blob_bytes_excluding(
        &self,
        user_id: Uuid,
        exclude: Uuid,
    ) -> Result<i64, AppError> {
        let bytes: i64 = sqlx::query_scalar(
            r#"SELECT COALESCE(SUM(octet_length(rb.data)), 0)::bigint
               FROM resource_blobs rb
               JOIN resources r ON r.id = rb.resource_id
               WHERE r.user_id = $1 AND r.deleted_at IS NULL AND rb.resource_id <> $2"#,
        )
        .bind(user_id)
        .bind(exclude)
        .fetch_one(&self.pool)
        .await?;
        Ok(bytes)
    }
```

**What it does** — total bytes of the user's **live** resource binaries, excluding one resource id (an overwrite is measured by its new size, not double-counted). Soft-deleted resources are excluded, so deleting attachments actually frees quota (issue #24).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

### fn count_live_notes_for_user

**Identification** — method of `impl Store`; marker `// md:impl Store > fn count_live_notes_for_user`.

**Code** — complete and verbatim:

```rust
    // md:impl Store > fn count_live_notes_for_user
    pub async fn count_live_notes_for_user(&self, user_id: Uuid) -> Result<i64, AppError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notes WHERE owner_id = $1 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }
```

**What it does** — live owned notes (the `MAX_NOTES_PER_USER` check).

**Dependencies** — `sqlx` query (`query!` / `query_as!`) run on `self.pool` or a passed executor against the Postgres schema in `migrations/`; human-readable columns cross `self.cipher` (`encrypt`/`decrypt`) where applicable. Expects the referenced tables/columns to exist and the row shape to match the mapped struct.

**Used by** — the relay handlers that route to it (`http.rs` REST endpoints, `sync.rs` change materialisation, `collab.rs` line ops, and the maintenance loops in `main.rs`) — see the region overview under `## impl Store`.

**Repeated context** — server is the source of truth for materialised entities; resolution uses `incoming_wins` (version-vector + `(updated_at, last_writer)` tiebreak); encrypted-at-rest columns are decrypted only on the way out.

## fn replace_note_shares_from_notebook_tx

**Identification** — free async fn; marker
`// md:fn replace_note_shares_from_notebook_tx`.

**Code** — complete and verbatim:

```rust
// md:fn replace_note_shares_from_notebook_tx
async fn replace_note_shares_from_notebook_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    note_id: Uuid,
    notebook_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM note_shares WHERE note_id = $1")
        .bind(note_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        r#"INSERT INTO note_shares (note_id, user_id, capabilities)
           SELECT $1, user_id, capabilities FROM notebook_shares WHERE notebook_id = $2"#,
    )
    .bind(note_id)
    .bind(notebook_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
```

**What it does** — Inside a caller-supplied transaction: DELETE one note's
`note_shares`, then INSERT a copy of the notebook's `notebook_shares` — the
destructive cascade for the **move** case. Never touches note ownership: the
cascade governs collaborator grants only; the notebook owner's implicit `manage`
is *not* materialised (resolved at access time by
`permissions::resolve_note_access`), so ownership transfers need no share rewrite.

**Dependencies** — sqlx transaction. **Used by** —
`Store::apply_notebook_shares_to_note`.

**Repeated context** — Destructive-cascade contract (Front B stage 1b), restated:
a note in a notebook always carries an exact copy of the notebook's grant profile.

---

## fn cascade_notebook_to_notes_tx

**Identification** — free async fn; marker
`// md:fn cascade_notebook_to_notes_tx`.

**Code** — complete and verbatim:

```rust
// md:fn cascade_notebook_to_notes_tx
async fn cascade_notebook_to_notes_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    notebook_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM note_shares WHERE note_id IN
         (SELECT id FROM notes WHERE notebook_id = $1 AND deleted_at IS NULL)",
    )
    .bind(notebook_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO note_shares (note_id, user_id, capabilities)
           SELECT n.id, ns.user_id, ns.capabilities
           FROM notes n
           JOIN notebook_shares ns ON ns.notebook_id = n.notebook_id
           WHERE n.notebook_id = $1 AND n.deleted_at IS NULL"#,
    )
    .bind(notebook_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
```

**What it does** — The same replacement for **every live note** in a notebook (the
notebook-permission-change case): bulk DELETE of the child notes' shares, then a
`INSERT … SELECT` join copying the notebook's grants onto each.

**Dependencies** — sqlx transaction. **Used by** —
`Store::{create_or_update_notebook_share, delete_notebook_share,
cascade_notebook_to_notes}`.

**Repeated context** — as above; transactional with the triggering share write so
notes never hold a stale profile.

---
## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `Note` — defined here (EXTRACTED; 6 cross-file edge(s))
- `Store` — defined here (EXTRACTED; 4 cross-file edge(s))
- `User` — defined here (EXTRACTED; 3 cross-file edge(s))
- `EntityVersionRow` — defined here (EXTRACTED; 3 cross-file edge(s))
- `PageCursor` — defined here (EXTRACTED; 2 cross-file edge(s))
- `NoteShare` — defined here (EXTRACTED; 2 cross-file edge(s))
- `NotebookShare` — defined here (EXTRACTED; 2 cross-file edge(s))
- `Line` — defined here (EXTRACTED; 2 cross-file edge(s))
- `UserDevice` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.create_email_token()` — defined here (EXTRACTED; 2 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/crypto.rs` — at-rest encryption of note titles and line content (EXTRACTED: references×2; e.g. `Cipher`)
- `crates/keeplin-srv/src/error.rs` — the API error type (EXTRACTED: imports_from×1, references×90; e.g. `AppError`)
- `crates/keeplin-srv/src/mail.rs` — delegated email delivery (mail webhook) (EXTRACTED: references×2; e.g. `MailKind`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/auth.rs` — passwords, tokens, and the auth middleware (EXTRACTED: calls×1; e.g. `create_token()`)
- `crates/keeplin-srv/src/collab.rs` — the collaborative session engine (EXTRACTED: references×4; e.g. `deliver_event()`, `line_snapshot()`, `winner()`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: references×19; e.g. `.resolve()`, `paginated()`, `RegisterResponse`)
- `crates/keeplin-srv/src/permissions.rs` — note capabilities (EXTRACTED: references×3; e.g. `resolve_note_access()`, `resolve_notebook_access()`)
- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×1; e.g. `AppState`)
- `crates/keeplin-srv/src/sync.rs` — the device sync relay (EXTRACTED: references×1; e.g. `authenticate()`)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `PageCursor` | `// md:PageCursor` |
| 3 | `impl PageCursor` (container) | `// md:impl PageCursor` |
| 4 | `fn new` | `// md:impl PageCursor > fn new` |
| 5 | `fn encode` | `// md:impl PageCursor > fn encode` |
| 6 | `fn decode` | `// md:impl PageCursor > fn decode` |
| 7 | `fn token_hash` | `// md:fn token_hash` |
| 8 | `fn split_cursor` | `// md:fn split_cursor` |
| 9 | `User` | `// md:User` |
| 10 | `Note` | `// md:Note` |
| 11 | `NotePatch` | `// md:NotePatch` |
| 12 | `NOTE_COLS` | `// md:NOTE_COLS` |
| 13 | `NoteShare` | `// md:NoteShare` |
| 14 | `NotebookShare` | `// md:NotebookShare` |
| 15 | `Line` | `// md:Line` |
| 16 | `NoteOrder` | `// md:NoteOrder` |
| 17 | `CollabEvent` | `// md:CollabEvent` |
| 18 | `PresenceRow` | `// md:PresenceRow` |
| 19 | `UserDevice` | `// md:UserDevice` |
| 20 | `ChangeRow` | `// md:ChangeRow` |
| 21 | `HistoryKind` | `// md:HistoryKind` |
| 22 | `impl HistoryKind` (container) | `// md:impl HistoryKind` |
| 23 | `fn snapshot_key` | `// md:impl HistoryKind > fn snapshot_key` |
| 24 | `fn upsert_ops` | `// md:impl HistoryKind > fn upsert_ops` |
| 25 | `fn delete_ops` | `// md:impl HistoryKind > fn delete_ops` |
| 26 | `EntityVersionRow` | `// md:EntityVersionRow` |
| 27 | `Notebook` | `// md:Notebook` |
| 28 | `Tag` | `// md:Tag` |
| 29 | `ResourceMeta` | `// md:ResourceMeta` |
| 30 | `fn incoming_wins` | `// md:fn incoming_wins` |
| 31 | `Store` | `// md:Store` |
| 32 | `impl Store` (container) | `// md:impl Store` |
| 33 | `fn new` | `// md:impl Store > fn new` |
| 34 | `fn with_cipher` | `// md:impl Store > fn with_cipher` |
| 35 | `fn create_user` | `// md:impl Store > fn create_user` |
| 36 | `fn get_user_by_email` | `// md:impl Store > fn get_user_by_email` |
| 37 | `fn get_user_by_id` | `// md:impl Store > fn get_user_by_id` |
| 38 | `fn update_password` | `// md:impl Store > fn update_password` |
| 39 | `fn delete_user` | `// md:impl Store > fn delete_user` |
| 40 | `fn login_locked` | `// md:impl Store > fn login_locked` |
| 41 | `fn record_login_failure` | `// md:impl Store > fn record_login_failure` |
| 42 | `fn clear_login_failures` | `// md:impl Store > fn clear_login_failures` |
| 43 | `fn prune_login_attempts` | `// md:impl Store > fn prune_login_attempts` |
| 44 | `fn create_email_token` | `// md:impl Store > fn create_email_token` |
| 45 | `fn consume_email_token` | `// md:impl Store > fn consume_email_token` |
| 46 | `fn mark_email_verified` | `// md:impl Store > fn mark_email_verified` |
| 47 | `fn prune_email_tokens` | `// md:impl Store > fn prune_email_tokens` |
| 48 | `fn create_device` | `// md:impl Store > fn create_device` |
| 49 | `fn get_device` | `// md:impl Store > fn get_device` |
| 50 | `fn list_devices_by_user` | `// md:impl Store > fn list_devices_by_user` |
| 51 | `fn delete_device` | `// md:impl Store > fn delete_device` |
| 52 | `fn delete_all_devices` | `// md:impl Store > fn delete_all_devices` |
| 53 | `fn touch_device` | `// md:impl Store > fn touch_device` |
| 54 | `fn append_changes` | `// md:impl Store > fn append_changes` |
| 55 | `fn changes_after` | `// md:impl Store > fn changes_after` |
| 56 | `fn entity_history` | `// md:impl Store > fn entity_history` |
| 57 | `fn get_cursor` | `// md:impl Store > fn get_cursor` |
| 58 | `fn advance_cursor` | `// md:impl Store > fn advance_cursor` |
| 59 | `fn prune_delivered_changes` | `// md:impl Store > fn prune_delivered_changes` |
| 60 | `fn purge_deleted_resource_blobs` | `// md:impl Store > fn purge_deleted_resource_blobs` |
| 61 | `fn gc_line_tombstones` | `// md:impl Store > fn gc_line_tombstones` |
| 62 | `fn ping` | `// md:impl Store > fn ping` |
| 63 | `fn counts` | `// md:impl Store > fn counts` |
| 64 | `fn create_note` | `// md:impl Store > fn create_note` |
| 65 | `fn get_note` | `// md:impl Store > fn get_note` |
| 66 | `fn list_notes_for_user` | `// md:impl Store > fn list_notes_for_user` |
| 67 | `fn update_note_meta` | `// md:impl Store > fn update_note_meta` |
| 68 | `fn decrypt_note_title` | `// md:impl Store > fn decrypt_note_title` |
| 69 | `fn soft_delete_note` | `// md:impl Store > fn soft_delete_note` |
| 70 | `fn set_note_owner` | `// md:impl Store > fn set_note_owner` |
| 71 | `fn create_or_update_share` | `// md:impl Store > fn create_or_update_share` |
| 72 | `fn get_share` | `// md:impl Store > fn get_share` |
| 73 | `fn list_shares` | `// md:impl Store > fn list_shares` |
| 74 | `fn delete_share` | `// md:impl Store > fn delete_share` |
| 75 | `fn notebook_owner` | `// md:impl Store > fn notebook_owner` |
| 76 | `fn set_notebook_owner` | `// md:impl Store > fn set_notebook_owner` |
| 77 | `fn get_notebook_share` | `// md:impl Store > fn get_notebook_share` |
| 78 | `fn list_notebook_shares` | `// md:impl Store > fn list_notebook_shares` |
| 79 | `fn create_or_update_notebook_share` | `// md:impl Store > fn create_or_update_notebook_share` |
| 80 | `fn delete_notebook_share` | `// md:impl Store > fn delete_notebook_share` |
| 81 | `fn cascade_notebook_to_notes` | `// md:impl Store > fn cascade_notebook_to_notes` |
| 82 | `fn apply_notebook_shares_to_note` | `// md:impl Store > fn apply_notebook_shares_to_note` |
| 83 | `fn get_line` | `// md:impl Store > fn get_line` |
| 84 | `fn get_line_on` | `// md:impl Store > fn get_line_on` |
| 85 | `fn list_lines` | `// md:impl Store > fn list_lines` |
| 86 | `fn insert_line` | `// md:impl Store > fn insert_line` |
| 87 | `fn insert_line_on` | `// md:impl Store > fn insert_line_on` |
| 88 | `fn update_line` | `// md:impl Store > fn update_line` |
| 89 | `fn update_line_on` | `// md:impl Store > fn update_line_on` |
| 90 | `fn soft_delete_line` | `// md:impl Store > fn soft_delete_line` |
| 91 | `fn soft_delete_line_on` | `// md:impl Store > fn soft_delete_line_on` |
| 92 | `fn get_note_order` | `// md:impl Store > fn get_note_order` |
| 93 | `fn get_note_order_on` | `// md:impl Store > fn get_note_order_on` |
| 94 | `fn set_note_order` | `// md:impl Store > fn set_note_order` |
| 95 | `fn set_note_order_on` | `// md:impl Store > fn set_note_order_on` |
| 96 | `fn pool` | `// md:impl Store > fn pool` |
| 97 | `fn notify` | `// md:impl Store > fn notify` |
| 98 | `fn lock_note_order` | `// md:impl Store > fn lock_note_order` |
| 99 | `fn insert_collab_event` | `// md:impl Store > fn insert_collab_event` |
| 100 | `fn get_collab_event` | `// md:impl Store > fn get_collab_event` |
| 101 | `fn prune_collab_events` | `// md:impl Store > fn prune_collab_events` |
| 102 | `fn upsert_presence` | `// md:impl Store > fn upsert_presence` |
| 103 | `fn delete_presence` | `// md:impl Store > fn delete_presence` |
| 104 | `fn list_presence` | `// md:impl Store > fn list_presence` |
| 105 | `fn touch_instance_presence` | `// md:impl Store > fn touch_instance_presence` |
| 106 | `fn sweep_presence` | `// md:impl Store > fn sweep_presence` |
| 107 | `fn delete_instance_presence` | `// md:impl Store > fn delete_instance_presence` |
| 108 | `fn upsert_notebook` | `// md:impl Store > fn upsert_notebook` |
| 109 | `fn delete_notebook` | `// md:impl Store > fn delete_notebook` |
| 110 | `fn upsert_tag` | `// md:impl Store > fn upsert_tag` |
| 111 | `fn delete_tag` | `// md:impl Store > fn delete_tag` |
| 112 | `fn upsert_note_tag` | `// md:impl Store > fn upsert_note_tag` |
| 113 | `fn upsert_resource_meta` | `// md:impl Store > fn upsert_resource_meta` |
| 114 | `fn delete_resource` | `// md:impl Store > fn delete_resource` |
| 115 | `fn put_resource_blob` | `// md:impl Store > fn put_resource_blob` |
| 116 | `fn get_resource_blob` | `// md:impl Store > fn get_resource_blob` |
| 117 | `fn resource_owned_by` | `// md:impl Store > fn resource_owned_by` |
| 118 | `fn list_notebooks` | `// md:impl Store > fn list_notebooks` |
| 119 | `fn list_tags` | `// md:impl Store > fn list_tags` |
| 120 | `fn list_resources` | `// md:impl Store > fn list_resources` |
| 121 | `fn list_note_tag_ids` | `// md:impl Store > fn list_note_tag_ids` |
| 122 | `fn user_blob_bytes_excluding` | `// md:impl Store > fn user_blob_bytes_excluding` |
| 123 | `fn count_live_notes_for_user` | `// md:impl Store > fn count_live_notes_for_user` |
| 124 | `fn replace_note_shares_from_notebook_tx` | `// md:fn replace_note_shares_from_notebook_tx` |
| 125 | `fn cascade_notebook_to_notes_tx` | `// md:fn cascade_notebook_to_notes_tx` |
