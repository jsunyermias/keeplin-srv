# `crates/keeplin-srv/src/bin/reconcile-projections.rs` — durable projection queue support

Self-contained companion for `crates/keeplin-srv/src/bin/reconcile-projections.rs`.

## Overview

**Identification** — source block; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use keeplin_srv::{config::Config, projection, state::AppState};
use uuid::Uuid;
```

**What it does** — Implements the named durable-projection responsibility while preserving queue lifecycle and tenant scoping.

**Dependencies** — Exact dependencies are visible in the complete code; expects PostgreSQL transactions and keeplin-core models to preserve their documented contracts.

**Used by** — The in-process worker, synchronization ingress, metrics endpoint, or operator reconciliation command as applicable.

**Repeated context** — Jobs reference journal rows; successful jobs are removed and failures remain observable.

---

## fn main

**Identification** — source block; marker `// md:fn main`.

**Code** — complete and verbatim:

```rust
// md:fn main
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env();
    let mut user = None;
    let mut from = None;
    let mut to = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let value = arguments.next().context("every option requires a value")?;
        match argument.as_str() {
            "--user" => user = Some(value.parse::<Uuid>().context("invalid --user UUID")?),
            "--from" => {
                from = Some(
                    value
                        .parse::<DateTime<Utc>>()
                        .context("invalid --from timestamp")?,
                )
            }
            "--to" => {
                to = Some(
                    value
                        .parse::<DateTime<Utc>>()
                        .context("invalid --to timestamp")?,
                )
            }
            _ => anyhow::bail!("unknown option {argument}"),
        }
    }
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .acquire_timeout(Duration::from_secs(config.db_acquire_timeout_secs))
        .connect(&config.database_url)
        .await
        .context("connect to PostgreSQL")?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    let state = AppState::new(config, pool);
    let queued = projection::reconcile(&state.store, user, from, to).await?;
    projection::drain_available(&state, user, usize::MAX).await;
    let stats = projection::stats(&state.store).await?;
    println!(
        "queued={queued} outstanding={} retrying={} dead_lettered={}",
        stats.outstanding, stats.retrying, stats.dead_lettered
    );
    Ok(())
}
```

**What it does** — Implements the named durable-projection responsibility while preserving queue lifecycle and tenant scoping.

**Dependencies** — Exact dependencies are visible in the complete code; expects PostgreSQL transactions and keeplin-core models to preserve their documented contracts.

**Used by** — The in-process worker, synchronization ingress, metrics endpoint, or operator reconciliation command as applicable.

**Repeated context** — Jobs reference journal rows; successful jobs are removed and failures remain observable.

---

## Graph context

Graph output was unavailable; relationships below are authored.

**Nodes/edges this file contributes**

- Durable projection queue lifecycle (INFERRED)

**Direct dependencies**

- PostgreSQL and `keeplin-core::models::Change` (INFERRED)

**Direct dependents**

- Synchronization ingress and server startup (INFERRED)

**Invariants**

- A journaled change retains an outstanding or dead-lettered job until projection succeeds.
- A database row lock excludes concurrent application of one job.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `fn main` | `// md:fn main` |
