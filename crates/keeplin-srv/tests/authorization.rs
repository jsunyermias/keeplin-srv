// md:Overview
use std::{collections::BTreeSet, net::SocketAddr, sync::Arc};

use axum::Router;
use chrono::{Duration, Utc};
use keeplin_core::{
    models::{Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID},
    storage::note_log::VersionVector,
};
use keeplin_srv::{
    config::Config, http::router, permissions::Capabilities, state::AppState, store::Store,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;

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

// md:fn source_handlers
fn source_handlers() -> BTreeSet<String> {
    mutating_handlers(include_str!("../src/http.rs"))
}

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

// md:fn authorization_inventory_is_complete
#[test]
fn authorization_inventory_is_complete() {
    let handler_tenant_inventory = MUTATING_HANDLER_TENANT_CASES
        .iter()
        .chain(MUTATING_HANDLER_TENANT_UNCOVERED)
        .map(|(entry, _)| (*entry).to_string())
        .collect();
    assert_eq!(source_handlers(), handler_tenant_inventory);
    let handler_capability_inventory = MUTATING_HANDLER_CAPABILITY_CASES
        .iter()
        .chain(MUTATING_HANDLER_CAPABILITY_UNCOVERED)
        .map(|(entry, _)| (*entry).to_string())
        .collect();
    assert_eq!(source_handlers(), handler_capability_inventory);

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
    }
}

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

// md:fn entity_snapshot
async fn entity_snapshot(pool: &PgPool, table: &str, id: Uuid) -> String {
    let query = format!("SELECT to_jsonb(t)::text FROM {table} t WHERE id = $1");
    sqlx::query_scalar(&query)
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

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
