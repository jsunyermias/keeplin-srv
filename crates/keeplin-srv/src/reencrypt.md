# `reencrypt.rs` — one-off at-rest re-encrypt pass

Self-contained companion for `crates/keeplin-srv/src/reencrypt.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand `reencrypt.rs` without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `reencrypt.rs` carries exactly one marker comment of
the form `// md:<Header> > … > <Block header>`, whose path is the header chain of the
section documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview` at
the top of the file.

```rust
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::crypto::{Cipher, ENC_PREFIX};
use crate::error::AppError;
```

**What it does** — The one-off administrative re-encrypt pass for at-rest encryption
(issue keeplin#110 follow-up). Context: when `AT_REST_KEY` is enabled on an existing
deployment, rows written before the key stay plaintext forever — the at-rest `Cipher`
(`crypto.rs`) reads both plaintext and `enc:v1:`-tagged forms, so a live database can
healthily hold a mix, but nothing rewrites the old rows. This module scans the two
encrypted columns — `notes.title` and `lines.content` — selects the rows that do
**not** carry the `enc:v1:` tag, and rewrites them encrypted. It is deliberately
**library code** (so `tests/reencrypt.rs` can drive it under `#[sqlx::test]` without a
subprocess); the thin `src/bin/reencrypt.rs` binary (`keeplin-reencrypt`) wraps it for
operators. When and how to run it is documented in `RUNBOOK.md` ("Key rotation &
re-encryption").

Properties, all load-bearing for a tool run against a production database:

- **Idempotent** — already-encrypted rows are never selected (`NOT LIKE 'enc:v1:%'`);
  a completed pass re-run is a no-op.
- **Batched** — keyset-paginated batches (`id > last`, `ORDER BY id`,
  `LIMIT batch_size`), one transaction per batch; never a whole-table transaction or
  lock.
- **Resumable** — each batch commits independently; an interrupted run keeps its
  finished batches and the next run selects only what is left.
- **Safe against a live server** — every `UPDATE` is guarded with
  `AND <column> = <the plaintext we read>`; a row rewritten concurrently by the running
  server (same key, encrypts all new writes) is skipped, never clobbered.
- **`--dry-run`** — counts what would be rewritten; issues no `UPDATE` at all.
- **Progress logging** — one `tracing` line per batch per table.

**Dependencies** — `sqlx` (external): pool, dynamically built queries, transactions.
`uuid` (external): the keyset cursor. `tracing` (external). Internal:
`crate::crypto::{Cipher, ENC_PREFIX}` (`crypto.rs`), `crate::error::AppError`
(`error.rs`).

**Used by** — `src/bin/reencrypt.rs` (CLI wrapper: parses `--dry-run`/`--batch-size`,
builds pool + cipher from the environment, prints stats); `tests/reencrypt.rs`
(end-to-end: seeds plaintext through a keyless server, runs the pass, asserts
ciphertext + served plaintext + idempotence + dry-run inertness + keyless refusal).

**Repeated context** — The at-rest storage format, restated: a stored value is either
plaintext (untagged) or `enc:v1:<base64(12-byte nonce ‖ AES-256-GCM ciphertext)>`,
with a fresh random nonce per value; both forms always decrypt (`crypto.rs`), which is
what makes enabling the key on a live database safe and this migration unhurried.
At runtime, encryption is applied/removed **only** in `store.rs` (single choke point);
this module is the one sanctioned exception because the storage form *is* its job.

---

## Options

**Identification** — struct; marker `// md:Options`.

```rust
#[derive(Debug, Clone)]
pub struct Options {
    pub dry_run: bool,
    pub batch_size: i64,
}
```

**What it does** — Tuning/behaviour knobs for one pass. `dry_run`: report what would
change and write nothing (no `UPDATE` is even issued). `batch_size`: rows per batch,
one transaction each — bounds transaction size and memory.

**Dependencies** — none.

**Used by** — `run`/`reencrypt_column` (this file); built by
`bin/reencrypt.rs::parse_args` from CLI flags and by `tests/reencrypt.rs` directly
(batch size 1 exercises batching).

**Repeated context** — Small bounded transactions are a live-database courtesy: the
pass must be runnable beside production traffic without stalling it.

---

## impl Default for Options

**Identification** — trait impl; marker `// md:impl Default for Options`.

```rust
impl Default for Options { fn default() -> Self }
```

**What it does** — The defaults: `dry_run: false`, `batch_size: 500` — a safe default
for interactive use on a live database.

**Dependencies** — `Options` (this file).

**Used by** — `bin/reencrypt.rs::parse_args` (starts from defaults, applies flags);
`tests/reencrypt.rs`.

**Repeated context** — none.

---

## TableStats

**Identification** — struct; marker `// md:TableStats`.

```rust
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TableStats {
    pub scanned: u64,
    pub rewritten: u64,
    pub skipped_concurrent: u64,
}
```

**What it does** — Outcome counters for one `(table, column)` pair. `scanned`:
plaintext rows seen (equals would-be rewrites under `--dry-run`). `rewritten`: rows
actually rewritten to `enc:v1:` (always 0 under `--dry-run`). `skipped_concurrent`:
rows whose value changed between read and guarded write — the live server (same key)
already wrote them encrypted, so skipping is correct, not a loss.

**Dependencies** — none.

**Used by** — `reencrypt_column` (produces one), `Stats` (aggregates two),
`bin/reencrypt.rs` (prints), `tests/reencrypt.rs` (asserts exact counts).

**Repeated context** — `scanned == 0` on a re-run is the operator's completion signal
(idempotence, see *Overview*).

---

## Stats

**Identification** — struct; marker `// md:Stats`.

```rust
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub notes_title: TableStats,
    pub lines_content: TableStats,
}
```

**What it does** — Outcome of a whole pass: one `TableStats` per encrypted column.
Exactly two fields because exactly two columns are encrypted at rest; adding an
encrypted column means extending this struct **and** `run` **and** the `store.rs`
write paths together.

**Dependencies** — `TableStats` (this file).

**Used by** — `run` (returns it), `bin/reencrypt.rs` (prints it),
`tests/reencrypt.rs`.

**Repeated context** — The two-column list (`notes.title`, `lines.content`) is the
same one `store.rs` encrypts on write; the sites must stay in lockstep.

---

## fn run

**Identification** — public async function; marker `// md:fn run`.

```rust
pub async fn run(pool: &PgPool, cipher: &Cipher, opts: &Options) -> Result<Stats, AppError>
```

**What it does** — Runs the pass over both columns sequentially: `notes.title`, then
`lines.content`. Refuses to start (`AppError::Internal`) when the cipher is disabled:
running without a key would be a no-op that **reports success** — exactly the silent
misfire an administrative tool must refuse
(`tests/reencrypt.rs::refuses_to_run_without_a_key` pins this).

**Dependencies** — `Cipher::enabled` (`crypto.rs`), `reencrypt_column` (this file),
`AppError` (`error.rs`).

**Used by** — `bin/reencrypt.rs::main`; all three tests in `tests/reencrypt.rs`.

**Repeated context** — Fail-fast configuration (crate-wide convention): refuse to run
in a state where the tool would silently do the wrong thing — compare
`Cipher::from_key` rejecting an invalid `AT_REST_KEY` at server startup.

---

## fn reencrypt_column

**Identification** — private async function; marker `// md:fn reencrypt_column`.

```rust
async fn reencrypt_column(
    pool: &PgPool,
    cipher: &Cipher,
    opts: &Options,
    table: &str,
    column: &str,
) -> Result<TableStats, AppError>
```

**What it does** — Re-encrypts one `(table, column)` pair with keyset pagination on
`id`. `table`/`column` are compile-time literals from `run`'s two call sites, never
user input, so interpolating them into the SQL text is safe (all values are bound).
The loop:

1. `SELECT id, <column> AS value FROM <table> WHERE <column> NOT LIKE 'enc:v1:%' AND
   id > $last ORDER BY id LIMIT $batch`. The cursor starts at `Uuid::nil()`, which
   sorts before every real v4 id under Postgres uuid ordering. Empty page → done.
2. Add the page to `scanned`; advance the cursor to the page's last id — this
   guarantees forward progress even if every update in the batch is skipped.
3. Unless `dry_run`: one bounded transaction for the batch (an interruption loses at
   most this batch), and per row the guarded compare-and-swap
   `UPDATE <table> SET <column> = $encrypted WHERE id = $id AND <column> =
   $the_plaintext_we_read`. `rows_affected == 1` → `rewritten += 1`; `0` → the row
   changed under us — the live server already wrote it encrypted — so
   `skipped_concurrent += 1` and nothing is touched.
4. One progress log line per batch; a completion line per column.

Errors (sqlx, or an `encrypt` failure) propagate as `AppError`, aborting at a batch
boundary — safe to re-run.

**Dependencies** — `ENC_PREFIX`, `Cipher::encrypt` (`crypto.rs`); `sqlx`
query/transaction API, `Uuid::nil` (external); `Options`/`TableStats` (this file);
`AppError` (`error.rs`); `tracing`.

**Used by** — `run` (this file) only.

**Repeated context** — The optimistic per-row guard (instead of
`SELECT … FOR UPDATE`) is the pass's whole concurrency story: it holds no locks that
could stall the live server, and losing a race is *fine* because the winner's write is
already encrypted — correctness rests on both writers producing decryptable forms.
Keyset pagination (never OFFSET) keeps each page O(batch) regardless of table size and
is stable under concurrent inserts.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `run()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `reencrypt_column()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `Options` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.default()` — defined here (EXTRACTED; file-local)
- `TableStats` — defined here (EXTRACTED; file-local)
- `Stats` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/crypto.rs` — at-rest encryption of note titles and line content (EXTRACTED: references×2; e.g. `Cipher`)
- `crates/keeplin-srv/src/error.rs` — the API error type (EXTRACTED: imports_from×1, references×2; e.g. `AppError`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/bin/reencrypt.rs` — `keeplin-reencrypt` CLI wrapper (EXTRACTED: references×1; e.g. `parse_args()`)

## Coverage checklist

Every code block of `reencrypt.rs`, in source order, each documented above (five
points) and carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `struct Options` | `// md:Options` | Options |
| 3 | `impl Default for Options` | `// md:impl Default for Options` | impl Default for Options |
| 4 | `struct TableStats` | `// md:TableStats` | TableStats |
| 5 | `struct Stats` | `// md:Stats` | Stats |
| 6 | `fn run` | `// md:fn run` | fn run |
| 7 | `fn reencrypt_column` | `// md:fn reencrypt_column` | fn reencrypt_column |
