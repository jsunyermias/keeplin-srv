# `tests/collab_client_e2e.rs` — real daemon client ↔ real server

Self-contained companion for `crates/keeplin-srv/tests/collab_client_e2e.rs`. It
documents **every code block of the source file, in source order** — a reader with only
this file must be able to understand the test binary without opening anything else, so
project-wide conventions are deliberately re-explained here (hyper-redundancy is
intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the `#[path] mod common;` declaration and
imports. Marker `// md:Overview`.

```rust
#[path = "collab_e2e_common/mod.rs"]
mod common;

use common::*;
use keeplin_core::{models::Note, storage::NoteRepository};
use sqlx::PgPool;
```

**What it does** — Real-client end-to-end binary: the genuine client stack a keeplin
daemon mounts in server+collab mode — `CollabBackend<DbBackend>` (keeplin-core) —
driven against a real `keeplin-srv` on a throwaway PostgreSQL database
(`#[sqlx::test]`). This closes the gap the other suites leave: `tests/collab.rs`
drives `/api/ws` with hand-built frames (protocol level); `tests/integration.rs`
drives the relay with a raw `DbBackend`; **this binary** exercises the whole
client↔server contract exactly as a daemon would. It lives in its **own** test binary
so its background client tasks (reconnect loops, the second `/api/sync` connection)
die with the process instead of interfering with other tests (issue #51).

**Dependencies** — the shared harness `collab_e2e_common/mod.rs` (included by
`#[path]`); keeplin-core's `Note` model and `NoteRepository` trait; `sqlx::PgPool`
(injected by `#[sqlx::test]`).

**Used by** — `cargo test` (its own binary); CI.

**Repeated context** — Repo test conventions: tests drive the **real** client stack,
never a mock; each e2e scenario gets its own binary (issue #51 — never add a second
scenario here); every `#[sqlx::test]` gets a fresh migrated database. A deliberate
non-assertion: a note edited **in the same session that created it** is not asserted,
because the client's `create_note` pushes body ops before the Join's `Welcome`
arrives, so a late empty `Welcome` can transiently clobber the *local* optimistic
body — the server state is correct throughout; that client-side ordering is a
keeplin (`CollabBackend`) concern tracked separately.

---

## fn collab_client_writes_note_through_to_the_server

**Identification** — `#[sqlx::test(migrations = "../../migrations")]` async test;
marker `// md:fn collab_client_writes_note_through_to_the_server`.

**What it does** — The write-through scenario: spawn the server, register + login one
account, build the real collab device, and `create_note(Note::new("Title",
"hello world"))` through the client — which POSTs the note, joins the collaborative
session, and pushes the body as line ops. Asserts (via `wait_server_body`, polling
`GET /api/notes/:id/export`) that the **server materialises the lines** and the
exported body converges to `"hello world"`.

**Dependencies** — harness helpers `spawn_server`, `register`, `login`,
`collab_device`, `wait_server_body` (`collab_e2e_common/mod.rs`); keeplin-core
`Note::new` / `NoteRepository::create_note`.

**Used by** — `cargo test`.

**Repeated context** — Convergence is polled generously (`CONVERGE_TRIES`, ~30 s):
real-client convergence latency tracks database throughput, and a tight bound flakes
under a busy CI database. The materialised-body read is the server's derived
line-model join — the body is never stored as a blob.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `collab_client_writes_note_through_to_the_server()` — defined here (EXTRACTED; 2 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries (EXTRACTED: calls×2; e.g. `collab_device()`, `wait_server_body()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `mod common` + imports | `// md:Overview` |
| 2 | `fn collab_client_writes_note_through_to_the_server` | `// md:fn collab_client_writes_note_through_to_the_server` |
