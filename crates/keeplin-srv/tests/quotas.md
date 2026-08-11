# `tests/quotas.rs` — per-user quota enforcement tests

Self-contained companion for `crates/keeplin-srv/tests/quotas.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand the suite without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use keeplin_core::{
    models::{Resource, SYSTEM_RESOURCE_NOTE_ID},
    storage::{db::DbBackend, ResourceRepository, SyncBackend},
};
use keeplin_srv::{config::Config, http::router, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;
```

**What it does** — Tests of the two optional per-user quotas (both `0` = unlimited by
default) plus the registration switch, driven over real HTTP against a real server on
a throwaway `#[sqlx::test]` PostgreSQL database: the note-count cap
(`MAX_NOTES_PER_USER`, enforced at `POST /api/notes`), the total resource-blob
storage cap (`MAX_USER_STORAGE_BYTES`, enforced at `PUT /api/resources/:id/data`),
and `REGISTRATION_ENABLED=false` (issue #21). Quota rejections are
`507 Insufficient Storage`.

**Dependencies** — `keeplin_srv` (`Config`, `router`, `AppState`), keeplin-core
(`DbBackend`, `Resource`, repository/sync traits — the relay is needed to seed
resource metadata), `reqwest`, `sqlx`, `tempfile`, `tokio`, `serde_json`, `uuid`.

**Used by** — `cargo test`; CI.

**Repeated context** — Quotas are enforced **before** storage at the REST write
point; storage accounting measures actual stored bytes (`octet_length`), counts only
**live** blobs, and excludes the resource being overwritten (no double-count) —
`store.rs::user_blob_bytes_excluding`. Resources are per-user, so the storage quota
is naturally per-account.

---

## fn quota_config

**Identification** — helper; marker `// md:fn quota_config`.
`fn quota_config(max_user_storage_bytes: i64, max_notes_per_user: i64) -> Config`.

**Code** — complete and verbatim:

```rust
// md:fn quota_config
fn quota_config(max_user_storage_bytes: i64, max_notes_per_user: i64) -> Config {
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
        max_user_storage_bytes,
        max_notes_per_user,
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

**What it does** — The suite's `Config` literal with the two quota knobs as the only
variables (everything else standard test posture: open registration, no rate
limit/lockout/key).

**Dependencies** — `Config`. **Used by** — every test.

**Repeated context** — Config literals (never `from_env`) keep the environment out
of test behaviour; a new `Config` field breaks all suites loudly at compile time.

---

## fn spawn

**Identification** — helper; marker `// md:fn spawn`.
`async fn spawn(pool: PgPool, config: Config) -> SocketAddr`.

**Code** — complete and verbatim:

```rust
// md:fn spawn
async fn spawn(pool: PgPool, config: Config) -> SocketAddr {
    let state = Arc::new(AppState::new(config, pool));
    let app: Router = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}
```

**What it does** — Boots the real router with the given config on an ephemeral
loopback port (with `ConnectInfo`, required by the rate-limit middleware's
extractor), on a spawned task.

**Dependencies** — `AppState::new`, `router`. **Used by** — every test.

**Repeated context** — none.

---

## fn register

**Identification** — helper; marker `// md:fn register`. REST registration (fixed
password). **Dependencies** — `reqwest`. **Used by** — every quota test.
**Repeated context** — none.

**Code** — complete and verbatim:

```rust
// md:fn register
async fn register(addr: SocketAddr, email: &str) {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap();
}
```

## fn login

**Identification** — helper; marker `// md:fn login`. REST login returning the
device token. **Dependencies** — `reqwest`. **Used by** — every quota test.
**Repeated context** — none.

**Code** — complete and verbatim:

```rust
// md:fn login
async fn login(addr: SocketAddr, email: &str, device: &str) -> String {
    let body: Value = reqwest::Client::new()
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": email, "password": "password123", "device_name": device }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["token"].as_str().unwrap().to_string()
}
```

---

## fn post_note

**Identification** — helper; marker `// md:fn post_note`.
`async fn post_note(addr, token) -> u16` — POST `/api/notes` with a minimal body,
returning the HTTP status code (the tests assert 200 vs 507).

**Code** — complete and verbatim:

```rust
// md:fn post_note
async fn post_note(addr: SocketAddr, token: &str) -> u16 {
    post_note_response(addr, token).await.status().as_u16()
}
```

**Dependencies** — `reqwest`. **Used by** — the note-quota tests.

**Repeated context** — none.

---

## fn post_note_response

**Identification** — HTTP helper returning the complete response; marker `// md:fn post_note_response`.

**Code** — complete and verbatim:

```rust
// md:fn post_note_response
async fn post_note_response(addr: SocketAddr, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/notes"))
        .bearer_auth(token)
        .json(&json!({ "title": "n" }))
        .send()
        .await
        .unwrap()
}
```

**What it does** — Sends note creation while preserving status and body for wire-contract assertions.

**Dependencies** — `reqwest::Client::send` — performs the request; expects the response to retain its exact bytes.

**Used by** — quota refusal and concurrency tests.

**Repeated context** — Quota refusal bodies are compatibility surfaces.

---

## fn import_note

**Identification** — HTTP import test helper; marker `// md:fn import_note`.

**Code** — complete and verbatim:

```rust
// md:fn import_note
async fn import_note(addr: SocketAddr, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/import"))
        .bearer_auth(token)
        .json(&json!({ "title": "imported", "body": "one\ntwo" }))
        .send()
        .await
        .unwrap()
}
```

**What it does** — Sends a two-line authenticated note import and returns the full response for status and body assertions.

**Dependencies** — `reqwest` — performs the request; expects `/api/import` to preserve the quota refusal contract.

**Used by** — import quota tests.

**Repeated context** — imports create counted notes.

## fn device

**Identification** — helper; marker `// md:fn device`.
`async fn device(addr, token) -> DbBackend` — a real server-mode relay client on a
leaked temp SQLite file, connected to `ws://…/api/sync`.

**Code** — complete and verbatim:

```rust
// md:fn device
async fn device(addr: SocketAddr, token: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, &format!("ws://{addr}/api/sync"), token)
        .await
        .unwrap()
}
```

**Dependencies** — keeplin-core `DbBackend::new`, `tempfile`. **Used by** —
`seed_resource` callers (storage-quota tests).

**Repeated context** — Resource **metadata** only travels the relay; the blob is
out-of-band — which is exactly why the tests need a relay device to seed metadata
before `PUT`ting bytes.

---

## fn seed_resource

**Identification** — helper; marker `// md:fn seed_resource`.
`async fn seed_resource(dev: &DbBackend) -> Uuid`.

**Code** — complete and verbatim:

```rust
// md:fn seed_resource
async fn seed_resource(addr: SocketAddr, token: &str, dev: &DbBackend) -> Uuid {
    let resource = dev
        .create_resource(
            Resource::new(
                SYSTEM_RESOURCE_NOTE_ID,
                "f",
                "application/octet-stream",
                "f.bin",
                0,
            ),
            vec![],
        )
        .await
        .unwrap();
    let changes = dev
        .get_changes_since(chrono::DateTime::from_timestamp(0, 0).unwrap())
        .await
        .unwrap();
    dev.send_changes(changes).await.unwrap();
    for _ in 0..50 {
        let resources: Value = reqwest::Client::new()
            .get(format!("http://{addr}/api/resources"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if resources
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["id"] == resource.id.to_string())
        {
            return resource.id;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("resource did not materialize");
}
```

**What it does** — Creates resource metadata with an **empty** blob through the
relay (`create_resource` → `get_changes_since(epoch)` → `send_changes`, then a
short sleep for materialisation) and returns its id — so the test controls the
stored size purely via `put_blob`; the quota measures actual stored bytes, not the
declared `size`.

**Dependencies** — keeplin-core resource/sync APIs. **Used by** — the storage-quota
tests.

**Repeated context** — none.

---

## fn put_blob

**Identification** — helper; marker `// md:fn put_blob`.
`async fn put_blob(addr, token, id, len) -> u16` — PUT `len` bytes to
`/api/resources/:id/data`, returning the status code.

**Code** — complete and verbatim:

```rust
// md:fn put_blob
async fn put_blob(addr: SocketAddr, token: &str, id: Uuid, len: usize) -> u16 {
    put_blob_response(addr, token, id, len)
        .await
        .status()
        .as_u16()
}
```

**Dependencies** — `reqwest`. **Used by** — the storage-quota tests.

**Repeated context** — none.

---

## fn put_blob_response

**Identification** — HTTP blob helper returning the complete response; marker `// md:fn put_blob_response`.

**Code** — complete and verbatim:

```rust
// md:fn put_blob_response
async fn put_blob_response(
    addr: SocketAddr,
    token: &str,
    id: Uuid,
    len: usize,
) -> reqwest::Response {
    reqwest::Client::new()
        .put(format!("http://{addr}/api/resources/{id}/data"))
        .bearer_auth(token)
        .body(vec![7u8; len])
        .send()
        .await
        .unwrap()
}
```

**What it does** — Uploads bytes while preserving the response for concurrent assertions.

**Dependencies** — `reqwest::Client::send` — performs the request; expects completion only after the handler transaction resolves.

**Used by** — blob quota concurrency tests.

**Repeated context** — Stored bytes, not metadata size, consume this quota.

---

## fn install_quota_barrier

**Identification** — PostgreSQL trigger-barrier helper; marker `// md:fn install_quota_barrier`.

**Code** — complete and verbatim:

```rust
// md:fn install_quota_barrier
async fn install_quota_barrier(pool: &PgPool, table: &str) {
    sqlx::query(
        "CREATE FUNCTION quota_write_barrier() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(7100003); RETURN NEW; END $$",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(&format!(
        "CREATE TRIGGER quota_write_barrier BEFORE INSERT OR UPDATE ON {table} FOR EACH ROW EXECUTE FUNCTION quota_write_barrier()"
    ))
    .execute(pool)
    .await
    .unwrap();
}
```

**What it does** — Installs a transaction-scoped advisory-lock barrier immediately before a quota-bearing write.

**Dependencies** — `sqlx::query` — installs the function and trigger; expects the per-test database to isolate their names.

**Used by** — deterministic ADR 0003 rendezvous tests.

**Repeated context** — The barrier is test-only and sits between the deciding read and write.

---

## fn wait_for_barrier

**Identification** — bounded lock-observation helper; marker `// md:fn wait_for_barrier`.

**Code** — complete and verbatim:

```rust
// md:fn wait_for_barrier
async fn wait_for_barrier(pool: &PgPool, expected: i64) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks locks JOIN pg_database database ON database.oid = locks.database WHERE database.datname = current_database() AND locks.locktype = 'advisory' AND NOT locks.granted AND locks.objid = 7100003",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting >= expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
```

**What it does** — Polls `pg_locks` until the requested number of writers has reached the forced barrier, failing after five seconds.

**Dependencies** — `pg_locks` — exposes waiting advisory locks; expects database filtering to exclude other test databases.

**Used by** — deterministic quota concurrency tests.

**Repeated context** — Timeout failure is mutation evidence for an over-coarse production key.

---

## fn wait_for_advisory_waiters

**Identification** — bounded global advisory-wait observation helper; marker `// md:fn wait_for_advisory_waiters`.

**Code** — complete and verbatim:

```rust
// md:fn wait_for_advisory_waiters
async fn wait_for_advisory_waiters(pool: &PgPool, expected: i64) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks locks JOIN pg_database database ON database.oid = locks.database WHERE database.datname = current_database() AND locks.locktype = 'advisory' AND NOT locks.granted",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting >= expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
```

**What it does** — Waits until both the write-barrier participant and the serialized same-user request are visibly blocked.

**Dependencies** — `pg_locks` — exposes every waiting advisory request in the isolated database; expects a five-second timeout to turn a missing interleaving into a failure.

**Used by** — same-user note and blob serialization tests.

**Repeated context** — This removes scheduler luck from the rendezvous.

---

## fn hold_barrier

**Identification** — barrier owner helper; marker `// md:fn hold_barrier`.

**Code** — complete and verbatim:

```rust
// md:fn hold_barrier
async fn hold_barrier(pool: &PgPool) -> sqlx::pool::PoolConnection<sqlx::Postgres> {
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(7100003)")
        .execute(&mut *blocker)
        .await
        .unwrap();
    blocker
}
```

**What it does** — Holds the test advisory lock on a dedicated session.

**Dependencies** — `pg_advisory_lock` — blocks trigger participants; expects session ownership until explicit release.

**Used by** — deterministic quota concurrency tests.

**Repeated context** — Session locks require explicit release.

---

## fn release_barrier

**Identification** — barrier release helper; marker `// md:fn release_barrier`.

**Code** — complete and verbatim:

```rust
// md:fn release_barrier
async fn release_barrier(mut blocker: sqlx::pool::PoolConnection<sqlx::Postgres>) {
    sqlx::query("SELECT pg_advisory_unlock(7100003)")
        .execute(&mut *blocker)
        .await
        .unwrap();
}
```

**What it does** — Releases and returns the session that owns the test barrier.

**Dependencies** — `pg_advisory_unlock` — releases the exact test key; expects the same session that acquired it.

**Used by** — deterministic quota concurrency tests.

**Repeated context** — none.

---

## fn registration_can_be_disabled

**Identification** — `#[sqlx::test]`; marker
`// md:fn registration_can_be_disabled`.

**Code** — complete and verbatim:

```rust
// md:fn registration_can_be_disabled
#[sqlx::test(migrations = "../../migrations")]
async fn registration_can_be_disabled(pool: PgPool) {
    let mut config = quota_config(0, 0);
    config.registration_enabled = false;
    let addr = spawn(pool, config).await;

    let code = reqwest::Client::new()
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "a@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(code, 403, "registration must be closed when disabled");
}
```

**What it does** — With `registration_enabled = false`, `POST /api/register`
answers `403` (issue #21): the open signup endpoint is closed while everything else
still runs.

**Dependencies** — `quota_config`, `spawn`. **Used by** — `cargo test`.

**Repeated context** — Pins the issue #21 switch.

---

## fn note_quota_blocks_creation_past_the_limit

**Identification** — `#[sqlx::test]`; marker
`// md:fn note_quota_blocks_creation_past_the_limit`.

**Code** — complete and verbatim:

```rust
// md:fn note_quota_blocks_creation_past_the_limit
#[sqlx::test(migrations = "../../migrations")]
async fn note_quota_blocks_creation_past_the_limit(pool: PgPool) {
    let addr = spawn(pool, quota_config(0, 2)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;

    assert_eq!(post_note(addr, &token).await, 200);
    assert_eq!(post_note(addr, &token).await, 200);
    assert_eq!(
        post_note(addr, &token).await,
        507,
        "third note is over quota"
    );
}
```

**What it does** — Limit 2: the first two `POST /api/notes` are 200, the third is
**507**.

**Dependencies** — the helpers. **Used by** — `cargo test`.

**Repeated context** — The count is of **live owned** notes
(`count_live_notes_for_user`) — soft-deleted notes don't consume quota.

---

## fn note_quota_blocks_import_past_the_limit

**Identification** — import bypass regression test; marker `// md:fn note_quota_blocks_import_past_the_limit`.

**Code** — complete and verbatim:

```rust
// md:fn note_quota_blocks_import_past_the_limit
#[sqlx::test(migrations = "../../migrations")]
async fn note_quota_blocks_import_past_the_limit(pool: PgPool) {
    let addr = spawn(pool, quota_config(0, 1)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;

    assert_eq!(import_note(addr, &token).await.status().as_u16(), 200);
    let create_refusal = post_note_response(addr, &token).await;
    let import_refusal = import_note(addr, &token).await;
    assert_eq!(create_refusal.status().as_u16(), 507);
    assert_eq!(import_refusal.status().as_u16(), 507);
    let create_body = create_refusal.bytes().await.unwrap();
    let import_body = import_refusal.bytes().await.unwrap();
    assert_eq!(create_body, import_body);
    assert_eq!(
        import_body.as_ref(),
        br#"{"error":"note limit reached (1)"}"#
    );
}
```

**What it does** — Fills a one-note quota through import, then proves the formerly unguarded path returns the established byte-level 507 refusal.

**Dependencies** — `import_note` — exercises the real endpoint; expects the second call to create no note or lines.

**Used by** — PostgreSQL integration suite.

**Repeated context** — the synchronization-path refusal remains deferred.

## fn quota_write_inventory_is_complete

**Identification** — production write and advisory-lock structural inventory; marker `// md:fn quota_write_inventory_is_complete`.

**Code** — complete and verbatim:

```rust
// md:fn quota_write_inventory_is_complete
#[test]
fn quota_write_inventory_is_complete() {
    let http = include_str!("../src/http.rs");
    let sync = include_str!("../src/sync.rs");
    let projection = include_str!("../src/projection.rs");
    let store = include_str!("../src/store.rs");

    assert_eq!(http.matches(".create_note(").count(), 1);
    assert_eq!(http.matches(".create_note_on(").count(), 2);
    assert!(http.matches("lock_note_quota").count() >= 2);
    assert_eq!(http.matches(".put_resource_blob(").count(), 1);
    assert_eq!(http.matches(".put_resource_blob_on(").count(), 1);
    assert!(http.contains("lock_blob_quota"));
    assert_eq!(projection.matches(".apply_resource_create(").count(), 1);
    assert!(!sync.contains("lock_blob_quota"));
    assert!(!projection.contains("lock_blob_quota"));
    assert_eq!(
        store.matches("pg_advisory_xact_lock").count(),
        1,
        "all production advisory locks must use the shared domain constructor"
    );
    for domain in [
        "NoteOrder",
        "TagProjection",
        "NoteTagProjection",
        "ResourceProjection",
        "NoteQuota",
        "BlobQuota",
    ] {
        assert!(store.contains(domain), "missing lock domain {domain}");
    }
    assert!(store.contains("count_live_notes_for_user_on"));
    assert!(!http.contains(".count_live_notes_for_user("));
    assert!(!http.contains(".user_blob_bytes_excluding("));
    assert!(!http.contains("run_serializable"));
    let note_quota = store
        .split("// md:impl Store > fn lock_note_quota")
        .nth(1)
        .unwrap()
        .split(&["//", " md:"].concat())
        .next()
        .unwrap();
    let blob_quota = store
        .split("// md:impl Store > fn lock_blob_quota")
        .nth(1)
        .unwrap()
        .split(&["//", " md:"].concat())
        .next()
        .unwrap();
    assert!(note_quota.contains("AdvisoryLockDomain::NoteQuota"));
    assert!(!note_quota.contains("AdvisoryLockDomain::NoteOrder"));
    assert!(blob_quota.contains("AdvisoryLockDomain::BlobQuota"));
    for marker in ["// md:fn create_note", "// md:fn import_note"] {
        let handler = http
            .split(marker)
            .nth(1)
            .unwrap()
            .split(&["//", " md:"].concat())
            .next()
            .unwrap();
        assert!(
            handler.find("lock_note_quota").unwrap()
                < handler.find("count_live_notes_for_user_on").unwrap()
        );
    }
    let blob_handler = http
        .split("// md:fn put_resource_data")
        .nth(1)
        .unwrap()
        .split(&["//", " md:"].concat())
        .next()
        .unwrap();
    assert!(
        blob_handler.find("lock_blob_quota").unwrap()
            < blob_handler.find("user_blob_bytes_excluding_on").unwrap()
    );
}
```

**What it does** — Pins HTTP counted-object call sites, the explicitly deferred sync blob site, executor-aware quota reads, absence of either pool-backed quota-read method regardless of argument formatting, six named lock domains, the single production advisory-lock constructor, and absence of ADR 0002 retries.

**Dependencies** — `include_str!` — reads production sources at compile time; expects literal call sites and domain variants to remain inventory-visible.

**Used by** — non-database test suite and mutation evidence for ADR 0003 rows 1, 7, 11, 11b, 13, and 15.

**Repeated context** — any new counted-object write must be classified before this inventory changes.

## fn blob_write_to_tombstoned_resource_is_refused

**Identification** — disabled-quota tombstone-write regression test; marker `// md:fn blob_write_to_tombstoned_resource_is_refused`.

**Code** — complete and verbatim:

```rust
// md:fn blob_write_to_tombstoned_resource_is_refused
#[sqlx::test(migrations = "../../migrations")]
async fn blob_write_to_tombstoned_resource_is_refused(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(0, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let dev = device(addr, &token).await;
    let resource_id = seed_resource(addr, &token, &dev).await;
    sqlx::query("UPDATE resources SET deleted_at = now() WHERE id = $1")
        .bind(resource_id)
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(put_blob(addr, &token, resource_id, 64).await, 404);
    let stored: i64 = sqlx::query_scalar(
        "SELECT octet_length(data)::bigint FROM resource_blobs WHERE resource_id = $1",
    )
    .bind(resource_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, 0);
}
```

**What it does** — Tombstones a materialized resource while quota enforcement is disabled, attempts an HTTP blob upload, and requires both a 404 refusal and preservation of the zero-byte blob seeded by synchronization. Reverting the live-resource predicate makes the request return 200 and replaces the seeded blob with the 64-byte request body, killing both assertions.

**Dependencies** —
- `quota_config` — disables storage quota for this server; expects zero to select the handler's non-quota branch.
- `seed_resource` — materializes resource metadata and a zero-byte blob through synchronization; expects the returned ID to belong to the authenticated user.
- `sqlx::query` — tombstones the target directly; expects `resources.deleted_at` to control liveness without deleting metadata.
- `put_blob` — exercises the HTTP PUT endpoint; expects a refused store write to map to 404.
- `sqlx::query_scalar` — measures the target's persisted blob length; expects synchronization to have seeded one empty row and a refused HTTP write to leave it unchanged.

**Used by** — PostgreSQL integration suite and predicate-revert mutation check.

**Repeated context** — tombstone-write refusal is independent of quota configuration.

## fn over_limit_blob_write_to_tombstoned_resource_is_not_found

**Identification** — quota-enabled status precedence regression test; marker `// md:fn over_limit_blob_write_to_tombstoned_resource_is_not_found`.

**Code** — complete and verbatim:

```rust
// md:fn over_limit_blob_write_to_tombstoned_resource_is_not_found
#[sqlx::test(migrations = "../../migrations")]
async fn over_limit_blob_write_to_tombstoned_resource_is_not_found(pool: PgPool) {
    let limit = 100;
    let addr = spawn(pool.clone(), quota_config(limit, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let dev = device(addr, &token).await;
    let live_resource_id = seed_resource(addr, &token, &dev).await;
    sqlx::query("UPDATE resource_blobs SET data = $2 WHERE resource_id = $1")
        .bind(live_resource_id)
        .bind(vec![7u8; limit as usize + 1])
        .execute(&pool)
        .await
        .unwrap();
    let tombstoned_resource_id = seed_resource(addr, &token, &dev).await;
    sqlx::query("UPDATE resources SET deleted_at = now() WHERE id = $1")
        .bind(tombstoned_resource_id)
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(put_blob(addr, &token, tombstoned_resource_id, 1).await, 404);
}
```

**What it does** — Creates one separate live resource whose persisted 101-byte blob already exceeds the enabled 100-byte limit, tombstones a zero-byte target, then requires PUT on that target to return `404`. Removing the early live-resource gate exposes quota precedence and changes the result to `507`, killing this test.

**Dependencies** —
- `quota_config` — enables a 100-byte storage limit; expects positive values to select the quota transaction branch.
- `seed_resource` — creates distinct live and target resources with zero-byte blob rows; expects synchronization to materialize both before direct test setup.
- `sqlx::query` — makes the separate live blob exceed the limit and tombstones the target; expects direct fixture setup to leave the target blob empty.
- `put_blob` — exercises the HTTP PUT endpoint; expects the early tombstone decision to precede aggregate quota refusal.

**Used by** — PostgreSQL integration suite and early-gate-revert mutation check.

**Repeated context** — quota accounting remains live-resource-only; the over-limit bytes belong to a separate live resource, not the tombstoned target.

## fn tombstoned_blobs_cannot_exceed_the_storage_limit

**Identification** — measured retained-storage bound regression test; marker `// md:fn tombstoned_blobs_cannot_exceed_the_storage_limit`.

**Code** — complete and verbatim:

```rust
// md:fn tombstoned_blobs_cannot_exceed_the_storage_limit
#[sqlx::test(migrations = "../../migrations")]
async fn tombstoned_blobs_cannot_exceed_the_storage_limit(pool: PgPool) {
    let limit = 100;
    let addr = spawn(pool.clone(), quota_config(limit, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let dev = device(addr, &token).await;
    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email = $1")
        .bind("a@example.com")
        .fetch_one(&pool)
        .await
        .unwrap();

    for _ in 0..3 {
        let resource_id = seed_resource(addr, &token, &dev).await;
        sqlx::query("UPDATE resources SET deleted_at = now() WHERE id = $1")
            .bind(resource_id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(put_blob(addr, &token, resource_id, 60).await, 404);
        let stored: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(octet_length(rb.data)), 0) FROM resource_blobs rb JOIN resources r ON r.id = rb.resource_id WHERE r.user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(stored <= limit);
    }
}
```

**What it does** — Repeats materialize, tombstone, and refused 60-byte upload cycles under a 100-byte limit. After every attempt it measures all bytes physically held for the user, deliberately without a `deleted_at` filter, and proves the measured total never exceeds the limit. Reverting the predicate makes the first status assertion fail; if statuses were ignored, the second cycle would measure 120 bytes and fail the bound.

**Dependencies** —
- `quota_config` — enables a 100-byte live-resource storage cap; expects the handler to take its quota transaction branch.
- `seed_resource` — creates each distinct resource through synchronization; expects resource metadata to materialize before tombstoning.
- `sqlx::query` — tombstones each target; expects retained metadata to remain joinable to blob storage.
- `put_blob` — attempts the real HTTP write; expects tombstoned targets to map to 404.
- `sqlx::query_scalar` — resolves the user and sums `octet_length(resource_blobs.data)` across all that user's resources; expects the unfiltered join to represent physical retained bytes.

**Used by** — PostgreSQL integration suite and predicate-revert mutation check.

**Repeated context** — the production quota aggregate remains live-resource-only; this test measures a broader physical-storage safety property without redefining quota accounting.

## fn tombstoned_resource_blob_remains_readable

**Identification** — retained tombstone-read regression test; marker `// md:fn tombstoned_resource_blob_remains_readable`.

**Code** — complete and verbatim:

```rust
// md:fn tombstoned_resource_blob_remains_readable
#[sqlx::test(migrations = "../../migrations")]
async fn tombstoned_resource_blob_remains_readable(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(0, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let dev = device(addr, &token).await;
    let resource_id = seed_resource(addr, &token, &dev).await;
    assert_eq!(put_blob(addr, &token, resource_id, 64).await, 200);
    sqlx::query("UPDATE resources SET deleted_at = now() WHERE id = $1")
        .bind(resource_id)
        .execute(&pool)
        .await
        .unwrap();

    let response = reqwest::Client::new()
        .get(format!("http://{addr}/api/resources/{resource_id}/data"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.bytes().await.unwrap().as_ref(), &[7u8; 64]);
}
```

**What it does** — Stores a blob while its resource is live, tombstones only the metadata, and proves authenticated GET still returns the exact retained bytes with status 200.

**Dependencies** —
- `put_blob` — seeds the known byte pattern through the live HTTP write path; expects a 64-byte body filled with byte value seven.
- `sqlx::query` — tombstones metadata without deleting the blob; expects retention to preserve `resource_blobs`.
- `reqwest::Client::get` — exercises the HTTP read path; expects ownership authorization to include tombstoned metadata during the retention window.
- `reqwest::Response::bytes` — reads the returned body; expects byte-for-byte preservation.

**Used by** — PostgreSQL integration suite guarding the intentional GET behavior.

**Repeated context** — resource purge may later reclaim tombstoned blobs, but tombstoning alone retains them for reads.

## fn quota_paths_do_not_use_serializable_retry_or_service_unavailable

**Identification** — ADR 0003 row 13 structural regression test; marker `// md:fn quota_paths_do_not_use_serializable_retry_or_service_unavailable`.

**Code** — complete and verbatim:

```rust
// md:fn quota_paths_do_not_use_serializable_retry_or_service_unavailable
#[test]
fn quota_paths_do_not_use_serializable_retry_or_service_unavailable() {
    let http = include_str!("../src/http.rs");

    for marker in [
        "// md:fn create_note",
        "// md:fn import_note",
        "// md:fn put_resource_data",
    ] {
        let handler = http
            .split(marker)
            .nth(1)
            .unwrap()
            .split(&["//", " md:"].concat())
            .next()
            .unwrap();
        for forbidden in [
            "serializable(",
            "AppError::ServiceUnavailable",
            "StatusCode::SERVICE_UNAVAILABLE",
            "503",
        ] {
            assert!(
                !handler.contains(forbidden),
                "quota handler {marker} contains forbidden retry/503 token {forbidden}"
            );
        }
    }
}
```

**What it does** — Extracts each quota-bearing HTTP handler by its companion marker and rejects the ADR 0002 retry helper plus the native enum, status constant, and numeric forms of a 503 response. Adding `serializable(state.clone(), ...)` around any listed handler or returning service unavailable makes this test fail; internal-error returns remain allowed.

**Dependencies** — `include_str!` — reads the production HTTP source at compile time; expects each quota handler to retain its unique companion marker and the next marker to delimit its body.

**Used by** — non-database test suite and mutation evidence for ADR 0003 row 13.

**Repeated context** — ADR 0003 deliberately uses advisory-lock waiting with ordinary internal-error recovery rather than ADR 0002's bounded serialization retry and exhaustion response.

## fn concurrent_blob_quota_writes_serialize_before_the_deciding_read

**Identification** — ADR 0003 rows 4 and 6 concurrency test; marker `// md:fn concurrent_blob_quota_writes_serialize_before_the_deciding_read`.

**Code** — complete and verbatim:

```rust
// md:fn concurrent_blob_quota_writes_serialize_before_the_deciding_read
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_blob_quota_writes_serialize_before_the_deciding_read(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(100, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let dev = device(addr, &token).await;
    let left_id = seed_resource(addr, &token, &dev).await;
    let right_id = seed_resource(addr, &token, &dev).await;
    install_quota_barrier(&pool, "resource_blobs").await;
    let blocker = hold_barrier(&pool).await;
    let left_token = token.clone();
    let left = tokio::spawn(async move { put_blob_response(addr, &left_token, left_id, 60).await });
    wait_for_barrier(&pool, 1).await;
    let right_token = token.clone();
    let right =
        tokio::spawn(async move { put_blob_response(addr, &right_token, right_id, 60).await });
    wait_for_advisory_waiters(&pool, 2).await;
    assert!(!right.is_finished());
    release_barrier(blocker).await;
    let left = left.await.unwrap();
    let right = right.await.unwrap();
    let mut statuses = [left.status().as_u16(), right.status().as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 507]);
    let stored: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(octet_length(data)), 0) FROM resource_blobs")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, 60);
}
```

**What it does** — Forces two same-user blob writes across the read/write boundary and proves only one jointly-over-limit write commits.

**Dependencies** — `install_quota_barrier` — forces the interleaving; expects the first writer to retain its quota lock while blocked.

**Used by** — ADR 0003 verification matrix rows 4 and 6.

**Repeated context** — Moving the production lock after the aggregate read makes both writes commit and this test fail.

---

## fn concurrent_note_quota_writes_serialize_and_keep_the_refusal_body

**Identification** — ADR 0003 rows 5, 6, and 12 test; marker `// md:fn concurrent_note_quota_writes_serialize_and_keep_the_refusal_body`.

**Code** — complete and verbatim:

```rust
// md:fn concurrent_note_quota_writes_serialize_and_keep_the_refusal_body
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_note_quota_writes_serialize_and_keep_the_refusal_body(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(0, 1)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    install_quota_barrier(&pool, "notes").await;
    let blocker = hold_barrier(&pool).await;
    let left_token = token.clone();
    let left = tokio::spawn(async move { post_note_response(addr, &left_token).await });
    wait_for_barrier(&pool, 1).await;
    let right_token = token.clone();
    let right = tokio::spawn(async move { post_note_response(addr, &right_token).await });
    wait_for_advisory_waiters(&pool, 2).await;
    assert!(!right.is_finished());
    release_barrier(blocker).await;
    let responses = [left.await.unwrap(), right.await.unwrap()];
    let mut success = 0;
    let mut contended_refusal = None;
    for response in responses {
        if response.status().as_u16() == 200 {
            success += 1;
        } else {
            assert_eq!(response.status().as_u16(), 507);
            contended_refusal = Some(response.bytes().await.unwrap());
        }
    }
    assert_eq!(success, 1);
    let uncontended = post_note_response(addr, &token).await;
    assert_eq!(uncontended.status().as_u16(), 507);
    assert_eq!(
        contended_refusal.unwrap(),
        uncontended.bytes().await.unwrap()
    );
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notes WHERE deleted_at IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 1);
}
```

**What it does** — Forces same-user note creation contention, proves exactly one commit, and compares contended and uncontended refusal bytes.

**Dependencies** — `post_note_response` — preserves wire bytes; expects both refusal paths to use `AppError` serialization.

**Used by** — ADR 0003 verification matrix rows 5, 6, and 12.

**Repeated context** — The established body omits the error variant's display prefix.

---

## fn quota_locks_are_scoped_by_user

**Identification** — ADR 0003 rows 8 and 9 concurrency test; marker `// md:fn quota_locks_are_scoped_by_user`.

**Code** — complete and verbatim:

```rust
// md:fn quota_locks_are_scoped_by_user
#[sqlx::test(migrations = "../../migrations")]
async fn quota_locks_are_scoped_by_user(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(0, 1)).await;
    register(addr, "a@example.com").await;
    register(addr, "b@example.com").await;
    let left_token = login(addr, "a@example.com", "dev-a").await;
    let right_token = login(addr, "b@example.com", "dev-b").await;
    install_quota_barrier(&pool, "notes").await;
    let blocker = hold_barrier(&pool).await;
    let left = tokio::spawn(async move { post_note_response(addr, &left_token).await });
    let right = tokio::spawn(async move { post_note_response(addr, &right_token).await });
    wait_for_barrier(&pool, 2).await;
    release_barrier(blocker).await;
    assert_eq!(left.await.unwrap().status().as_u16(), 200);
    assert_eq!(right.await.unwrap().status().as_u16(), 200);
}
```

**What it does** — Proves two users both reach the between-read-and-write barrier concurrently.

**Dependencies** — `wait_for_barrier` — requires two waiters; expects a constant production key mutation to time out.

**Used by** — ADR 0003 verification matrix rows 8 and 9.

**Repeated context** — Quota serialization is per user.

---

## fn note_and_blob_quota_locks_use_distinct_domains

**Identification** — ADR 0003 row 10 concurrency test; marker `// md:fn note_and_blob_quota_locks_use_distinct_domains`.

**Code** — complete and verbatim:

```rust
// md:fn note_and_blob_quota_locks_use_distinct_domains
#[sqlx::test(migrations = "../../migrations")]
async fn note_and_blob_quota_locks_use_distinct_domains(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(100, 1)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let dev = device(addr, &token).await;
    let resource_id = seed_resource(addr, &token, &dev).await;
    install_quota_barrier(&pool, "notes").await;
    sqlx::query("CREATE TRIGGER blob_quota_write_barrier BEFORE INSERT OR UPDATE ON resource_blobs FOR EACH ROW EXECUTE FUNCTION quota_write_barrier()")
        .execute(&pool)
        .await
        .unwrap();
    let blocker = hold_barrier(&pool).await;
    let note_token = token.clone();
    let note = tokio::spawn(async move { post_note_response(addr, &note_token).await });
    let blob_token = token.clone();
    let blob =
        tokio::spawn(async move { put_blob_response(addr, &blob_token, resource_id, 50).await });
    wait_for_barrier(&pool, 2).await;
    release_barrier(blocker).await;
    assert_eq!(note.await.unwrap().status().as_u16(), 200);
    assert_eq!(blob.await.unwrap().status().as_u16(), 200);
}
```

**What it does** — Proves one user's note and blob writers both reach their write barriers without cross-quota blocking.

**Dependencies** — `AdvisoryLockDomain` production behavior — expects note and blob quotas to hash distinct domain names.

**Used by** — ADR 0003 verification matrix row 10.

**Repeated context** — Independent quota dimensions must not create false sharing.

---

## fn failed_quota_lock_wait_is_internal_and_retryable

**Identification** — ADR 0003 row 14 recovery test; marker `// md:fn failed_quota_lock_wait_is_internal_and_retryable`.

**Code** — complete and verbatim:

```rust
// md:fn failed_quota_lock_wait_is_internal_and_retryable
#[sqlx::test(migrations = "../../migrations")]
async fn failed_quota_lock_wait_is_internal_and_retryable(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(0, 1)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email = 'a@example.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query(
        "SELECT pg_advisory_lock(hashtextextended(concat('note-quota', ':', $1::text), 0))",
    )
    .bind(user_id.to_string())
    .execute(&mut *blocker)
    .await
    .unwrap();
    let max = pool.options().get_max_connections();
    let mut reserved = Vec::new();
    for _ in 1..max {
        reserved.push(pool.acquire().await.unwrap());
    }
    let mut request_connection = reserved.pop().unwrap();
    sqlx::query("SET lock_timeout = '100ms'")
        .execute(&mut *request_connection)
        .await
        .unwrap();
    drop(request_connection);
    let failed = post_note_response(addr, &token).await;
    assert_eq!(failed.status().as_u16(), 500);
    assert_ne!(failed.status().as_u16(), 507);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notes WHERE owner_id = $1")
        .bind(user_id)
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    assert_eq!(count, 0);
    sqlx::query(
        "SELECT pg_advisory_unlock(hashtextextended(concat('note-quota', ':', $1::text), 0))",
    )
    .bind(user_id.to_string())
    .execute(&mut *blocker)
    .await
    .unwrap();
    drop(reserved);
    drop(blocker);
    assert_eq!(post_note(addr, &token).await, 200);
}
```

**What it does** — Forces a quota lock timeout, checks the internal response and empty aggregate, then releases contention and retries successfully.

**Dependencies** — PostgreSQL `lock_timeout` — aborts the wait; expects transaction rollback and `AppError::Database` mapping to 500.

**Used by** — ADR 0003 verification matrix row 14.

**Repeated context** — A lock failure is not evidence that the user exceeded quota.

---

## fn failure_between_quota_read_and_write_rolls_back_and_releases_lock

**Identification** — ADR 0003 row 16 recovery test; marker `// md:fn failure_between_quota_read_and_write_rolls_back_and_releases_lock`.

**Code** — complete and verbatim:

```rust
// md:fn failure_between_quota_read_and_write_rolls_back_and_releases_lock
#[sqlx::test(migrations = "../../migrations")]
async fn failure_between_quota_read_and_write_rolls_back_and_releases_lock(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(0, 1)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email = 'a@example.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE FUNCTION fail_quota_note_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected quota write failure'; END $$")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_quota_note_write BEFORE INSERT ON notes FOR EACH ROW EXECUTE FUNCTION fail_quota_note_write()")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(post_note(addr, &token).await, 500);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notes WHERE owner_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let locks: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_locks locks JOIN pg_database database ON database.oid = locks.database WHERE database.datname = current_database() AND locks.locktype = 'advisory' AND locks.granted")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(locks, 0);
    sqlx::query("DROP TRIGGER fail_quota_note_write ON notes")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(post_note(addr, &token).await, 200);
}
```

**What it does** — Injects a write failure after the deciding read and verifies no note or transaction lock survives rollback before a successful retry.

**Dependencies** — PostgreSQL trigger exceptions — abort the handler transaction; expects transaction-scoped advisory locks to release automatically.

**Used by** — ADR 0003 verification matrix row 16.

**Repeated context** — The deciding read, write, and lock share one transaction.

---

## fn note_quota_disabled_by_default

**Identification** — `#[sqlx::test]`; marker
`// md:fn note_quota_disabled_by_default`.

**Code** — complete and verbatim:

```rust
// md:fn note_quota_disabled_by_default
#[sqlx::test(migrations = "../../migrations")]
async fn note_quota_disabled_by_default(pool: PgPool) {
    let addr = spawn(pool, quota_config(0, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    for _ in 0..5 {
        assert_eq!(post_note(addr, &token).await, 200);
    }
}
```

**What it does** — Limit `0` (the default): five creations all succeed — `0` means
unlimited, the backward-compatible posture.

**Dependencies** — the helpers. **Used by** — `cargo test`.

**Repeated context** — `0`-disables is the crate-wide convention for optional
limits.

---

## fn storage_quota_blocks_upload_over_the_limit

**Identification** — `#[sqlx::test]`; marker
`// md:fn storage_quota_blocks_upload_over_the_limit`.

**Code** — complete and verbatim:

```rust
// md:fn storage_quota_blocks_upload_over_the_limit
#[sqlx::test(migrations = "../../migrations")]
async fn storage_quota_blocks_upload_over_the_limit(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(100, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let dev = device(addr, &token).await;

    let a = seed_resource(addr, &token, &dev).await;
    let b = seed_resource(addr, &token, &dev).await;

    assert_eq!(put_blob(addr, &token, a, 50).await, 200);
    assert_eq!(put_blob(addr, &token, a, 50).await, 200);
    assert_eq!(put_blob(addr, &token, b, 60).await, 507);
    assert_eq!(put_blob(addr, &token, b, 40).await, 200);
}
```

**What it does** — Limit 100 bytes, two seeded resources A and B: 50 into A → 200;
re-upload 50 into A → 200 (an **overwrite is not double-counted** — measured by its
new size); 60 into B → **507** (50+60 > 100); 40 into B → 200 (50+40 ≤ 100).

**Dependencies** — the helpers. **Used by** — `cargo test`.

**Repeated context** — Pins the `user_blob_bytes_excluding` accounting rule.

---

## fn storage_quota_isolated_per_user

**Identification** — `#[sqlx::test]`; marker
`// md:fn storage_quota_isolated_per_user`.

**Code** — complete and verbatim:

```rust
// md:fn storage_quota_isolated_per_user
#[sqlx::test(migrations = "../../migrations")]
async fn storage_quota_isolated_per_user(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(100, 0)).await;
    register(addr, "a@example.com").await;
    register(addr, "b@example.com").await;
    let ta = login(addr, "a@example.com", "dev-a").await;
    let tb = login(addr, "b@example.com", "dev-b").await;
    let da = device(addr, &ta).await;
    let db = device(addr, &tb).await;

    let ra = seed_resource(addr, &ta, &da).await;
    let rb = seed_resource(addr, &tb, &db).await;

    assert_eq!(put_blob(addr, &ta, ra, 100).await, 200);
    assert_eq!(put_blob(addr, &tb, rb, 100).await, 200);
    let ra2 = seed_resource(addr, &ta, &da).await;
    assert_eq!(put_blob(addr, &ta, ra2, 1).await, 507);
}
```

**What it does** — Two accounts, limit 100 each: A fills its budget (100 → 200);
B still uploads its own 100 → 200 (unaffected); A's next 1-byte upload → **507**.

**Dependencies** — the helpers. **Used by** — `cargo test`.

**Repeated context** — Quota scoping is per-user, like all durable data.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `spawn()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `quota_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `register()` — defined here (EXTRACTED; file-local)
- `login()` — defined here (EXTRACTED; file-local)
- `post_note()` — defined here (EXTRACTED; file-local)
- `device()` — defined here (EXTRACTED; file-local)
- `seed_resource()` — defined here (EXTRACTED; file-local)
- `put_blob()` — defined here (EXTRACTED; file-local)
- `registration_can_be_disabled()` — defined here (EXTRACTED; file-local)
- `note_quota_blocks_creation_past_the_limit()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×2; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports | `// md:Overview` |
| 2 | `fn quota_config` | `// md:fn quota_config` |
| 3 | `fn spawn` | `// md:fn spawn` |
| 4 | `fn register` | `// md:fn register` |
| 5 | `fn login` | `// md:fn login` |
| 6 | `fn post_note` | `// md:fn post_note` |
| 6a | `fn post_note_response` | `// md:fn post_note_response` |
| 6a | `fn import_note` | `// md:fn import_note` |
| 7 | `fn device` | `// md:fn device` |
| 8 | `fn seed_resource` | `// md:fn seed_resource` |
| 9 | `fn put_blob` | `// md:fn put_blob` |
| 9a | `fn put_blob_response` | `// md:fn put_blob_response` |
| 9b | `fn install_quota_barrier` | `// md:fn install_quota_barrier` |
| 9c | `fn wait_for_barrier` | `// md:fn wait_for_barrier` |
| 9ca | `fn wait_for_advisory_waiters` | `// md:fn wait_for_advisory_waiters` |
| 9d | `fn hold_barrier` | `// md:fn hold_barrier` |
| 9e | `fn release_barrier` | `// md:fn release_barrier` |
| 10 | `fn registration_can_be_disabled` | `// md:fn registration_can_be_disabled` |
| 11 | `fn note_quota_blocks_creation_past_the_limit` | `// md:fn note_quota_blocks_creation_past_the_limit` |
| 11a | `fn note_quota_blocks_import_past_the_limit` | `// md:fn note_quota_blocks_import_past_the_limit` |
| 11b | `fn quota_write_inventory_is_complete` | `// md:fn quota_write_inventory_is_complete` |
| 11bb | `fn blob_write_to_tombstoned_resource_is_refused` | `// md:fn blob_write_to_tombstoned_resource_is_refused` |
| 11bba | `fn over_limit_blob_write_to_tombstoned_resource_is_not_found` | `// md:fn over_limit_blob_write_to_tombstoned_resource_is_not_found` |
| 11bc | `fn tombstoned_blobs_cannot_exceed_the_storage_limit` | `// md:fn tombstoned_blobs_cannot_exceed_the_storage_limit` |
| 11bd | `fn tombstoned_resource_blob_remains_readable` | `// md:fn tombstoned_resource_blob_remains_readable` |
| 11ba | `fn quota_paths_do_not_use_serializable_retry_or_service_unavailable` | `// md:fn quota_paths_do_not_use_serializable_retry_or_service_unavailable` |
| 11c | `fn concurrent_blob_quota_writes_serialize_before_the_deciding_read` | `// md:fn concurrent_blob_quota_writes_serialize_before_the_deciding_read` |
| 11d | `fn concurrent_note_quota_writes_serialize_and_keep_the_refusal_body` | `// md:fn concurrent_note_quota_writes_serialize_and_keep_the_refusal_body` |
| 11e | `fn quota_locks_are_scoped_by_user` | `// md:fn quota_locks_are_scoped_by_user` |
| 11f | `fn note_and_blob_quota_locks_use_distinct_domains` | `// md:fn note_and_blob_quota_locks_use_distinct_domains` |
| 11g | `fn failed_quota_lock_wait_is_internal_and_retryable` | `// md:fn failed_quota_lock_wait_is_internal_and_retryable` |
| 11h | `fn failure_between_quota_read_and_write_rolls_back_and_releases_lock` | `// md:fn failure_between_quota_read_and_write_rolls_back_and_releases_lock` |
| 12 | `fn note_quota_disabled_by_default` | `// md:fn note_quota_disabled_by_default` |
| 13 | `fn storage_quota_blocks_upload_over_the_limit` | `// md:fn storage_quota_blocks_upload_over_the_limit` |
| 14 | `fn storage_quota_isolated_per_user` | `// md:fn storage_quota_isolated_per_user` |
