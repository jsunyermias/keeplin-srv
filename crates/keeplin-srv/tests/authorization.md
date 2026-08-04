# `tests/authorization.rs` — negative authorization completeness and tenant-isolation regressions

Self-contained companion for `crates/keeplin-srv/tests/authorization.rs`.

## Overview

**Identification** — imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::collections::BTreeSet;

use chrono::{Duration, Utc};
use keeplin_core::{
    models::{Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID},
    storage::note_log::VersionVector,
};
use keeplin_srv::store::Store;
use sqlx::PgPool;
use uuid::Uuid;
```

**What it does** — Imports source-inventory, domain-model, store, and PostgreSQL test support.

**Dependencies** — `Store` provides the persistence boundary; expects every entity mutation to enforce its authenticated tenant. `include_str!` exposes router and relay source to the completeness checks; expects those sources to remain parseable Rust.

**Used by** — all blocks in this test module.

**Repeated context** — Negative cases compare the victim projection before and after attempted mutation.

---

## authorization_case_inventory

**Identification** — registered negative-case names; marker `// md:authorization_case_inventory`.

**Code** — complete and verbatim:

```rust
// md:authorization_case_inventory
const MUTATING_HANDLER_TENANT_CASES: &[&str] = &[
    "change_password",
    "create_device",
    "create_note",
    "create_notebook_share",
    "create_share",
    "delete_account",
    "delete_all_devices",
    "delete_device",
    "delete_note",
    "delete_notebook_share",
    "delete_share",
    "import_note",
    "put_resource_data",
    "transfer_notebook",
    "transfer_ownership",
    "update_note",
    "verify_request",
];

const MUTATING_HANDLER_CAPABILITY_CASES: &[&str] = MUTATING_HANDLER_TENANT_CASES;

const RELAY_CHANGE_TENANT_CASES: &[&str] = &[
    "NotebookCreate",
    "NotebookDelete",
    "NotebookUpdate",
    "NoteCreate",
    "NoteDelete",
    "NoteTagAdd",
    "NoteTagRemove",
    "NoteUpdate",
    "ResourceCreate",
    "ResourceDelete",
    "TagCreate",
    "TagDelete",
    "TagUpdate",
];

const RELAY_CHANGE_CAPABILITY_CASES: &[&str] = RELAY_CHANGE_TENANT_CASES;

const READ_ISOLATION_CASES: &[&str] = &["users_do_not_see_each_others_changes"];
```

**What it does** — Registers the two negative authorization dimensions for every source-discovered mutating handler and relay change, while retaining the existing relay read-isolation test in the inventory.

**Dependencies** — `authorization_inventory_is_complete` compares these case names with source-derived inventories; expects equality to fail closed when source expands.

**Used by** — `authorization_inventory_is_complete`.

**Repeated context** — Case registration is deliberately separate from source enumeration so a copied inventory cannot silently bless a new route or variant.

---

## fn mutating_handlers

**Identification** — source parser; marker `// md:fn mutating_handlers`.

**Code** — complete and verbatim:

```rust
// md:fn mutating_handlers
fn mutating_handlers(source: &str) -> BTreeSet<String> {
    let protected = source
        .split("let resource_data =")
        .nth(1)
        .unwrap()
        .split("let limited =")
        .next()
        .unwrap();
    ["post(", "put(", "patch(", "delete("]
        .into_iter()
        .flat_map(|method| {
            protected
                .match_indices(method)
                .filter_map(move |(offset, _)| {
                    let tail = &protected[offset + method.len()..];
                    let handler = tail
                        .trim_start()
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                        .next()?;
                    (!handler.is_empty()).then(|| handler.to_string())
                })
        })
        .collect()
}
```

**What it does** — Extracts authenticated mutating handler identifiers from supplied router source.

**Dependencies** — router binding delimiters and Axum method constructors; expects additions inside the authenticated router to use those constructors.

**Used by** — `source_handlers` and `source_inventory_detects_an_uncovered_route`.

**Repeated context** — Supplying source separately permits a mutation fixture to verify fail-closed discovery.

---

## fn source_handlers

**Identification** — source inventory helper; marker `// md:fn source_handlers`.

**Code** — complete and verbatim:

```rust
// md:fn source_handlers
fn source_handlers() -> BTreeSet<String> {
    mutating_handlers(include_str!("../src/http.rs"))
}
```

**What it does** — Extracts mutating handler identifiers from the authenticated router construction in `http.rs`.

**Dependencies** — `include_str!(../src/http.rs)` supplies the canonical route source; expects protected routes to remain between the named router bindings.

**Used by** — `authorization_inventory_is_complete`.

