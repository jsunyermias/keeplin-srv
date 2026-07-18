# `tests/materialize.rs` — domain-entity materialisation tests

Self-contained companion for `crates/keeplin-srv/tests/materialize.rs`. It documents
**every code block of the source file, in source order** — a reader with only this file
must be able to understand the suite without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**What it does** — End-to-end tests of the server materialising the keeplin-core
domain entities (notebooks, tags, note↔tag associations, resource metadata +
binaries) that arrive over the `/api/sync` relay, driven by the **real relay client**
(keeplin-core's `DbBackend`) against a real server on a throwaway `#[sqlx::test]`
PostgreSQL database — the "server is the truth, client DB is a cache" model. Plus
store-level tests of deterministic vv convergence, pruning survival, per-user batch
dedup (issue #26), phantom-device pruning (issue #23) and quota/purge hygiene
(issue #24).

Coverage note: this suite drives the **relay-mode** client (`DbBackend` alone),
whose `ResourceCreate` still carries the binary inline — deliberately exercising the
server's backward-compat path. The collab-mode client uploads out-of-band and strips
`data` from the relayed change; that path is covered by
`tests/collab_client_resources_e2e.rs`.

**Dependencies** — keeplin-core (`DbBackend`, models, repository/sync traits,
`VersionVector`), `keeplin_srv` (`Config`, `router`, `AppState`, `Store`),
`reqwest`, `sqlx`, `tempfile`, `chrono`, `serde_json`, `uuid`.

**Used by** — `cargo test`; CI.

**Repeated context** — Materialisation model, restated: `sync.rs::materialize`
parses each relayed `Change`, resolves it by version vector against the stored row
(`store.rs::incoming_wins` = keeplin-core's `note_log::resolve`) under
`SELECT … FOR UPDATE`, and upserts — so the server converges to the same winner as
every client, and the materialised tables (not the journal) are the durable truth.

---

## fn test_config

**Identification** — helper; marker `// md:fn test_config`. The standard test
`Config` literal (open registration, everything optional off).

**Dependencies** — `Config`. **Used by** — `spawn_server`. **Repeated context** —
config literals keep the environment out of tests.

---

## fn spawn_server

**Identification** — helper; marker `// md:fn spawn_server`. Boots the real router
on an ephemeral loopback port with `ConnectInfo`, on a spawned task.

**Dependencies** — `AppState::new`, `router`. **Used by** — the HTTP-level tests.
**Repeated context** — none.

---

## fn register

**Identification** — helper; marker `// md:fn register`. REST registration over real
HTTP (asserts 200). **Dependencies** — `reqwest`. **Used by** — the HTTP-level
tests. **Repeated context** — none.

## fn login

**Identification** — helper; marker `// md:fn login`. REST login returning the
device token. **Dependencies** — `reqwest`. **Used by** — the HTTP-level tests.
**Repeated context** — none.

---

## fn device

**Identification** — helper; marker `// md:fn device`. A real relay client
(`DbBackend`) on a leaked temp SQLite file, connected to `ws://…/api/sync`.

**Dependencies** — keeplin-core, `tempfile`. **Used by** — the relay-driven tests.
**Repeated context** — relay-mode (no collab wrapper): `ResourceCreate` carries the
binary inline — the backward-compat path this suite covers on purpose.

---

## fn epoch

**Identification** — helper; marker `// md:fn epoch`. The Unix-epoch timestamp used
as the "everything" lower bound for `get_changes_since`.

**Dependencies** — chrono. **Used by** — `push`. **Repeated context** — none.

---

## fn push

**Identification** — helper; marker `// md:fn push`. Sends every local change of a
device to the relay (`get_changes_since(epoch)` → `send_changes`) and sleeps 200 ms
to give the server a moment to materialise the batch.

**Dependencies** — keeplin-core sync API. **Used by** — the relay-driven tests.

**Repeated context** — The sleep is a *convenience*, not a guarantee: assertions
that need a specific materialised artefact (notably the resource-blob tests) poll
with a bounded retry on top of it, because under a busy CI database materialisation
can exceed the grace period.

---

## fn get_json

**Identification** — helper; marker `// md:fn get_json`. Authenticated GET
returning parsed JSON.

**Dependencies** — `reqwest`. **Used by** — the HTTP-level tests.
**Repeated context** — none.

---

## fn notebook_materialises_and_is_served

**Identification** — `#[sqlx::test]`; marker
`// md:fn notebook_materialises_and_is_served`.

**What it does** — Create a notebook through the real client, push; `GET
/api/notebooks` lists exactly it (id + title).

**Dependencies / Used by** — the helpers; `cargo test`.

**Repeated context** — The REST read side serves the materialised table for cold
rehydration.

---

## fn tag_and_association_materialise

**Identification** — `#[sqlx::test]`; marker
`// md:fn tag_and_association_materialise`.

**What it does** — Create note + tag + association, push; `GET /api/tags` lists the
tag and `GET /api/notes/:id/tags` returns the tag id.

**Dependencies / Used by** — the helpers; `cargo test`. **Repeated context** — the
association is itself a versioned entity (`note_tags`).

---

## fn removing_a_tag_association_tombstones_it

**Identification** — `#[sqlx::test]`; marker
`// md:fn removing_a_tag_association_tombstones_it`.

**What it does** — After `remove_note_tag` + push, `…/tags` is empty — the
association was tombstoned (soft-delete), not deleted.

**Dependencies / Used by** — the helpers; `cargo test`. **Repeated context** —
soft-delete keeps the row competing in resolution; the read filters live rows.

---

## fn resource_metadata_and_blob_materialise

**Identification** — `#[sqlx::test]`; marker
`// md:fn resource_metadata_and_blob_materialise`.

**What it does** — Create a resource whose binary travels **inside** the
`ResourceCreate` (relay-mode client, backward-compat path), push. Asserts the
metadata is listed, then **polls** `GET /api/resources/:id/data` (bounded, ~10 s)
until it returns the exact bytes — polling because metadata upsert and blob write
land in sequence during async materialisation, and a fixed post-push sleep is not a
guarantee under CI load.

**Dependencies / Used by** — the helpers; `cargo test`.

**Repeated context** — Backward compatibility: `sync.rs::materialize` stores an
inline `data` payload to `resource_blobs` only when the metadata upsert reports the
incoming version won.

---

## fn streaming_blob_upload_then_download

**Identification** — `#[sqlx::test]`; marker
`// md:fn streaming_blob_upload_then_download`.

**What it does** — The Option B (out-of-band) path against relay-created metadata:
after push, **poll** `PUT /api/resources/:id/data` (bounded) until it answers 200 —
the PUT 404s while the metadata is still materialising — then `GET` returns exactly
the replaced 4 KiB.

**Dependencies / Used by** — the helpers; `cargo test`.

**Repeated context** — The 404-until-materialised behaviour is the same contract
the real collab client handles with its own upload retry
(`collab_client_resources_e2e.rs`).

---

## fn uploading_to_unknown_resource_is_rejected

**Identification** — `#[sqlx::test]`; marker
`// md:fn uploading_to_unknown_resource_is_rejected`.

**What it does** — `PUT …/data` for a random id → `404`: no metadata, no upload.

**Dependencies / Used by** — the helpers; `cargo test`. **Repeated context** — the
metadata row is the authorisation anchor for blob writes.

---

## fn deleting_a_notebook_removes_it_from_listings

**Identification** — `#[sqlx::test]`; marker
`// md:fn deleting_a_notebook_removes_it_from_listings`.

**What it does** — Delete a materialised notebook, push; `GET /api/notebooks` is
empty (tombstoned, filtered from live listings).

**Dependencies / Used by** — the helpers; `cargo test`. **Repeated context** —
soft-delete + live-row reads.

---

## fn users_do_not_see_each_others_entities

**Identification** — `#[sqlx::test]`; marker
`// md:fn users_do_not_see_each_others_entities`.

**What it does** — User A materialises a notebook; user B's listing stays empty —
per-user isolation of the materialised entities.

**Dependencies / Used by** — the helpers; `cargo test`. **Repeated context** — all
durable data is user-scoped; sharing is explicit and note/notebook-level only.

---

## fn concurrent_notebook_edits_converge_deterministically

**Identification** — `#[sqlx::test]` (store-level); marker
`// md:fn concurrent_notebook_edits_converge_deterministically`.

**What it does** — Two concurrent edits to one notebook id (neither vv dominates;
B has the later timestamp): applied in either order, B wins — and in the reverse
order the stale A-write reports "not written" (`upsert_notebook` → `false`).
Deterministic, order-independent convergence at the store level.

**Dependencies** — `Store::{create_user, upsert_notebook, list_notebooks}`.
**Used by** — `cargo test`.

**Repeated context** — Pins `incoming_wins` = vv dominance + `(timestamp, writer)`
LWW tiebreak, identical to every client.

---

## fn materialised_entities_survive_journal_pruning

**Identification** — `#[sqlx::test]`; marker
`// md:fn materialised_entities_survive_journal_pruning`.

**What it does** — Materialise a notebook, then simulate full delivery (advance
every device cursor to max seq) and prune the **entire** journal
(`prune_delivered_changes` with a future cutoff): rows go, journal is empty, and
`GET /api/notebooks` still serves the notebook — the materialised table, not the
journal, is the truth. This is the safety argument behind pruning (issue #23).

**Dependencies** — the helpers + `Store::{advance_cursor,
prune_delivered_changes}`, raw sqlx. **Used by** — `cargo test`.

**Repeated context** — Journal = delivery buffer + history window; materialised
tables = state. Cold rehydration reads the tables over REST.

---

## fn same_batch_id_across_users_is_not_deduplicated

**Identification** — `#[sqlx::test]` (store-level); marker
`// md:fn same_batch_id_across_users_is_not_deduplicated`.

**What it does** — The same client `batch_id` used by two different users is NOT
deduplicated across accounts (issue #26 — dedup is per user:
`UNIQUE (user_id, batch_id, batch_index)`), while a user's own retry of the same
batch still dedupes to empty.

**Dependencies** — `Store::{create_user, create_device, append_changes}`.
**Used by** — `cargo test`.

**Repeated context** — Pins the issue #26 fix: a cross-user batch-id collision (or
a malicious guess) can no longer suppress another account's changes.

---

## fn a_never_connected_device_does_not_block_pruning

**Identification** — `#[sqlx::test]` (store-level); marker
`// md:fn a_never_connected_device_does_not_block_pruning`.

**What it does** — One connected device (cursor advanced) plus one phantom device
that never connected (no cursor row): pruning still reclaims the delivered rows —
the phantom does not hold the journal hostage (issue #23).

**Dependencies** — `Store` journal/cursor methods. **Used by** — `cargo test`.

**Repeated context** — Only devices **with a cursor row** participate in the
pruning minimum; a fresh device cold-rehydrates from REST + snapshots rather than
replaying from seq 0.

---

## fn deleted_resource_frees_quota_and_blob_is_purgeable

**Identification** — `#[sqlx::test]` (store-level); marker
`// md:fn deleted_resource_frees_quota_and_blob_is_purgeable`.

**What it does** — A live 3-byte resource counts 3 against quota; after a
dominating soft-delete it counts 0 (deleting frees quota), and
`purge_deleted_resource_blobs` reclaims the blob while the metadata tombstone
stays (issue #24).

**Dependencies** — `Store` resource/quota/purge methods. **Used by** —
`cargo test`.

**Repeated context** — Blob bytes are reclaimable; convergence metadata is not —
the tombstone must keep competing in resolution.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `test_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `spawn_server()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `register()` — defined here (EXTRACTED; file-local)
- `login()` — defined here (EXTRACTED; file-local)
- `device()` — defined here (EXTRACTED; file-local)
- `epoch()` — defined here (EXTRACTED; file-local)
- `push()` — defined here (EXTRACTED; file-local)
- `get_json()` — defined here (EXTRACTED; file-local)
- `notebook_materialises_and_is_served()` — defined here (EXTRACTED; file-local)
- `tag_and_association_materialise()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×1; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports | `// md:Overview` |
| 2 | `fn test_config` | `// md:fn test_config` |
| 3 | `fn spawn_server` | `// md:fn spawn_server` |
| 4 | `fn register` | `// md:fn register` |
| 5 | `fn login` | `// md:fn login` |
| 6 | `fn device` | `// md:fn device` |
| 7 | `fn epoch` | `// md:fn epoch` |
| 8 | `fn push` | `// md:fn push` |
| 9 | `fn get_json` | `// md:fn get_json` |
| 10 | `fn notebook_materialises_and_is_served` | `// md:fn notebook_materialises_and_is_served` |
| 11 | `fn tag_and_association_materialise` | `// md:fn tag_and_association_materialise` |
| 12 | `fn removing_a_tag_association_tombstones_it` | `// md:fn removing_a_tag_association_tombstones_it` |
| 13 | `fn resource_metadata_and_blob_materialise` | `// md:fn resource_metadata_and_blob_materialise` |
| 14 | `fn streaming_blob_upload_then_download` | `// md:fn streaming_blob_upload_then_download` |
| 15 | `fn uploading_to_unknown_resource_is_rejected` | `// md:fn uploading_to_unknown_resource_is_rejected` |
| 16 | `fn deleting_a_notebook_removes_it_from_listings` | `// md:fn deleting_a_notebook_removes_it_from_listings` |
| 17 | `fn users_do_not_see_each_others_entities` | `// md:fn users_do_not_see_each_others_entities` |
| 18 | `fn concurrent_notebook_edits_converge_deterministically` | `// md:fn concurrent_notebook_edits_converge_deterministically` |
| 19 | `fn materialised_entities_survive_journal_pruning` | `// md:fn materialised_entities_survive_journal_pruning` |
| 20 | `fn same_batch_id_across_users_is_not_deduplicated` | `// md:fn same_batch_id_across_users_is_not_deduplicated` |
| 21 | `fn a_never_connected_device_does_not_block_pruning` | `// md:fn a_never_connected_device_does_not_block_pruning` |
| 22 | `fn deleted_resource_frees_quota_and_blob_is_purgeable` | `// md:fn deleted_resource_frees_quota_and_blob_is_purgeable` |
