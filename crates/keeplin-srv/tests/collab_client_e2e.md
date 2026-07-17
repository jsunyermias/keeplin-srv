# `tests/collab_client_e2e.rs` — real daemon client ↔ real server

## What is tested

The genuine client stack a keeplin daemon mounts in server+collab mode —
`CollabBackend<DbBackend>` (keeplin-core) — driven against a real `keeplin-srv` instance on a
throwaway PostgreSQL database (`#[sqlx::test]`). This closes the gap left by the other suites:

- `tests/collab.rs` drives `/api/ws` with hand-built frames (protocol level).
- `tests/integration.rs` drives the relay with a raw `DbBackend`.
- **this suite** drives the whole real client (relay + collab channel together) over the network, so
  the client↔server **contract** is exercised in CI exactly as a daemon would.

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `collab_client_writes_note_through_to_the_server` | the real client creates a note and pushes its body as line ops | the server materialises the lines; the exported body converges |
| `reconnecting_client_rebuilds_note_from_snapshot` | write + disconnect, then a **fresh** client (empty local DB, same account) connects | it discovers the note, rebuilds the body from the `Welcome` snapshot ("client DB is a cache"), and an edit after the clean join converges back on the server |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `spawn_server` | boot the router on an ephemeral port (with `ConnectInfo`) |
| `register` / `login` | REST account setup; `login` returns a device token |
| `collab_device` | build the daemon's real stack: `CollabBackend<DbBackend>` pointed at `/api/sync` + `/api/ws`, started with itself as the top |
| `wait_server_body` | poll `GET /api/notes/:id/export` until the materialised body matches (tolerates the pre-lines `404` window) |
| `wait_local_body` | poll a client's local note body until it matches |

## Notes & gotchas

- The tests assert the **server-side** contract (convergence, snapshot rebuild, edit-after-clean-join).
  A note edited **in the same session that created it** is intentionally not asserted: the client's
  `create_note` pushes body ops before the Join's `Welcome` arrives, so a late empty `Welcome` can
  transiently clobber the *local* optimistic body until the next reconnect — the server state is
  correct throughout. That client-side ordering is a keeplin (`CollabBackend`) concern, tracked
  separately; this suite deliberately avoids depending on it.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `collab_client_writes_note_through_to_the_server()` — defined here (EXTRACTED; 2 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries (EXTRACTED: calls×2; e.g. `collab_device()`, `wait_server_body()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Drives the REAL client stack (`CollabBackend<DbBackend>`), never a mock (repo convention #4).
- Each e2e scenario lives in its own test binary (issue #51) — do not add scenarios to this binary.
- Runs against a throwaway `#[sqlx::test]` PostgreSQL database.

## Related files

- `../src/collab.rs` — the server side of the `/api/ws` channel.
- `../src/sync.rs` — the relay the client's `DbBackend` speaks to.
- `keeplin/keeplin-core/src/collab/mod.md` — the client being exercised.
