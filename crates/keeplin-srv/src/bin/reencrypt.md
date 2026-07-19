# `bin/reencrypt.rs` — `keeplin-reencrypt` CLI wrapper

Self-contained companion for `crates/keeplin-srv/src/bin/reencrypt.rs`. It documents
**every code block of the source file, in source order** — a reader with only this file
must be able to understand `bin/reencrypt.rs` without opening anything else, so
project-wide conventions are deliberately re-explained here (hyper-redundancy is
intended).

**How to navigate**: every block in `bin/reencrypt.rs` carries exactly one marker
comment of the form `// md:<Header> > … > <Block header>`, whose path is the header
chain of the section documenting it here (starting below the file title). Grep the
marker text to jump code → doc; grep the section's block name (or the marker path) in
the `.rs` to jump doc → code. Each block section covers five fixed points:
**Identification**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the binary's imports. Marker `// md:Overview`
at the top of the file.

**Code** — complete and verbatim:

```rust
// md:Overview
use anyhow::Context;
use keeplin_srv::{config::Config, crypto::Cipher, reencrypt};
```

**What it does** — `keeplin-reencrypt`: the operator entry point for the one-off
at-rest re-encrypt pass (issue keeplin#110 follow-up), which rewrites plaintext
`notes.title` / `lines.content` rows to the `enc:v1:` encrypted form. **All real
logic lives in `src/reencrypt.rs`** (library code, so `tests/reencrypt.rs` drives the
pass in-process); this binary only parses flags, loads the **same `Config` the server
uses** (same `.env`/environment: `DATABASE_URL`, `JWT_SECRET`, `AT_REST_KEY`), opens
a small pool, runs the pass, and prints the outcome.

Usage:

```text
keeplin-reencrypt [--dry-run] [--batch-size N]
```

Behaviour contract: requires a valid `AT_REST_KEY` (refuses without one — nothing to
encrypt to); safe to run against a live server (new writes are already encrypted and
the pass skips rows that change under it); idempotent and resumable (re-run after an
interruption and it picks up the remaining plaintext rows; a completed pass re-run
reports 0 rows found). Exit is non-zero on any failure — unknown flag,
missing/invalid key, unreachable database, mid-pass error — and a mid-pass failure is
safe because completed batches stay committed. Operational procedure: `RUNBOOK.md`
("Key rotation & re-encryption").

**Dependencies** — `anyhow` (context/bail/ensure), `tokio` (runtime), `dotenvy`,
`tracing_subscriber`, `sqlx` (pool). Internal: `keeplin_srv::config::Config`
(`config.rs`), `keeplin_srv::crypto::Cipher` (`crypto.rs`),
`keeplin_srv::reencrypt::{run, Options}` (`reencrypt.rs`).

**Used by** — operators, via the `keeplin-reencrypt` binary (declared by its location
under `src/bin/`). No code imports it.

**Repeated context** — The at-rest model, restated: values are stored plaintext or
tagged `enc:v1:<base64(nonce‖ciphertext)>`; both decrypt (`crypto.rs`), so a mixed
database is healthy and this migration can run at the operator's pace beside a live
server. Thin-binary/library split is the crate's testing convention (compare
`main.rs` vs the library router).

---

## fn parse_args

**Identification** — private function; marker `// md:fn parse_args`.

**Code** — complete and verbatim:

```rust
// md:fn parse_args
fn parse_args() -> anyhow::Result<reencrypt::Options> {
    let mut opts = reencrypt::Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => opts.dry_run = true,
            "--batch-size" => {
                let value = args
                    .next()
                    .context("--batch-size requires a value")?
                    .parse::<i64>()
                    .context("--batch-size must be a positive integer")?;
                anyhow::ensure!(value > 0, "--batch-size must be positive");
                opts.batch_size = value;
            }
            "--help" | "-h" => {
                println!("Usage: keeplin-reencrypt [--dry-run] [--batch-size N]");
                println!();
                println!("Rewrites plaintext notes.title / lines.content rows to the");
                println!("enc:v1: at-rest-encrypted form. Requires DATABASE_URL and");
                println!("AT_REST_KEY in the environment (same Config as the server).");
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other} (try --help)"),
        }
    }
    Ok(opts)
}
```

**What it does** — Minimal hand-rolled flag parsing (no CLI dependency), starting
from `reencrypt::Options::default()` (`dry_run: false`, `batch_size: 500`):

