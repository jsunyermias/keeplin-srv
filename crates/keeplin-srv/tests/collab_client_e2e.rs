// md:Overview
#[path = "collab_e2e_common/mod.rs"]
mod common;

use common::*;
use keeplin_core::{models::Note, storage::NoteRepository};
use sqlx::PgPool;

// md:fn collab_client_writes_note_through_to_the_server
#[sqlx::test(migrations = "../../migrations")]
async fn collab_client_writes_note_through_to_the_server(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = collab_device(addr, &token).await;

    let note = a
        .create_note(Note::new("Title", "hello world"))
        .await
        .unwrap();
    wait_server_body(addr, &token, note.id, "hello world").await;
}