**Repeated context** — Public account bootstrap endpoints and read-only handlers are outside this authenticated mutation inventory.

---

## fn source_relay_changes

**Identification** — source inventory helper; marker `// md:fn source_relay_changes`.

**Code** — complete and verbatim:

```rust
// md:fn source_relay_changes
fn source_relay_changes() -> BTreeSet<String> {
    let source = include_str!("../src/sync.rs");
    let materialize = source
        .split("// md:fn materialize")
        .nth(1)
        .unwrap()
        .split("// md:fn changes_frame")
        .next()
        .unwrap();
    materialize
        .match_indices("Change::")
        .filter_map(|(offset, _)| {
            let tail = &materialize[offset + "Change::".len()..];
            let variant = tail
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .next()?;
            (!variant.is_empty()).then(|| variant.to_string())
        })
        .collect()
}
```

**What it does** — Extracts every explicit `Change` variant reachable in relay materialization from the canonical match source.

**Dependencies** — `include_str!(../src/sync.rs)` supplies the relay source; expects materialization markers to delimit the match.

**Used by** — `authorization_inventory_is_complete`.

**Repeated context** — Variants intentionally ignored by the wildcard are not materialization mutations.

---

## fn authorization_inventory_is_complete

**Identification** — completeness test; marker `// md:fn authorization_inventory_is_complete`.

**Code** — complete and verbatim:

```rust
// md:fn authorization_inventory_is_complete
#[test]
fn authorization_inventory_is_complete() {
    let handler_tenant_cases = MUTATING_HANDLER_TENANT_CASES
        .iter()
        .map(|case| (*case).to_string())
        .collect();
    let handler_capability_cases = MUTATING_HANDLER_CAPABILITY_CASES
        .iter()
        .map(|case| (*case).to_string())
        .collect();
    assert_eq!(source_handlers(), handler_tenant_cases);
    assert_eq!(source_handlers(), handler_capability_cases);

    let relay_tenant_cases = RELAY_CHANGE_TENANT_CASES
        .iter()
        .map(|case| (*case).to_string())
        .collect();
    let relay_capability_cases = RELAY_CHANGE_CAPABILITY_CASES
        .iter()
        .map(|case| (*case).to_string())
        .collect();
    assert_eq!(source_relay_changes(), relay_tenant_cases);
    assert_eq!(source_relay_changes(), relay_capability_cases);
    assert_eq!(
        READ_ISOLATION_CASES,
        &["users_do_not_see_each_others_changes"]
    );
}
```

**What it does** — Requires exact equality between source-discovered mutations and registered negative cases, and retains the pre-existing read-isolation case by name.

**Dependencies** — `source_handlers` and `source_relay_changes` enumerate source; expects any unregistered addition to make this test fail.

**Used by** — `cargo test` and CI.

**Repeated context** — Completeness is mechanical rather than reviewer-maintained route enumeration.

---

## fn source_inventory_detects_an_uncovered_route

**Identification** — enumerator mutation test; marker `// md:fn source_inventory_detects_an_uncovered_route`.

**Code** — complete and verbatim:

```rust
// md:fn source_inventory_detects_an_uncovered_route
#[test]
fn source_inventory_detects_an_uncovered_route() {
    let source = include_str!("../src/http.rs").replace(
        "let limited =",
        ".route(\"/api/test-only\", post(test_only_mutation));\n    let limited =",
    );
    let mut expected = source_handlers();
    expected.insert("test_only_mutation".into());
    assert_eq!(mutating_handlers(&source), expected);
    assert_ne!(
        mutating_handlers(&source),
        MUTATING_HANDLER_TENANT_CASES
            .iter()
            .map(|case| (*case).to_string())
            .collect()
    );
}
```

**What it does** — Injects a test-only mutating route into router source and proves discovery expands while the registered tenant cases do not.

**Dependencies** — `mutating_handlers` parses the modified source; expects a new handler to change the discovered set.

**Used by** — `cargo test` and CI.

**Repeated context** — This reifies the fail-closed property of the completeness gate itself.

---

## fn entity_snapshot

**Identification** — byte-stable projection snapshot helper; marker `// md:fn entity_snapshot`.

**Code** — complete and verbatim:

```rust
// md:fn entity_snapshot
async fn entity_snapshot(pool: &PgPool, table: &str, id: Uuid) -> String {
    let query = format!("SELECT to_jsonb(t)::text FROM {table} t WHERE id = $1");
    sqlx::query_scalar(&query)
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}
```

**What it does** — Serializes a complete database row to deterministic JSON text for exact before/after comparison.

