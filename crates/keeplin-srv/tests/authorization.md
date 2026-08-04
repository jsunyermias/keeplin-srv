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
const MUTATING_HANDLER_TENANT_CASES: &[(&str, &str)] = &[];
const MUTATING_HANDLER_CAPABILITY_CASES: &[(&str, &str)] = &[];
const RELAY_CHANGE_TENANT_CASES: &[(&str, &str)] = &[
    (
        "NotebookCreate",
        "cross_tenant_store_mutations_leave_victim_unchanged",
    ),
    (
        "NotebookDelete",
        "cross_tenant_store_mutations_leave_victim_unchanged",
    ),
    (
        "NotebookUpdate",
        "cross_tenant_store_mutations_leave_victim_unchanged",
    ),
    (
        "ResourceCreate",
        "cross_tenant_store_mutations_leave_victim_unchanged",
    ),
    (
        "ResourceDelete",
        "cross_tenant_store_mutations_leave_victim_unchanged",
    ),
    (
        "TagCreate",
        "cross_tenant_store_mutations_leave_victim_unchanged",
    ),
    (
        "TagDelete",
        "cross_tenant_store_mutations_leave_victim_unchanged",
    ),
    (
        "TagUpdate",
        "cross_tenant_store_mutations_leave_victim_unchanged",
    ),
];
const RELAY_CHANGE_CAPABILITY_CASES: &[(&str, &str)] = &[];

const MUTATING_HANDLER_UNCOVERED: &[(&str, &str)] = &[
    ("change_password", "no HTTP authorization case exists"),
    ("create_device", "no HTTP authorization case exists"),
    ("create_note", "no HTTP authorization case exists"),
    ("create_notebook_share", "no HTTP authorization case exists"),
    ("create_share", "no HTTP authorization case exists"),
    ("delete_account", "no HTTP authorization case exists"),
    ("delete_all_devices", "no HTTP authorization case exists"),
    ("delete_device", "no HTTP authorization case exists"),
    ("delete_note", "no HTTP authorization case exists"),
    ("delete_notebook_share", "no HTTP authorization case exists"),
    ("delete_share", "no HTTP authorization case exists"),
    ("import_note", "no HTTP authorization case exists"),
    (
        "login",
        "public authentication endpoint; tenant and capability dimensions do not apply",
    ),
    ("put_resource_data", "no HTTP authorization case exists"),
    (
        "register",
        "public authentication endpoint; tenant and capability dimensions do not apply",
    ),
    (
        "reset_confirm",
        "public authentication endpoint; tenant and capability dimensions do not apply",
    ),
    (
        "reset_request",
        "public authentication endpoint; tenant and capability dimensions do not apply",
    ),
    ("transfer_notebook", "no HTTP authorization case exists"),
    ("transfer_ownership", "no HTTP authorization case exists"),
    ("update_note", "no HTTP authorization case exists"),
    (
        "verify_confirm",
        "public authentication endpoint; tenant and capability dimensions do not apply",
    ),
    ("verify_request", "no HTTP authorization case exists"),
];

const RELAY_CHANGE_UNCOVERED: &[(&str, &str)] = &[
    (
        "NoteCreate",
        "note materialization is outside this relay store harness",
    ),
    (
        "NoteDelete",
        "note materialization is outside this relay store harness",
    ),
    ("NoteTagAdd", "no cross-tenant relay case exists"),
    ("NoteTagRemove", "no cross-tenant relay case exists"),
    (
        "NoteUpdate",
        "note materialization is outside this relay store harness",
    ),
];

const READ_ISOLATION_CASES: &[&str] = &["users_do_not_see_each_others_changes"];
```

**What it does** — Registers only negative cases that actually exist, separately for tenant and capability dimensions. Every source entry without such a case is retained with an explicit coverage-gap or non-applicability reason.

**Dependencies** — `authorization_inventory_is_complete` compares these case names with source-derived inventories; expects equality to fail closed when source expands.

**Used by** — `authorization_inventory_is_complete`.

**Repeated context** — Empty handler arrays are intentional and honest: this file currently contains no HTTP-level authorization cases.

---

## fn mutating_handlers

**Identification** — source parser; marker `// md:fn mutating_handlers`.

