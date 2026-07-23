# `tests/collab_client_resources_e2e.rs` — out-of-band resource blob e2e (real client)

Self-contained companion for
`crates/keeplin-srv/tests/collab_client_resources_e2e.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be
able to understand the test binary without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

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
use keeplin_core::{
    models::{Resource, SYSTEM_RESOURCE_NOTE_ID},
    storage::ResourceRepository,
    storage::SyncBackend,
};
use sqlx::{PgPool, Row};
```

**What it does** — Real-client e2e binary for the **out-of-band resource binary
path**. The contract under test (client side in keeplin's `collab/mod.rs`):
(1) `create_resource` eagerly relays the **blob-stripped** `ResourceCreate` over
`/api/sync`, then `PUT`s the binary to `/api/resources/:id/data` (with a short retry
while the server materialises the metadata — blob uploads for unknown resources are
404); (2) the relay journal **never carries the binary** — every journaled
`ResourceCreate` has `data` absent/null; (3) a second device receives only metadata
over the relay and fetches the bytes through the client's server-download fallback
(`read_resource` → `GET …/data`). Own test binary (issue #51).

**Dependencies** — the shared harness; keeplin-core `Resource`,
`ResourceRepository`, `SyncBackend` (`receive_changes`/`apply_change`); raw sqlx for
the journal-payload assertion; `reqwest`.

**Used by** — `cargo test`; CI.

**Repeated context** — The blob/metadata split (issue #24 storage model + the
out-of-band upload): metadata is a versioned, soft-deletable entity that rides the
relay and materialises server-side; the bytes are a separate, quota-checked
`resource_blobs` row that never enters the journal — keeping journal rows small and
prunable.

---

## fn resource_blob_travels_out_of_band_through_the_real_client

**Identification** — `#[sqlx::test]` async test; marker
`// md:fn resource_blob_travels_out_of_band_through_the_real_client`.

**Code** — complete and verbatim:

```rust
// md:fn resource_blob_travels_out_of_band_through_the_real_client
#[sqlx::test(migrations = "../../migrations")]
async fn resource_blob_travels_out_of_band_through_the_real_client(pool: PgPool) {
    let addr = spawn_server(pool.clone()).await;
    register(addr, "a@example.com").await;
    let token_a = login(addr, "a@example.com", "dev-a").await;
    let a = collab_device(addr, &token_a).await;

    let bytes: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let meta = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "photo",
        "image/png",
        "photo.png",
        bytes.len() as u64,
    );
    let created = a.create_resource(meta, bytes.clone()).await.unwrap();

    let client = reqwest::Client::new();
    let mut served = Vec::new();
    for _ in 0..CONVERGE_TRIES {
        if let Ok(resp) = client
            .get(format!("http://{addr}/api/resources/{}/data", created.id))
            .bearer_auth(&token_a)
            .send()
            .await
        {
            if resp.status().is_success() {
                served = resp.bytes().await.unwrap().to_vec();
                if served == bytes {
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(served, bytes, "server must serve the out-of-band blob");

    let rows = sqlx::query(
        "SELECT payload FROM changes WHERE payload->>'op' = 'resource_create'
           AND payload->'resource'->>'id' = $1",
    )
    .bind(created.id.to_string())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        !rows.is_empty(),
        "the metadata change must have been relayed"
    );
    for row in rows {
        let payload: serde_json::Value = row.get("payload");
        assert!(
            payload.get("data").is_none_or(|d| d.is_null()),
            "the relayed ResourceCreate must not carry the binary: {payload}"
        );
    }

    let b = collab_device(addr, &login(addr, "a@example.com", "dev-b").await).await;
    let mut fetched = Vec::new();
    for _ in 0..CONVERGE_TRIES {
        let incoming = b.receive_changes().await.unwrap();
        for change in incoming {
            b.apply_change(change).await.unwrap();
        }
        if let Ok((_, data)) = b.read_resource(created.id).await {
            fetched = data;
            if fetched == bytes {
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        fetched, bytes,
        "a second device must fetch the blob from the server (it never rode the relay)"
    );
}
```

**What it does** — Device A creates a 4 KiB resource through the real collab stack.
Assertions, in order: (1) polling `GET /api/resources/:id/data` with A's token, the
server eventually serves **exactly** the created bytes; (2) raw SQL over
`changes.payload` finds at least one journaled `resource_create` for the id and
**every** such payload has `data` absent or null — the relay stayed blob-free;
(3) a second device B (same account) pumps `receive_changes`/`apply_change` until
`read_resource` returns exactly the bytes — obtained via the client's
server-download fallback, since the relay never carried them. All polls bounded by
`CONVERGE_TRIES`.

**Dependencies** — harness helpers `spawn_server`, `register`, `login`,
`collab_device`, `CONVERGE_TRIES`; keeplin-core resource APIs; sqlx row access.

**Used by** — `cargo test`.

**Repeated context** — Asserting on the **raw journal payloads** (not through any
API) is deliberate: it pins the wire-level guarantee that binaries never ride the
relay, independent of client behaviour.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `resource_blob_travels_out_of_band_through_the_real_client()` — defined here (EXTRACTED; 1 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries (EXTRACTED: calls×1; e.g. `collab_device()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `mod common` + imports | `// md:Overview` |
| 2 | `fn resource_blob_travels_out_of_band_through_the_real_client` | `// md:fn resource_blob_travels_out_of_band_through_the_real_client` |