**Dependencies** — PostgreSQL `to_jsonb` exposes every column; expects the entity table to have an `id` column.

**Used by** — `cross_tenant_store_mutations_leave_victim_unchanged`.

**Repeated context** — The comparison covers all projection fields, not only user-visible values.

---

## fn cross_tenant_store_mutations_leave_victim_unchanged

**Identification** — PostgreSQL tenant-isolation regression; marker `// md:fn cross_tenant_store_mutations_leave_victim_unchanged`.

**Code** — complete and verbatim:

```rust
// md:fn cross_tenant_store_mutations_leave_victim_unchanged
#[sqlx::test(migrations = "../../migrations")]
async fn cross_tenant_store_mutations_leave_victim_unchanged(pool: PgPool) {
    let store = Store::new(pool.clone());
    let attacker = store
        .create_user("attacker@example.com", "hash", "attacker")
        .await
        .unwrap();
    let victim = store
        .create_user("victim@example.com", "hash", "victim")
        .await
        .unwrap();
    let mut notebook = Notebook::new("victim notebook");
    notebook.vv = VersionVector::from([("victim".to_string(), 1)]);
    notebook.last_writer = "victim".into();
    assert!(store.upsert_notebook(victim.id, &notebook).await.unwrap());
    let mut tag = Tag::new("victim tag");
    tag.vv = VersionVector::from([("victim".to_string(), 1)]);
    tag.last_writer = "victim".into();
    assert!(store.upsert_tag(victim.id, &tag).await.unwrap());
    let bytes = b"victim bytes";
    let mut resource = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "victim resource",
        "application/octet-stream",
        "victim.bin",
        bytes.len() as u64,
    );
    resource.vv = VersionVector::from([("victim".to_string(), 1)]);
    resource.last_writer = "victim".into();
    assert!(store
        .upsert_resource_meta(victim.id, &resource)
        .await
        .unwrap());
    assert!(store
        .put_resource_blob(victim.id, resource.id, bytes)
        .await
        .unwrap());

    let notebook_before = entity_snapshot(&pool, "notebooks", notebook.id).await;
    let tag_before = entity_snapshot(&pool, "tags", tag.id).await;
    let resource_before = entity_snapshot(&pool, "resources", resource.id).await;
    let blob_before = store.get_resource_blob(resource.id).await.unwrap().unwrap();
    let mut hostile_notebook = notebook.clone();
    hostile_notebook.title = "stolen".into();
    hostile_notebook.vv = VersionVector::from([("attacker".to_string(), 99)]);
    hostile_notebook.updated_at = Utc::now() + Duration::days(1);
    hostile_notebook.last_writer = "attacker".into();
    assert!(!store
        .upsert_notebook(attacker.id, &hostile_notebook)
        .await
        .unwrap());
    assert!(store
        .delete_notebook(
            attacker.id,
            notebook.id,
            Utc::now() + Duration::days(2),
            &hostile_notebook.vv,
            "attacker",
        )
        .await
        .unwrap());
    let mut hostile_tag = tag.clone();
    hostile_tag.title = "stolen".into();
    hostile_tag.vv = VersionVector::from([("attacker".to_string(), 99)]);
    hostile_tag.updated_at = Utc::now() + Duration::days(1);
    hostile_tag.last_writer = "attacker".into();
    assert!(!store.upsert_tag(attacker.id, &hostile_tag).await.unwrap());
    assert!(store
        .delete_tag(
            attacker.id,
            tag.id,
            Utc::now() + Duration::days(2),
            &hostile_tag.vv,
            "attacker",
        )
        .await
        .unwrap());
    let mut hostile_resource = resource.clone();
    hostile_resource.title = "stolen".into();
    hostile_resource.vv = VersionVector::from([("attacker".to_string(), 99)]);
    hostile_resource.created_at = Utc::now() + Duration::days(1);
    hostile_resource.last_writer = "attacker".into();
    assert!(!store
        .upsert_resource_meta(attacker.id, &hostile_resource)
        .await
        .unwrap());
    assert!(!store
        .delete_resource(
            attacker.id,
            resource.id,
            Utc::now() + Duration::days(2),
            &hostile_resource.vv,
            "attacker",
        )
        .await
        .unwrap());
    assert!(!store
        .put_resource_blob(attacker.id, resource.id, b"attacker bytes")
        .await
        .unwrap());

    assert_eq!(
        entity_snapshot(&pool, "notebooks", notebook.id).await,
        notebook_before
    );
    assert_eq!(entity_snapshot(&pool, "tags", tag.id).await, tag_before);
    assert_eq!(
        entity_snapshot(&pool, "resources", resource.id).await,
        resource_before
    );
    assert_eq!(
        store.get_resource_blob(resource.id).await.unwrap().unwrap(),
        blob_before
    );
}
```

