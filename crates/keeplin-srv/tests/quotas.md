# `tests/quotas.rs` — per-user quota enforcement tests

## What is tested

The two optional per-user quotas, driven over real HTTP against a `keeplin-srv` instance on a
throwaway PostgreSQL database (`#[sqlx::test]`), each with a custom-configured limit:

- **note count** (`MAX_NOTES_PER_USER`) at `POST /api/notes`
- **total resource-blob storage** (`MAX_USER_STORAGE_BYTES`) at `PUT /api/resources/:id/data`

Both reject with `507 Insufficient Storage` when exceeded.

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `note_quota_blocks_creation_past_the_limit` | limit 2, create 3 notes | first two `200`, third `507` |
| `note_quota_disabled_by_default` | limit `0`, create 5 | all `200` |
| `storage_quota_blocks_upload_over_the_limit` | limit 100 B | 50 ok, re-upload 50 ok (overwrite not double-counted), +60 `507`, +40 ok |
| `storage_quota_isolated_per_user` | limit 100 B, two users | each fills its own budget independently |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `quota_config(storage, notes)` | a `Config` with the two quota knobs set |
| `spawn` | boot the router (with `ConnectInfo`) on an ephemeral port |
| `register` / `login` / `device` | account setup + a server-mode `DbBackend` |
| `seed_resource` | create resource metadata (empty blob) through the relay so a `PUT` can then set its size |
| `post_note` / `put_blob` | drive the enforced endpoints, returning the status code |

## Notes

- `seed_resource` creates the metadata with an **empty** blob so the test controls the stored size
  purely through `put_blob`; the storage quota measures actual `octet_length`, not the declared size.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `spawn()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `quota_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `register()` — defined here (EXTRACTED; file-local)
- `login()` — defined here (EXTRACTED; file-local)
- `post_note()` — defined here (EXTRACTED; file-local)
- `device()` — defined here (EXTRACTED; file-local)
- `seed_resource()` — defined here (EXTRACTED; file-local)
- `put_blob()` — defined here (EXTRACTED; file-local)
- `registration_can_be_disabled()` — defined here (EXTRACTED; file-local)
- `note_quota_blocks_creation_past_the_limit()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×2; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Quota rejections are `507` and enforced before storage: blob-storage cap (`MAX_USER_STORAGE_BYTES`) and live-note cap (`MAX_NOTES_PER_USER`).
- `0` means unlimited and must stay the default-compatible behaviour.
- Throwaway `#[sqlx::test]` database; real HTTP surface.

## Related files

- `../src/http.rs` — where the quotas are enforced.
- `../src/store.rs` — `user_blob_bytes_excluding`, `count_live_notes_for_user`.
- `../src/config.rs` — the `MAX_USER_STORAGE_BYTES` / `MAX_NOTES_PER_USER` knobs.