- `--dry-run` → `opts.dry_run = true`.
- `--batch-size N` → requires a value, must parse as `i64` and be positive
  (`anyhow::ensure!`), else a contextual error.
- `--help` / `-h` → print usage (including the environment requirements) and
  `exit(0)`.
- anything else → `bail!("unknown argument: … (try --help)")` — unknown flags are
  errors, not ignored, so a typo can't silently run a real pass.

**Dependencies** — `reencrypt::Options` (`reencrypt.rs`); `anyhow`; `std::env::args`.

**Used by** — `fn main` (this file) only.

**Repeated context** — Administrative tools fail loudly on anything unexpected
(same philosophy as the pass refusing to run keyless).

---

## fn main

**Identification** — `#[tokio::main] async fn main() -> anyhow::Result<()>`; marker
`// md:fn main`.

**Code** — complete and verbatim:

```rust
// md:fn main
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("keeplin_srv=info".parse()?),
        )
        .init();

    let opts = parse_args()?;
    let config = Config::from_env();
    let cipher = Cipher::from_key(config.at_rest_key.as_deref())
        .map_err(|e| anyhow::anyhow!("AT_REST_KEY: {e}"))?;
    anyhow::ensure!(
        cipher.enabled(),
        "AT_REST_KEY must be set: without it there is nothing to re-encrypt to"
    );

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&config.database_url)
        .await
        .context("connect to PostgreSQL")?;

    let stats = reencrypt::run(&pool, &cipher, &opts)
        .await
        .map_err(|e| anyhow::anyhow!("re-encrypt pass failed: {e}"))?;

    let mode = if opts.dry_run { "DRY RUN — " } else { "" };
    println!(
        "{mode}notes.title: {} plaintext row(s) found, {} rewritten, {} skipped (concurrent)",
        stats.notes_title.scanned,
        stats.notes_title.rewritten,
        stats.notes_title.skipped_concurrent
    );
    println!(
        "{mode}lines.content: {} plaintext row(s) found, {} rewritten, {} skipped (concurrent)",
        stats.lines_content.scanned,
        stats.lines_content.rewritten,
        stats.lines_content.skipped_concurrent
    );
    Ok(())
}
```

**What it does** — The wrapper sequence:

1. `dotenvy::dotenv().ok()` — same `.env` loading as the server.
2. Logging: pretty `tracing_subscriber` with a `keeplin_srv=info` default directive
   (so the pass's per-batch progress lines are visible).
3. `parse_args()`.
4. `Config::from_env()` — the server's exact config path, so it also enforces
   `DATABASE_URL` and the strong-`JWT_SECRET` gate (or `KEEPLIN_DEV_INSECURE=1`).
5. `Cipher::from_key(at_rest_key)` — a malformed key is a contextual error; then
   `ensure!(cipher.enabled())` — an **unset** key is refused: "without it there is
   nothing to re-encrypt to". (The pass itself re-checks; this check gives the
   operator the clearer message earlier.)
6. A **2-connection pool** — the pass is a single sequential scan per column and
   must not compete with a live server for database capacity.
7. `reencrypt::run(&pool, &cipher, &opts)` — the actual pass.
8. Print one summary line per column — `found / rewritten / skipped (concurrent)`,
   prefixed `DRY RUN — ` when applicable — and exit 0.

Any error propagates through `anyhow` → non-zero exit.

**Dependencies** — `parse_args` (this file); `Config::from_env` (`config.rs`);
`Cipher::from_key`/`enabled` (`crypto.rs`); `reencrypt::run` (`reencrypt.rs`);
`sqlx::PgPoolOptions`, `dotenvy`, `tracing_subscriber` (external).

**Used by** — the operating system; nothing imports it.

**Repeated context** — The printed counters mirror `reencrypt::TableStats`
(`scanned`/`rewritten`/`skipped_concurrent`); `scanned == 0` on a re-run is the
"migration complete" signal. `skipped (concurrent)` rows need no attention — the
live server already wrote them encrypted.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `parse_args()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `main()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/reencrypt.rs` — one-off at-rest re-encrypt pass (EXTRACTED: references×1; e.g. `Options`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

Every code block of `bin/reencrypt.rs`, in source order, each documented above (five
points) and carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `fn parse_args` | `// md:fn parse_args` | fn parse_args |
| 3 | `fn main` | `// md:fn main` | fn main |
