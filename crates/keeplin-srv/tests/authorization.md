# `tests/authorization.rs` — negative authorization completeness and tenant-isolation regressions

Self-contained companion for `crates/keeplin-srv/tests/authorization.rs`.

## Overview

**Identification** — imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::{
    collections::BTreeSet,
    io::Write,
    net::SocketAddr,
    process::Command,
    sync::{Arc, Mutex},
};

use axum::{body::Body, extract::ConnectInfo, http::Request, Json, Router};
use chrono::{Duration, Utc};
use keeplin_core::{
    models::{Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID},
    storage::note_log::VersionVector,
};
use keeplin_srv::{
    config::{Config, PermissionScheme},
    http::router,
    permissions::{
        resolve_note_access, resolve_note_access_on, resolve_notebook_access_on, Capabilities,
    },
    state::AppState,
    store::{NotePatch, Store},
};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;
use uuid::Uuid;
```

**What it does** — Imports source-inventory, domain-model, store, and PostgreSQL test support.

**Dependencies** — `Store` provides the persistence boundary; expects every entity mutation to enforce its authenticated tenant. `include_str!` exposes router and relay source to the completeness checks; expects those sources to remain parseable Rust.

**Used by** — all blocks in this test module.

**Repeated context** — Negative cases compare the victim projection before and after attempted mutation.

---

## CapturedLogs

**Identification** — cloneable in-memory tracing writer; marker `// md:CapturedLogs`.

**Code** — complete and verbatim:

```rust
// md:CapturedLogs
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);
```

**What it does** — Shares captured tracing bytes across writer instances and the asserting test.

**Dependencies** — `Arc<Mutex<Vec<u8>>>` — retains emitted bytes safely; expects poisoned locks to fail the test.

**Used by** — `capturing_subscriber` and log-observability tests.

**Repeated context** — Test-only diagnostics are asserted after the awaited operation.

---

## impl Write for CapturedLogs

**Identification** — tracing output writer; marker `// md:impl Write for CapturedLogs`.

**Code** — complete and verbatim:

```rust
// md:impl Write for CapturedLogs
impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
```

**What it does** — Appends every formatted event and treats flushing as complete because storage is memory-resident.

**Dependencies** — `std::io::Write` — tracing formatter sink contract; expects complete buffer appends.

**Used by** — `tracing_subscriber::fmt`.

**Repeated context** — none.

---

## impl CapturedLogs

**Identification** — captured-text accessor; marker `// md:impl CapturedLogs`.

**Code** — complete and verbatim:

```rust
// md:impl CapturedLogs
impl CapturedLogs {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}
```

**What it does** — Snapshots captured UTF-8 output for assertions.

**Dependencies** — `String::from_utf8` — decodes formatter output; expects tracing to emit UTF-8.

**Used by** — revocation-failure and migration-report tests.

**Repeated context** — none.

---

## fn capturing_subscriber

**Identification** — tracing capture factory; marker `// md:fn capturing_subscriber`.

**Code** — complete and verbatim:

```rust
// md:fn capturing_subscriber
fn capturing_subscriber() -> (impl tracing::Subscriber + Send + Sync, CapturedLogs) {
    let logs = CapturedLogs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    (subscriber, logs)
}
```

**What it does** — Builds an ANSI-free subscriber accepting all levels and returns its shared capture.

**Dependencies** — `tracing_subscriber::fmt` — formats structured events; expects custom writers to receive SQLx notices and application warnings.

**Used by** — ADR 0001 verification rows 14 and 22.

**Repeated context** — The subscriber is attached to the exact future or server task being observed.

---

## authorization_case_inventory

**Identification** — registered negative-case names; marker `// md:authorization_case_inventory`.

**Code** — complete and verbatim:

```rust
// md:authorization_case_inventory
const MUTATING_HANDLER_TENANT_CASES: &[(&str, &str)] = &[
    (
        "change_password",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "create_device",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "create_note",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "create_notebook_share",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "create_share",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "delete_account",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "delete_all_devices",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "delete_device",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "delete_note",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "delete_notebook_share",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "delete_share",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "import_note",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "put_resource_data",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "transfer_notebook",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "transfer_ownership",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "update_note",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
    (
        "verify_request",
        "cross_tenant_http_mutations_leave_victim_unchanged",
    ),
];
const MUTATING_HANDLER_CAPABILITY_CASES: &[(&str, &str)] = &[
    (
        "create_notebook_share",
        "denied_http_capabilities_leave_entities_unchanged",
    ),
    (
        "create_share",
        "denied_http_capabilities_leave_entities_unchanged",
    ),
    (
        "delete_note",
        "denied_http_capabilities_leave_entities_unchanged",
    ),
    (
        "delete_notebook_share",
        "denied_http_capabilities_leave_entities_unchanged",
    ),
    (
        "delete_share",
        "denied_http_capabilities_leave_entities_unchanged",
    ),
    (
        "transfer_notebook",
        "denied_http_capabilities_leave_entities_unchanged",
    ),
    (
        "transfer_ownership",
        "denied_http_capabilities_leave_entities_unchanged",
    ),
    (
        "update_note",
        "denied_http_capabilities_leave_entities_unchanged",
    ),
];
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
    (
        "NoteTagAdd",
        "cross_tenant_store_mutations_leave_victim_unchanged",
    ),
    (
        "NoteTagRemove",
        "cross_tenant_store_mutations_leave_victim_unchanged",
    ),
    (
        "NoteCreate",
        "note_changes_are_explicitly_non_materializing",
    ),
    (
        "NoteDelete",
        "note_changes_are_explicitly_non_materializing",
    ),
    (
        "NoteUpdate",
        "note_changes_are_explicitly_non_materializing",
    ),
];
const RELAY_CHANGE_CAPABILITY_CASES: &[(&str, &str)] = &[];

const MUTATING_HANDLER_TENANT_UNCOVERED: &[(&str, &str)] = &[
    (
        "login",
        "public authentication endpoint; tenant and capability dimensions do not apply",
    ),
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
    (
        "verify_confirm",
        "public authentication endpoint; tenant and capability dimensions do not apply",
    ),
];

const MUTATING_HANDLER_CAPABILITY_UNCOVERED: &[(&str, &str)] = &[
    (
        "change_password",
        "account credentials have no delegated-access capability model",
    ),
    (
        "create_device",
        "devices have no delegated-access capability model",
    ),
    (
        "create_note",
        "creating an own note has no pre-existing shared entity or capability",
    ),
    (
        "delete_account",
        "accounts have no delegated-access capability model",
    ),
    (
        "delete_all_devices",
        "devices have no delegated-access capability model",
    ),
    (
        "delete_device",
        "devices have no delegated-access capability model",
    ),
    (
        "import_note",
        "import creates an own note and has no delegated capability target",
    ),
    (
        "put_resource_data",
        "resource blobs are owner-scoped and cannot be delegated",
    ),
    (
        "verify_request",
        "email verification has no delegated-access capability model",
    ),
    (
        "login",
        "public authentication endpoint; capability authorization does not apply",
    ),
    (
        "register",
        "public authentication endpoint; capability authorization does not apply",
    ),
    (
        "reset_confirm",
        "public authentication endpoint; capability authorization does not apply",
    ),
    (
        "reset_request",
        "public authentication endpoint; capability authorization does not apply",
    ),
    (
        "verify_confirm",
        "public authentication endpoint; capability authorization does not apply",
    ),
];

const RELAY_CHANGE_TENANT_UNCOVERED: &[(&str, &str)] = &[];

const RELAY_CHANGE_CAPABILITY_UNCOVERED: &[(&str, &str)] = &[
    ("NotebookCreate", "relay changes are confined to the authenticated user's namespace and carry no delegated principal or capability grant"),
    ("NotebookDelete", "relay changes are confined to the authenticated user's namespace and carry no delegated principal or capability grant"),
    ("NotebookUpdate", "relay changes are confined to the authenticated user's namespace and carry no delegated principal or capability grant"),
    ("ResourceCreate", "relay resources are owner-scoped and have no delegated capability model"),
    ("ResourceDelete", "relay resources are owner-scoped and have no delegated capability model"),
    ("TagCreate", "relay tags are owner-scoped and have no delegated capability model"),
    ("TagDelete", "relay tags are owner-scoped and have no delegated capability model"),
    ("TagUpdate", "relay tags are owner-scoped and have no delegated capability model"),
    ("NoteCreate", "note variants are not materialized by the relay and therefore reach no capability-governed mutation"),
    ("NoteDelete", "note variants are not materialized by the relay and therefore reach no capability-governed mutation"),
    ("NoteTagAdd", "the row is scoped to the authenticated user, but its tag_id reference is not tenant-scoped; known defect keeplin-srv#115"),
    ("NoteTagRemove", "the row is scoped to the authenticated user, but its tag_id reference is not tenant-scoped; known defect keeplin-srv#115"),
    ("NoteUpdate", "note variants are not materialized by the relay and therefore reach no capability-governed mutation"),
];

const READ_ISOLATION_CASES: &[&str] = &["users_do_not_see_each_others_changes"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HandlerKind {
    ReadOnly,
    Mutating,
}

struct HandlerAuthorization {
    handler: &'static str,
    kind: HandlerKind,
    inputs: &'static [&'static str],
}

const HTTP_HANDLER_AUTHORIZATION: &[HandlerAuthorization] = &[
    HandlerAuthorization {
        handler: "change_password",
        kind: HandlerKind::Mutating,
        inputs: &["authenticated identity", "credential check"],
    },
    HandlerAuthorization {
        handler: "crate::collab::handler",
        kind: HandlerKind::Mutating,
        inputs: &["authenticated identity", "collaboration session policy"],
    },
    HandlerAuthorization {
        handler: "crate::sync::handler",
        kind: HandlerKind::Mutating,
        inputs: &["authenticated identity", "device identity"],
    },
    HandlerAuthorization {
        handler: "create_device",
        kind: HandlerKind::Mutating,
        inputs: &["authenticated identity"],
    },
    HandlerAuthorization {
        handler: "create_note",
        kind: HandlerKind::Mutating,
        inputs: &["authenticated identity", "quota"],
    },
    HandlerAuthorization {
        handler: "create_notebook_share",
        kind: HandlerKind::Mutating,
        inputs: &[
            "notebook resolver",
            "authenticated identity",
            "ownership query",
        ],
    },
    HandlerAuthorization {
        handler: "create_share",
        kind: HandlerKind::Mutating,
        inputs: &["note resolver", "authenticated identity"],
    },
    HandlerAuthorization {
        handler: "delete_account",
        kind: HandlerKind::Mutating,
        inputs: &["authenticated identity", "credential check"],
    },
    HandlerAuthorization {
        handler: "delete_all_devices",
        kind: HandlerKind::Mutating,
        inputs: &["authenticated identity"],
    },
    HandlerAuthorization {
        handler: "delete_device",
        kind: HandlerKind::Mutating,
        inputs: &["authenticated identity", "ownership query"],
    },
    HandlerAuthorization {
        handler: "delete_note",
        kind: HandlerKind::Mutating,
        inputs: &["note resolver", "authenticated identity"],
    },
    HandlerAuthorization {
        handler: "delete_notebook_share",
        kind: HandlerKind::Mutating,
        inputs: &["notebook resolver", "authenticated identity"],
    },
    HandlerAuthorization {
        handler: "delete_share",
        kind: HandlerKind::Mutating,
        inputs: &["note resolver", "authenticated identity"],
    },
    HandlerAuthorization {
        handler: "export_note",
        kind: HandlerKind::ReadOnly,
        inputs: &["note resolver", "authenticated identity"],
    },
    HandlerAuthorization {
        handler: "get_note",
        kind: HandlerKind::ReadOnly,
        inputs: &["note resolver", "authenticated identity"],
    },
    HandlerAuthorization {
        handler: "get_resource_data",
        kind: HandlerKind::ReadOnly,
        inputs: &["authenticated identity", "ownership query"],
    },
    HandlerAuthorization {
        handler: "health",
        kind: HandlerKind::ReadOnly,
        inputs: &["public endpoint policy"],
    },
    HandlerAuthorization {
        handler: "import_note",
        kind: HandlerKind::Mutating,
        inputs: &["authenticated identity", "quota"],
    },
    HandlerAuthorization {
        handler: "list_devices",
        kind: HandlerKind::ReadOnly,
        inputs: &["authenticated identity"],
    },
    HandlerAuthorization {
        handler: "list_note_tags",
        kind: HandlerKind::ReadOnly,
        inputs: &["authenticated identity", "ownership query"],
    },
    HandlerAuthorization {
        handler: "list_notebook_shares",
        kind: HandlerKind::ReadOnly,
        inputs: &["notebook resolver", "authenticated identity"],
    },
    HandlerAuthorization {
        handler: "list_notebooks",
        kind: HandlerKind::ReadOnly,
        inputs: &["authenticated identity"],
    },
    HandlerAuthorization {
        handler: "list_notes",
        kind: HandlerKind::ReadOnly,
        inputs: &["authenticated identity"],
    },
    HandlerAuthorization {
        handler: "list_resources",
        kind: HandlerKind::ReadOnly,
        inputs: &["authenticated identity", "ownership query"],
    },
    HandlerAuthorization {
        handler: "list_shares",
        kind: HandlerKind::ReadOnly,
        inputs: &["note resolver", "authenticated identity"],
    },
    HandlerAuthorization {
        handler: "list_tags",
        kind: HandlerKind::ReadOnly,
        inputs: &["authenticated identity"],
    },
    HandlerAuthorization {
        handler: "login",
        kind: HandlerKind::Mutating,
        inputs: &["credential check", "rate limit"],
    },
    HandlerAuthorization {
        handler: "metrics",
        kind: HandlerKind::ReadOnly,
        inputs: &["authenticated identity"],
    },
    HandlerAuthorization {
        handler: "note_history",
        kind: HandlerKind::ReadOnly,
        inputs: &["note resolver", "authenticated identity", "history policy"],
    },
    HandlerAuthorization {
        handler: "notebook_history",
        kind: HandlerKind::ReadOnly,
        inputs: &[
            "notebook resolver",
            "authenticated identity",
            "history policy",
        ],
    },
    HandlerAuthorization {
        handler: "put_resource_data",
        kind: HandlerKind::Mutating,
        inputs: &["authenticated identity", "ownership query", "quota"],
    },
    HandlerAuthorization {
        handler: "ready",
        kind: HandlerKind::ReadOnly,
        inputs: &["public endpoint policy", "database readiness"],
    },
    HandlerAuthorization {
        handler: "register",
        kind: HandlerKind::Mutating,
        inputs: &["public endpoint policy", "rate limit"],
    },
    HandlerAuthorization {
        handler: "reset_confirm",
        kind: HandlerKind::Mutating,
        inputs: &["credential token"],
    },
    HandlerAuthorization {
        handler: "reset_request",
        kind: HandlerKind::Mutating,
        inputs: &["public endpoint policy", "rate limit"],
    },
    HandlerAuthorization {
        handler: "transfer_notebook",
        kind: HandlerKind::Mutating,
        inputs: &[
            "notebook resolver",
            "authenticated identity",
            "ownership query",
        ],
    },
    HandlerAuthorization {
        handler: "transfer_ownership",
        kind: HandlerKind::Mutating,
        inputs: &["note resolver", "authenticated identity"],
    },
    HandlerAuthorization {
        handler: "update_note",
        kind: HandlerKind::Mutating,
        inputs: &[
            "note resolver",
            "authenticated identity",
            "move policy",
            "quota",
        ],
    },
    HandlerAuthorization {
        handler: "verify_confirm",
        kind: HandlerKind::Mutating,
        inputs: &["credential token"],
    },
    HandlerAuthorization {
        handler: "verify_request",
        kind: HandlerKind::Mutating,
        inputs: &["authenticated identity"],
    },
    HandlerAuthorization {
        handler: "version",
        kind: HandlerKind::ReadOnly,
        inputs: &["public endpoint policy"],
    },
];

const SERIALIZABLE_INVARIANT_HANDLERS: &[&str] = &[
    "update_note",
    "delete_note",
    "create_share",
    "delete_share",
    "transfer_ownership",
    "create_notebook_share",
    "delete_notebook_share",
    "transfer_notebook",
];

fn routed_handlers(source: &str) -> Vec<String> {
    let router = source
        .split(concat!("// ", "md:fn router"))
        .nth(1)
        .unwrap()
        .split(concat!("// ", "md:", "PROTOCOL_VERSION"))
        .next()
        .unwrap();
    ["get(", "post(", "put(", "patch(", "delete("]
        .into_iter()
        .flat_map(|method| {
            router.match_indices(method).filter_map(move |(offset, _)| {
                if offset > 0
                    && (router.as_bytes()[offset - 1].is_ascii_alphanumeric()
                        || router.as_bytes()[offset - 1] == b'_')
                {
                    return None;
                }
                let tail = &router[offset + method.len()..];
                let handler = tail
                    .trim_start()
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
                    .next()?;
                (!handler.is_empty()).then(|| handler.to_string())
            })
        })
        .collect()
}

#[test]
fn handler_authorization_inventory_covers_router() {
    let routed = routed_handlers(include_str!("../src/http.rs"));
    let classified: Vec<_> = HTTP_HANDLER_AUTHORIZATION
        .iter()
        .map(|entry| entry.handler.to_string())
        .collect();
    assert_eq!(routed.len(), routed.iter().collect::<BTreeSet<_>>().len());
    assert_eq!(
        classified.len(),
        classified.iter().collect::<BTreeSet<_>>().len()
    );
    assert_eq!(
        routed.into_iter().collect::<BTreeSet<_>>(),
        classified.into_iter().collect()
    );
    assert!(HTTP_HANDLER_AUTHORIZATION.iter().all(|entry| {
        !entry.inputs.is_empty() && entry.inputs.iter().all(|input| !input.is_empty())
    }));
    assert!(HTTP_HANDLER_AUTHORIZATION
        .iter()
        .any(|entry| entry.kind == HandlerKind::ReadOnly));
    assert!(HTTP_HANDLER_AUTHORIZATION
        .iter()
        .any(|entry| entry.kind == HandlerKind::Mutating));
    let expanded_source = include_str!("../src/http.rs").replace(
        concat!("// ", "md:", "PROTOCOL_VERSION"),
        concat!(
            ".route(\"/api/test-only\", post(test_only_handler));\n",
            "// ",
            "md:",
            "PROTOCOL_VERSION"
        ),
    );
    let expanded: BTreeSet<_> = routed_handlers(&expanded_source).into_iter().collect();
    let classified: BTreeSet<_> = HTTP_HANDLER_AUTHORIZATION
        .iter()
        .map(|entry| entry.handler.to_string())
        .collect();
    assert!(expanded.contains("test_only_handler"));
    assert_ne!(expanded, classified);
}

#[test]
fn serializable_invariant_inventory_is_exact_and_deferred() {
    let expected: BTreeSet<_> = [
        "update_note",
        "delete_note",
        "create_share",
        "delete_share",
        "transfer_ownership",
        "create_notebook_share",
        "delete_notebook_share",
        "transfer_notebook",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        SERIALIZABLE_INVARIANT_HANDLERS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        expected
    );
    let source = include_str!("../src/http.rs");
    for handler in SERIALIZABLE_INVARIANT_HANDLERS {
        let body = source
            .split(&format!("// {}fn {handler}", "md:"))
            .nth(1)
            .unwrap()
            .split("// md:")
            .next()
            .unwrap();
        assert!(
            !body.contains("SERIALIZABLE"),
            "{handler} starts SERIALIZABLE in phase 1"
        );
    }
}
```

