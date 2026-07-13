//! Real-client e2e: a fresh client rebuilds a note from the `Welcome` snapshot
//! on reconnect ("the client DB is a cache"). Lives in its own test binary so
//! its background client tasks cannot interfere with other tests (issue #51) —
//! see `collab_e2e_common/mod.rs`.

#[path = "collab_e2e_common/mod.rs"]
mod common;

use common::*;
use keeplin_core::{models::Note, storage::NoteRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn reconnecting_client_rebuilds_note_from_snapshot(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;

    // First client writes a note and disconnects.
    let note_id = {
        let a = collab_device(addr, &token).await;
        let note = a
            .create_note(Note::new("Persisted", "durable body"))
            .await
            .unwrap();
        wait_server_body(addr, &token, note.id, "durable body").await;
        note.id
        // `a` is dropped here: its connections close.
    };

    // A fresh client with an empty local database and the same account
    // discovers the note on connect, joins it, and rebuilds the body from the
    // server's Welcome snapshot — the "client DB is a cache" property.
    let b = collab_device(addr, &token).await;
    wait_local_body(&b, note_id, "durable body").await;

    // Having joined cleanly (its mirror settled from the Welcome), an edit from
    // this client converges back on the server.
    let mut edited = b.read_note(note_id).await.unwrap();
    edited.body = "edited after reconnect".into();
    b.update_note(edited).await.unwrap();
    wait_server_body(addr, &token, note_id, "edited after reconnect").await;
}
