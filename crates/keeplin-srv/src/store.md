# `store.rs` — the PostgreSQL data-access layer

Self-contained companion for `crates/keeplin-srv/src/store.rs`. It documents **every code
block of the source file, in source order** — a reader with only this file must be able to
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

```rust
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

```rust
pub struct PageCursor { pub ts: DateTime<Utc>, pub id: Uuid }
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

**Identification** — impl block; marker `// md:impl PageCursor`. Methods:

- `fn new(ts, id)` (marker `// md:impl PageCursor > fn new`) — constructor.
- `fn encode(&self) -> String` (marker `// md:impl PageCursor > fn encode`) —
  `"<timestamp_micros>_<uuid>"`.
- `fn decode(token) -> Option<Self>` (marker `// md:impl PageCursor > fn decode`) —
  parses a previously encoded token; `None` for any malformed input so the handler
  answers `400` rather than trusting it.

**Dependencies** — chrono parsing. **Used by** — `http.rs` and the list queries.
**Repeated context** — none.

---

## fn token_hash

**Identification** — private fn; marker `// md:fn token_hash`.
`fn token_hash(raw: &str) -> String` — SHA-256 hex of an email-flow token — **the only
form ever stored** (a database dump cannot be replayed into a takeover).

**Dependencies** — `sha2`. **Used by** — `create_email_token`,
`consume_email_token`. **Repeated context** — email-flow token model (issue #49).

---

## fn split_cursor

**Identification** — private fn; marker `// md:fn split_cursor`.
`fn split_cursor(Option<PageCursor>) -> (Option<DateTime<Utc>>, Option<Uuid>)` —
splits an optional cursor into the `(timestamp, id)` binds; `None` maps to
`(NULL, NULL)`, which the `$3 IS NULL` guard in each query turns into "no keyset
filter" (first page).

**Dependencies** — `PageCursor`. **Used by** — the four paginated list methods.
**Repeated context** — none.

---

## Row types

The `FromRow`/serialisable row mappings, in source order. Each is one block with its
own marker; compressed five-point entries:

### User

Marker `// md:User`. `{ id, email, password_hash (never serialised), display_name,
created_at, email_verified_at }` — an account; `email_verified_at: None` = never
proved ownership (issue #49). **Used by** auth/account handlers, `send_flow_mail`.

### Note

Marker `// md:Note`. `{ id, title, owner_id, notebook_id, is_todo, todo_due,
todo_completed, created_at, updated_at, deleted_at }` — note **metadata** (the body is
derived from lines). `notebook_id: None` = the inbox. `title` is stored encrypted
when the cipher is enabled; every read path decrypts before returning. **Used by**
`http.rs`, `permissions.rs` (`resolve_note_access` takes `&Note`).

### NotePatch

Marker `// md:NotePatch`. Tri-state partial update: `None` = leave unchanged,
`Some(inner)` = set (so `Some(None)` clears a nullable field). **Used by**
`http.rs::update_note` → `update_note_meta`.

### NOTE_COLS

Marker `// md:NOTE_COLS`. The shared column list every note query selects/returns —
one definition so the `Note` mapping cannot drift per query.

### NoteShare

Marker `// md:NoteShare`. `{ note_id, user_id, capabilities, created_at }` — one
grant; `capabilities` is the **normalised** bitmask (`permissions::Capabilities`).
`created_at` doubles as the `HISTORY_VISIBILITY=access` window start. **Used by**
share handlers, `resolve_note_access`, `access_cutoff`.

### NotebookShare

Marker `// md:NotebookShare`. Same shape for notebooks; the **source** rows the
destructive cascade copies onto child notes. **Used by** notebook share handlers,
`resolve_notebook_access`.

### Line

Marker `// md:Line`. `{ id, note_id, content, created_at, updated_at, deleted_at,
vv: Json<VersionVector>, last_writer }` — one collaborative line: an independently
versioned entity with soft-delete. `content` stored encrypted when enabled; `vv` is
opaque JSONB (resolution happens in `collab.rs`). **Used by** `collab.rs`
(`apply_op`, snapshots), `http.rs::materialize_body`.

### NoteOrder

Marker `// md:NoteOrder`. `{ note_id, order: Vec<Uuid>, updated_at, vv,
last_writer }` — the versioned order of a note's lines (`NoteLines` in the design
doc): its own entity, resolved like a line. **Used by** `collab.rs`, `http.rs`.

### CollabEvent

Marker `// md:CollabEvent`. `{ seq, note_id, origin_instance, origin_conn, user_id,
ops }` — one row of the cross-instance op fan-out outbox (issue #45); `ops` is the
serialised `Vec<LineOp>`, opaque here. **Used by** `bus.rs`,
`collab.rs::deliver_event`.

### PresenceRow

Marker `// md:PresenceRow`. `{ user_id, display_name, cursor }` — one merged
presence entry across instances (issue #45); `cursor` is the opaque
`protocol::Cursor` as stored JSON. **Used by** `collab.rs::deliver_presence`.

### UserDevice

Marker `// md:UserDevice`. `{ id, user_id, device_name, created_at, last_seen_at }` —
one device login; the row whose existence *is* token validity (revocation by
deletion). **Used by** `auth.rs`, `sync.rs`, device handlers.

### ChangeRow

Marker `// md:ChangeRow`. `{ seq, origin_device_id, payload }` — one journal row as
fetched for delivery: sequence, the pushing device (echo suppression), the opaque
`Change` payload. **Used by** `sync.rs::deliver_backlog`.

---

## HistoryKind

**Identification** — enum; marker `// md:HistoryKind`. `Note | Notebook` — which
journaled entity kind a history query targets.

**Used by** — `entity_history`, `http.rs` history handlers.
**Repeated context** — none.

## impl HistoryKind

**Identification** — impl block; marker `// md:impl HistoryKind`. Methods (own
markers `// md:impl HistoryKind > fn …`):

- `fn snapshot_key` — the JSON key carrying the snapshot in create/update payloads
  (`"note"` / `"notebook"`).
- `fn upsert_ops` — the `op` tags of create/update payloads; note ops include the v1
  short aliases (`"create"`, `"update"`) the client still accepts on read.
- `fn delete_ops` — the `op` tags of delete payloads.

**Used by** — `entity_history` (query construction + row classification).
**Repeated context** — payloads stay opaque; only these envelope keys are inspected.

---

## EntityVersionRow

**Identification** — struct; marker `// md:EntityVersionRow`.
`{ timestamp, device_id, entity: Option<Value> }` — one reconstructed version for the
history endpoints: when it was written, by which sync device, and the snapshot exactly
as the device pushed it (opaque — client-encrypted fields stay ciphertext). `entity:
None` = tombstone. **Used by** `entity_history`, `http.rs` history handlers.

---

## Notebook / Tag / ResourceMeta

**Identification** — three REST row structs; markers `// md:Notebook`, `// md:Tag`,
`// md:ResourceMeta`.

**What it does** — The materialised domain entities as served over REST (metadata
only; `vv`/`last_writer` are internal to resolution and not exposed). `ResourceMeta`
excludes the binary payload — fetched separately from `resource_blobs` via
`GET /api/resources/:id/data`.

**Used by** — the `list_*` reads and `http.rs`. **Repeated context** —
server-as-source-of-truth for these entities; client DB is a cache.

---

## fn incoming_wins

**Identification** — private fn; marker `// md:fn incoming_wins`.

```rust
fn incoming_wins(local_vv, local_ts, local_writer, incoming_vv, incoming_ts,
                 incoming_writer) -> bool
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

```rust
#[derive(Clone)]
pub struct Store { pool: Pool<Postgres>, cipher: crate::crypto::Cipher }
```

**What it does** — The data-access handle: the bounded pool plus the at-rest cipher
(issue keeplin#110; disabled/passthrough unless `AT_REST_KEY` is set). Cloneable
(pool and cipher are cheap handles).

**Used by** — `AppState.store` and everything through it.
**Repeated context** — cipher choke point (see *Overview*).

---

## impl Store

**Identification** — the inherent impl block; marker `// md:impl Store`. Every method
below carries its own marker `// md:impl Store > fn <name>` and is documented in
source order, grouped by the file's own regions.

### Constructors

- **fn new** — store with encryption **disabled** (plaintext); used by tests and as
  the default. Production wires a real cipher via `with_cipher`.
- **fn with_cipher** — store with a configured at-rest cipher
  (`AppState::new` calls this).

### Users

- **fn create_user** — INSERT returning the row; unique-violation (duplicate email)
  → `AppError::Conflict`. Emails arrive already normalised (`http.rs`).
- **fn get_user_by_email** / **fn get_user_by_id** — straightforward lookups
  (include `password_hash` for verification; it is never serialised).
- **fn update_password** — replace the Argon2 hash (issue #31).
- **fn delete_user** — account deletion (issue #31): every FK back to `users`
  (devices, cursors, journal, notes + lines/order/shares, notebooks, tags,
  resources + blobs, note_tags) is `ON DELETE CASCADE`, so one statement tears down
  the whole account. Returns whether the user existed. The deliberate exception to
  soft-delete (privacy action, not a replicated edit).

### Login lockout (brute force, issue #21/migration 0011)

- **fn login_locked** — is the email currently locked out? `COALESCE(locked_until >
  now(), false)`: a row whose lock was never armed has NULL `locked_until`, and
  `NULL > now()` must read as "not locked".
- **fn record_login_failure** — one **atomic upsert** records a failure for the
  submitted email (whether or not an account exists — uniform, no existence
  oracle): restarts the counter when the previous failure is older than the lockout
  window; arms `locked_until` when the counter reaches `max_failures`. Atomicity
  means concurrent failures across replicas never lose a count (issue #45).
- **fn clear_login_failures** — a successful login (or completed reset) wipes the
  email's history.
- **fn prune_login_attempts** — maintenance: drop rows whose last activity predates
  the cutoff (their lock long expired).

### Email-flow tokens (issue #49, migration 0012)

- **fn create_email_token** — mint a single-use token for a kind, valid `ttl_secs`:
  32 random bytes (OsRng) → URL-safe base64; the **raw** token is returned once (to
  hand to the mail webhook); only its SHA-256 (`token_hash`) is stored. Anti
  mail-bombing: refuses (`429 TooManyAttempts`) once the user already has
  `MAX_LIVE_EMAIL_TOKENS` (5) unexpired unused tokens of that kind (the reset flow
  hides even this behind its uniform 200).
- **fn consume_email_token** — single-use + unexpired, **atomically**: `used_at` is
  set in the same UPDATE that checks it, so a token racing itself across replicas
  is safe. Returns the owning user, or `None` for unknown/expired/used.
- **fn mark_email_verified** — stamp `email_verified_at`
  (`COALESCE(email_verified_at, now())` — idempotent, keeps the first time).
- **fn prune_email_tokens** — maintenance: drop tokens expired before the cutoff.

### Devices

- **fn create_device** — one row per login; the id goes into the JWT.
- **fn get_device** — the revocation check's lookup (REST middleware + both WS
  handshakes).
- **fn list_devices_by_user** — the caller's devices, oldest first.
- **fn delete_device** — delete one of the user's devices, revoking its token
  immediately (the auth middleware and both WebSocket handshakes re-check device
  existence). Returns whether a row was deleted.
- **fn delete_all_devices** — sign out everywhere (issue #31); returns the count.
- **fn touch_device** — stamp `last_seen_at` (relay connect/disconnect).

### Change journal

- **fn append_changes** — append a batch to the user's journal in one transaction:
  per payload, `INSERT … ON CONFLICT (user_id, batch_id, batch_index) DO NOTHING
  RETURNING seq`. Duplicate re-sends are silently skipped, so a client retry after
  a reconnect never creates duplicate rows; dedup is **per user** (issue #26 — a
  cross-user `batch_id` collision cannot suppress another account's changes).
  Returns the seqs actually inserted (empty for a pure duplicate → caller skips
  fan-out).
- **fn changes_after** — up to `limit` rows with `seq > after_seq` in order. Rows
  from every device are returned (including the caller's own) so the delivery
  cursor can advance past them; the caller filters out its own before sending.
- **fn entity_history** — an entity's past versions, newest first (`seq DESC`) —
  the server-side counterpart of the client's `HistoryRepository` (the client's
  local journal holds only its own device's changes; the server journal holds every
  device's, across every user). History is **per-entity** (issue #27): matched by
  `op` tag + snapshot id across all users' rows; the HTTP handler authorises read
  access *before* calling. Two independent lower bounds: `not_before` compares the
  journal row's `received_at` (retention age); `authored_not_before` compares the
  **payload's own causal timestamp** — snapshot `updated_at` for create/update, the
  top-level `deleted_at` for tombstones, via the `keeplin_try_timestamptz` safe
  cast (migration 0013; one malformed client timestamp degrades to the
  `received_at` fallback instead of failing every read). It deliberately does
  **not** use `received_at`: journal re-delivery (a reinstalled client re-pushing
  from epoch) mints fresh `received_at` values for pre-access content and would
  leak it to a collaborator under the `access` policy — an honest-client boundary
  (a forged `updated_at` can still cheat; see `SECURITY.md`). `user_scope`:
  `None` = across all users (shared, materialised entity); `Some(user)` = that
  account only (relay-only entity). Returns `EntityVersionRow`s; payloads stay
  opaque (only the envelope is inspected; snapshots returned verbatim).

### Delivery cursors

- **fn get_cursor** — a device's `last_seq` (0 if never connected).
- **fn advance_cursor** — upsert with `GREATEST(existing, new)` so a stale
  connection racing a newer one can never move the watermark backwards.

### Retention / maintenance / metrics

- **fn prune_delivered_changes** — delete journal rows older than the cutoff that
  every **connected** device of the owning user has passed (`seq <=
  MIN(last_seq)` over devices **with a cursor row**). A device that was logged in
  but never connected has no cursor row and no longer blocks pruning forever
  (issue #23) — safe because a fresh/long-absent device does not replay the
  journal from 0: it cold-rehydrates materialised entities over REST and rebuilds
  note state from collab snapshots (pinned by the
  `materialised_entities_survive_journal_pruning` test). A user with **no**
  connected devices prunes nothing (`MIN` over no rows → 0 via COALESCE).
- **fn purge_deleted_resource_blobs** — reclaim blob bytes of resources
  soft-deleted before the cutoff; the **metadata tombstone is kept** (it must keep
  competing in resolution so the delete converges) — only dead bytes go
  (issue #24; mirrors the client's `purge_deleted_resources`).
- **fn gc_line_tombstones** — compact old line tombstones (design §6.4): delete
  lines soft-deleted before the cutoff, then per affected note **read-modify-write
  the order under `SELECT … FOR UPDATE`** so a concurrent collaborative
  `Insert`/`Move` (which rewrites the whole order) cannot be clobbered
  (issue #25): the concurrent order UPDATE blocks until this commits and lands on
  top; a membership drop it did not know about is re-applied by the next GC pass —
  never a lost edit. Only membership changes; the order's version metadata is
  untouched (compaction is not an edit).
- **fn ping** — `SELECT 1` for the readiness probe (issue #36).
- **fn counts** — aggregate `(users, live notes, lines, tombstoned lines)` for
  `/api/metrics`.

### Notes

- **fn create_note** — create the note **and its empty versioned line order** in
  one transaction. `id` may be client-supplied (a daemon keeps its local note id);
  duplicate → `Conflict`. Title stored via `cipher.encrypt`, returned decrypted.
- **fn get_note** — live note by id (`deleted_at IS NULL`); title decrypted.
- **fn list_notes_for_user** — notes visible to the user: owned, shared
  (`note_shares`), or filed in a notebook they own (the folder-owner rule,
  mirroring `permissions::resolve_note_access`), newest first; keyset-paginated on
  `(updated_at, id)`; titles decrypted.
- **fn update_note_meta** — apply a `NotePatch`: `COALESCE`/`CASE` binds so an
  absent field is untouched while an explicit null clears a nullable column;
  bumps `updated_at`; title encrypted on the way in.
- **fn decrypt_note_title** — private helper decrypting an optional read.
- **fn soft_delete_note** — tombstone (sets `deleted_at`, bumps `updated_at`).
- **fn set_note_owner** — ownership transfer (owner-only, enforced at the HTTP
  layer); ownership is separate from grants.

### Note shares

- **fn create_or_update_share** — upsert a grant (bitmask arrives normalised and
  capped from `http.rs`).
- **fn get_share** — one grantee's row (also feeds the access-history cutoff).
- **fn list_shares** — all grants on a note.
- **fn delete_share** — revoke (or self-remove).

### Notebook ownership & shares (Front B stage 1b)

- **fn notebook_owner** — `notebooks.user_id` of a live notebook, else `None`.
- **fn set_notebook_owner** — transfer; the caller re-cascades separately.
- **fn get_notebook_share** / **fn list_notebook_shares** — lookups.
- **fn create_or_update_notebook_share** — upsert the grant **and** run the
  destructive cascade onto every child note, in one transaction.
- **fn delete_notebook_share** — revoke + re-cascade, one transaction.
- **fn cascade_notebook_to_notes** — re-cascade without changing grants (after an
  ownership transfer).
- **fn apply_notebook_shares_to_note** — adopt the notebook's grants onto **one**
  note (the move-into case), destructively.

### Lines

Each line method has a pool-based form and an `_on(executor)` form (issue #45): the
collaborative op batch runs every read/write on the **one connection holding the
note's advisory lock**, so the batch serialises across instances and cannot
deadlock against the bounded pool.

- **fn get_line** / **fn get_line_on** — lookup; content decrypted.
- **fn list_lines** — every line of a note, **tombstones included** (snapshots need
  them); contents decrypted.
- **fn insert_line** / **fn insert_line_on** — insert with vv/writer/timestamp;
  content encrypted in, returned decrypted.
- **fn update_line** / **fn update_line_on** — overwrite content + version
  metadata (an applied `Update`); **also clears `deleted_at`** — a causally newer
  edit revives a tombstone, matching keeplin-core's note semantics.
- **fn soft_delete_line** / **fn soft_delete_line_on** — tombstone (an applied
  `Delete`); the row stays for convergence and remains in the order until GC.

### Line order

- **fn get_note_order** / **fn get_note_order_on** — the order entity
  (`order_json`, `vv`, `last_writer`, `updated_at`).
- **fn set_note_order** / **fn set_note_order_on** — overwrite the order with its
  new merged vv (an applied `Insert`/`Move`).

### Cross-instance bus primitives (issue #45)

- **fn pool** — the pool, so `bus.rs` can open a dedicated `PgListener`.
- **fn notify** — `SELECT pg_notify($1, $2)` (the function form takes the payload
  as a bind; the statement form would need interpolation).
- **fn lock_note_order** — open a transaction and take
  `pg_advisory_xact_lock(hashtextextended(note_id, 0))`; the lock lives until the
  returned transaction commits (caller's batch end) or drops (error → rollback →
  release). Serialises a note's order read-modify-write across instances.
- **fn insert_collab_event** — append an applied op batch to the `collab_events`
  outbox, returning its `seq` (the value NOTIFYed to siblings).
- **fn get_collab_event** — load an outbox row for local delivery.
- **fn prune_collab_events** — the outbox is a delivery buffer, not history;
  aged rows are dropped (maintenance loop, 5-minute TTL).
- **fn upsert_presence** — record/refresh one connection's presence row, keyed
  `(note_id, instance_id, conn_id)`.
- **fn delete_presence** — remove one connection's row (leave/disconnect).
- **fn list_presence** — all rows for a note across instances (caller merges per
  user).
- **fn touch_instance_presence** — heartbeat: bump `updated_at` on all this
  instance's rows so a live instance is never swept.
- **fn sweep_presence** — drop rows not heartbeated since the cutoff (crashed
  instances).
- **fn delete_instance_presence** — remove all of one instance's rows
  (startup/shutdown cleanup).

### Domain-entity materialisation (server = source of truth)

Notebooks, tags, note↔tag associations and resource metadata arrive as `Change`s
over `/api/sync`; the relay materialises them here. Every write resolves against
the stored row with `incoming_wins` under `SELECT … FOR UPDATE`, so concurrent
updates to one entity serialise; each entity id is created on a single device, so
the not-yet-present branch cannot race another creator. All return `bool` =
"the incoming version won and was written".

- **fn upsert_notebook** — create/update if it wins.
- **fn delete_notebook** — tombstone if it wins; an **unknown** notebook gets a
  minimal tombstone row so a later stale create/update cannot resurrect it.
- **fn upsert_tag** / **fn delete_tag** — same pattern for tags.
- **fn upsert_note_tag** — the association is itself versioned and
  soft-deletable: add = `deleted_at NULL`, remove = `deleted_at = updated_at`.
- **fn upsert_resource_meta** — create if it wins; resolution timestamp is
  `deleted_at ?? created_at`, matching keeplin-core (resources carry no
  `updated_at`). The binary is uploaded separately.
- **fn delete_resource** — tombstone if it wins; an unknown resource is a no-op
  (`false`) — a later create arrives with its own vv and resolves normally.
- **fn put_resource_blob** — store/replace the binary (FK requires the metadata).
- **fn get_resource_blob** — fetch the binary.
- **fn resource_owned_by** — does a metadata row exist for this user (authorises
  blob upload/download; resources are per-user, not shareable).

### Domain-entity reads (cold rehydration)

- **fn list_notebooks** / **fn list_tags** / **fn list_resources** — the user's
  live rows, keyset-paginated on `(created_at, id)`.
- **fn list_note_tag_ids** — live tag ids on a note (association present and both
  ends live).

### Per-user quotas

- **fn user_blob_bytes_excluding** — total bytes of the user's **live** resource
  binaries, excluding one resource id (an overwrite is measured by its new size,
  not double-counted). Soft-deleted resources are excluded, so deleting
  attachments actually frees quota (issue #24).
- **fn count_live_notes_for_user** — live owned notes (the `MAX_NOTES_PER_USER`
  check).

---

## fn replace_note_shares_from_notebook_tx

**Identification** — free async fn; marker
`// md:fn replace_note_shares_from_notebook_tx`.

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

Every code block of `store.rs`, in source order. Top-level blocks first; the
`impl Store` methods (each marked `// md:impl Store > fn <name>`) are listed by name
in their file order — all are documented in the *impl Store* section above.

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports | `// md:Overview` |
| 2 | `struct PageCursor` | `// md:PageCursor` |
| 3 | `impl PageCursor` (+ `fn new`/`fn encode`/`fn decode`) | `// md:impl PageCursor` (+ `> fn …`) |
| 4 | `fn token_hash` | `// md:fn token_hash` |
| 5 | `fn split_cursor` | `// md:fn split_cursor` |
| 6 | `struct User` | `// md:User` |
| 7 | `struct Note` | `// md:Note` |
| 8 | `struct NotePatch` | `// md:NotePatch` |
| 9 | `NOTE_COLS` | `// md:NOTE_COLS` |
| 10 | `struct NoteShare` | `// md:NoteShare` |
| 11 | `struct NotebookShare` | `// md:NotebookShare` |
| 12 | `struct Line` | `// md:Line` |
| 13 | `struct NoteOrder` | `// md:NoteOrder` |
| 14 | `struct CollabEvent` | `// md:CollabEvent` |
| 15 | `struct PresenceRow` | `// md:PresenceRow` |
| 16 | `struct UserDevice` | `// md:UserDevice` |
| 17 | `struct ChangeRow` | `// md:ChangeRow` |
| 18 | `enum HistoryKind` | `// md:HistoryKind` |
| 19 | `impl HistoryKind` (+ 3 fns) | `// md:impl HistoryKind` (+ `> fn …`) |
| 20 | `struct EntityVersionRow` | `// md:EntityVersionRow` |
| 21 | `struct Notebook` | `// md:Notebook` |
| 22 | `struct Tag` | `// md:Tag` |
| 23 | `struct ResourceMeta` | `// md:ResourceMeta` |
| 24 | `fn incoming_wins` | `// md:fn incoming_wins` |
| 25 | `struct Store` | `// md:Store` |
| 26 | `impl Store` — methods, in file order: `new`, `with_cipher`, `create_user`, `get_user_by_email`, `get_user_by_id`, `update_password`, `delete_user`, `login_locked`, `record_login_failure`, `clear_login_failures`, `prune_login_attempts`, `create_email_token`, `consume_email_token`, `mark_email_verified`, `prune_email_tokens`, `create_device`, `get_device`, `list_devices_by_user`, `delete_device`, `delete_all_devices`, `touch_device`, `append_changes`, `changes_after`, `entity_history`, `get_cursor`, `advance_cursor`, `prune_delivered_changes`, `purge_deleted_resource_blobs`, `gc_line_tombstones`, `ping`, `counts`, `create_note`, `get_note`, `list_notes_for_user`, `update_note_meta`, `decrypt_note_title`, `soft_delete_note`, `set_note_owner`, `create_or_update_share`, `get_share`, `list_shares`, `delete_share`, `notebook_owner`, `set_notebook_owner`, `get_notebook_share`, `list_notebook_shares`, `create_or_update_notebook_share`, `delete_notebook_share`, `cascade_notebook_to_notes`, `apply_notebook_shares_to_note`, `get_line`, `get_line_on`, `list_lines`, `insert_line`, `insert_line_on`, `update_line`, `update_line_on`, `soft_delete_line`, `soft_delete_line_on`, `get_note_order`, `get_note_order_on`, `set_note_order`, `set_note_order_on`, `pool`, `notify`, `lock_note_order`, `insert_collab_event`, `get_collab_event`, `prune_collab_events`, `upsert_presence`, `delete_presence`, `list_presence`, `touch_instance_presence`, `sweep_presence`, `delete_instance_presence`, `upsert_notebook`, `delete_notebook`, `upsert_tag`, `delete_tag`, `upsert_note_tag`, `upsert_resource_meta`, `delete_resource`, `put_resource_blob`, `get_resource_blob`, `resource_owned_by`, `list_notebooks`, `list_tags`, `list_resources`, `list_note_tag_ids`, `user_blob_bytes_excluding`, `count_live_notes_for_user` | `// md:impl Store` + `// md:impl Store > fn <name>` each |
| 27 | `fn replace_note_shares_from_notebook_tx` | `// md:fn replace_note_shares_from_notebook_tx` |
| 28 | `fn cascade_notebook_to_notes_tx` | `// md:fn cascade_notebook_to_notes_tx` |
