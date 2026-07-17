# `tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries

## Purpose

Shared setup for the `collab_client_*_e2e` test binaries. Each e2e test lives in its **own**
integration-test binary (cargo runs test binaries sequentially), so the real client's
background tasks — reconnect loops, the second `/api/sync` connection — die with the process
instead of hammering the shared `#[sqlx::test]` PostgreSQL harness while the next test runs;
that cross-test interference is what made these tests flaky in one binary (issue #51).

## Helpers

| Utility | Purpose |
|---------|---------|
| `test_config()` | standard test `Config` (registration open, no quotas/rate limit/at-rest key) |
| `spawn_server(pool)` | boot the real router on an ephemeral port with `ConnectInfo` |
| `register` / `login` | REST account setup; `login` returns the device token |
| `collab_device(addr, token)` | the exact stack the daemon mounts in server+collab mode: `DbBackend` (relay, `/api/sync`) wrapped in `CollabBackend` (`/api/ws`), `start`ed with itself as stack top — `start` runs the `GET /version` handshake and must negotiate cleanly against this matching server |
| `CONVERGE_TRIES` | generous polling bound (~30 s): real-client convergence tracks database throughput; a tight bound flakes under a busy CI database |
| `wait_server_body` | poll `GET /api/notes/:id/export` until the materialised body matches |
| `wait_local_body` | poll a client's local note body until it matches |

## Invariant

Every new real-client e2e scenario gets its **own** `tests/<name>_e2e.rs` binary including
this module via `#[path = "collab_e2e_common/mod.rs"]` — never add a second e2e scenario to
an existing binary.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `collab_device()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `wait_server_body()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `test_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `spawn_server()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `wait_local_body()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `register()` — defined here (EXTRACTED; file-local)
- `login()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×1; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/tests/collab_client_e2e.rs` — real daemon client ↔ real server (EXTRACTED: calls×2; e.g. `collab_client_writes_note_through_to_the_server()`)
- `crates/keeplin-srv/tests/collab_client_reconnect_e2e.rs` — reconnect/rebuild e2e (real client) (EXTRACTED: calls×3; e.g. `reconnecting_client_rebuilds_note_from_snapshot()`)
- `crates/keeplin-srv/tests/collab_client_resources_e2e.rs` — out-of-band resource blob e2e (real client) (EXTRACTED: calls×1; e.g. `resource_blob_travels_out_of_band_through_the_real_client()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Every real-client e2e scenario gets its OWN test binary including this module via `#[path]` (issue #51) — never share a binary between scenarios.
- `collab_device` builds the exact daemon stack (`DbBackend` + `CollabBackend`) and `start` must negotiate the `/version` handshake cleanly.
- Convergence polls stay generous (`CONVERGE_TRIES`); tightening them reintroduces CI flake.

## Related files

- `tests/collab_client_e2e.rs` — write-through scenario.
- `tests/collab_client_reconnect_e2e.rs` — snapshot-rebuild scenario.
- `tests/collab_client_resources_e2e.rs` — out-of-band resource blob scenario.
