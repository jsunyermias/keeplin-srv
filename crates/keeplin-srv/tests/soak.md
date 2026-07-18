# `tests/soak.rs` — multi-instance collaborative soak/load drill

Self-contained companion for `crates/keeplin-srv/tests/soak.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand the drill without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports and the `Ws` type alias. Marker
`// md:Overview`.

**What it does** — The multi-instance collaborative **soak/load drill**
(production-readiness item: prove the issue #45 cross-instance path under real
concurrency, not just the happy path). **`#[ignore]`d** — a load drill, not a unit of
CI; run explicitly:

```bash
DATABASE_URL=postgres://… cargo test --release --test soak -- --ignored --nocapture
# knobs: SOAK_EDITORS (default 8), SOAK_OPS (default 25 per editor)
```

Scenario: two bus-enabled server instances (Postgres LISTEN/NOTIFY) share one
database; `SOAK_EDITORS` editors — each with its own device/login, half per
instance — join one shared note and concurrently insert `SOAK_OPS` lines each over
raw `/api/ws` WebSockets. **Phase 1** asserts both instances settle on a
byte-identical exported body (and reports throughput/convergence time). **Phase 2**
kills instance B mid-session; the editors on A keep writing and everything must
still converge on A — the "kill a replica mid-edit" drill.

**Dependencies** — `tokio_tungstenite` (raw WS clients), `futures_util`,
`keeplin_srv` (`Config`, `router`, `AppState`, `bus::spawn`), `reqwest`, `sqlx`,
`serde_json`, `uuid`, `chrono`.