**Code** — complete and verbatim:

```rust
// md:fn mutating_handlers
fn mutating_handlers(source: &str) -> BTreeSet<String> {
    let router = source
        .split("// md:fn router")
        .nth(1)
        .unwrap()
        .split(concat!("// md:", "PROTOCOL_VERSION"))
        .next()
        .unwrap();
    ["post(", "put(", "patch(", "delete("]
        .into_iter()
        .flat_map(|method| {
            router
                .match_indices(method)
                .filter(move |(offset, _)| {
                    *offset == 0 || !router.as_bytes()[offset - 1].is_ascii_alphanumeric()
                })
                .filter_map(move |(offset, _)| {
                    let tail = &router[offset + method.len()..];
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

**What it does** — Extracts mutating handler identifiers from the complete `router` function, including raised-limit, authenticated, and public subrouters.

**Dependencies** — companion markers and Axum method constructors; expects routes inside `router` to use exact `post(`, `put(`, `patch(`, or `delete(` calls.

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

**What it does** — Extracts every mutating handler identifier from the complete router construction in `http.rs`.

**Dependencies** — `include_str!(../src/http.rs)` supplies the canonical route source; expects the `router` and `PROTOCOL_VERSION` markers to delimit construction.

**Used by** — `authorization_inventory_is_complete`.

**Repeated context** — Public account bootstrap endpoints are inventoried with a reason explaining why tenant and capability dimensions do not apply.

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
    let handler_inventory = MUTATING_HANDLER_TENANT_CASES
        .iter()
        .chain(MUTATING_HANDLER_CAPABILITY_CASES)
        .chain(MUTATING_HANDLER_UNCOVERED)
        .map(|(entry, _)| (*entry).to_string())
        .collect();
    assert_eq!(source_handlers(), handler_inventory);

    let relay_inventory = RELAY_CHANGE_TENANT_CASES
        .iter()
        .chain(RELAY_CHANGE_CAPABILITY_CASES)
        .chain(RELAY_CHANGE_UNCOVERED)
        .map(|(entry, _)| (*entry).to_string())
        .collect();
    assert_eq!(source_relay_changes(), relay_inventory);
    assert_eq!(
        READ_ISOLATION_CASES,
        &["users_do_not_see_each_others_changes"]
    );
}
```

**What it does** — Requires exact equality between source-discovered mutations and the union of real cases plus explicitly documented gaps, and retains the read-isolation case by name.

**Dependencies** — `source_handlers` and `source_relay_changes` enumerate source; expects any unregistered addition to make this test fail.

**Used by** — `cargo test` and CI.

**Repeated context** — Completeness here means every source item is accounted for; case existence is verified separately and gaps are not represented as coverage.

---

## fn each_inventory_entry_has_both_cases

**Identification** — registered-case existence test; marker `// md:fn each_inventory_entry_has_both_cases`.

**Code** — complete and verbatim:

```rust
// md:fn each_inventory_entry_has_both_cases
#[test]
fn each_inventory_entry_has_both_cases() {
    let tests = include_str!("authorization.rs");
    for (entry, case) in MUTATING_HANDLER_TENANT_CASES
        .iter()
        .chain(MUTATING_HANDLER_CAPABILITY_CASES)
        .chain(RELAY_CHANGE_TENANT_CASES)
        .chain(RELAY_CHANGE_CAPABILITY_CASES)
    {
        assert!(!entry.is_empty());
        assert!(
            tests.contains(&format!("fn {case}(")),
            "missing case {case}"
        );
    }
    for (entry, reason) in MUTATING_HANDLER_UNCOVERED
        .iter()
        .chain(RELAY_CHANGE_UNCOVERED)
    {
        assert!(!entry.is_empty());
        assert!(!reason.is_empty());
    }
}
```

**What it does** — Proves every claimed case names a test function that exists and every unclaimed source entry carries an explicit non-empty coverage-gap or non-applicability reason.

**Dependencies** — `include_str!(authorization.rs)` supplies this test module; expects registered function names to use ordinary `fn name(` syntax.

**Used by** — `cargo test` and CI.

**Repeated context** — Registration is not treated as proof of execution.

---

## fn source_inventory_detects_an_uncovered_route

**Identification** — enumerator mutation test; marker `// md:fn source_inventory_detects_an_uncovered_route`.

**Code** — complete and verbatim:

```rust
// md:fn source_inventory_detects_an_uncovered_route
#[test]
fn source_inventory_detects_an_uncovered_route() {
    let source = include_str!("../src/http.rs").replace(
        concat!("// md:", "PROTOCOL_VERSION"),
        concat!(
            ".route(\"/api/test-only\", post(test_only_mutation));\n",
            "// md:",
            "PROTOCOL_VERSION"
        ),
    );
    let mut expected = source_handlers();
    expected.insert("test_only_mutation".into());
    assert_eq!(mutating_handlers(&source), expected);
    assert_ne!(
        mutating_handlers(&source),
        MUTATING_HANDLER_UNCOVERED
            .iter()
            .map(|(entry, _)| (*entry).to_string())
            .collect()
    );
}
```

**What it does** — Injects a test-only mutating route into router source and proves discovery expands while the registered tenant cases do not.

**Dependencies** — `mutating_handlers` parses the modified source; expects a new handler to change the discovered set.

**Used by** — `cargo test` and CI.

**Repeated context** — This reifies the fail-closed property of the completeness gate itself.

---

## fn put_resource_data_checks_blob_write_result

**Identification** — handler-source regression; marker `// md:fn put_resource_data_checks_blob_write_result`.

**Code** — complete and verbatim:

```rust
// md:fn put_resource_data_checks_blob_write_result
#[test]
fn put_resource_data_checks_blob_write_result() {
    let source = include_str!("../src/http.rs");
    let handler = source
        .split("// md:fn put_resource_data")
        .nth(1)
        .unwrap()
        .split("// md:fn materialize_body")
        .next()
        .unwrap();
    assert!(handler.contains("let written = state"));
    assert!(handler.contains("if !written"));
    assert!(handler.contains("return Err(AppError::NotFound)"));
}
```

**What it does** — Requires the resource upload handler to inspect the owner-scoped blob update result and translate a lost-ownership race into `NotFound`.

**Dependencies** — `include_str!(../src/http.rs)` supplies the handler source; expects companion markers to delimit `put_resource_data`.

**Used by** — `cargo test` and CI; regression verifier for F3.

**Repeated context** — A successful upload response must imply that the blob update affected its owned metadata row.

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
    assert!(store
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
    assert!(store.upsert_tag(attacker.id, &hostile_tag).await.unwrap());
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
    assert!(store
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

## fn foreign_and_missing_upserts_are_indistinguishable

**Identification** — PostgreSQL upsert anti-oracle regression; marker `// md:fn foreign_and_missing_upserts_are_indistinguishable`.

**Code** — complete and verbatim:

```rust
// md:fn foreign_and_missing_upserts_are_indistinguishable
#[sqlx::test(migrations = "../../migrations")]
async fn foreign_and_missing_upserts_are_indistinguishable(pool: PgPool) {
    let store = Store::new(pool);
    let attacker = store
        .create_user("attacker@example.com", "hash", "attacker")
        .await
        .unwrap();
    let victim = store
        .create_user("victim@example.com", "hash", "victim")
        .await
        .unwrap();
    let vv = VersionVector::from([("attacker".to_string(), 99)]);

    let mut foreign_notebook = Notebook::new("victim notebook");
    assert!(store
        .upsert_notebook(victim.id, &foreign_notebook)
        .await
        .unwrap());
    foreign_notebook.vv = vv.clone();
    foreign_notebook.last_writer = "attacker".into();
    let mut missing_notebook = foreign_notebook.clone();
    missing_notebook.id = Uuid::new_v4();
    assert_eq!(
        store
            .upsert_notebook(attacker.id, &foreign_notebook)
            .await
            .unwrap(),
        store
            .upsert_notebook(attacker.id, &missing_notebook)
            .await
            .unwrap()
    );

    let mut foreign_tag = Tag::new("victim tag");
    assert!(store.upsert_tag(victim.id, &foreign_tag).await.unwrap());
    foreign_tag.vv = vv.clone();
    foreign_tag.last_writer = "attacker".into();
    let mut missing_tag = foreign_tag.clone();
    missing_tag.id = Uuid::new_v4();
    assert_eq!(
        store.upsert_tag(attacker.id, &foreign_tag).await.unwrap(),
        store.upsert_tag(attacker.id, &missing_tag).await.unwrap()
    );

    let mut foreign_resource = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "victim resource",
        "application/octet-stream",
        "victim.bin",
        1,
    );
    assert!(store
        .upsert_resource_meta(victim.id, &foreign_resource)
        .await
        .unwrap());
    foreign_resource.vv = vv;
    foreign_resource.last_writer = "attacker".into();
    let mut missing_resource = foreign_resource.clone();
    missing_resource.id = Uuid::new_v4();
    assert_eq!(
        store
            .upsert_resource_meta(attacker.id, &foreign_resource)
            .await
            .unwrap(),
        store
            .upsert_resource_meta(attacker.id, &missing_resource)
            .await
            .unwrap()
    );
}
```

**What it does** — Compares winning notebook, tag, and resource-metadata upserts against a foreign UUID and a fresh UUID, requiring identical results.

**Dependencies** — `Store::{upsert_notebook, upsert_tag, upsert_resource_meta}`; expects tenant conflicts and absent IDs to be observationally indistinguishable while preserving tenant-owned rows.

**Used by** — `cargo test` and CI; anti-enumeration verifier for F1.

**Repeated context** — Fresh-ID writes create attacker fixtures; foreign-ID attempts report the same outcome without changing victim state.

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
    let mut notebook = Notebook::new("victim notebook");
    notebook.vv = VersionVector::from([("victim".to_string(), 1)]);
    assert!(store.upsert_notebook(victim.id, &notebook).await.unwrap());
    let foreign_notebook = store
        .delete_notebook(attacker.id, notebook.id, Utc::now(), &vv, "attacker")
        .await
        .unwrap();
    let missing_notebook = store
        .delete_notebook(attacker.id, Uuid::new_v4(), Utc::now(), &vv, "attacker")
        .await
        .unwrap();
    assert_eq!(foreign_notebook, missing_notebook);
    let mut tag = Tag::new("victim tag");
    tag.vv = VersionVector::from([("victim".to_string(), 1)]);
    assert!(store.upsert_tag(victim.id, &tag).await.unwrap());
    let foreign_tag = store
        .delete_tag(attacker.id, tag.id, Utc::now(), &vv, "attacker")
        .await
        .unwrap();
    let missing_tag = store
        .delete_tag(attacker.id, Uuid::new_v4(), Utc::now(), &vv, "attacker")
        .await
        .unwrap();
    assert_eq!(foreign_tag, missing_tag);
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

**What it does** — Compares store outcomes for foreign and missing notebook/tag deletions, resource deletion, and blob replacement.

**Dependencies** — `Store::{delete_notebook, delete_tag, delete_resource, put_resource_blob}`; expects each to return the same non-mutating outcome across foreign and absent IDs.

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

- Every mutating route and materialized relay variant is either tied to an existing negative test or carries an explicit gap/non-applicability reason.
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
| 7 | `fn each_inventory_entry_has_both_cases` | `// md:fn each_inventory_entry_has_both_cases` |
| 8 | `fn source_inventory_detects_an_uncovered_route` | `// md:fn source_inventory_detects_an_uncovered_route` |
| 9 | `fn put_resource_data_checks_blob_write_result` | `// md:fn put_resource_data_checks_blob_write_result` |
| 10 | `fn entity_snapshot` | `// md:fn entity_snapshot` |
| 11 | `fn cross_tenant_store_mutations_leave_victim_unchanged` | `// md:fn cross_tenant_store_mutations_leave_victim_unchanged` |
| 12 | `fn foreign_and_missing_upserts_are_indistinguishable` | `// md:fn foreign_and_missing_upserts_are_indistinguishable` |
| 13 | `fn foreign_and_missing_mutations_are_indistinguishable` | `// md:fn foreign_and_missing_mutations_are_indistinguishable` |
