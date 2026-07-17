# `tests/collab_client_resources_e2e.rs` — out-of-band resource blob e2e (real client)

## What is tested

The **out-of-band resource binary path** driven end to end through the real collab-mode client
(`CollabBackend<DbBackend>`, the exact stack the daemon mounts in server+collab mode) against a
real keeplin-srv on a throwaway PostgreSQL database (`#[sqlx::test]`). Lives in its **own test
binary** (issue #51: e2e client background tasks must die with the process, not leak into the
next test's database).

The contract under test (client side implemented in keeplin's `collab/mod.rs`):

1. `create_resource` **eagerly relays the blob-stripped `ResourceCreate`**, then `PUT`s the
   binary to `/api/resources/:id/data` (with a short retry while the server materialises the
   metadata — the server 404s blob uploads for unknown resources).
2. The relay journal **never carries the binary**: every journaled `ResourceCreate` for the
   resource has `data` absent/null.
3. A second device receives only metadata over the relay and fetches the bytes through the
   client's server-download fallback (`read_resource` → `GET …/data`).

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `resource_blob_travels_out_of_band_through_the_real_client` | device A `create_resource` (4 KiB) through the collab stack; device B receives the relayed metadata | server serves the exact bytes on `GET …/data`; raw `changes.payload` rows for the `ResourceCreate` are blob-free; B's `read_resource` returns the exact bytes via server download |

## Fixtures and helpers

Shared harness `collab_e2e_common/mod.rs`: `spawn_server`, `register`/`login`, `collab_device`
(a `DbBackend` on `/api/sync` wrapped in `CollabBackend` on `/api/ws`, `start`ed with itself as
stack top), `CONVERGE_TRIES` (generous polling bound for real-client convergence).

## Related files

- keeplin `keeplin-core/src/collab/mod.rs` — `create_resource`/`upload_blob`/`read_resource`, the client half of this contract.
- `../src/http.rs` — `put_resource_data` (404 for unmaterialised metadata, quota checks) and `get_resource_data`.
- `../src/sync.rs` — materialises the blob-stripped `ResourceCreate` metadata.
- `tests/materialize.rs` — the relay-mode sibling (binary inline in the `Change`, backward-compat path).
