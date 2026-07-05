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

## Related files

- `../src/collab.rs` — the code under test.
- `tests/integration.md` — the complementary relay tests.