**What it does** — Registers real negative cases separately for tenant and capability dimensions,
and records every router handler exactly once with read-only/mutating classification and non-empty
authorization inputs. It also records ADR 0002's exact eight-handler serializable invariant set as
data without enabling that isolation level in phase 1.

**Dependencies** — `authorization_inventory_is_complete` compares these case names with source-derived inventories; expects equality to fail closed when source expands.

**Used by** — `authorization_inventory_is_complete`.

**Repeated context** — Protected HTTP mutations have tenant cases; capability cases cover shared note/notebook operations, while entity classes without delegation are explicitly excepted.

---

## fn authorization_reads_observe_transaction_local_state

**Identification** — PostgreSQL executor-seam integration test; marker `// md:fn authorization_reads_observe_transaction_local_state`.

**Code** — complete and verbatim:

```rust
// md:fn authorization_reads_observe_transaction_local_state
#[sqlx::test(migrations = "../../migrations")]
async fn authorization_reads_observe_transaction_local_state(pool: PgPool) {
    let store = Store::new(pool.clone());
    let owner = store
        .create_user("seam-owner@example.com", "hash", "Owner")
        .await
        .unwrap();
    let member = store
        .create_user("seam-member@example.com", "hash", "Member")
        .await
        .unwrap();
    let note = store
        .create_note(None, "transaction seam", owner.id)
        .await
        .unwrap();
    let direct_note = store
        .create_note(None, "direct transaction seam", owner.id)
        .await
        .unwrap();
    let notebook_id = Uuid::new_v4();
    let resource_id = Uuid::new_v4();
    let transaction_user_id = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO note_shares (note_id, user_id, capabilities) VALUES ($1, $2, $3)")
        .bind(direct_note.id)
        .bind(member.id)
        .bind(Capabilities::WRITE)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(resolve_note_access_on(
        &store,
        &mut tx,
        &direct_note,
        member.id,
        PermissionScheme::Strict,
    )
    .await
    .unwrap()
    .can_write());
    sqlx::query(
        "INSERT INTO notebooks (id, user_id, title, created_at, updated_at, vv, last_writer)
         VALUES ($1, $2, 'notebook', now(), now(), '{}', 'test')",
    )
    .bind(notebook_id)
    .bind(owner.id)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO notebook_shares (notebook_id, user_id, capabilities) VALUES ($1, $2, $3)",
    )
    .bind(notebook_id)
    .bind(member.id)
    .bind(Capabilities::READ)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("UPDATE notes SET notebook_id = $2 WHERE id = $1")
        .bind(note.id)
        .bind(notebook_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut transaction_note = note.clone();
    transaction_note.notebook_id = Some(notebook_id);
    assert!(resolve_note_access_on(
        &store,
        &mut tx,
        &transaction_note,
        member.id,
        PermissionScheme::Strict,
    )
    .await
    .unwrap()
    .can_read());
    assert!(
        resolve_notebook_access_on(&store, &mut tx, notebook_id, member.id)
            .await
            .unwrap()
            .can_read()
    );
    assert_eq!(
        store
            .inherited_note_principals_on(&mut tx, notebook_id)
            .await
            .unwrap()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        [
            (owner.id, Capabilities::ALL),
            (member.id, Capabilities::READ)
        ]
        .into_iter()
        .collect()
    );
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, display_name)
         VALUES ($1, 'transaction-user@example.com', 'hash', 'Transaction User')",
    )
    .bind(transaction_user_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    assert_eq!(
        store
            .get_user_by_id_on(&mut tx, transaction_user_id)
            .await
            .unwrap()
            .unwrap()
            .id,
        transaction_user_id
    );
    sqlx::query(
        "INSERT INTO resources
         (id, user_id, title, mime_type, file_name, size, created_at, vv, last_writer)
         VALUES ($1, $2, 'resource', 'application/octet-stream', 'resource.bin', 4, now(), '{}', 'test')",
    )
    .bind(resource_id)
    .bind(owner.id)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO resource_blobs (resource_id, data) VALUES ($1, $2)")
        .bind(resource_id)
        .bind(vec![1_u8, 2, 3, 4])
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(store
        .resource_owned_by_on(&mut tx, resource_id, owner.id)
        .await
        .unwrap());
    assert_eq!(
        store
            .user_blob_bytes_excluding_on(&mut tx, owner.id, Uuid::nil())
            .await
            .unwrap(),
        4
    );
    tx.rollback().await.unwrap();
}
```

**What it does** — Proves that both permission resolvers and the executor-aware reads behind the move, account credential, resource ownership, and storage-quota guards observe rows and changes visible only inside one uncommitted transaction. Note resolution covers both query branches: a direct note grant visible only in the transaction and an inherited notebook grant whose notebook, share, and note membership are visible only there.

**Dependencies**

- `resolve_note_access_on` and `resolve_notebook_access_on` — evaluate authorization through the supplied connection; expects: every resolver query remains on that connection.
- `Store::*_on` guard reads — inspect transaction-local account, notebook, share, resource, and blob state; expects: no implementation silently falls back to the pool.
- `sqlx::Transaction` — supplies the uncommitted PostgreSQL state; expects: rollback removes all fixture writes.

**Used by** — ADR 0002 verification-plan row 7 and issue #136 acceptance criteria 1–2.

**Repeated context** — The test deliberately rolls back; visibility before commit is the evidence, not persistence after commit.

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
                    *offset == 0
                        || !(router.as_bytes()[offset - 1].is_ascii_alphanumeric()
                            || router.as_bytes()[offset - 1] == b'_')
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

## fn route_registration_is_confined_to_router

**Identification** — crate-wide route-registration placement regression; marker `// md:fn route_registration_is_confined_to_router`.

**Code** — complete and verbatim:

```rust
// md:fn route_registration_is_confined_to_router
#[test]
fn route_registration_is_confined_to_router() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(crate_root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let outside_router = if path.file_name().and_then(|name| name.to_str()) == Some("http.rs") {
            let (before, tail) = source.split_once("pub fn router(").unwrap();
            let (_, after) = tail.split_once("pub const PROTOCOL_VERSION").unwrap();
            format!("{before}{after}")
        } else {
            source
        };
        for registration in [
            ".route(",
            ".route_service(",
            ".merge(",
            ".nest(",
            ".nest_service(",
            ".on(",
        ] {
            assert!(
                !outside_router.contains(registration),
                "{} registers {registration} outside http::router",
                path.display()
            );
        }
    }
}
```

**What it does** — Scans every Rust module in the server crate and rejects `.route`, `.route_service`, `.merge`, `.nest`, `.nest_service`, or `.on` registrations outside the delimited `http::router` body, keeping the handler inventory complete when routing code evolves. It intentionally omits `.any` because the current `collab.rs` legitimately uses `Iterator::any` outside the router, which would be a false positive for this textual guard.

**Dependencies** — `CARGO_MANIFEST_DIR`, `std::fs::read_dir`, and the `router`/`PROTOCOL_VERSION` declarations locate and delimit source; expects all route composition to remain literal inside `http::router`.

**Used by** — `cargo test` and CI; mechanical guard for F11.

**Repeated context** — The handler parser deliberately consumes one monolithic router body, and this test enforces that structural invariant.

---

## fn source_relay_changes

**Identification** — source inventory helper; marker `// md:fn source_relay_changes`.

**Code** — complete and verbatim:

```rust
// md:fn source_relay_changes
fn source_relay_changes() -> BTreeSet<String> {
    let source = include_str!("../src/sync.rs");
    let materialize = source
        .split(concat!("// md:", "fn materialize"))
        .nth(1)
        .unwrap()
        .split(concat!("// md:", "fn changes_frame"))
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
    let handler_tenant_inventory = MUTATING_HANDLER_TENANT_CASES
        .iter()
        .chain(MUTATING_HANDLER_TENANT_UNCOVERED)
        .map(|(entry, _)| (*entry).to_string())
        .collect();
    assert_eq!(
        source_handlers(),
        handler_tenant_inventory,
        "mutation inventory recognizes only literal .route( registrations inside http::router; helpers, macros, or table-driven registrations without that token are undetected"
    );
    let handler_capability_inventory = MUTATING_HANDLER_CAPABILITY_CASES
        .iter()
        .chain(MUTATING_HANDLER_CAPABILITY_UNCOVERED)
        .map(|(entry, _)| (*entry).to_string())
        .collect();
    assert_eq!(
        source_handlers(),
        handler_capability_inventory,
        "mutation inventory recognizes only literal .route( registrations inside http::router; helpers, macros, or table-driven registrations without that token are undetected"
    );

    let relay_tenant_inventory = RELAY_CHANGE_TENANT_CASES
        .iter()
        .chain(RELAY_CHANGE_TENANT_UNCOVERED)
        .map(|(entry, _)| (*entry).to_string())
        .collect();
    assert_eq!(source_relay_changes(), relay_tenant_inventory);
    let relay_capability_inventory = RELAY_CHANGE_CAPABILITY_CASES
        .iter()
        .chain(RELAY_CHANGE_CAPABILITY_UNCOVERED)
        .map(|(entry, _)| (*entry).to_string())
        .collect();
    assert_eq!(source_relay_changes(), relay_capability_inventory);
    assert_eq!(
        READ_ISOLATION_CASES,
        &["users_do_not_see_each_others_changes"]
    );
}
```

**What it does** — Requires exact equality separately for tenant and capability inventories, using real cases plus explicit non-applicability entries, and retains the read-isolation case by name. The derivation is lexical and recognizes only literal `.route(` registrations inside `http::router`; helper-, macro-, loop-, or table-driven registration without that token is undetected, and the assertion failure text states this boundary.

**Dependencies** — `source_handlers` and `source_relay_changes` enumerate source; expects any unregistered addition to make this test fail.

**Used by** — `cargo test` and CI.

**Repeated context** — Completeness is dimension-specific; case attribution and covered/excepted disjunction are verified separately.

---

## fn inventory_classifications_are_disjoint_and_cases_are_tests

**Identification** — disjoint-classification and registered-test existence test; marker `// md:fn inventory_classifications_are_disjoint_and_cases_are_tests`.

**Code** — complete and verbatim:

