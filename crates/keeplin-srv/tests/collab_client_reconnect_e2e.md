# `tests/collab_client_reconnect_e2e.rs` — reconnect/rebuild e2e (real client)

Self-contained companion for `crates/keeplin-srv/tests/collab_client_reconnect_e2e.rs`.
It documents **every code block of the source file, in source order, with its complete code embedded** — a reader with
only this file must be able to understand the test binary without opening anything
else, so project-wide conventions are deliberately re-explained here (hyper-redundancy
is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the `#[path] mod common;` declaration and
imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
#[path = "collab_e2e_common/mod.rs"]
mod common;

use common::*;
use keeplin_core::{models::Note, storage::NoteRepository};
use sqlx::PgPool;
```

**What it does** — Real-client e2e binary for the **"client DB is a cache"**
property: a fresh client with an empty local database must rebuild a note from the
server's `Welcome` snapshot on connect, and its subsequent edits must converge back.
Own test binary (issue #51: e2e background client tasks must die with the process,
not leak into the next test's database).

**Dependencies** — the shared harness `collab_e2e_common/mod.rs`; keeplin-core
`Note`/`NoteRepository`; `sqlx::PgPool`.

**Used by** — `cargo test`; CI.

**Repeated context** — Snapshot-rebuild model, restated: the collaborative channel
keeps no op history; on (re)connect a client receives the full `Welcome` snapshot
(order entity + all lines, tombstones included) and reconstructs its state — which
is exactly why server-side pruning/dropping of collab messages is safe.

---

## fn reconnecting_client_rebuilds_note_from_snapshot

**Identification** — `#[sqlx::test]` async test; marker
`// md:fn reconnecting_client_rebuilds_note_from_snapshot`.

**Code** — complete and verbatim:

```rust
// md:fn reconnecting_client_rebuilds_note_from_snapshot
#[sqlx::test(migrations = "../../migrations")]
async fn reconnecting_client_rebuilds_note_from_snapshot(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;

    let note_id = {
        let a = collab_device(addr, &token).await;
        let note = a
            .create_note(Note::new("Persisted", "durable body"))
            .await
            .unwrap();
        wait_server_body(addr, &token, note.id, "durable body").await;
        note.id
    };

    let b = collab_device(addr, &token).await;
    wait_local_body(&b, note_id, "durable body").await;

    let mut edited = b.read_note(note_id).await.unwrap();
    edited.body = "edited after reconnect".into();
    b.update_note(edited).await.unwrap();
    wait_server_body(addr, &token, note_id, "edited after reconnect").await;
}
```

**What it does** — Three acts: (1) client A creates a note (`"Persisted"` /
`"durable body"`) through the real stack, the server materialises it
(`wait_server_body`), and A is **dropped** — its connections close. (2) A fresh
client B — same account, brand-new empty local database — connects, discovers the
note, joins it, and `wait_local_body` asserts B's local body equals the server's:
the rebuild came from the `Welcome` snapshot. (3) Having joined cleanly (its mirror
settled from the `Welcome`), B edits the note and `wait_server_body` asserts the
edit converges back on the server.

**Dependencies** — harness helpers `spawn_server`, `register`, `login`,
`collab_device`, `wait_server_body`, `wait_local_body`; keeplin-core
`read_note`/`update_note`.

**Used by** — `cargo test`.

**Repeated context** — The edit-after-clean-join assertion matters: it proves the
rebuilt client is a *first-class* replica (its vv state lets new edits win
resolution), not a read-only copy.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `reconnecting_client_rebuilds_note_from_snapshot()` — defined here (EXTRACTED; 3 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries (EXTRACTED: calls×3; e.g. `collab_device()`, `wait_local_body()`, `wait_server_body()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `mod common` + imports | `// md:Overview` |
| 2 | `fn reconnecting_client_rebuilds_note_from_snapshot` | `// md:fn reconnecting_client_rebuilds_note_from_snapshot` |
