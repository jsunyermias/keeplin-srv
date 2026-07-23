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
