# `tests/collab.rs` — collaborative channel & hardening tests

## What is tested

End-to-end tests of the `/api/ws` collaborative protocol and the production-hardening surfaces,
driven over a **real WebSocket and real HTTP** against a `keeplin-srv` instance backed by a
throwaway PostgreSQL database (`#[sqlx::test]` creates one per test). No mocking — the tests
register users, log devices in, open sockets, and send raw protocol frames. Ops are signed
with the login's `device_id` (the vv actor).

## Test cases

### Protocol

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `join_receives_welcome_snapshot` | join an empty note | `Welcome` with empty order/lines, then `Presence` of one |
| `ops_propagate_between_participants` | A inserts, B updates | each sees the other's `Op`; body converges |
| `concurrent_updates_resolve_deterministically` | A and B edit the same line concurrently | both replicas converge to the tiebreak winner |
| `stale_op_is_ignored` | replay an op with the same `vv` | ignored; state unchanged |
| `move_reorders_lines` | `Move` a line to the front | order reflects the move |
| `import_then_export_roundtrip` | import a flat body, export it | body round-trips; `Welcome` shows the lines |

### Permissions & safety

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `viewer_can_watch_but_not_edit` | viewer sends an `Op` | `Error{code:"forbidden"}`; body unchanged |
| `outsider_cannot_join` | non-shared user joins | `Error{code:"forbidden"}` |
| `forged_writer_is_rejected` | sign an op with another device id | `Error{code:"bad_writer"}` |
| `presence_shows_other_participants` | two users join, one moves cursor | presence lists both; cursor propagates |

### Hardening

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `ws_accepts_authorization_header` | connect with the token in the `Authorization` header only | `Welcome` received (no `?token=`) |
| `deleting_a_device_revokes_its_token` | delete a device, reuse its token | `200` before, `401` after |
| `gc_compacts_old_tombstones` | delete a line, run GC past the window | reclaims exactly 1; body and order intact |
| `metrics_reports_counts` | hit `/api/metrics` | user/note counts correct |
| `rate_limit_throttles_and_spares_health` | burst past the budget | 4th request `429`; `/health` always `200` |
| `note_move_requires_write_on_destination_notebook` | B moves a note into A's notebook with no / read-only / write access | 403 / 403 / 200; unknown destination `404` |
| `notebook_owner_can_manage_child_notes_they_do_not_own` | B files their note in A's notebook | A (notebook owner) can GET/PATCH and lists the note, but cannot DELETE it (ownership stays with B) |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `spawn_server` / `spawn_server_with_state` | boot the router on an ephemeral port (with `ConnectInfo`); the latter also returns `AppState` for store-level assertions |
| `spawn_rate_limited(pool, per_min)` | boot with a custom rate-limit budget |
| `user` / `create_note` / `share` | REST setup helpers returning ids/tokens |
| `ws_connect` / `send` / `recv_until` | socket helpers; `recv_until` skips unrelated frames with a timeout |
| `insert_op` / `update_op` / `join` | build protocol frames |
| `wait_export` | poll the export endpoint until the body converges (no fixed sleeps) |

## Coverage gaps

- The device **relay** (`/api/sync`) is covered by `tests/integration.rs`, not here.
- Reconnection/backoff of the client lives in keeplin-core's own suite.
- The GC test polls the store's line set (not the exported body) for the settled state, because
  the body is transiently ambiguous — see the comment on `gc_compacts_old_tombstones`.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `spawn_server_with_state()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `test_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `spawn_instance()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `spawn_rate_limited()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `spawn_server()` — defined here (EXTRACTED; file-local)
- `user()` — defined here (EXTRACTED; file-local)
- `create_note()` — defined here (EXTRACTED; file-local)
- `share()` — defined here (EXTRACTED; file-local)
- `ws_connect()` — defined here (EXTRACTED; file-local)
- `send()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×1; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×3; e.g. `router()`)
- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×1; e.g. `AppState`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Exercises the collaborative wire protocol end to end over real WebSockets: resolution determinism, replay-ignoring, roles, and forged-`last_writer` rejection must stay covered.
- Throwaway `#[sqlx::test]` database per test.
- Viewer/outsider denial paths are part of the contract, not incidental coverage.

## Related files

- `../src/collab.rs` — the code under test.
- `tests/integration.md` — the complementary relay tests.
