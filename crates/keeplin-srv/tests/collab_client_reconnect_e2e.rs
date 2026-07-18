// md:Overview
#[path = "collab_e2e_common/mod.rs"]
mod common;

use common::*;
use keeplin_core::{models::Note, storage::NoteRepository};
use sqlx::PgPool;

// md:fn reconnecting_client_rebuilds_note_from_snapshot
#[sqlx::test(migrations = "../../migrations")]
async fn reconnecting_client_rebuilds_note_from_snapshot(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;

    let note_id = {
        let a = collab_device(addr, &token).await;
        let note = a
            .create_note(Note::new("Persisted", "durable body"))
            .await
            .unwrap();
        wait_server_body(addr, &token, note.id, "durable body").await;
        note.id
    };

    let b = collab_device(addr, &token).await;
    wait_local_body(&b, note_id, "durable body").await;

    let mut edited = b.read_note(note_id).await.unwrap();
    edited.body = "edited after reconnect".into();
    b.update_note(edited).await.unwrap();
    wait_server_body(addr, &token, note_id, "edited after reconnect").await;
}
