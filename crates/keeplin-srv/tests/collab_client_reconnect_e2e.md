# `tests/collab_client_reconnect_e2e.rs` — reconnect/rebuild e2e (real client)

## What is tested

The "client DB is a cache" property through the real collab stack
(`CollabBackend<DbBackend>`) against a real keeplin-srv on a throwaway PostgreSQL database:
a fresh client with an **empty local database** must rebuild a note from the server's
`Welcome` snapshot on connect, and its subsequent edits must converge back. Lives in its own
test binary (issue #51: e2e background tasks must die with the process).

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `reconnecting_client_rebuilds_note_from_snapshot` | client A writes a note and is dropped; client B (same account, empty local DB) connects | B discovers + joins the note and materialises the body from the `Welcome` snapshot; an edit from B then converges on the server |

## Fixtures and helpers

Shared harness `collab_e2e_common/mod.rs`: `spawn_server`, `register`/`login`,
`collab_device`, `wait_server_body` / `wait_local_body` (generous convergence polls).

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `reconnecting_client_rebuilds_note_from_snapshot()` — defined here (EXTRACTED; 3 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries (EXTRACTED: calls×3; e.g. `collab_device()`, `wait_local_body()`, `wait_server_body()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Proves the 'client DB is a cache' property with a REAL client whose local database starts empty.
- Own test binary (issue #51); throwaway `#[sqlx::test]` database.

## Related files

- keeplin `keeplin-core/src/collab/mod.rs` — discovery, Join/Welcome reconcile, pending-push.
- `../src/collab.rs` — the server session sending the `Welcome` snapshot.
- `tests/collab_client_e2e.rs` — the write-through sibling (create → server materialises).