```rust
// md:fn inventory_classifications_are_disjoint_and_cases_are_tests
#[test]
fn inventory_classifications_are_disjoint_and_cases_are_tests() {
    let tests = include_str!("authorization.rs");
    for (covered, excepted) in [
        (
            MUTATING_HANDLER_TENANT_CASES,
            MUTATING_HANDLER_TENANT_UNCOVERED,
        ),
        (
            MUTATING_HANDLER_CAPABILITY_CASES,
            MUTATING_HANDLER_CAPABILITY_UNCOVERED,
        ),
        (RELAY_CHANGE_TENANT_CASES, RELAY_CHANGE_TENANT_UNCOVERED),
        (
            RELAY_CHANGE_CAPABILITY_CASES,
            RELAY_CHANGE_CAPABILITY_UNCOVERED,
        ),
    ] {
        let covered_entries: BTreeSet<_> = covered.iter().map(|(entry, _)| entry).collect();
        let excepted_entries: BTreeSet<_> = excepted.iter().map(|(entry, _)| entry).collect();
        assert!(covered_entries.is_disjoint(&excepted_entries));
        assert_eq!(covered_entries.len(), covered.len());
        assert_eq!(excepted_entries.len(), excepted.len());
    }
    for (entry, case) in MUTATING_HANDLER_TENANT_CASES
        .iter()
        .chain(MUTATING_HANDLER_CAPABILITY_CASES)
        .chain(RELAY_CHANGE_TENANT_CASES)
        .chain(RELAY_CHANGE_CAPABILITY_CASES)
    {
        assert!(!entry.is_empty());
        assert!(
            tests.contains(&format!("#[test]\nfn {case}("))
                || tests.contains(&format!(
                    "#[sqlx::test(migrations = \"../../migrations\")]\nasync fn {case}("
                )),
            "missing test case {case}"
        );
    }
    for (entry, reason) in MUTATING_HANDLER_TENANT_UNCOVERED
        .iter()
        .chain(MUTATING_HANDLER_CAPABILITY_UNCOVERED)
        .chain(RELAY_CHANGE_TENANT_UNCOVERED)
        .chain(RELAY_CHANGE_CAPABILITY_UNCOVERED)
    {
        assert!(!entry.is_empty());
        assert!(!reason.is_empty());
    }
}
```

**What it does** — Proves each covered/excepted pair is disjoint and duplicate-free, every claimed case names an attributed test, and every exception has a reason.

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
        MUTATING_HANDLER_TENANT_UNCOVERED
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

## fn relay_materialization_uses_authenticated_session_identity

**Identification** — relay identity-flow source regression; marker `// md:fn relay_materialization_uses_authenticated_session_identity`.

**Code** — complete and verbatim:

```rust
// md:fn relay_materialization_uses_authenticated_session_identity
#[test]
fn relay_materialization_uses_authenticated_session_identity() {
    let source = include_str!("../src/sync.rs");
    let handler = source
        .split("async fn handle_incoming(")
        .nth(1)
        .unwrap()
        .split("async fn materialize(")
        .next()
        .unwrap();
    assert!(handler.contains("user_id: Uuid,"));
    assert!(handler.contains("materialize(state, user_id, &changes).await;"));
}
```

**What it does** — Requires `handle_incoming` to accept the authenticated session `user_id` and pass that exact local variable to `materialize`, preventing payload-derived identity from selecting the mutation tenant.

**Dependencies** — `include_str!(../src/sync.rs)` and the `handle_incoming`/`materialize` declarations delimit the handler; expects the authenticated identity parameter and materialization call to retain their explicit source forms.

**Used by** — `cargo test` and CI; regression verifier for F12.

**Repeated context** — WebSocket authentication happens before frame handling; frame contents are changes, never an authority for tenant identity.

---

## fn note_changes_are_explicitly_non_materializing

**Identification** — relay note-variant regression; marker `// md:fn note_changes_are_explicitly_non_materializing`.

**Code** — complete and verbatim:

```rust
// md:fn note_changes_are_explicitly_non_materializing
#[test]
fn note_changes_are_explicitly_non_materializing() {
    let source = include_str!("../src/sync.rs");
    let materialize = source
        .split("// md:fn materialize")
        .nth(1)
        .unwrap()
        .split("// md:fn changes_frame")
        .next()
        .unwrap();
    assert!(materialize.contains(
        "Change::NoteCreate { .. } | Change::NoteUpdate { .. } | Change::NoteDelete { .. } =>"
    ));
    assert!(materialize.contains("=> {\n                Ok(())\n            }"));
}
```

**What it does** — Pins all three note variants to the explicit no-op arm of relay materialization.

**Dependencies** — `include_str!(../src/sync.rs)` supplies canonical relay source; expects markers to delimit `materialize`.

**Used by** — relay tenant inventory and F9 evidence.

**Repeated context** — note bodies are handled outside relay materialization.

---

## fn authorization_test_config

**Identification** — HTTP test configuration; marker `// md:fn authorization_test_config`.

**Code** — complete and verbatim:

```rust
// md:fn authorization_test_config
fn authorization_test_config() -> Config {
    Config {
        port: 0,
        database_url: String::new(),
        jwt_secret: "test-secret".into(),
        token_ttl_days: 1,
        retention_days: 0,
        lines_gc_days: 0,
        resource_purge_days: 0,
        db_max_connections: 5,
        db_acquire_timeout_secs: 10,
        db_idle_timeout_secs: 600,
        db_max_lifetime_secs: 1800,
        rate_limit_per_min: 0,
        shutdown_grace_secs: 5,
        log_json: false,
        max_upload_bytes: 100 * 1024 * 1024,
        max_note_body_bytes: 0,
        max_user_storage_bytes: 0,
        max_notes_per_user: 0,
        registration_enabled: true,
        at_rest_key: None,
        mail_webhook_url: None,
        mail_webhook_token: None,
        email_token_ttl_secs: 3600,
        email_verification_required: false,
        login_max_failures: 0,
        login_lockout_secs: 300,
        history_since_access: false,
        permission_scheme: keeplin_srv::config::PermissionScheme::Strict,
    }
}
```

**What it does** — Builds deterministic integration configuration with rate limiting disabled.

**Dependencies** — `Config`; expects test limits not to mask authorization results.

**Used by** — `spawn_authorization_server`.

**Repeated context** — the listener selects its own loopback port.

---

## fn spawn_authorization_server

**Identification** — real-router server helper; marker `// md:fn spawn_authorization_server`.

**Code** — complete and verbatim:

```rust
// md:fn spawn_authorization_server
async fn spawn_authorization_server(pool: PgPool) -> SocketAddr {
    let state = Arc::new(AppState::new(authorization_test_config(), pool));
    let app: Router = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}
```

**What it does** — Starts the production router on `127.0.0.1:0` over a SQLx test pool.

**Dependencies** — `AppState::new`, `router`, `TcpListener::bind`, `axum::serve`; expects production middleware to execute.

**Used by** — HTTP authorization regressions.

**Repeated context** — mirrors `tests/integration.rs::spawn_server`.

---

## fn spawn_notice_webhook

**Identification** — in-process revocation-notice webhook fixture; marker `// md:fn spawn_notice_webhook`.

**Code** — complete and verbatim:

```rust
// md:fn spawn_notice_webhook
async fn spawn_notice_webhook() -> (SocketAddr, Arc<tokio::sync::Mutex<Vec<Value>>>) {
    let inbox: Arc<tokio::sync::Mutex<Vec<Value>>> = Arc::default();
    let captured = inbox.clone();
    let app = Router::new().route(
        "/mail",
        axum::routing::post(move |Json(payload): Json<Value>| {
            let captured = captured.clone();
            async move {
                captured.lock().await.push(payload);
                "ok"
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, inbox)
}
```

**What it does** — Starts a loopback webhook that records each JSON mail payload in a shared inbox, allowing authorization tests to distinguish a committed move from an incorrect revocation notification.

**Dependencies** — `Router::new`, `axum::routing::post`, and `Json` — capture production mail webhook requests; expects Axum extraction to preserve the submitted JSON payload. `TcpListener::bind` and `axum::serve` — expose the fixture on an ephemeral loopback port; expects the spawned server to remain available for the test lifetime. `tokio::sync::Mutex` — serializes inbox access; expects the HTTP response to occur only after the payload is stored.

**Used by** — `moving_from_deleted_notebook_does_not_notify_stale_principals`.

**Repeated context** — Notification delivery is synchronous in `update_note`, so the inbox is complete when the PATCH response arrives.

---

## fn register_and_login

**Identification** — authenticated-principal helper; marker `// md:fn register_and_login`.

**Code** — complete and verbatim:

```rust
// md:fn register_and_login
async fn register_and_login(addr: SocketAddr, email: &str) -> String {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({
            "email": email,
            "password": "password123",
            "device_name": "authorization-test"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["token"].as_str().unwrap().to_string()
}
```

**What it does** — Registers and logs in a user and returns its bearer token.

**Dependencies** — register/login HTTP endpoints; expects successful bootstrap responses.

**Used by** — HTTP authorization regressions.

**Repeated context** — public bootstrap is not a tenant capability boundary.

---

## fn authed_json

**Identification** — authenticated JSON request helper; marker `// md:fn authed_json`.

**Code** — complete and verbatim:

```rust
// md:fn authed_json
async fn authed_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    addr: SocketAddr,
    path: &str,
    token: &str,
    body: Value,
) -> reqwest::Response {
    client
        .request(method, format!("http://{addr}{path}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}
```

**What it does** — Sends any method and JSON body with bearer authentication for exact status assertions.

**Dependencies** — `reqwest::Client::request`; expects the real HTTP response unchanged.

**Used by** — HTTP authorization regressions.

**Repeated context** — denied cases assert both status and post-state.

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

## fn relation_snapshot

**Identification** — deterministic relation snapshot helper; marker `// md:fn relation_snapshot`.

**Code** — complete and verbatim:

```rust
// md:fn relation_snapshot
async fn relation_snapshot(pool: &PgPool, table: &str, column: &str, id: Uuid) -> String {
    let query = format!(
        "SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text)::text, '[]') FROM {table} t WHERE {column} = $1"
    );
    sqlx::query_scalar(&query)
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}
```

**What it does** — Serializes every relation row selected by a UUID foreign-key column in stable order.

**Dependencies** — PostgreSQL `jsonb_agg` and `to_jsonb`; expects textual JSON to support byte comparison.

**Used by** — share, note-tag, and HTTP authorization regressions.

**Repeated context** — empty relations serialize as `[]`.

---

## fn cross_tenant_http_mutations_leave_victim_unchanged

**Identification** — full-router cross-tenant regression; marker `// md:fn cross_tenant_http_mutations_leave_victim_unchanged`.

**Code** — complete and verbatim:

```rust
// md:fn cross_tenant_http_mutations_leave_victim_unchanged
#[sqlx::test(migrations = "../../migrations")]
async fn cross_tenant_http_mutations_leave_victim_unchanged(pool: PgPool) {
    let addr = spawn_authorization_server(pool.clone()).await;
    let attacker_token = register_and_login(addr, "http-attacker@example.com").await;
    let victim_token = register_and_login(addr, "http-victim@example.com").await;
    let disposable_token = register_and_login(addr, "http-disposable@example.com").await;
    let target_token = register_and_login(addr, "http-target@example.com").await;
    let store = Store::new(pool.clone());
    let attacker = store
        .get_user_by_email("http-attacker@example.com")
        .await
        .unwrap()
        .unwrap();
    let victim = store
        .get_user_by_email("http-victim@example.com")
        .await
        .unwrap()
        .unwrap();
    let target = store
        .get_user_by_email("http-target@example.com")
        .await
        .unwrap()
        .unwrap();
    let victim_device = store
        .list_devices_by_user(victim.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let victim_note = store
        .create_note(None, "victim note", victim.id)
        .await
        .unwrap();
    store
        .create_or_update_share(victim_note.id, target.id, Capabilities::READ)
        .await
        .unwrap();
    let notebook = Notebook::new("victim notebook");
    assert!(store.upsert_notebook(victim.id, &notebook).await.unwrap());
    store
        .create_or_update_notebook_share(notebook.id, target.id, Capabilities::READ)
        .await
        .unwrap();
    let resource = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "victim resource",
        "application/octet-stream",
        "victim.bin",
        12,
    );
    assert!(store
        .upsert_resource_meta(victim.id, &resource)
        .await
        .unwrap());
    assert!(store
        .put_resource_blob(victim.id, resource.id, b"victim bytes")
        .await
        .unwrap());

    let victim_user_before = entity_snapshot(&pool, "users", victim.id).await;
    let victim_device_before = entity_snapshot(&pool, "user_devices", victim_device.id).await;
    let victim_note_before = entity_snapshot(&pool, "notes", victim_note.id).await;
    let victim_note_shares_before =
        relation_snapshot(&pool, "note_shares", "note_id", victim_note.id).await;
    let victim_notebook_before = entity_snapshot(&pool, "notebooks", notebook.id).await;
    let victim_notebook_shares_before =
        relation_snapshot(&pool, "notebook_shares", "notebook_id", notebook.id).await;
    let victim_resource_before = entity_snapshot(&pool, "resources", resource.id).await;
    let victim_blob_before = store.get_resource_blob(resource.id).await.unwrap().unwrap();
    let client = reqwest::Client::new();

    let cases = [
        authed_json(
            &client,
            reqwest::Method::POST,
            addr,
            "/api/notes",
            &attacker_token,
            json!({ "id": victim_note.id, "title": "collision" }),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::PATCH,
            addr,
            &format!("/api/notes/{}", victim_note.id),
            &attacker_token,
            json!({ "title": "stolen" }),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::DELETE,
            addr,
            &format!("/api/notes/{}", victim_note.id),
            &attacker_token,
            json!({}),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::POST,
            addr,
            &format!("/api/notes/{}/share", victim_note.id),
            &attacker_token,
            json!({ "user_id": attacker.id, "capabilities": Capabilities::READ }),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::DELETE,
            addr,
            &format!("/api/notes/{}/share/{}", victim_note.id, target.id),
            &attacker_token,
            json!({}),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::POST,
            addr,
            &format!("/api/notes/{}/transfer", victim_note.id),
            &attacker_token,
            json!({ "user_id": attacker.id }),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::POST,
            addr,
            &format!("/api/notebooks/{}/share", notebook.id),
            &attacker_token,
            json!({ "user_id": attacker.id, "capabilities": Capabilities::READ }),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::DELETE,
            addr,
            &format!("/api/notebooks/{}/share/{}", notebook.id, target.id),
            &attacker_token,
            json!({}),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::POST,
            addr,
            &format!("/api/notebooks/{}/transfer", notebook.id),
            &attacker_token,
            json!({ "user_id": attacker.id }),
        )
        .await,
    ];
    assert_eq!(cases[0].status(), 409);
    for response in &cases[1..] {
        assert_eq!(response.status(), 403);
    }
    let response = client
        .put(format!("http://{addr}/api/resources/{}/data", resource.id))
        .bearer_auth(&attacker_token)
        .body("attacker bytes")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let response = authed_json(
        &client,
        reqwest::Method::DELETE,
        addr,
        &format!("/api/devices/{}", victim_device.id),
        &attacker_token,
        json!({}),
    )
    .await;
    assert_eq!(response.status(), 404);
    let response = authed_json(
        &client,
        reqwest::Method::POST,
        addr,
        "/api/devices",
        &attacker_token,
        json!({ "device_name": "attacker-extra" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = authed_json(
        &client,
        reqwest::Method::POST,
        addr,
        "/api/import",
        &attacker_token,
        json!({ "title": "attacker import", "body": "one\ntwo" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = authed_json(
        &client,
        reqwest::Method::POST,
        addr,
        "/api/account/verify/request",
        &attacker_token,
        json!({}),
    )
    .await;
    assert_eq!(response.status(), 501);
    let response = authed_json(
        &client,
        reqwest::Method::POST,
        addr,
        "/api/account/password",
        &attacker_token,
        json!({ "current_password": "password123", "new_password": "changed123" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = authed_json(
        &client,
        reqwest::Method::DELETE,
        addr,
        "/api/account",
        &disposable_token,
        json!({ "password": "password123" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = authed_json(
        &client,
        reqwest::Method::DELETE,
        addr,
        "/api/devices",
        &attacker_token,
        json!({}),
    )
    .await;
    assert_eq!(response.status(), 200);

    assert_eq!(
        entity_snapshot(&pool, "users", victim.id).await,
        victim_user_before
    );
    assert_eq!(
        entity_snapshot(&pool, "user_devices", victim_device.id).await,
        victim_device_before
    );
    assert_eq!(
        entity_snapshot(&pool, "notes", victim_note.id).await,
        victim_note_before
    );
    assert_eq!(
        relation_snapshot(&pool, "note_shares", "note_id", victim_note.id).await,
        victim_note_shares_before
    );
    assert_eq!(
        entity_snapshot(&pool, "notebooks", notebook.id).await,
        victim_notebook_before
    );
    assert_eq!(
        relation_snapshot(&pool, "notebook_shares", "notebook_id", notebook.id).await,
        victim_notebook_shares_before
    );
    assert_eq!(
        entity_snapshot(&pool, "resources", resource.id).await,
        victim_resource_before
    );
    assert_eq!(
        store.get_resource_blob(resource.id).await.unwrap().unwrap(),
        victim_blob_before
    );
    let victim_response = client
        .get(format!("http://{addr}/api/notes/{}", victim_note.id))
        .bearer_auth(victim_token)
        .send()
        .await
        .unwrap();
    assert_eq!(victim_response.status(), 200);
    let target_response = client
        .get(format!("http://{addr}/api/notes/{}", victim_note.id))
        .bearer_auth(target_token)
        .send()
        .await
        .unwrap();
    assert_eq!(target_response.status(), 200);
}
```

