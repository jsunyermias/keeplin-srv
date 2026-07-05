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

## Related files

- `../src/http.rs` — where the quotas are enforced.
- `../src/store.rs` — `user_blob_bytes_excluding`, `count_live_notes_for_user`.
- `../src/config.rs` — the `MAX_USER_STORAGE_BYTES` / `MAX_NOTES_PER_USER` knobs.
