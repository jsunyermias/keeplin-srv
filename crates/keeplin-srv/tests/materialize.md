# `tests/materialize.rs` — domain-entity materialisation tests

## What is tested

End-to-end tests of the server materialising the keeplin-core domain entities (notebooks, tags,
note↔tag associations, resource metadata + binaries) that arrive over the `/api/sync` relay, driven
by the **real client** (keeplin-core's `DbBackend`) against a `keeplin-srv` instance on a throwaway
PostgreSQL database (`#[sqlx::test]`). This is the "server is the truth, client DB is a cache" model:
entities pushed over the relay become durable, queryable, version-vector-resolved server state.

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `notebook_materialises_and_is_served` | create a notebook, push | `GET /api/notebooks` returns it |
| `tag_and_association_materialise` | create tag + note + associate, push | `GET /api/tags` and `…/tags` reflect it |
| `removing_a_tag_association_tombstones_it` | remove the association, push | `…/tags` becomes empty |
| `resource_metadata_and_blob_materialise` | create a resource (binary in the change), push | metadata listed; `GET …/data` returns the bytes (backward-compat path) |
| `streaming_blob_upload_then_download` | `PUT …/data` a new blob | `GET …/data` returns the replaced bytes (Option B path) |
| `uploading_to_unknown_resource_is_rejected` | `PUT …/data` for an unknown id | `404` (metadata must exist first) |
| `deleting_a_notebook_removes_it_from_listings` | delete a notebook, push | it disappears from `GET /api/notebooks` |
| `users_do_not_see_each_others_entities` | two users | one never sees the other's notebook |
| `concurrent_notebook_edits_converge_deterministically` | two concurrent edits to one id | same winner regardless of apply order (store level) |
| `materialised_entities_survive_journal_pruning` | prune the whole journal after delivery | the notebook is still served — the table is the truth, not the journal |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `spawn_server` | boot the router on an ephemeral port with `ConnectInfo` |
| `register` / `login` / `device` | REST account setup; build a server-mode `DbBackend` on `/api/sync` |
| `push` | send all local changes and let the server materialise the batch |
| `get_json` | authenticated `GET` returning parsed JSON |

## Coverage gaps

- These tests drive the **relay-mode** client (`DbBackend` alone), whose `ResourceCreate` still
  carries the binary inline — deliberately exercising the server's backward-compat path. The
  **collab-mode** client (`CollabBackend`) uploads out-of-band (`upload_blob` →
  `PUT /api/resources/:id/data`) and strips `data` from the relayed change; that path is driven
  through the real client in `tests/collab_client_resources_e2e.rs` (blob served, journal
  blob-free, second device downloads from the server).

## Graph context

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

**Invariants** (restated on purpose; a change to this file must keep these true)

- Tests run against throwaway `#[sqlx::test]` databases with the REAL relay client (`DbBackend`), not mocks.
- These tests cover the relay-mode inline-binary (backward-compat) path on purpose; the collab-mode out-of-band path is covered by `collab_client_resources_e2e.rs`.
- Materialised tables — not the journal — are asserted as the source of truth (see the pruning-survival test).

## Related files

- `../src/sync.rs` — the `materialize` hook under test.
- `../src/store.rs` — the resolve-and-upsert methods.
- `../../../migrations/0004_domain_entities.md` — the tables asserted against.
