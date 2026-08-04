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

// md:fn source_handlers
fn source_handlers() -> BTreeSet<String> {
    mutating_handlers(include_str!("../src/http.rs"))
}

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

// md:fn entity_snapshot
async fn entity_snapshot(pool: &PgPool, table: &str, id: Uuid) -> String {
    let query = format!("SELECT to_jsonb(t)::text FROM {table} t WHERE id = $1");
    sqlx::query_scalar(&query)
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

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