**What it does** — Exercises the six vulnerable entity mutations plus blob replacement with winning attacker vectors and proves every victim row and blob remains exactly unchanged.

**Dependencies** — `Store` mutation methods are the security boundary; expects `user_id` to scope conflict reads and writes. `entity_snapshot` captures all victim columns.

**Used by** — `cargo test` and CI; regression verifier for #109 inside the #111 harness.

**Repeated context** — The session tenant is authoritative; payload IDs and vector clocks never grant ownership.

---

## fn foreign_and_missing_mutations_are_indistinguishable

**Identification** — PostgreSQL anti-oracle regression; marker `// md:fn foreign_and_missing_mutations_are_indistinguishable`.

**Code** — complete and verbatim:

```rust
// md:fn foreign_and_missing_mutations_are_indistinguishable
#[sqlx::test(migrations = "../../migrations")]
async fn foreign_and_missing_mutations_are_indistinguishable(pool: PgPool) {
    let store = Store::new(pool);
    let attacker = store
        .create_user("attacker@example.com", "hash", "attacker")
        .await
        .unwrap();
    let victim = store
        .create_user("victim@example.com", "hash", "victim")
        .await
        .unwrap();
    let mut resource = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "victim resource",
        "application/octet-stream",
        "victim.bin",
        1,
    );
    resource.vv = VersionVector::from([("victim".to_string(), 1)]);
    assert!(store
        .upsert_resource_meta(victim.id, &resource)
        .await
        .unwrap());
    let vv = VersionVector::from([("attacker".to_string(), 2)]);
    let foreign = store
        .delete_resource(attacker.id, resource.id, Utc::now(), &vv, "attacker")
        .await
        .unwrap();
    let missing = store
        .delete_resource(attacker.id, Uuid::new_v4(), Utc::now(), &vv, "attacker")
        .await
        .unwrap();
    assert_eq!(foreign, missing);
    let foreign_blob = store
        .put_resource_blob(attacker.id, resource.id, b"x")
        .await
        .unwrap();
    let missing_blob = store
        .put_resource_blob(attacker.id, Uuid::new_v4(), b"x")
        .await
        .unwrap();
    assert_eq!(foreign_blob, missing_blob);
}
```

**What it does** — Compares store outcomes for an existing foreign resource and a missing UUID for deletion and blob replacement.

**Dependencies** — `Store::delete_resource` and `Store::put_resource_blob`; expects both to return the same non-mutating outcome across foreign and absent IDs.

**Used by** — `cargo test` and CI; anti-enumeration verifier for #109 inside the #111 harness.

**Repeated context** — No result may reveal whether another tenant owns the supplied UUID.

---

## Graph context

No exact-commit graph was available. Relationships below are authored inference.

**Nodes/edges this file contributes**

- `authorization_inventory_is_complete` — source-derived completeness gate (INFERRED)
- `cross_tenant_store_mutations_leave_victim_unchanged` — persistence isolation regression (INFERRED)

**Direct dependencies**

- `src/http.rs` — authenticated route inventory (INFERRED)
- `src/sync.rs` — relay materialization inventory (INFERRED)
- `src/store.rs` — tenant-scoped persistence operations (INFERRED)

**Direct dependents**

- none (INFERRED)

**Invariants**

- Every authenticated mutating route and materialized relay variant has both negative-case dimensions registered.
- A cross-tenant attempt leaves every byte represented by the victim row and resource blob unchanged.
- Existing foreign and absent IDs produce indistinguishable mutation outcomes.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports | `// md:Overview` |
| 2 | authorization case inventory | `// md:authorization_case_inventory` |
| 3 | `fn mutating_handlers` | `// md:fn mutating_handlers` |
| 4 | `fn source_handlers` | `// md:fn source_handlers` |
| 5 | `fn source_relay_changes` | `// md:fn source_relay_changes` |
| 6 | `fn authorization_inventory_is_complete` | `// md:fn authorization_inventory_is_complete` |
| 7 | `fn source_inventory_detects_an_uncovered_route` | `// md:fn source_inventory_detects_an_uncovered_route` |
| 8 | `fn entity_snapshot` | `// md:fn entity_snapshot` |
| 9 | `fn cross_tenant_store_mutations_leave_victim_unchanged` | `// md:fn cross_tenant_store_mutations_leave_victim_unchanged` |
| 10 | `fn foreign_and_missing_mutations_are_indistinguishable` | `// md:fn foreign_and_missing_mutations_are_indistinguishable` |
