# `tests/soak.rs` — multi-instance collaborative soak/load drill

## What is tested

The #45 cross-instance guarantees under real concurrency, not just the happy path: two
bus-connected server instances (Postgres LISTEN/NOTIFY) share one database while
`SOAK_EDITORS` concurrent editors (default 8, half per instance, each its own device/login)
insert `SOAK_OPS` lines each (default 25) into one shared note over raw `/api/ws`
WebSockets. **`#[ignore]`d in CI** — it is a load drill, run explicitly:

```bash
DATABASE_URL=postgres://… cargo test --release --test soak -- --ignored --nocapture
```

## Phases and assertions

| Phase | Scenario | Asserted |
|-------|----------|----------|
| 1 — concurrent load | all editors write through both instances | both instances settle on a **byte-identical** exported body; total line count sane; throughput + convergence time reported |
| 2 — replica death | instance B's task is killed mid-session | editors on A keep writing and everything still converges on A (the "kill a replica mid-edit" drill) |

Causally-concurrent inserts at the same position that lose the deterministic tiebreak are
dropped **by design** (the live client re-diffs and self-heals); the drill reports that ratio
rather than failing on it.

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `test_config()` | standard test `Config` (no quotas, no rate limit) |
| `spawn_instance` | bus-enabled instance returning the `JoinHandle` phase 2 kills |
| raw `tokio-tungstenite` clients | editors speak the collab protocol directly (Join/Op frames) for maximal op pressure |

## Coverage gaps

- This drives the raw wire protocol, not the `CollabBackend` client stack (covered by the
  `collab_client_*_e2e` binaries); it measures convergence under load, not client behaviour.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `test_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `spawn_instance()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `env_or()` — defined here (EXTRACTED; file-local)
- `ws_connect()` — defined here (EXTRACTED; file-local)
- `export_body()` — defined here (EXTRACTED; file-local)
- `merge_vv()` — defined here (EXTRACTED; file-local)
- `editor()` — defined here (EXTRACTED; file-local)
- `wait_quiescent_identical()` — defined here (EXTRACTED; file-local)
- `soak_two_instances_under_concurrent_editors()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×1; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- `#[ignore]`d — a load drill run explicitly, never part of default CI.
- Asserts byte-identical convergence across two bus-connected instances and survival of a mid-session replica kill.
- Dropped causally-concurrent same-position inserts are by design (client self-heals); the drill reports the ratio instead of failing on it.

## Related files

- `../src/bus.rs` — the cross-instance LISTEN/NOTIFY bus under load here.
- `../src/collab.rs` — the per-note session/resolution logic being hammered.
- `RUNBOOK.md` ("Load / soak drill") — how and when operators run this.