**Used by** — operators/CI on demand (`--ignored`); `RUNBOOK.md` ("Load / soak
drill").

**Repeated context** — Multi-instance model (issue #45), restated: instances share
only PostgreSQL; collab ops fan out to sibling subscribers via the `collab_events`
outbox + `collab_op` NOTIFY, order writes serialise on the per-note advisory lock,
and a connection only ever talks to one instance. This drill drives the **raw wire
protocol** for maximal op pressure — client behaviour is covered by the
`collab_client_*_e2e` binaries.

---

## fn test_config

**Identification** — helper; marker `// md:fn test_config`. Standard test `Config`
(10-connection pool — two instances share the drill's load).

**Dependencies** — `Config`. **Used by** — `spawn_instance`.
**Repeated context** — none.

---

## fn spawn_instance

**Identification** — helper; marker `// md:fn spawn_instance`.
`async fn spawn_instance(pool) -> (SocketAddr, JoinHandle<()>)`.

**What it does** — Spawns a **bus-enabled** instance (`AppState::new` +
`bus::spawn(state)` + router on an ephemeral port). Returns the serve task's
`JoinHandle` so phase 2 can kill the instance (`abort`).

**Dependencies** — `AppState::new`, `bus::spawn`, `router`. **Used by** — the
drill.

**Repeated context** — Each `AppState::new` mints a fresh `instance_id`, so the two
instances correctly skip their own bus echoes.

---

## fn env_or

**Identification** — helper; marker `// md:fn env_or`. Parse a numeric env knob
with a default (`SOAK_EDITORS`, `SOAK_OPS`).

**Dependencies** — `std::env`. **Used by** — the drill. **Repeated context** —
none.

---

## fn ws_connect

**Identification** — helper; marker `// md:fn ws_connect`. Opens a raw
`tokio-tungstenite` WebSocket to `ws://…/api/ws?token=…` (the query-string token
fallback).

**Dependencies** — `tokio_tungstenite`. **Used by** — `editor`.
**Repeated context** — none.

---

## fn export_body

**Identification** — helper; marker `// md:fn export_body`. Authenticated
`GET /api/notes/:id/export`, returning the materialised body string.

**Dependencies** — `reqwest`. **Used by** — `wait_quiescent_identical`.
**Repeated context** — the export is the derived line-model join — the
convergence observable.

---

## fn merge_vv

**Identification** — helper; marker `// md:fn merge_vv`. Pointwise-max merge of an
op/order version vector (as JSON) into the editor's causal view — the same merge
rule the server and client use.

**Dependencies** — `serde_json`. **Used by** — `editor`.
**Repeated context** — version vectors: per-device counters; merging = pointwise
max = absorbing another actor's history.

---

## fn editor

**Identification** — helper task; marker `// md:fn editor`.

```rust
async fn editor(addr, token, device_id, note_id, ops: usize)
```

**What it does** — One simulated editor: connect, `Join` the note, and **wait for
the `Welcome`** (seeding the causal view from the snapshot's order vv) so ops
cannot race the subscription. Then insert `ops` lines **at the head**
(`after_line_id: null` — order-contended on purpose), signing each op *causally*
the way the real client does: the sent vv is everything this editor has seen
(Welcome + absorbed broadcasts, merged via `merge_vv`) plus its own bumped
component. Between sends it non-blockingly drains incoming `Op` broadcasts into
the causal view; at the end it drains briefly so the server can flush. A
causally-stale insert is dropped by design; a causal one must be applied.

**Dependencies** — `ws_connect`, `merge_vv`; raw protocol frames.
**Used by** — both phases of the drill.

**Repeated context** — Device-as-actor: each editor logs in as its own device and
signs `last_writer` with that device id — forged or shared writers would be
rejected (`bad_writer`).

---

## fn wait_quiescent_identical

**Identification** — helper; marker `// md:fn wait_quiescent_identical`.

```rust
async fn wait_quiescent_identical(addrs, token, note_id, budget)
    -> Result<(Duration, usize), String>
```

**What it does** — Polls the exports until **every** instance returns the
byte-identical body **twice in a row** (quiescent *and* cross-instance
consistent — the issue #45 guarantee), returning the settle time and line count;
errs after the budget. Under head-of-note contention the server legitimately drops
causally-concurrent-and-older inserts (design §5) — the real client re-diffs and
self-heals — so the drill asserts *consistency* and reports the applied/sent ratio
as a metric, never failing on drops.

**Dependencies** — `export_body`. **Used by** — both phases.

**Repeated context** — "Identical twice in a row" distinguishes convergence from a
coincidentally-equal snapshot mid-churn.

---

## fn soak_two_instances_under_concurrent_editors

**Identification** — `#[sqlx::test]` + `#[ignore]`; marker
`// md:fn soak_two_instances_under_concurrent_editors`.

**What it does** — The drill: spawn instances A and B; register the owner, create
the shared note on A; **phase 1** — `SOAK_EDITORS` editors (own login each,
alternating instances) run concurrently, then both instances must settle
identically within 120 s (`applied > 0`); throughput, settle time and the
applied/sent ratio are printed. **Phase 2** — `handle_b.abort()` kills B
mid-session; `max(editors/2, 1)` survivor editors keep writing on A, which must
settle again with `applied2 > applied` — the cluster stays writable after a
replica death. Prints `SOAK: PASS` on success.

**Dependencies** — every helper above. **Used by** — explicit `--ignored` runs.

**Repeated context** — What failure would mean: phase 1 divergence = a lost update
or missed cross-instance delivery (advisory lock/outbox bug); phase 2 stall = the
surviving instance depended on its dead sibling (bus liveness bug).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

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

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports + `type Ws` | `// md:Overview` |
| 2 | `fn test_config` | `// md:fn test_config` |
| 3 | `fn spawn_instance` | `// md:fn spawn_instance` |
| 4 | `fn env_or` | `// md:fn env_or` |
| 5 | `fn ws_connect` | `// md:fn ws_connect` |
| 6 | `fn export_body` | `// md:fn export_body` |
| 7 | `fn merge_vv` | `// md:fn merge_vv` |
| 8 | `fn editor` | `// md:fn editor` |
| 9 | `fn wait_quiescent_identical` | `// md:fn wait_quiescent_identical` |
| 10 | `fn soak_two_instances_under_concurrent_editors` | `// md:fn soak_two_instances_under_concurrent_editors` |
