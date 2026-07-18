# `tests/collab.rs` — collaborative channel & hardening tests

Self-contained companion for `crates/keeplin-srv/tests/collab.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand the suite without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context** (compressed for
straightforward tests).

---

## Overview

**Identification** — file-level block: the imports and the `Ws` type alias. Marker
`// md:Overview`.

**What it does** — End-to-end tests of the `/api/ws` collaborative protocol and the
production-hardening surfaces, driven over a **real WebSocket and real HTTP** against
a real server on a throwaway `#[sqlx::test]` PostgreSQL database. No mocking: the
tests register users, log devices in, open raw `tokio-tungstenite` sockets, and send
hand-built protocol frames (deliberately not importing `protocol.rs`, so a wire-shape
drift breaks these tests). Ops are signed with the login's `device_id` — the vv
actor. Also covers the capability model (Front B), the notebook cascade, the
folder-owner rule, the inbox nil-UUID mapping, probes, metrics auth and the rate
limiter.

**Dependencies** — `tokio_tungstenite`, `futures_util`, `keeplin_srv` (`Config`,
`router`, `AppState`, `bus::spawn`, `Store` via `state.store`), keeplin-core models
(notebook fixtures), `reqwest`, `sqlx`, `serde_json`, `uuid`.

**Used by** — `cargo test`; CI.

**Repeated context** — Protocol contract exercised here: `Join` → `Welcome` (full
snapshot: versioned order + all lines) then `Presence`; `Op` batches are validated
(writer identity, limits, existence), resolved by version vector with the
`(timestamp, writer)` LWW tiebreak, persisted, and fanned out with a monotonic
`server_seq`; errors are per-frame (`Error{code}`) and never close the connection.

---

## Helpers

Compressed entries; each block carries its own marker:

### fn test_config
Marker `// md:fn test_config`. The standard test `Config` literal.

### fn spawn_server
Marker `// md:fn spawn_server`. `spawn_server_with_state(pool).await.0` — router
only, no bus.

