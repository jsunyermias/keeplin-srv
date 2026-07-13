//! Real-client e2e: the daemon's collaborative stack (`CollabBackend<DbBackend>`)
//! writes a note **through** to the server. Lives in its own test binary so its
//! background client tasks cannot interfere with other tests (issue #51) — see
//! `collab_e2e_common/mod.rs`.

#[path = "collab_e2e_common/mod.rs"]
mod common;

use common::*;
use keeplin_core::{models::Note, storage::NoteRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn collab_client_writes_note_through_to_the_server(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = collab_device(addr, &token).await;

    // Create a note through the real client: it POSTs the note, joins the
    // collaborative session and pushes the body as line ops. The server
    // materialises those lines.
    let note = a
        .create_note(Note::new("Title", "hello world"))
        .await
        .unwrap();
    wait_server_body(addr, &token, note.id, "hello world").await;
}