**What it does** — Exercises every protected mutating handler, asserts exact statuses, and proves all victim account, device, note, share, notebook, resource, and blob projections remain byte-identical.

**Dependencies** — production router, `reqwest`, `Store`, and snapshot helpers; expects bearer identity to scope every side effect.

**Used by** — protected-handler tenant inventory.

**Repeated context** — self-directed routes mutate only attacker state before victim snapshots are compared.

---

## fn denied_http_capabilities_leave_entities_unchanged

**Identification** — denied-capability HTTP regression; marker `// md:fn denied_http_capabilities_leave_entities_unchanged`.

**Code** — complete and verbatim:

```rust
// md:fn denied_http_capabilities_leave_entities_unchanged
#[sqlx::test(migrations = "../../migrations")]
async fn denied_http_capabilities_leave_entities_unchanged(pool: PgPool) {
    let addr = spawn_authorization_server(pool.clone()).await;
    let owner_token = register_and_login(addr, "cap-owner@example.com").await;
    let reader_token = register_and_login(addr, "cap-reader@example.com").await;
    let target_token = register_and_login(addr, "cap-target@example.com").await;
    let store = Store::new(pool.clone());
    let owner = store
        .get_user_by_email("cap-owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let reader = store
        .get_user_by_email("cap-reader@example.com")
        .await
        .unwrap()
        .unwrap();
    let target = store
        .get_user_by_email("cap-target@example.com")
        .await
        .unwrap()
        .unwrap();
    let note = store
        .create_note(None, "owner note", owner.id)
        .await
        .unwrap();
    store
        .create_or_update_share(note.id, reader.id, Capabilities::READ)
        .await
        .unwrap();
    store
        .create_or_update_share(note.id, target.id, Capabilities::READ)
        .await
        .unwrap();
    let notebook = Notebook::new("owner notebook");
    assert!(store.upsert_notebook(owner.id, &notebook).await.unwrap());
    store
        .create_or_update_notebook_share(notebook.id, reader.id, Capabilities::READ)
        .await
        .unwrap();
    store
        .create_or_update_notebook_share(notebook.id, target.id, Capabilities::READ)
        .await
        .unwrap();
    let note_before = entity_snapshot(&pool, "notes", note.id).await;
    let note_shares_before = relation_snapshot(&pool, "note_shares", "note_id", note.id).await;
    let notebook_before = entity_snapshot(&pool, "notebooks", notebook.id).await;
    let notebook_shares_before =
        relation_snapshot(&pool, "notebook_shares", "notebook_id", notebook.id).await;
    let client = reqwest::Client::new();
    let cases = [
        authed_json(
            &client,
            reqwest::Method::PATCH,
            addr,
            &format!("/api/notes/{}", note.id),
            &reader_token,
            json!({ "title": "forbidden" }),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::DELETE,
            addr,
            &format!("/api/notes/{}", note.id),
            &reader_token,
            json!({}),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::POST,
            addr,
            &format!("/api/notes/{}/share", note.id),
            &reader_token,
            json!({ "user_id": target.id, "capabilities": Capabilities::READ }),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::DELETE,
            addr,
            &format!("/api/notes/{}/share/{}", note.id, target.id),
            &reader_token,
            json!({}),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::POST,
            addr,
            &format!("/api/notes/{}/transfer", note.id),
            &reader_token,
            json!({ "user_id": target.id }),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::POST,
            addr,
            &format!("/api/notebooks/{}/share", notebook.id),
            &reader_token,
            json!({ "user_id": target.id, "capabilities": Capabilities::READ }),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::DELETE,
            addr,
            &format!("/api/notebooks/{}/share/{}", notebook.id, target.id),
            &reader_token,
            json!({}),
        )
        .await,
        authed_json(
            &client,
            reqwest::Method::POST,
            addr,
            &format!("/api/notebooks/{}/transfer", notebook.id),
            &reader_token,
            json!({ "user_id": target.id }),
        )
        .await,
    ];
    for response in cases {
        assert_eq!(response.status(), 403);
    }
    assert_eq!(entity_snapshot(&pool, "notes", note.id).await, note_before);
    assert_eq!(
        relation_snapshot(&pool, "note_shares", "note_id", note.id).await,
        note_shares_before
    );
    assert_eq!(
        entity_snapshot(&pool, "notebooks", notebook.id).await,
        notebook_before
    );
    assert_eq!(
        relation_snapshot(&pool, "notebook_shares", "notebook_id", notebook.id).await,
        notebook_shares_before
    );
    let owner_response = client
        .get(format!("http://{addr}/api/notes/{}", note.id))
        .bearer_auth(owner_token)
        .send()
        .await
        .unwrap();
    assert_eq!(owner_response.status(), 200);
    let target_response = client
        .get(format!("http://{addr}/api/notes/{}", note.id))
        .bearer_auth(target_token)
        .send()
        .await
        .unwrap();
    assert_eq!(target_response.status(), 200);
}
```

**What it does** — Gives a reader real `READ` access, asserts 403 for every higher-capability note/notebook mutation, and proves entities and shares remain byte-identical.

**Dependencies** — `Capabilities::READ`, production router, `Store`, and snapshot helpers; expects capability resolution before effects.

**Used by** — protected-handler capability inventory.

**Repeated context** — owner and target reads prove access remains live after denials.

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
    let victim_note = store
        .create_note(None, "victim note", victim.id)
        .await
        .unwrap();
    assert!(store
        .upsert_note_tag(
            victim.id,
            victim_note.id,
            tag.id,
            Utc::now(),
            None,
            &tag.vv,
            "victim",
        )
        .await
        .unwrap());
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
    let note_tags_before = relation_snapshot(&pool, "note_tags", "user_id", victim.id).await;
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
    let _ = store
        .upsert_note_tag(
            attacker.id,
            victim_note.id,
            tag.id,
            Utc::now() + Duration::days(2),
            None,
            &hostile_tag.vv,
            "attacker",
        )
        .await
        .unwrap();
    let _ = store
        .upsert_note_tag(
            attacker.id,
            victim_note.id,
            tag.id,
            Utc::now() + Duration::days(3),
            Some(Utc::now() + Duration::days(3)),
            &hostile_tag.vv,
            "attacker",
        )
        .await
        .unwrap();
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
        relation_snapshot(&pool, "note_tags", "user_id", victim.id).await,
        note_tags_before
    );
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

## fn known_defect_115_list_note_tag_ids_exposes_foreign_tag_reference

**Identification** — PostgreSQL characterization test for the known cross-tenant note-tag oracle tracked by keeplin-srv#115; marker `// md:fn known_defect_115_list_note_tag_ids_exposes_foreign_tag_reference`.

**Code** — complete and verbatim:

```rust
// md:fn known_defect_115_list_note_tag_ids_exposes_foreign_tag_reference
#[sqlx::test(migrations = "../../migrations")]
async fn known_defect_115_list_note_tag_ids_exposes_foreign_tag_reference(pool: PgPool) {
    let store = Store::new(pool);
    let attacker = store
        .create_user("attacker@example.com", "hash", "attacker")
        .await
        .unwrap();
    let victim = store
        .create_user("victim@example.com", "hash", "victim")
        .await
        .unwrap();
    let attacker_note = store
        .create_note(None, "attacker note", attacker.id)
        .await
        .unwrap();
    let mut victim_tag = Tag::new("victim tag");
    victim_tag.vv = VersionVector::from([("victim".to_string(), 1)]);
    victim_tag.last_writer = "victim".into();
    assert!(store.upsert_tag(victim.id, &victim_tag).await.unwrap());
    assert!(store
        .upsert_note_tag(
            attacker.id,
            attacker_note.id,
            victim_tag.id,
            Utc::now(),
            None,
            &victim_tag.vv,
            "attacker",
        )
        .await
        .unwrap());

    assert_eq!(
        store
            .list_note_tag_ids(attacker.id, attacker_note.id)
            .await
            .unwrap(),
        vec![victim_tag.id]
    );
}
```

**What it does** — Characterizes, but does not bless, known defect keeplin-srv#115: a note-tag row owned by the attacker may reference a live tag owned by the victim, and `list_note_tag_ids` currently exposes that foreign UUID. The exact-vector assertion is deliberately temporary and must fail when #115 correctly filters the referenced tag by tenant, forcing that fixing PR to replace this characterization with the secure empty-result assertion.

**Dependencies** — `Store::{create_user, create_note, upsert_tag, upsert_note_tag, list_note_tag_ids}` constructs the cross-tenant reference and observes it; expects the current defective join to return the victim's live tag UUID until keeplin-srv#115 changes that behavior.

**Used by** — `cargo test` and CI as an explicit known-defect fixture that must be revised by keeplin-srv#115.

**Repeated context** — This is a defect fixation, not a security guarantee. The row's `user_id` is attacker-scoped, but its `tag_id` reference is not tenant-scoped.

---

## fn foreign_and_missing_upserts_are_indistinguishable

**Identification** — PostgreSQL upsert anti-oracle regression; marker `// md:fn foreign_and_missing_upserts_are_indistinguishable`.

**Code** — complete and verbatim:

```rust
// md:fn foreign_and_missing_upserts_are_indistinguishable
#[sqlx::test(migrations = "../../migrations")]
async fn foreign_and_missing_upserts_are_indistinguishable(pool: PgPool) {
    let store = Store::new(pool.clone());
    let attacker = store
        .create_user("attacker@example.com", "hash", "attacker")
        .await
        .unwrap();
    let victim = store
        .create_user("victim@example.com", "hash", "victim")
        .await
        .unwrap();
    let stored_vv = VersionVector::from([("victim".to_string(), 2)]);
    let losing_vv = VersionVector::from([("victim".to_string(), 1)]);
    let victim_time = Utc::now();
    let losing_time = victim_time + Duration::days(1);

    let mut foreign_notebook = Notebook::new("victim notebook");
    foreign_notebook.vv = stored_vv.clone();
    foreign_notebook.updated_at = victim_time;
    assert!(store
        .upsert_notebook(victim.id, &foreign_notebook)
        .await
        .unwrap());
    foreign_notebook.vv = losing_vv.clone();
    foreign_notebook.updated_at = losing_time;
    foreign_notebook.last_writer = "attacker".into();
    let mut missing_notebook = foreign_notebook.clone();
    missing_notebook.id = Uuid::new_v4();
    let foreign = store
        .upsert_notebook(attacker.id, &foreign_notebook)
        .await
        .unwrap();
    let missing = store
        .upsert_notebook(attacker.id, &missing_notebook)
        .await
        .unwrap();
    assert!(foreign);
    assert!(missing);

    let mut foreign_tag = Tag::new("victim tag");
    foreign_tag.vv = stored_vv.clone();
    foreign_tag.updated_at = victim_time;
    assert!(store.upsert_tag(victim.id, &foreign_tag).await.unwrap());
    foreign_tag.vv = losing_vv.clone();
    foreign_tag.updated_at = losing_time;
    foreign_tag.last_writer = "attacker".into();
    let mut missing_tag = foreign_tag.clone();
    missing_tag.id = Uuid::new_v4();
    let foreign = store.upsert_tag(attacker.id, &foreign_tag).await.unwrap();
    let missing = store.upsert_tag(attacker.id, &missing_tag).await.unwrap();
    assert!(foreign);
    assert!(missing);

    let mut foreign_resource = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "victim resource",
        "application/octet-stream",
        "victim.bin",
        1,
    );
    foreign_resource.vv = stored_vv;
    foreign_resource.created_at = victim_time;
    assert!(store
        .upsert_resource_meta(victim.id, &foreign_resource)
        .await
        .unwrap());
    foreign_resource.vv = losing_vv;
    foreign_resource.created_at = losing_time;
    foreign_resource.last_writer = "attacker".into();
    let mut missing_resource = foreign_resource.clone();
    missing_resource.id = Uuid::new_v4();
    let foreign = store
        .upsert_resource_meta(attacker.id, &foreign_resource)
        .await
        .unwrap();
    let missing = store
        .upsert_resource_meta(attacker.id, &missing_resource)
        .await
        .unwrap();
    assert!(foreign);
    assert!(missing);
}
```

**What it does** — Stores notebook, tag, and resource metadata at vector `{victim: 2}`, then submits a chronologically newer but vectorially dominated `{victim: 1}` object as the attacker against both the foreign UUID and a fresh UUID, requiring both outcomes to be `true`.

**Dependencies** — `Store::{upsert_notebook, upsert_tag, upsert_resource_meta}`; expects tenant conflicts and absent IDs to be observationally indistinguishable while preserving tenant-owned rows.

**Used by** — `cargo test` and CI; anti-enumeration verifier for F1.