### fn spawn_instance
Marker `// md:fn spawn_instance`. A **bus-enabled** instance (issue #45) — only the
cross-instance test uses it, so the other tests avoid holding a permanent
`PgListener` connection.

### fn spawn_server_with_state
Marker `// md:fn spawn_server_with_state`. Like `spawn_server` but also returns the
`Arc<AppState>`, for tests that poke the store directly (tombstone GC, notebook
fixtures).

### fn user
Marker `// md:fn user`. Register + login; returns `(user_id, device_id, token)` —
ops must be signed with the **device** id.

### fn create_note
Marker `// md:fn create_note`. `POST /api/notes`, returns the note id.

### fn share
Marker `// md:fn share`. Grant by email with test roles mapped to capability bits:
`editor` = READ|WRITE (3), `viewer` = READ (1).

### fn ws_connect
Marker `// md:fn ws_connect`. Raw WS to `ws://…/api/ws?token=…`.

### fn send
Marker `// md:fn send`. Send one JSON frame.

### fn recv_until
Marker `// md:fn recv_until`. Receive frames until a predicate matches (skipping
presence chatter and other noise), panicking after a bounded wait — the suite's
convergence primitive on the socket side.

### fn join / fn insert_op / fn update_op
Markers `// md:fn join`, `// md:fn insert_op`, `// md:fn update_op`. Frame builders
for `Join` and single-op `Insert`/`Update` envelopes (vv, writer, timestamp
explicit).

### fn export_body
Marker `// md:fn export_body`. `GET /api/notes/:id/export` → the materialised body.

### fn wait_export
Marker `// md:fn wait_export`. Poll the export until it equals the expected body
(~5 s), panicking with the last seen value — ops apply asynchronously to the HTTP
surface, and fixed sleeps are exactly what flakes on slow CI runners.

### T1 / T2 / T3
Marker `// md:Timestamps`. Three fixed, ordered RFC3339 timestamps used as
deterministic op times.

---

## Protocol tests

### fn join_receives_welcome_snapshot
Marker `// md:fn join_receives_welcome_snapshot`. Joining an empty note yields
`Welcome` (empty order/lines) and then a `Presence` list containing yourself.

### fn ops_propagate_between_participants
Marker `// md:fn ops_propagate_between_participants`. A inserts (B receives the
`Op` with A's **user** id and a `server_seq ≥ 1`); B updates having seen A's write
(vv covering both device components); A receives it; the exported body reflects the
final state.

### fn ops_and_presence_propagate_across_instances
Marker `// md:fn ops_and_presence_propagate_across_instances`. Issue #45: two
bus-enabled instances over one database; A on instance A, B on instance B. Presence
**merges** across replicas (A eventually sees 2 users); ops flow both directions
via the outbox + NOTIFY; both replicas converge on the same materialised body.

### fn concurrent_updates_resolve_deterministically
Marker `// md:fn concurrent_updates_resolve_deterministically`. Both edit the same
line from the same base (`{A:1}`): neither vector dominates, so the deterministic
`(timestamp, writer)` tiebreak decides — B's later-stamped edit wins on every
replica regardless of processing order.

### fn stale_op_is_ignored
Marker `// md:fn stale_op_is_ignored`. A replay carrying the same vv (writer
component does not advance) can never win; converging to "v2" proves the genuine
update applied and the replay was dropped — idempotent application.

### fn move_reorders_lines
Marker `// md:fn move_reorders_lines`. Three inserts then a `Move` of the last line
to the front (`after_line_id: null`); the export shows the reordered body.

---

## Permission tests

### fn viewer_can_watch_but_not_edit
Marker `// md:fn viewer_can_watch_but_not_edit`. A `viewer` (READ) joins fine but
its `Op` gets `Error{forbidden}` and the body stays empty — the collaborative
channel enforces the same `can_write` gate as REST.

### fn revoking_a_share_stops_edits_mid_session
Marker `// md:fn revoking_a_share_stops_edits_mid_session`. Issue #30: B edits
while shared; A revokes the share while B **stays connected**; B's next edit gets
`Error{forbidden}` immediately — access is re-resolved per op batch, never cached
for the connection's life.

### fn outsider_cannot_join
Marker `// md:fn outsider_cannot_join`. A non-shared user's `Join` gets
`Error{forbidden}`.

### fn presence_shows_other_participants
Marker `// md:fn presence_shows_other_participants`. Two joined users: A sees a
presence list with both display names; B's `Cursor` frame shows up attached to B's
entry in A's next presence broadcast (presence is user-scoped; lists are full
replacements).

### fn import_then_export_roundtrip
Marker `// md:fn import_then_export_roundtrip`. Import a 4-line flat body →
export returns it byte-identical; the plain `GET /api/notes/:id` carries the same
materialised body (design §3.4); a `Join` snapshot shows the 4 lines/order entries.

### fn forged_writer_is_rejected
Marker `// md:fn forged_writer_is_rejected`. B signs an op with **A's** device id →
`Error{bad_writer}`: `last_writer` must equal the sender's authenticated device
(clients cannot forge edits in someone else's name).

---

## Hardening tests

### fn ws_accepts_authorization_header
Marker `// md:fn ws_accepts_authorization_header`. Connecting with the token only
in the `Authorization: Bearer` header (no `?token=`) works — the preferred,
log-safe form.

### fn deleting_a_device_revokes_its_token
Marker `// md:fn deleting_a_device_revokes_its_token`. A second device's token
works until the device is deleted from the first device; then REST answers 401 —
revocation-by-deletion on the REST surface.

### fn deleting_a_device_revokes_its_collab_token
Marker `// md:fn deleting_a_device_revokes_its_collab_token`. Issue #20: the same
revocation on `/api/ws` — the revoked device's token, which connected fine before,
is rejected at the WS handshake afterwards.

### fn gc_compacts_old_tombstones
Marker `// md:fn gc_compacts_old_tombstones`. Two lines, one deleted with a
months-old tombstone. The test polls the **store's line set** (2 lines, exactly 1
tombstoned) rather than the exported body before running GC — the body reads
"viva" both before line 2 exists and after the delete, so polling it could race GC
against the not-yet-landed tombstone. `gc_line_tombstones(30 days)` reclaims
exactly 1; the body is unchanged and the id is gone from both `lines` and the
order.

### fn version_endpoint_advertises_capabilities
Marker `// md:fn version_endpoint_advertises_capabilities`. `GET /version`
(unauthenticated): name, `protocol_version ≥ 1`, and the `history` capability
present (issues #39/#114).

### fn health_and_readiness_probes
Marker `// md:fn health_and_readiness_probes`. `/health` → `200 ok` (liveness
stub); `/ready` → `200 ready` with the database up (real DB round-trip,
issue #36). Both unauthenticated, never rate-limited.

### fn metrics_reports_counts
Marker `// md:fn metrics_reports_counts`. Issue #22: anonymous `/api/metrics` →
401; authenticated → correct `users`/`notes` counts plus the live collab gauges.

### fn spawn_rate_limited
Marker `// md:fn spawn_rate_limited`. Helper: a server with
`rate_limit_per_min = N`.

### fn rate_limit_throttles_and_spares_health
Marker `// md:fn rate_limit_throttles_and_spares_health`. Budget 10/min: hammering
an authenticated route yields some 200s then a 429 (the limiter short-circuits
before the handler); `/health` never throttles (orchestrator probes must always
pass).

---

## Capability-model tests (Front B)

### fn share_caps / fn note_status
Markers `// md:fn share_caps`, `// md:fn note_status`. Helpers: grant with an
explicit capability bitmask (returning the status), and GET/PATCH/DELETE a note
returning the status.

### fn capability_grants_enforce_hierarchy_and_escalation
Marker `// md:fn capability_grants_enforce_hierarchy_and_escalation`. READ-only B:
can GET, cannot PATCH, cannot share (no `share_write`). Upgraded to `SHARE_WRITE`
(normalises to read|write|share_read|share_write = 15, **not** manage): B can grant
C read+write (within its own caps) but granting `MANAGE` → 403 — **no privilege
escalation**: a grant is capped to the granter's own capabilities.

### fn ownership_transfer_moves_delete_rights
Marker `// md:fn ownership_transfer_moves_delete_rights`. After
`POST …/transfer` to B: A (no implicit residual access) cannot DELETE (403); B, the
new owner, can (200) — delete/transfer are owner-only powers that no capability bit
confers.

### fn move_note
Marker `// md:fn move_note`. Helper: PATCH `notebook_id`, asserting 200.

### fn notebook_share_cascades_to_child_notes
Marker `// md:fn notebook_share_cascades_to_child_notes`. A materialised notebook
(seeded via `store.upsert_notebook`) with a note moved in: B has no access; sharing
the **notebook** (read) cascades read onto the child note (GET 200, PATCH still
403); revoking the notebook share re-cascades and B loses access — the destructive
cascade in both directions.

### fn notebook_share_caps / fn move_note_status
Markers `// md:fn notebook_share_caps`, `// md:fn move_note_status`. Helpers
returning status codes for notebook grants and note moves.

### fn note_move_requires_write_on_destination_notebook
Marker `// md:fn note_move_requires_write_on_destination_notebook`. Issue #13:
moving B's note into A's notebook is 403 with no destination access **and** with
only read; write on the destination allows it (200); an unknown destination is 404
— consent on both sides, because the move adopts the destination's grants
(disclosure + share replacement).

### fn notebook_owner_can_manage_child_notes_they_do_not_own
Marker `// md:fn notebook_owner_can_manage_child_notes_they_do_not_own`.
Issue #15 (folder-owner model): B's note filed in A's notebook — A holds no
`note_shares` row (the cascade copies only `notebook_shares`), yet as notebook
owner A can GET/PATCH the child note and sees it in `GET /api/notes`; but DELETE
stays with B (403 for A, 200 for B) — implicit `manage`, never ownership.

### fn nil_notebook_id_patch_means_inbox_and_keeps_shares
Marker `// md:fn nil_notebook_id_patch_means_inbox_and_keeps_shares`.
keeplin-core models the inbox as the nil UUID; the server as `NULL`. A PATCH with
the nil UUID is a move **to the inbox**: 200 (not a 404 destination check),
stored `notebook_id` is null, and **no destructive cascade ran** — the
collaborator's share survives.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `spawn_server()`, `spawn_instance()`, `spawn_server_with_state()`, `spawn_rate_limited()` — defined here (EXTRACTED)
- the helper fns and every test fn — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×1; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×3; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports + `type Ws` | `// md:Overview` |
| 2 | `fn test_config` | `// md:fn test_config` |
| 3 | `fn spawn_server` | `// md:fn spawn_server` |
| 4 | `fn spawn_instance` | `// md:fn spawn_instance` |
| 5 | `fn spawn_server_with_state` | `// md:fn spawn_server_with_state` |
| 6 | `fn user` | `// md:fn user` |
| 7 | `fn create_note` | `// md:fn create_note` |
| 8 | `fn share` | `// md:fn share` |
| 9 | `fn ws_connect` | `// md:fn ws_connect` |
| 10 | `fn send` | `// md:fn send` |
| 11 | `fn recv_until` | `// md:fn recv_until` |
| 12 | `fn join` | `// md:fn join` |
| 13 | `fn insert_op` | `// md:fn insert_op` |
| 14 | `fn update_op` | `// md:fn update_op` |
| 15 | `fn export_body` | `// md:fn export_body` |
| 16 | `fn wait_export` | `// md:fn wait_export` |
| 17 | `T1`/`T2`/`T3` | `// md:Timestamps` |
| 18 | `fn join_receives_welcome_snapshot` | `// md:fn join_receives_welcome_snapshot` |
| 19 | `fn ops_propagate_between_participants` | `// md:fn ops_propagate_between_participants` |
| 20 | `fn ops_and_presence_propagate_across_instances` | `// md:fn ops_and_presence_propagate_across_instances` |
| 21 | `fn concurrent_updates_resolve_deterministically` | `// md:fn concurrent_updates_resolve_deterministically` |
| 22 | `fn stale_op_is_ignored` | `// md:fn stale_op_is_ignored` |
| 23 | `fn move_reorders_lines` | `// md:fn move_reorders_lines` |
| 24 | `fn viewer_can_watch_but_not_edit` | `// md:fn viewer_can_watch_but_not_edit` |
| 25 | `fn revoking_a_share_stops_edits_mid_session` | `// md:fn revoking_a_share_stops_edits_mid_session` |
| 26 | `fn outsider_cannot_join` | `// md:fn outsider_cannot_join` |
| 27 | `fn presence_shows_other_participants` | `// md:fn presence_shows_other_participants` |
| 28 | `fn import_then_export_roundtrip` | `// md:fn import_then_export_roundtrip` |
| 29 | `fn forged_writer_is_rejected` | `// md:fn forged_writer_is_rejected` |
| 30 | `fn ws_accepts_authorization_header` | `// md:fn ws_accepts_authorization_header` |
| 31 | `fn deleting_a_device_revokes_its_token` | `// md:fn deleting_a_device_revokes_its_token` |
| 32 | `fn deleting_a_device_revokes_its_collab_token` | `// md:fn deleting_a_device_revokes_its_collab_token` |
| 33 | `fn gc_compacts_old_tombstones` | `// md:fn gc_compacts_old_tombstones` |
| 34 | `fn version_endpoint_advertises_capabilities` | `// md:fn version_endpoint_advertises_capabilities` |
| 35 | `fn health_and_readiness_probes` | `// md:fn health_and_readiness_probes` |
| 36 | `fn metrics_reports_counts` | `// md:fn metrics_reports_counts` |
| 37 | `fn spawn_rate_limited` | `// md:fn spawn_rate_limited` |
| 38 | `fn rate_limit_throttles_and_spares_health` | `// md:fn rate_limit_throttles_and_spares_health` |
| 39 | `fn share_caps` | `// md:fn share_caps` |
| 40 | `fn note_status` | `// md:fn note_status` |
| 41 | `fn capability_grants_enforce_hierarchy_and_escalation` | `// md:fn capability_grants_enforce_hierarchy_and_escalation` |
| 42 | `fn ownership_transfer_moves_delete_rights` | `// md:fn ownership_transfer_moves_delete_rights` |
| 43 | `fn move_note` | `// md:fn move_note` |
| 44 | `fn notebook_share_cascades_to_child_notes` | `// md:fn notebook_share_cascades_to_child_notes` |
| 45 | `fn notebook_share_caps` | `// md:fn notebook_share_caps` |
| 46 | `fn move_note_status` | `// md:fn move_note_status` |
| 47 | `fn note_move_requires_write_on_destination_notebook` | `// md:fn note_move_requires_write_on_destination_notebook` |
| 48 | `fn notebook_owner_can_manage_child_notes_they_do_not_own` | `// md:fn notebook_owner_can_manage_child_notes_they_do_not_own` |
| 49 | `fn nil_notebook_id_patch_means_inbox_and_keeps_shares` | `// md:fn nil_notebook_id_patch_means_inbox_and_keeps_shares` |
