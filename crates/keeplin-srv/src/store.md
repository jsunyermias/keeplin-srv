# `store.rs` — the PostgreSQL data-access layer

## Purpose

The single place any SQL lives. `Store` wraps a `sqlx::PgPool` and exposes typed async methods
for every entity: users, devices, the collaborative note model (notes, lines, line order,
shares), and the relay journal (changes, delivery cursors), plus maintenance queries. Handlers
and the two WebSocket engines call `Store`; nothing else touches the database.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `Store` | struct | holds the `PgPool`; all methods live here |
| `User`, `UserDevice`, `Note`, `NoteShare` | struct (`FromRow`) | row mappings |
| `Line`, `NoteOrder` | struct | a collaborative line and a note's versioned line order |
| `NotePatch` | struct | partial note-metadata update (absent = unchanged, `Some(None)` = clear) |
| `ChangeRow` | struct | one relay-journal row fetched for delivery |
| `Notebook`, `Tag`, `ResourceMeta` | struct (`FromRow`) | REST row mappings for the materialised domain entities |
| `incoming_wins` (fn) | free fn | wraps keeplin-core's `note_log::resolve` so materialisation picks the same winner as clients |

## Public API (by area)

**Users**: `create_user`, `get_user_by_email`, `get_user_by_id`.
**Devices**: `create_device`, `get_device`, `list_devices_by_user`, `delete_device` (revokes a
token), `touch_device` (last-seen).
**Relay journal**: `append_changes` (dedupes per-user by `(user_id, batch_id, batch_index)` — issue #26), `changes_after`,
`get_cursor`, `advance_cursor`, `prune_delivered_changes` (only devices that have connected — i.e. have a delivery cursor — block pruning; a never-connected device does not, issue #23).
**Notes**: `create_note` (optional client id → `Conflict` on dup), `get_note`,
`list_notes_for_user` (owned + shared + filed in a notebook the user owns), `update_note_meta`, `soft_delete_note`.
**Shares** (capability bitset, `permissions.md`): `create_or_update_share`, `get_share`, `list_shares`, `delete_share`, `set_note_owner`.
**History** (Front D stage 2; per-entity, issue #27):
`entity_history(HistoryKind, id, limit, not_before, authored_not_before, user_scope)`
reads an entity's past versions newest first (`seq DESC`), matching note/notebook `Change`
payloads by their `op` tag and snapshot id; only the envelope is inspected — snapshots stay
opaque. Two independent lower bounds: `not_before` filters on the journal row's
`received_at` (retention age); `authored_not_before` filters on the **payload's own causal
timestamp** (snapshot `updated_at`, or top-level `deleted_at` for tombstones, via the
`keeplin_try_timestamptz` safe cast from migration 0013, `COALESCE`d to `received_at` for
legacy payloads) — this is the `HISTORY_VISIBILITY=access` collaborator window, and it must
**never** be switched back to `received_at`: journal re-delivery (a reinstalled client
re-pushing from epoch) mints fresh `received_at` values for pre-access content and would
leak it (honest-client boundary; a forged `updated_at` can still cheat — SECURITY.md).
`user_scope = None` reads across **all** users (per-entity history for a shared,
server-materialised entity — the HTTP handler authorises read access first); `Some(user)`
restricts to one account (a relay-only entity with no server owner/share model). Returns
`EntityVersionRow { timestamp, device_id, entity? }` (`entity` `None` = tombstone).

**Notebook permissions**: `notebook_owner`, `set_notebook_owner`, `create_or_update_notebook_share`/`get_notebook_share`/`list_notebook_shares`/`delete_notebook_share`, and the destructive cascade (`cascade_notebook_to_notes`, `apply_notebook_shares_to_note`) that replaces child notes' `note_shares` with the notebook's grants on a notebook-perm change or a note move.
**Lines**: `get_line`, `list_lines`, `insert_line`, `update_line`, `soft_delete_line`.
**Line order**: `get_note_order`, `set_note_order`.
**Domain entities** (materialised from the relay, server = truth): `upsert_notebook` / `delete_notebook`,
`upsert_tag` / `delete_tag`, `upsert_note_tag` (add/remove), `upsert_resource_meta` / `delete_resource`,
`put_resource_blob` / `get_resource_blob` / `resource_owned_by`, and the reads `list_notebooks`,
`list_tags`, `list_resources`, `list_note_tag_ids`. Each write resolves via `incoming_wins` under a
`SELECT … FOR UPDATE` lock. The list reads (and `list_notes_for_user`) take `(limit, cursor)` for
keyset pagination (`PageCursor` on `(created_at, id)`, or `(updated_at, id)` for notes); `None` limit
returns every row (issue #29).
**Quotas**: `user_blob_bytes_excluding` (total **live** blob bytes minus one resource — soft-deleted resources do not count, so deleting frees quota, issue #24), `count_live_notes_for_user`.
**Maintenance / metrics**: `gc_line_tombstones` (reads-modifies-writes each note's order under `SELECT … FOR UPDATE` so a concurrent collaborative order write is not clobbered, issue #25), `purge_deleted_resource_blobs` (reclaims blob bytes of long-deleted resources; metadata tombstone kept, issue #24), `counts`.

## Database schema

Owned by the SQL migrations, documented in `migrations/*.md`:

- `users`, `user_devices` (0001) — accounts and device logins.
- `changes`, `device_cursors` (0001) — the relay journal and per-device watermarks.
- `notes`, `lines`, `note_line_order`, `note_shares` (0002) — the collaborative note model.
- `notes.notebook_id` + to-do columns (0003) — full note metadata the server stores.
- `notebooks`, `tags`, `note_tags`, `resources`, `resource_blobs` (0004) — the domain entities the
  server materialises from the relay so it is their source of truth.

## Design notes

- **Resolution**: for the collaborative line/order rows the store is mechanism-free — `vv` columns
  are `JSONB` it reads/writes, and `collab.rs` decides who wins. For the domain entities materialised
  from the relay (notebooks/tags/associations/resources) the store *does* resolve, via `incoming_wins`
  (a thin wrapper over keeplin-core's `note_log::resolve`), under a `SELECT … FOR UPDATE` lock so
  concurrent updates to one entity serialise. Each such id is created on a single device, so the
  not-yet-present branch cannot race another creator.
- `create_note` accepts a client-supplied id so a daemon uploading a local note keeps the same
  id; a duplicate maps to `AppError::Conflict` via the unique-violation branch.
- `update_note_meta` uses `COALESCE`/`CASE` so an absent field is untouched while an explicit
  null clears a nullable column — the semantics `NotePatch` encodes.
- `gc_line_tombstones` deletes long-dead lines **and** drops their ids from each note's
  `order_json`, leaving the order's version metadata untouched (compaction is not an edit).

## Graph context

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

**Invariants** (restated on purpose; a change to this file must keep these true)

- `Store` is the only module that touches SQL and the only place the at-rest `Cipher` encrypts/decrypts (`notes.title`, `lines.content`) — no handler reads or writes those columns directly.
- Conflict resolution is version vectors (`note_log::resolve` semantics) everywhere; a dominated write is ignored, concurrency falls to the deterministic `(updated_at, last_writer)` tiebreak.
- Deletes are soft (tombstones kept for convergence); hard reclamation happens only in the explicit GC/purge passes.
- `entity_history`'s access window (`authored_not_before`) filters on the payload's own `updated_at`/`deleted_at` via `keeplin_try_timestamptz`, never on `received_at`.
- Journal pruning only removes rows already delivered to every connected device and older than the retention window; materialised tables are the source of truth.

## Related files

- `migrations/*.md` — the schema these queries assume.
- `collab.md` — the version-vector resolution layered on the line/order rows.
- `sync.md` — the relay journal consumer.