**Repeated context** — The newer incoming timestamp points opposite the strict vector domination, so this test fixes vector comparison as the primary resolution rule rather than accidentally allowing timestamp-first resolution.

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
    let stored_vv = VersionVector::from([("victim".to_string(), 2)]);
    let losing_vv = VersionVector::from([("victim".to_string(), 1)]);
    let victim_time = Utc::now();
    let losing_time = victim_time + Duration::days(1);
    let mut resource = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "victim resource",
        "application/octet-stream",
        "victim.bin",
        1,
    );
    resource.vv = stored_vv.clone();
    resource.created_at = victim_time;
    assert!(store
        .upsert_resource_meta(victim.id, &resource)
        .await
        .unwrap());
    let mut notebook = Notebook::new("victim notebook");
    notebook.vv = stored_vv.clone();
    notebook.updated_at = victim_time;
    assert!(store.upsert_notebook(victim.id, &notebook).await.unwrap());
    let foreign_notebook = store
        .delete_notebook(
            attacker.id,
            notebook.id,
            losing_time,
            &losing_vv,
            "attacker",
        )
        .await
        .unwrap();
    let missing_notebook = store
        .delete_notebook(
            attacker.id,
            Uuid::new_v4(),
            losing_time,
            &losing_vv,
            "attacker",
        )
        .await
        .unwrap();
    assert_eq!(foreign_notebook, missing_notebook);
    assert!(foreign_notebook);
    let mut tag = Tag::new("victim tag");
    tag.vv = stored_vv;
    tag.updated_at = victim_time;
    assert!(store.upsert_tag(victim.id, &tag).await.unwrap());
    let foreign_tag = store
        .delete_tag(attacker.id, tag.id, losing_time, &losing_vv, "attacker")
        .await
        .unwrap();
    let missing_tag = store
        .delete_tag(
            attacker.id,
            Uuid::new_v4(),
            losing_time,
            &losing_vv,
            "attacker",
        )
        .await
        .unwrap();
    assert_eq!(foreign_tag, missing_tag);
    assert!(foreign_tag);
    let foreign = store
        .delete_resource(
            attacker.id,
            resource.id,
            losing_time,
            &losing_vv,
            "attacker",
        )
        .await
        .unwrap();
    let missing = store
        .delete_resource(
            attacker.id,
            Uuid::new_v4(),
            losing_time,
            &losing_vv,
            "attacker",
        )
        .await
        .unwrap();
    assert_eq!(foreign, missing);
    assert!(!foreign);
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

**What it does** — Submits a chronologically newer but strictly vectorially dominated `{victim: 1}` delete against entities stored at `{victim: 2}`. Notebook and tag foreign/missing outcomes must both be `true`; resource-delete foreign/missing outcomes must both remain `false`, and blob-write outcomes remain indistinguishable.

**Dependencies** — `Store::{delete_notebook, delete_tag, delete_resource, put_resource_blob}`; expects each to return the same non-mutating outcome across foreign and absent IDs.

**Used by** — `cargo test` and CI; anti-enumeration verifier for #109 inside the #111 harness.

**Repeated context** — No result may reveal whether another tenant owns the supplied UUID. `delete_resource` has no current loser-vector gap because its missing path returns `false`; its case guards that behavior against later refactoring.

---

## fn write_grantee_cannot_move_foreign_note_or_change_direct_grants

**Identification** — PostgreSQL integration test; marker `// md:fn write_grantee_cannot_move_foreign_note_or_change_direct_grants`.

**Code** — complete and verbatim:

```rust
// md:fn write_grantee_cannot_move_foreign_note_or_change_direct_grants
#[sqlx::test(migrations = "../../migrations")]
async fn write_grantee_cannot_move_foreign_note_or_change_direct_grants(pool: PgPool) {
    let addr = spawn_authorization_server(pool.clone()).await;
    let owner_token = register_and_login(addr, "move-owner@example.com").await;
    let attacker_token = register_and_login(addr, "move-attacker@example.com").await;
    let _carol_token = register_and_login(addr, "move-carol@example.com").await;
    let store = Store::new(pool.clone());
    let owner = store
        .get_user_by_email("move-owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let attacker = store
        .get_user_by_email("move-attacker@example.com")
        .await
        .unwrap()
        .unwrap();
    let carol = store
        .get_user_by_email("move-carol@example.com")
        .await
        .unwrap()
        .unwrap();
    let note = store.create_note(None, "victim", owner.id).await.unwrap();
    store
        .create_or_update_share(note.id, attacker.id, Capabilities::WRITE)
        .await
        .unwrap();
    store
        .create_or_update_share(note.id, carol.id, Capabilities::READ)
        .await
        .unwrap();
    let notebook = Notebook::new("attacker notebook");
    assert!(store.upsert_notebook(attacker.id, &notebook).await.unwrap());
    let before = relation_snapshot(&pool, "note_shares", "note_id", note.id).await;
    let response = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &attacker_token,
        json!({ "notebook_id": notebook.id }),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(
        store.get_note(note.id).await.unwrap().unwrap().notebook_id,
        None
    );
    assert_eq!(
        relation_snapshot(&pool, "note_shares", "note_id", note.id).await,
        before
    );
    let owner_response = reqwest::Client::new()
        .get(format!("http://{addr}/api/notes/{}", note.id))
        .bearer_auth(owner_token)
        .send()
        .await
        .unwrap();
    assert_eq!(owner_response.status(), 200);
}
```

**What it does** — Gives an attacker direct write access, attempts to move the owner's note into the attacker's notebook, and proves the request is forbidden while the note location, direct grants, and owner access remain unchanged.

**Dependencies** — `spawn_authorization_server` — runs the real router; expects the supplied pool clone to share the migrated database. `register_and_login` — creates authenticated principals; expects distinct tenant identities. `Store::{create_note, create_or_update_share, upsert_notebook, get_note}` — prepares and verifies persistent state; expects direct grants and note placement to be independently observable. `relation_snapshot` — captures exact `note_shares`; expects stable ordering. `authed_json` — submits the authenticated PATCH; expects bearer authorization to reach the normal handler.

**Used by** — `cargo test --workspace`; regression coverage for the move-out guard.

**Repeated context** — Direct write permission never authorizes reparenting another owner's note or rewriting its direct grants.

---

## fn strict_inheritance_is_computed_bounded_and_revocable

**Identification** — PostgreSQL integration test; marker `// md:fn strict_inheritance_is_computed_bounded_and_revocable`.

**Code** — complete and verbatim:

```rust
// md:fn strict_inheritance_is_computed_bounded_and_revocable
#[sqlx::test(migrations = "../../migrations")]
async fn strict_inheritance_is_computed_bounded_and_revocable(pool: PgPool) {
    let addr = spawn_authorization_server(pool.clone()).await;
    let owner_token = register_and_login(addr, "scheme-owner@example.com").await;
    let grantee_token = register_and_login(addr, "scheme-grantee@example.com").await;
    let target_token = register_and_login(addr, "scheme-target@example.com").await;
    let store = Store::new(pool);
    let owner = store
        .get_user_by_email("scheme-owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let grantee = store
        .get_user_by_email("scheme-grantee@example.com")
        .await
        .unwrap()
        .unwrap();
    let target = store
        .get_user_by_email("scheme-target@example.com")
        .await
        .unwrap()
        .unwrap();
    let notebook = Notebook::new("shared");
    assert!(store.upsert_notebook(owner.id, &notebook).await.unwrap());
    store
        .create_or_update_notebook_share(notebook.id, grantee.id, Capabilities::ALL)
        .await
        .unwrap();
    let note = store
        .create_note(None, "contained", owner.id)
        .await
        .unwrap();
    let note = store
        .update_note_meta(
            note.id,
            &NotePatch {
                notebook_id: Some(Some(notebook.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let access = resolve_note_access(&store, &note, grantee.id, PermissionScheme::Strict)
        .await
        .unwrap();
    assert_eq!(access.caps.bits(), Capabilities::READ | Capabilities::WRITE);
    assert!(!access.can_share_write());
    let response = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::POST,
        addr,
        &format!("/api/notes/{}/share", note.id),
        &grantee_token,
        json!({ "user_id": target.id, "capabilities": Capabilities::READ }),
    )
    .await;
    assert_eq!(response.status(), 403);
    let owner_access = resolve_note_access(&store, &note, owner.id, PermissionScheme::Strict)
        .await
        .unwrap();
    assert_eq!(owner_access.caps.bits(), Capabilities::ALL);
    let owner_share = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::POST,
        addr,
        &format!("/api/notes/{}/share", note.id),
        &owner_token,
        json!({ "user_id": target.id, "capabilities": Capabilities::READ }),
    )
    .await;
    assert_eq!(owner_share.status(), 200);
    let target_read = reqwest::Client::new()
        .get(format!("http://{addr}/api/notes/{}", note.id))
        .bearer_auth(target_token)
        .send()
        .await
        .unwrap();
    assert_eq!(target_read.status(), 200);
    store
        .delete_notebook_share(notebook.id, grantee.id)
        .await
        .unwrap();
    assert!(
        resolve_note_access(&store, &note, grantee.id, PermissionScheme::Strict)
            .await
            .is_err()
    );
}
```

**What it does** — Proves strict notebook inheritance is computed as read/write without share authority and disappears immediately when the notebook grant is deleted.

**Dependencies** — `Store::{upsert_notebook, create_or_update_notebook_share, create_note, update_note_meta, delete_notebook_share}` — builds a contained note and revokes the parent relationship; expects `NotePatch::notebook_id` to establish containment without note-share materialization. `resolve_note_access` — computes effective access; expects strict inheritance to consult current notebook state. `PermissionScheme::Strict` — selects accepted default semantics; expects inherited sharing to remain disallowed. `Capabilities::{ALL, READ, WRITE}` — establishes and checks masks; expects bit composition to remain stable.

**Used by** — `cargo test --workspace`; regression coverage for computed, bounded, revocable inheritance.

**Repeated context** — Removing a parent grant must revoke child access without a cascade rewrite.

---

## fn deleted_notebook_revokes_inherited_note_access

**Identification** — PostgreSQL integration regression for soft-deleted notebook authorization; marker `// md:fn deleted_notebook_revokes_inherited_note_access`.

**Code** — complete and verbatim:

```rust
// md:fn deleted_notebook_revokes_inherited_note_access
#[sqlx::test(migrations = "../../migrations")]
async fn deleted_notebook_revokes_inherited_note_access(pool: PgPool) {
    let store = Store::new(pool);
    let owner = store
        .create_user("deleted-notebook-owner@example.com", "x", "owner")
        .await
        .unwrap();
    let grantee = store
        .create_user("deleted-notebook-grantee@example.com", "x", "grantee")
        .await
        .unwrap();
    let mut notebook = Notebook::new("deleted notebook");
    notebook.vv = VersionVector::from([("owner".to_string(), 1)]);
    notebook.last_writer = "owner".into();
    assert!(store.upsert_notebook(owner.id, &notebook).await.unwrap());
    store
        .create_or_update_notebook_share(notebook.id, grantee.id, Capabilities::READ)
        .await
        .unwrap();
    let note_id = Uuid::new_v4();
    let note = store
        .create_note(Some(note_id), "contained", owner.id)
        .await
        .unwrap();
    let note = store
        .update_note_meta(
            note.id,
            &NotePatch {
                notebook_id: Some(Some(notebook.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();

    resolve_note_access(&store, &note, grantee.id, PermissionScheme::Strict)
        .await
        .unwrap();
    assert!(store
        .list_notes_for_user(grantee.id, None, None)
        .await
        .unwrap()
        .iter()
        .any(|listed| listed.id == note.id));

    let deletion_vv = VersionVector::from([("owner".to_string(), 2)]);
    assert!(store
        .delete_notebook(
            owner.id,
            notebook.id,
            Utc::now() + Duration::days(1),
            &deletion_vv,
            "owner",
        )
        .await
        .unwrap());

    assert!(matches!(
        resolve_note_access(&store, &note, grantee.id, PermissionScheme::Strict).await,
        Err(keeplin_srv::error::AppError::Forbidden)
    ));
    assert!(!store
        .list_notes_for_user(grantee.id, None, None)
        .await
        .unwrap()
        .iter()
        .any(|listed| listed.id == note.id));
}
```

**What it does** — Creates a foreign note inside a shared notebook, proves the grantee can both resolve and enumerate inherited access, soft-deletes the notebook, then requires the inherited resolver path to return `Forbidden` and the listing path to omit the note.

**Dependencies** — `Store::create_user` — creates distinct owner and grantee principals; expects unique emails to produce isolated users. `Store::{upsert_notebook, create_or_update_notebook_share}` — establishes the inherited-access parent; expects a live shared notebook to grant read access. `VersionVector` and `Notebook::last_writer` — make the deletion causally dominate the inserted notebook revision under conflict resolution. `Store::create_note` — creates the foreign note with an explicit note ID; expects its first argument to be the note ID. `Store::update_note_meta` and `NotePatch::notebook_id` — place the note in the notebook; expects nested `Some` to mean assignment rather than no change or removal. `resolve_note_access` — checks effective strict access; expects deleted notebooks never to supply inheritance. `Store::list_notes_for_user` — checks enumeration; expects deleted notebooks never to expose contained notes through notebook ownership or shares. `Store::delete_notebook` — soft-deletes the parent; expects the dominating vector to win and persist `deleted_at`. `AppError::Forbidden` — distinguishes revoked authorization from an arbitrary failure.

**Used by** — `cargo test --workspace`; mechanical regression for COD-122-01.

**Repeated context** — A soft-deleted authorization parent is absent for both object access and enumeration, even while its historical share row remains stored.

---

## fn moving_from_deleted_notebook_does_not_notify_stale_principals

**Identification** — PostgreSQL-backed HTTP mutation regression for the move guard's inherited-principal enumeration; marker `// md:fn moving_from_deleted_notebook_does_not_notify_stale_principals`.

**Code** — complete and verbatim:

```rust
// md:fn moving_from_deleted_notebook_does_not_notify_stale_principals
#[sqlx::test(migrations = "../../migrations")]
async fn moving_from_deleted_notebook_does_not_notify_stale_principals(pool: PgPool) {
    let (mail_addr, inbox) = spawn_notice_webhook().await;
    let mut config = authorization_test_config();
    config.mail_webhook_url = Some(format!("http://{mail_addr}/mail"));
    let state = Arc::new(AppState::new(config, pool.clone()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let owner_token = register_and_login(addr, "deleted-move-owner@example.com").await;
    let _grantee_token = register_and_login(addr, "deleted-move-grantee@example.com").await;
    let store = Store::new(pool);
    let owner = store
        .get_user_by_email("deleted-move-owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let grantee = store
        .get_user_by_email("deleted-move-grantee@example.com")
        .await
        .unwrap()
        .unwrap();
    let mut notebook = Notebook::new("deleted move source");
    notebook.vv = VersionVector::from([("owner".to_string(), 1)]);
    notebook.last_writer = "owner".into();
    assert!(store.upsert_notebook(owner.id, &notebook).await.unwrap());
    store
        .create_or_update_notebook_share(notebook.id, grantee.id, Capabilities::READ)
        .await
        .unwrap();
    let note_id = Uuid::new_v4();
    let note = store
        .create_note(Some(note_id), "move from deleted", owner.id)
        .await
        .unwrap();
    let note = store
        .update_note_meta(
            note.id,
            &NotePatch {
                notebook_id: Some(Some(notebook.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let deletion_vv = VersionVector::from([("owner".to_string(), 2)]);
    assert!(store
        .delete_notebook(
            owner.id,
            notebook.id,
            Utc::now() + Duration::days(1),
            &deletion_vv,
            "owner",
        )
        .await
        .unwrap());

    let moved = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &owner_token,
        json!({ "notebook_id": Uuid::nil() }),
    )
    .await;
    assert_eq!(moved.status(), 200);
    assert_eq!(
        store.get_note(note.id).await.unwrap().unwrap().notebook_id,
        None
    );
    let stale_recipient_kinds: Vec<String> = inbox
        .lock()
        .await
        .iter()
        .filter(|payload| payload["to"] == grantee.email)
        .map(|payload| payload["kind"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(stale_recipient_kinds, ["verify_email"]);

    let live_grantee_token = register_and_login(addr, "live-move-grantee@example.com").await;
    let mallory_token = register_and_login(addr, "live-notebook-owner@example.com").await;
    let live_grantee = store
        .get_user_by_email("live-move-grantee@example.com")
        .await
        .unwrap()
        .unwrap();
    let mallory = store
        .get_user_by_email("live-notebook-owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let live_notebook = Notebook::new("live move source");
    assert!(store
        .upsert_notebook(mallory.id, &live_notebook)
        .await
        .unwrap());
    let client = reqwest::Client::new();
    let owner_share = authed_json(
        &client,
        reqwest::Method::POST,
        addr,
        &format!("/api/notebooks/{}/share", live_notebook.id),
        &mallory_token,
        json!({ "user_id": owner.id, "capabilities": Capabilities::WRITE }),
    )
    .await;
    assert_eq!(owner_share.status(), 200);
    let grantee_share = authed_json(
        &client,
        reqwest::Method::POST,
        addr,
        &format!("/api/notebooks/{}/share", live_notebook.id),
        &mallory_token,
        json!({ "user_id": live_grantee.id, "capabilities": Capabilities::READ }),
    )
    .await;
    assert_eq!(grantee_share.status(), 200);
    let live_note = store
        .create_note(Some(Uuid::new_v4()), "move from live", owner.id)
        .await
        .unwrap();
    let placed = authed_json(
        &client,
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", live_note.id),
        &owner_token,
        json!({ "notebook_id": live_notebook.id }),
    )
    .await;
    assert_eq!(placed.status(), 200);
    let owner_revoked = authed_json(
        &client,
        reqwest::Method::DELETE,
        addr,
        &format!("/api/notebooks/{}/share/{}", live_notebook.id, owner.id),
        &mallory_token,
        json!({}),
    )
    .await;
    assert_eq!(owner_revoked.status(), 200);
    let ungranted_token = register_and_login(addr, "live-ungranted@example.com").await;
    let listed: Vec<Value> = client
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(&live_grantee_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(listed
        .iter()
        .any(|item| item["id"] == live_note.id.to_string()));
    let ungranted_listed: Vec<Value> = client
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(ungranted_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!ungranted_listed
        .iter()
        .any(|item| item["id"] == live_note.id.to_string()));

    let live_moved = authed_json(
        &client,
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", live_note.id),
        &owner_token,
        json!({ "notebook_id": Uuid::nil() }),
    )
    .await;
    assert_eq!(live_moved.status(), 200);
    let delivered = inbox.lock().await.iter().any(|payload| {
        payload["kind"] == "access_revoked"
            && payload["to"] == live_grantee.email
            && payload["resource_id"] == live_note.id.to_string()
    });
    assert!(delivered);
    assert!(!store
        .list_notes_for_user(live_grantee.id, None, None)
        .await
        .unwrap()
        .iter()
        .any(|listed| listed.id == live_note.id));
    let stale_recipient_kinds: Vec<String> = inbox
        .lock()
        .await
        .iter()
        .filter(|payload| payload["to"] == grantee.email)
        .map(|payload| payload["kind"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(stale_recipient_kinds, ["verify_email"]);
}
```

**What it does** — Creates a note in a shared notebook, causally soft-deletes that notebook, and moves the note to the inbox through the production PATCH handler. The move must commit without sending any `access_revoked` notice to the historical grantee, because deletion already ended that principal's inherited access. It then materializes a live notebook owned by Mallory with `Notebook::new` defaults. Through authenticated handlers, Mallory grants the note owner `WRITE` and a distinct grantee `READ`, the owner moves its note into Mallory's notebook, and Mallory revokes the owner's notebook grant. The test positively establishes that the grantee can list the contained note before the owner moves it back to the inbox. Because the mover no longer controls Mallory's grant, strict authorization permits that exit; the test waits up to two seconds for the expected `access_revoked` payload, verifies that the live grantee can no longer list the moved note, and finally rechecks that the deleted grantee has exactly its one registration `VerifyEmail` payload and no revocation notice. The exact singleton assertion makes the allow-list control fail if registration emits nothing as well as if an unexpected kind appears. This specifically detects `inherited_note_principals` returning share rows from deleted notebooks.

**Dependencies** — `spawn_notice_webhook` — captures actual notification payloads; expects a completed PATCH response to imply all synchronous notice attempts have reached the webhook. `register_and_login`, `router`, and `authed_json` — create the owner, grantees, and Mallory and exercise production share, revoke, and move handlers; expect Mallory to control its notebook grants, `WRITE` to authorize the owner's entry move, revocation to leave the contained note in place, and note ownership to authorize the later unilateral exit. `Store::{upsert_notebook, create_or_update_notebook_share, create_note, update_note_meta}` — establish the deleted fixture and persist both notes plus Mallory's `Notebook::new` live notebook; expect the notebook owner passed to `upsert_notebook` to determine control without forged version metadata. The live fixture deliberately does not use the direct share or metadata-update methods. `VersionVector` and `Store::delete_notebook` — make only the deleted fixture's deletion causally dominate creation; expect its historical notebook-share row to remain while the deleted notebook becomes authorization-inactive. `Store::inherited_note_principals` through `update_note` — enumerates principals potentially affected by each move; expects a deleted source notebook to contribute none and Mallory's live source notebook to contribute its grantee as a principal not controlled by the mover. `Mailer::send_notice` through `notify_access_revoked` — exposes enumeration as webhook payloads; expects an `access_revoked` notice to identify the live grantee and moved note. `tokio::time::sleep` — bounds polling for the positive notice at approximately two seconds; expects webhook delivery to complete within that integration-test allowance. `Store::list_notes_for_user` — proves the live grantee has inherited access before the exit and observes its removal afterward; expects containment plus the live notebook grant to make the note listable until the PATCH commits.

**Used by** — `cargo test --workspace`; mutation verifier for the deleted-notebook JOIN in `Store::inherited_note_principals`.

**Repeated context** — A principal whose inheritance ended at notebook deletion is not affected by a later note move and must not receive an `access_revoked` notice. Under strict authorization, losing inherited access blocks a move only when the mover controls the grant; Mallory's ownership makes the live grantee non-controlled and therefore exercises the notification path while preserving the note owner's unilateral exit. Registration independently emits `VerifyEmail` whenever the test mailer is enabled, even when verification is not required. The production handler awaits notification dispatch inline, and the live positive control supplies the final happens-after barrier for the repeated revocation-absence check.

---

## fn note_owner_has_unilateral_exit_and_failed_notice_does_not_rollback

**Identification** — PostgreSQL-backed HTTP regression; marker `// md:fn note_owner_has_unilateral_exit_and_failed_notice_does_not_rollback`.

**Code** — complete and verbatim:

```rust
// md:fn note_owner_has_unilateral_exit_and_failed_notice_does_not_rollback
#[sqlx::test(migrations = "../../migrations")]
async fn note_owner_has_unilateral_exit_and_failed_notice_does_not_rollback(pool: PgPool) {
    let (subscriber, logs) = capturing_subscriber();
    let dispatch = tracing::Dispatch::new(subscriber);
    let mut config = authorization_test_config();
    config.mail_webhook_url = Some("http://127.0.0.1:1/unreachable".into());
    let state = Arc::new(AppState::new(config, pool.clone()));
    let app = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_app = app.clone();
    tokio::spawn(async move {
        axum::serve(
            listener,
            server_app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let alice_token = register_and_login(addr, "exit-alice@example.com").await;
    let _mallory_token = register_and_login(addr, "exit-mallory@example.com").await;
    let bob_token = register_and_login(addr, "exit-bob@example.com").await;
    let store = Store::new(pool);
    let alice = store
        .get_user_by_email("exit-alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let mallory = store
        .get_user_by_email("exit-mallory@example.com")
        .await
        .unwrap()
        .unwrap();
    let bob = store
        .get_user_by_email("exit-bob@example.com")
        .await
        .unwrap()
        .unwrap();
    let notebook = Notebook::new("mallory");
    assert!(store.upsert_notebook(mallory.id, &notebook).await.unwrap());
    store
        .create_or_update_notebook_share(notebook.id, alice.id, Capabilities::WRITE)
        .await
        .unwrap();
    store
        .create_or_update_notebook_share(notebook.id, bob.id, Capabilities::ALL)
        .await
        .unwrap();
    let note = store.create_note(None, "alice", alice.id).await.unwrap();
    let note = store
        .update_note_meta(
            note.id,
            &NotePatch {
                notebook_id: Some(Some(notebook.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let (response, bob_response) = async {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/notes/{}", note.id))
                    .extension(ConnectInfo(addr))
                    .header("authorization", format!("Bearer {alice_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "notebook_id": Uuid::nil() }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bob_response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/notes/{}", note.id))
                    .extension(ConnectInfo(addr))
                    .header("authorization", format!("Bearer {bob_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        (response, bob_response)
    }
    .with_subscriber(dispatch)
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        store.get_note(note.id).await.unwrap().unwrap().notebook_id,
        None
    );
    assert_eq!(bob_response.status(), 403);
    let recorded = logs.text();
    assert!(recorded.contains("access revocation notice failed"));
    assert!(recorded.contains(&bob.id.to_string()));
    assert!(recorded.contains(&note.id.to_string()));
    assert!(recorded.contains("mail_kind=\"access_revoked\""));
}
```

**What it does** — Proves a note owner can leave another user's notebook despite a fully privileged inherited principal and that an unreachable revocation-notice webhook cannot roll back the committed move. The mutation and Bob's follow-up read are driven directly through cloned router services inside the test-local tracing dispatch, so the handler warning is captured without relying on subscriber propagation into Axum connection tasks.

**Dependencies** — `Store::{create_note, update_note_meta}` and `NotePatch::notebook_id` — establish a note inside the foreign notebook; expects the fixture to distinguish note identity from containment. `update_note` — performs the guarded move; expects ownership to bound the guard. `Mailer::send_notice` — attempts notification after commit; expects delivery failure to remain non-blocking. `resolve_note_access` — reflected by the final HTTP denial; expects inheritance to disappear immediately.

**Used by** — `cargo test --workspace`; ADR 0001 verification rows 12 and 14.

**Repeated context** — Notification is owed but never authority to veto an owner's exit. A process-global tracing subscriber is deliberately avoided because PostgreSQL tests run concurrently; only the two relevant router futures enter this test's capture scope.

---

## fn controlled_inherited_loss_requires_preserve_or_revoke

**Identification** — PostgreSQL-backed move-guard regression; marker `// md:fn controlled_inherited_loss_requires_preserve_or_revoke`.

**Code** — complete and verbatim:

```rust
// md:fn controlled_inherited_loss_requires_preserve_or_revoke
#[sqlx::test(migrations = "../../migrations")]
async fn controlled_inherited_loss_requires_preserve_or_revoke(pool: PgPool) {
    let addr = spawn_authorization_server(pool.clone()).await;
    let alice_token = register_and_login(addr, "guard-alice@example.com").await;
    let _bob_token = register_and_login(addr, "guard-bob@example.com").await;
    let store = Store::new(pool);
    let alice = store
        .get_user_by_email("guard-alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let bob = store
        .get_user_by_email("guard-bob@example.com")
        .await
        .unwrap()
        .unwrap();
    let source = Notebook::new("source");
    let destination = Notebook::new("destination");
    assert!(store.upsert_notebook(alice.id, &source).await.unwrap());
    assert!(store.upsert_notebook(alice.id, &destination).await.unwrap());
    store
        .create_or_update_notebook_share(source.id, bob.id, Capabilities::ALL)
        .await
        .unwrap();
    let note = store.create_note(None, "guarded", alice.id).await.unwrap();
    let note = store
        .update_note_meta(
            note.id,
            &NotePatch {
                notebook_id: Some(Some(source.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let blocked = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &alice_token,
        json!({ "notebook_id": destination.id }),
    )
    .await;
    assert_eq!(blocked.status(), 403);
    let body: Value = blocked.json().await.unwrap();
    assert!(body.to_string().contains(&bob.id.to_string()));
    assert_eq!(
        store.get_note(note.id).await.unwrap().unwrap().notebook_id,
        Some(source.id)
    );
    store
        .create_or_update_share(note.id, bob.id, Capabilities::READ | Capabilities::WRITE)
        .await
        .unwrap();
    let moved = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &alice_token,
        json!({ "notebook_id": destination.id }),
    )
    .await;
    assert_eq!(moved.status(), 200);
    let access = resolve_note_access(
        &store,
        &store.get_note(note.id).await.unwrap().unwrap(),
        bob.id,
        PermissionScheme::Strict,
    )
    .await
    .unwrap();
    assert_eq!(access.caps.bits(), Capabilities::READ | Capabilities::WRITE);
}
```

**What it does** — Proves a controlled inherited loss blocks and names the affected principal without moving the note, then proves a direct preserving grant permits the same move and remains effective.

**Dependencies** — `Store::{create_note, update_note_meta}` and `NotePatch::notebook_id` — establish actual source containment; expects the fixture not to confuse the optional note ID with its notebook ID. `update_note` — evaluates and commits moves; expects the control-bounded loss guard to run before persistence. `Store::create_or_update_share` — installs the Preserve branch; expects direct provenance to survive containment changes. `resolve_note_access` — verifies the resulting capability set.

**Used by** — `cargo test --workspace`; ADR 0001 verification rows 8 and 9.

**Repeated context** — Only principals controlled by the mover block; direct grants do not become containment-derived.

---

## fn legitimate_move_has_exact_strict_ceiling_and_direct_grants_survive_notebook_mutations

**Identification** — PostgreSQL-backed exact-ceiling and grant-provenance regression; marker `// md:fn legitimate_move_has_exact_strict_ceiling_and_direct_grants_survive_notebook_mutations`.

**Code** — complete and verbatim:

```rust
// md:fn legitimate_move_has_exact_strict_ceiling_and_direct_grants_survive_notebook_mutations
#[sqlx::test(migrations = "../../migrations")]
async fn legitimate_move_has_exact_strict_ceiling_and_direct_grants_survive_notebook_mutations(
    pool: PgPool,
) {
    let store = Store::new(pool);
    let owner = store
        .create_user("ceiling-owner@example.com", "x", "owner")
        .await
        .unwrap();
    let member = store
        .create_user("ceiling-member@example.com", "x", "member")
        .await
        .unwrap();
    let note = store.create_note(None, "ceiling", owner.id).await.unwrap();
    let notebook = Notebook::new("ceiling");
    assert!(store.upsert_notebook(owner.id, &notebook).await.unwrap());
    store
        .create_or_update_notebook_share(notebook.id, member.id, Capabilities::ALL)
        .await
        .unwrap();
    store
        .create_or_update_share(note.id, member.id, Capabilities::READ)
        .await
        .unwrap();
    let note = store
        .update_note_meta(
            note.id,
            &NotePatch {
                notebook_id: Some(Some(notebook.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let access = resolve_note_access(&store, &note, member.id, PermissionScheme::Strict)
        .await
        .unwrap();
    assert_eq!(access.caps.bits(), Capabilities::READ | Capabilities::WRITE);
    store
        .create_or_update_notebook_share(notebook.id, member.id, Capabilities::WRITE)
        .await
        .unwrap();
    store
        .delete_notebook_share(notebook.id, member.id)
        .await
        .unwrap();
    let direct = resolve_note_access(&store, &note, member.id, PermissionScheme::Strict)
        .await
        .unwrap();
    assert_eq!(direct.caps.bits(), Capabilities::READ);
}
```

**What it does** — Moves an owner-controlled note into a shared notebook, fixes the effective strict capability set exactly at `READ | WRITE`, then mutates and removes the notebook share and proves the independent direct `READ` grant survives unchanged.

**Dependencies** — `Store::{create_or_update_notebook_share, delete_notebook_share, create_or_update_share, update_note_meta}` — builds and mutates both grant sources; expects notebook-share mutations never to write `note_shares`. `resolve_note_access` — observes the exact normalized union after the move and after parent revocation.

**Used by** — `cargo test --workspace`; ADR 0001 verification rows 3 and 7.

**Repeated context** — Effective capabilities are normalized before comparison; direct grants never depend on containment.

---

## fn inbox_move_preserves_direct_and_drops_uncontrolled_inheritance

**Identification** — PostgreSQL-backed Inbox transition regression; marker `// md:fn inbox_move_preserves_direct_and_drops_uncontrolled_inheritance`.

**Code** — complete and verbatim:

```rust
// md:fn inbox_move_preserves_direct_and_drops_uncontrolled_inheritance
#[sqlx::test(migrations = "../../migrations")]
async fn inbox_move_preserves_direct_and_drops_uncontrolled_inheritance(pool: PgPool) {
    let addr = spawn_authorization_server(pool.clone()).await;
    let owner_token = register_and_login(addr, "inbox-owner@example.com").await;
    let _notebook_owner_token = register_and_login(addr, "inbox-notebook@example.com").await;
    let _inherited_token = register_and_login(addr, "inbox-inherited@example.com").await;
    let _direct_token = register_and_login(addr, "inbox-direct@example.com").await;
    let store = Store::new(pool);
    let owner = store
        .get_user_by_email("inbox-owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let notebook_owner = store
        .get_user_by_email("inbox-notebook@example.com")
        .await
        .unwrap()
        .unwrap();
    let inherited = store
        .get_user_by_email("inbox-inherited@example.com")
        .await
        .unwrap()
        .unwrap();
    let direct = store
        .get_user_by_email("inbox-direct@example.com")
        .await
        .unwrap()
        .unwrap();
    let notebook = Notebook::new("foreign");
    assert!(store
        .upsert_notebook(notebook_owner.id, &notebook)
        .await
        .unwrap());
    store
        .create_or_update_notebook_share(notebook.id, inherited.id, Capabilities::WRITE)
        .await
        .unwrap();
    let note = store.create_note(None, "foreign", owner.id).await.unwrap();
    let note = store
        .update_note_meta(
            note.id,
            &NotePatch {
                notebook_id: Some(Some(notebook.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    store
        .create_or_update_share(note.id, direct.id, Capabilities::WRITE)
        .await
        .unwrap();
    let response = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &owner_token,
        json!({ "notebook_id": Uuid::nil() }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let moved = store.get_note(note.id).await.unwrap().unwrap();
    assert!(
        resolve_note_access(&store, &moved, inherited.id, PermissionScheme::Strict)
            .await
            .is_err()
    );
    let direct_access = resolve_note_access(&store, &moved, direct.id, PermissionScheme::Strict)
        .await
        .unwrap();
    assert_eq!(
        direct_access.caps.bits(),
        Capabilities::from_bits(Capabilities::WRITE).bits()
    );
}
```

**What it does** — Places an owner's note in a foreign notebook, moves it to the Inbox through HTTP, and proves an inherited-only principal loses access on the next resolution while an exact normalized direct `WRITE` grant remains.

**Dependencies** — `update_note` — performs the authorized move; expects uncontrolled inherited loss not to block the note owner. `resolve_note_access` — checks both post-move principals from current persistence state.

**Used by** — `cargo test --workspace`; ADR 0001 verification row 5.

**Repeated context** — `create_note` receives a note ID; containment is established separately with `NotePatch::notebook_id`.

---

## fn inherited_manage_blocks_but_direct_manage_survives_move

**Identification** — PostgreSQL-backed equal-principal provenance regression; marker `// md:fn inherited_manage_blocks_but_direct_manage_survives_move`.

**Code** — complete and verbatim:

```rust
// md:fn inherited_manage_blocks_but_direct_manage_survives_move
#[sqlx::test(migrations = "../../migrations")]
async fn inherited_manage_blocks_but_direct_manage_survives_move(pool: PgPool) {
    let addr = spawn_authorization_server(pool.clone()).await;
    let owner_token = register_and_login(addr, "manage-owner@example.com").await;
    let _member_token = register_and_login(addr, "manage-member@example.com").await;
    let store = Store::new(pool);
    let owner = store
        .get_user_by_email("manage-owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let member = store
        .get_user_by_email("manage-member@example.com")
        .await
        .unwrap()
        .unwrap();
    let source = Notebook::new("manage source");
    let destination = Notebook::new("manage destination");
    assert!(store.upsert_notebook(owner.id, &source).await.unwrap());
    assert!(store.upsert_notebook(owner.id, &destination).await.unwrap());
    store
        .create_or_update_notebook_share(source.id, member.id, Capabilities::MANAGE)
        .await
        .unwrap();
    let note = store.create_note(None, "manage", owner.id).await.unwrap();
    let note = store
        .update_note_meta(
            note.id,
            &NotePatch {
                notebook_id: Some(Some(source.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let blocked = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &owner_token,
        json!({ "notebook_id": destination.id }),
    )
    .await;
    assert_eq!(blocked.status(), 403);
    store
        .delete_notebook_share(source.id, member.id)
        .await
        .unwrap();
    let revoked = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &owner_token,
        json!({ "notebook_id": destination.id }),
    )
    .await;
    assert_eq!(revoked.status(), 200);
    let after_revoke = store.get_note(note.id).await.unwrap().unwrap();
    assert!(
        resolve_note_access(&store, &after_revoke, member.id, PermissionScheme::Strict)
            .await
            .is_err()
    );
    let returned = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &owner_token,
        json!({ "notebook_id": source.id }),
    )
    .await;
    assert_eq!(returned.status(), 200);
    store
        .create_or_update_share(note.id, member.id, Capabilities::MANAGE)
        .await
        .unwrap();
    let moved = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &owner_token,
        json!({ "notebook_id": destination.id }),
    )
    .await;
    assert_eq!(moved.status(), 200);
    let access = resolve_note_access(
        &store,
        &store.get_note(note.id).await.unwrap().unwrap(),
        member.id,
        PermissionScheme::Strict,
    )
    .await
    .unwrap();
    assert_eq!(access.caps.bits(), Capabilities::ALL);
}
```

**What it does** — Proves containment-derived `MANAGE` blocks a move exactly, then revokes that source, grants direct `MANAGE`, performs the same move, and verifies the normalized full set survives.

**Dependencies** — `update_note` — evaluates the move guard; expects only containment-dependent loss to block. `Capabilities::from_bits` through `resolve_note_access` — normalizes `MANAGE` to the exact full capability set.

**Used by** — `cargo test --workspace`; ADR 0001 verification rows 10 and 11.

**Repeated context** — Direct grants remain valid across containment changes and therefore do not participate in the loss guard.

---

## fn strict_notebook_owner_cannot_share_or_eject_foreign_note

**Identification** — PostgreSQL-backed strict-policy regression; marker `// md:fn strict_notebook_owner_cannot_share_or_eject_foreign_note`.

**Code** — complete and verbatim:

```rust
// md:fn strict_notebook_owner_cannot_share_or_eject_foreign_note
#[sqlx::test(migrations = "../../migrations")]
async fn strict_notebook_owner_cannot_share_or_eject_foreign_note(pool: PgPool) {
    let addr = spawn_authorization_server(pool.clone()).await;
    let _note_owner_token = register_and_login(addr, "strict-note-owner@example.com").await;
    let notebook_owner_token = register_and_login(addr, "strict-notebook-owner@example.com").await;
    let _target_token = register_and_login(addr, "strict-target@example.com").await;
    let store = Store::new(pool);
    let note_owner = store
        .get_user_by_email("strict-note-owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let notebook_owner = store
        .get_user_by_email("strict-notebook-owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let target = store
        .get_user_by_email("strict-target@example.com")
        .await
        .unwrap()
        .unwrap();
    let notebook = Notebook::new("strict foreign");
    assert!(store
        .upsert_notebook(notebook_owner.id, &notebook)
        .await
        .unwrap());
    let note = store
        .create_note(None, "strict foreign", note_owner.id)
        .await
        .unwrap();
    let note = store
        .update_note_meta(
            note.id,
            &NotePatch {
                notebook_id: Some(Some(notebook.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let access = resolve_note_access(&store, &note, notebook_owner.id, PermissionScheme::Strict)
        .await
        .unwrap();
    assert_eq!(access.caps.bits(), Capabilities::READ | Capabilities::WRITE);
    let share = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::POST,
        addr,
        &format!("/api/notes/{}/share", note.id),
        &notebook_owner_token,
        json!({ "user_id": target.id, "capabilities": Capabilities::READ }),
    )
    .await;
    assert_eq!(share.status(), 403);
    let eject = authed_json(
        &reqwest::Client::new(),
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &notebook_owner_token,
        json!({ "notebook_id": Uuid::nil() }),
    )
    .await;
    assert_eq!(eject.status(), 403);
    assert_eq!(
        store.get_note(note.id).await.unwrap().unwrap().notebook_id,
        Some(notebook.id)
    );
}
```

**What it does** — Enumerates the notebook owner's exact inherited ceiling over a foreign note, proves that ceiling cannot authorize note sharing, and proves strict policy rejects ejection without changing containment.

**Dependencies** — `resolve_note_access` — fixes the exact `READ | WRITE` ceiling. `create_share` and `update_note` HTTP handlers — expect strict inheritance to exclude share rights and strict ejection policy to require note ownership.

**Used by** — `cargo test --workspace`; ADR 0001 verification rows 16 and 18.

**Repeated context** — Notebook ownership is not note ownership; delete and transfer authority remain ownership-bound too.

---

## fn allowed_foreign_ejection_is_guarded_until_revoke_then_notifies_owner

**Identification** — PostgreSQL-backed ejection guard regression; marker `// md:fn allowed_foreign_ejection_is_guarded_until_revoke_then_notifies_owner`.

**Code** — complete and verbatim:

```rust
// md:fn allowed_foreign_ejection_is_guarded_until_revoke_then_notifies_owner
#[sqlx::test(migrations = "../../migrations")]
async fn allowed_foreign_ejection_is_guarded_until_revoke_then_notifies_owner(pool: PgPool) {
    let (mail_addr, inbox) = spawn_notice_webhook().await;
    let mut config = authorization_test_config();
    config.permission_scheme = PermissionScheme::EjectionAllowed;
    config.mail_webhook_url = Some(format!("http://{mail_addr}/mail"));
    let state = Arc::new(AppState::new(config, pool.clone()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let note_owner_token = register_and_login(addr, "ejection-note-owner@example.com").await;
    let notebook_owner_token =
        register_and_login(addr, "ejection-notebook-owner@example.com").await;
    let _grantee_token = register_and_login(addr, "ejection-grantee@example.com").await;
    inbox.lock().await.clear();
    let store = Store::new(pool);
    let note_owner = store
        .get_user_by_email("ejection-note-owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let notebook_owner = store
        .get_user_by_email("ejection-notebook-owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let grantee = store
        .get_user_by_email("ejection-grantee@example.com")
        .await
        .unwrap()
        .unwrap();
    let notebook = Notebook::new("ejection source");
    assert!(store
        .upsert_notebook(notebook_owner.id, &notebook)
        .await
        .unwrap());
    store
        .create_or_update_notebook_share(notebook.id, grantee.id, Capabilities::READ)
        .await
        .unwrap();
    let note = store
        .create_note(None, "foreign ejection", note_owner.id)
        .await
        .unwrap();
    let note = store
        .update_note_meta(
            note.id,
            &NotePatch {
                notebook_id: Some(Some(notebook.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let client = reqwest::Client::new();
    let blocked = authed_json(
        &client,
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &notebook_owner_token,
        json!({ "notebook_id": Uuid::nil() }),
    )
    .await;
    assert_eq!(blocked.status(), 403);
    let body: Value = blocked.json().await.unwrap();
    assert!(body.to_string().contains(&grantee.id.to_string()));
    assert_eq!(
        store.get_note(note.id).await.unwrap().unwrap().notebook_id,
        Some(notebook.id)
    );
    assert!(inbox.lock().await.is_empty());
    let revoked = authed_json(
        &client,
        reqwest::Method::DELETE,
        addr,
        &format!("/api/notebooks/{}/share/{}", notebook.id, grantee.id),
        &notebook_owner_token,
        json!({}),
    )
    .await;
    assert_eq!(revoked.status(), 200);
    let ejected = authed_json(
        &client,
        reqwest::Method::PATCH,
        addr,
        &format!("/api/notes/{}", note.id),
        &notebook_owner_token,
        json!({ "notebook_id": Uuid::nil() }),
    )
    .await;
    assert_eq!(ejected.status(), 200);
    assert_eq!(
        store.get_note(note.id).await.unwrap().unwrap().notebook_id,
        None
    );
    let notices = inbox.lock().await;
    assert!(notices.iter().any(|payload| {
        payload["kind"] == "access_revoked"
            && payload["to"] == grantee.email
            && payload["resource_id"] == notebook.id.to_string()
    }));
    assert!(notices.iter().any(|payload| {
        payload["kind"] == "access_revoked"
            && payload["to"] == note_owner.email
            && payload["resource_id"] == note.id.to_string()
    }));
    let owner_note = reqwest::Client::new()
        .get(format!("http://{addr}/api/notes/{}", note.id))
        .bearer_auth(note_owner_token)
        .send()
        .await
        .unwrap();
    assert_eq!(owner_note.status(), 200);
}
```

**What it does** — Under `EjectionAllowed`, proves the notebook owner is the guard actor: their controlled grantee blocks ejection by identity and containment remains unchanged with no premature notice. After the owner revokes that notebook grant, the retry succeeds, the note reaches Inbox, the grantee receives the revocation notice, and the foreign note owner receives the ejection notice.

**Dependencies** — `update_note` — enforces the move-out guard and ejection exception; expects controlled notebook grants to block before mutation. `delete_notebook_share` — supplies the explicit Revoke resolution; expects its awaited 200 to include the best-effort notice attempt. `spawn_notice_webhook` — captures both owed notices; expects handler completion to form the observation barrier.

**Used by** — `cargo test --workspace`; ADR 0001 verification row 19 and R4-05.

**Repeated context** — A refused controlled-loss move sends no notice; the later ejection always notifies the foreign note owner even though ownership preserves API access.

---

## fn unknown_permission_scheme_fails_process_startup

**Identification** — process-boundary startup regression; marker `// md:fn unknown_permission_scheme_fails_process_startup`.

**Code** — complete and verbatim:

```rust
// md:fn unknown_permission_scheme_fails_process_startup
#[test]
fn unknown_permission_scheme_fails_process_startup() {
    let output = Command::new(env!("CARGO_BIN_EXE_keeplin-srv"))
        .env("DATABASE_URL", "postgres://unused:unused@127.0.0.1/unused")
        .env("JWT_SECRET", "a-valid-test-secret")
        .env("PERMISSION_SCHEME", "surprising-default")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("unrecognized PERMISSION_SCHEME: surprising-default"));
}
```

**What it does** — Launches the real server binary with an unknown permission scheme, proves the process exits unsuccessfully, and fixes the exact startup diagnostic rather than testing only the enum parser.

**Dependencies** — `CARGO_BIN_EXE_keeplin-srv` — selects Cargo's built server executable; expects integration tests to exercise the production startup path. `Config::from_env` — validates `PERMISSION_SCHEME` before any database connection is attempted.

**Used by** — `cargo test --workspace`; ADR 0001 verification row 15.

**Repeated context** — Unknown policy names fail closed; there is no fallback to `strict` for an explicitly invalid value.

---

## fn direct_duplicate_survives_migration_and_rollback_restores_projection

**Identification** — PostgreSQL migration and recovery regression; marker `// md:fn direct_duplicate_survives_migration_and_rollback_restores_projection`.

**Code** — complete and verbatim:

```rust
// md:fn direct_duplicate_survives_migration_and_rollback_restores_projection
#[sqlx::test(migrations = "../../migrations")]
async fn direct_duplicate_survives_migration_and_rollback_restores_projection(pool: PgPool) {
    let store = Store::new(pool.clone());
    let owner = store
        .create_user("migration-owner@example.com", "x", "owner")
        .await
        .unwrap();
    let member = store
        .create_user("migration-member@example.com", "x", "member")
        .await
        .unwrap();
    let notebook = Notebook::new("migration");
    assert!(store.upsert_notebook(owner.id, &notebook).await.unwrap());
    let note = store
        .create_note(None, "migration", owner.id)
        .await
        .unwrap();
    let note = store
        .update_note_meta(
            note.id,
            &NotePatch {
                notebook_id: Some(Some(notebook.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    store
        .create_or_update_notebook_share(notebook.id, member.id, Capabilities::READ)
        .await
        .unwrap();
    store
        .create_or_update_share(note.id, member.id, Capabilities::READ)
        .await
        .unwrap();
    let before = relation_snapshot(&pool, "note_shares", "note_id", note.id).await;
    sqlx::raw_sql(include_str!(
        "../../../migrations/0017_direct_note_shares.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../migrations/0017_direct_note_shares.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        relation_snapshot(&pool, "note_shares", "note_id", note.id).await,
        before
    );
    store
        .delete_notebook_share(notebook.id, member.id)
        .await
        .unwrap();
    assert!(
        resolve_note_access(&store, &note, member.id, PermissionScheme::Strict)
            .await
            .is_ok()
    );
    store.delete_share(note.id, member.id).await.unwrap();
    store
        .create_or_update_notebook_share(notebook.id, member.id, Capabilities::WRITE)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../migrations/rollback/0017_rematerialize_notebook_shares.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    store
        .delete_notebook_share(notebook.id, member.id)
        .await
        .unwrap();
    let restored = resolve_note_access(&store, &note, member.id, PermissionScheme::Strict)
        .await
        .unwrap();
    assert_eq!(
        restored.caps.bits(),
        Capabilities::from_bits(Capabilities::WRITE).bits()
    );
}
```

**What it does** — Applies the audit migration twice without changing a deliberately ambiguous direct row, proves it remains authoritative after parent revocation, and proves the forward rollback rematerializes inherited access for legacy code.

**Dependencies** — `Store::{create_note, update_note_meta}` and `NotePatch::notebook_id` — establish a genuinely contained migration fixture; expects containment to be independent of the optional note ID. `migrations/0017_direct_note_shares.sql` — audits without mutation; expects repeat application to preserve identical state. `migrations/rollback/0017_rematerialize_notebook_shares.sql` — recreates the legacy projection; expects idempotent union semantics. `resolve_note_access` — verifies both direct survival and restored access.

**Used by** — `cargo test --workspace`; ADR 0001 verification rows 20, 21 and 23.

**Repeated context** — Ambiguity is audited for operator reconciliation and never resolved by deleting user grants.

---

## fn direct_share_migration_reports_each_unattributable_row

**Identification** — PostgreSQL migration-report regression; marker `// md:fn direct_share_migration_reports_each_unattributable_row`.

**Code** — complete and verbatim:

```rust
// md:fn direct_share_migration_reports_each_unattributable_row
#[sqlx::test(migrations = "../../migrations")]
async fn direct_share_migration_reports_each_unattributable_row(pool: PgPool) {
    let store = Store::new(pool.clone());
    let owner = store
        .create_user("report-owner@example.com", "x", "owner")
        .await
        .unwrap();
    let contained_member = store
        .create_user("report-contained@example.com", "x", "contained")
        .await
        .unwrap();
    let inbox_member = store
        .create_user("report-inbox@example.com", "x", "inbox")
        .await
        .unwrap();
    let second_inbox_member = store
        .create_user("report-second-inbox@example.com", "x", "second inbox")
        .await
        .unwrap();
    let notebook = Notebook::new("report");
    assert!(store.upsert_notebook(owner.id, &notebook).await.unwrap());
    let contained_note = store
        .create_note(None, "contained report", owner.id)
        .await
        .unwrap();
    let contained_note = store
        .update_note_meta(
            contained_note.id,
            &NotePatch {
                notebook_id: Some(Some(notebook.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let inbox_note = store
        .create_note(None, "inbox report", owner.id)
        .await
        .unwrap();
    let second_inbox_note = store
        .create_note(None, "second inbox report", owner.id)
        .await
        .unwrap();
    store
        .create_or_update_notebook_share(
            notebook.id,
            contained_member.id,
            Capabilities::READ | Capabilities::WRITE,
        )
        .await
        .unwrap();
    store
        .create_or_update_share(
            contained_note.id,
            contained_member.id,
            Capabilities::READ | Capabilities::WRITE,
        )
        .await
        .unwrap();
    store
        .create_or_update_share(inbox_note.id, inbox_member.id, Capabilities::MANAGE)
        .await
        .unwrap();
    store
        .create_or_update_share(
            second_inbox_note.id,
            second_inbox_member.id,
            Capabilities::READ,
        )
        .await
        .unwrap();
    let (subscriber, logs) = capturing_subscriber();
    sqlx::raw_sql(include_str!(
        "../../../migrations/0017_direct_note_shares.sql"
    ))
    .execute(&pool)
    .with_subscriber(subscriber)
    .await
    .unwrap();
    let report = logs.text();
    assert!(report.contains(&format!(
        "note_id={}, user_id={}, capabilities={}, kind=matches_containing_notebook_share",
        contained_note.id,
        contained_member.id,
        Capabilities::READ | Capabilities::WRITE
    )));
    assert!(report.contains(&format!(
        "note_id={}, user_id={}, capabilities={}, kind=inbox",
        inbox_note.id,
        inbox_member.id,
        Capabilities::MANAGE
    )));
    assert!(report.contains(&format!(
        "note_id={}, user_id={}, capabilities={}, kind=inbox",
        second_inbox_note.id,
        second_inbox_member.id,
        Capabilities::READ
    )));
}
```

**What it does** — Creates both ambiguous legacy classes, including two inbox rows with different note, user, and capability values, reruns migration 0017 under a tracing subscriber, and requires a separate notice carrying every row's exact note, user, capabilities, and classification. The repeated class distinguishes row-level reporting from per-class collapse.

**Dependencies** — `migrations/0017_direct_note_shares.sql` — emits one PostgreSQL NOTICE per ambiguous row; expects exact row-level fields. `sqlx::postgres::notice` through `capturing_subscriber` — makes server NOTICE output assertable without changing production behavior.

**Used by** — `cargo test --workspace`; ADR 0001 verification row 22 and the report that closes REV-122-01.

**Repeated context** — The migration reports and preserves ambiguous rows; it does not decide their provenance.

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
| 2 | `CapturedLogs` | `// md:CapturedLogs` |
| 3 | `impl Write for CapturedLogs` | `// md:impl Write for CapturedLogs` |
| 4 | `impl CapturedLogs` | `// md:impl CapturedLogs` |
| 5 | `fn capturing_subscriber` | `// md:fn capturing_subscriber` |
| 2 | authorization case inventory | `// md:authorization_case_inventory` |
| 3 | `fn mutating_handlers` | `// md:fn mutating_handlers` |
| 4 | `fn source_handlers` | `// md:fn source_handlers` |
| 5 | `fn route_registration_is_confined_to_router` | `// md:fn route_registration_is_confined_to_router` |
| 6 | `fn source_relay_changes` | `// md:fn source_relay_changes` |
| 7 | `fn authorization_inventory_is_complete` | `// md:fn authorization_inventory_is_complete` |
| 8 | `fn inventory_classifications_are_disjoint_and_cases_are_tests` | `// md:fn inventory_classifications_are_disjoint_and_cases_are_tests` |
| 9 | `fn source_inventory_detects_an_uncovered_route` | `// md:fn source_inventory_detects_an_uncovered_route` |
| 10 | `fn authorization_reads_observe_transaction_local_state` | `// md:fn authorization_reads_observe_transaction_local_state` |
| 11 | `fn put_resource_data_checks_blob_write_result` | `// md:fn put_resource_data_checks_blob_write_result` |
| 11 | `fn relay_materialization_uses_authenticated_session_identity` | `// md:fn relay_materialization_uses_authenticated_session_identity` |
| 12 | `fn note_changes_are_explicitly_non_materializing` | `// md:fn note_changes_are_explicitly_non_materializing` |
| 13 | `fn authorization_test_config` | `// md:fn authorization_test_config` |
| 14 | `fn spawn_authorization_server` | `// md:fn spawn_authorization_server` |
| 15 | `fn spawn_notice_webhook` | `// md:fn spawn_notice_webhook` |
| 16 | `fn register_and_login` | `// md:fn register_and_login` |
| 16 | `fn authed_json` | `// md:fn authed_json` |
| 17 | `fn entity_snapshot` | `// md:fn entity_snapshot` |
| 18 | `fn relation_snapshot` | `// md:fn relation_snapshot` |
| 19 | `fn cross_tenant_http_mutations_leave_victim_unchanged` | `// md:fn cross_tenant_http_mutations_leave_victim_unchanged` |
| 20 | `fn denied_http_capabilities_leave_entities_unchanged` | `// md:fn denied_http_capabilities_leave_entities_unchanged` |
| 21 | `fn cross_tenant_store_mutations_leave_victim_unchanged` | `// md:fn cross_tenant_store_mutations_leave_victim_unchanged` |
| 22 | `fn known_defect_115_list_note_tag_ids_exposes_foreign_tag_reference` | `// md:fn known_defect_115_list_note_tag_ids_exposes_foreign_tag_reference` |
| 23 | `fn foreign_and_missing_upserts_are_indistinguishable` | `// md:fn foreign_and_missing_upserts_are_indistinguishable` |
| 24 | `fn foreign_and_missing_mutations_are_indistinguishable` | `// md:fn foreign_and_missing_mutations_are_indistinguishable` |
| 25 | `fn write_grantee_cannot_move_foreign_note_or_change_direct_grants` | `// md:fn write_grantee_cannot_move_foreign_note_or_change_direct_grants` |
| 26 | `fn strict_inheritance_is_computed_bounded_and_revocable` | `// md:fn strict_inheritance_is_computed_bounded_and_revocable` |
| 27 | `fn deleted_notebook_revokes_inherited_note_access` | `// md:fn deleted_notebook_revokes_inherited_note_access` |
| 28 | `fn moving_from_deleted_notebook_does_not_notify_stale_principals` | `// md:fn moving_from_deleted_notebook_does_not_notify_stale_principals` |
| 28 | `fn note_owner_has_unilateral_exit_and_failed_notice_does_not_rollback` | `// md:fn note_owner_has_unilateral_exit_and_failed_notice_does_not_rollback` |
| 29 | `fn controlled_inherited_loss_requires_preserve_or_revoke` | `// md:fn controlled_inherited_loss_requires_preserve_or_revoke` |
| 30 | `fn legitimate_move_has_exact_strict_ceiling_and_direct_grants_survive_notebook_mutations` | `// md:fn legitimate_move_has_exact_strict_ceiling_and_direct_grants_survive_notebook_mutations` |
| 31 | `fn inbox_move_preserves_direct_and_drops_uncontrolled_inheritance` | `// md:fn inbox_move_preserves_direct_and_drops_uncontrolled_inheritance` |
| 32 | `fn inherited_manage_blocks_but_direct_manage_survives_move` | `// md:fn inherited_manage_blocks_but_direct_manage_survives_move` |
| 33 | `fn strict_notebook_owner_cannot_share_or_eject_foreign_note` | `// md:fn strict_notebook_owner_cannot_share_or_eject_foreign_note` |
| 34 | `fn allowed_foreign_ejection_is_guarded_until_revoke_then_notifies_owner` | `// md:fn allowed_foreign_ejection_is_guarded_until_revoke_then_notifies_owner` |
| 34 | `fn unknown_permission_scheme_fails_process_startup` | `// md:fn unknown_permission_scheme_fails_process_startup` |
| 35 | `fn direct_duplicate_survives_migration_and_rollback_restores_projection` | `// md:fn direct_duplicate_survives_migration_and_rollback_restores_projection` |
| 36 | `fn direct_share_migration_reports_each_unattributable_row` | `// md:fn direct_share_migration_reports_each_unattributable_row` |
