use chrono::{DateTime, Utc};
use keeplin_core::storage::note_log::VersionVector;
use serde::Serialize;
use sqlx::{types::Json, Pool, Postgres, Row};
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Note {
    pub id: Uuid,
    pub title: String,
    pub owner_id: Uuid,
    pub notebook_id: Option<Uuid>,
    pub is_todo: bool,
    pub todo_due: Option<DateTime<Utc>>,
    pub todo_completed: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A partial note-metadata update: `None` = leave unchanged, `Some(inner)` =
/// set (so `Some(None)` clears a nullable field).
#[derive(Debug, Default)]
pub struct NotePatch {
    pub title: Option<String>,
    pub notebook_id: Option<Option<Uuid>>,
    pub is_todo: Option<bool>,
    pub todo_due: Option<Option<DateTime<Utc>>>,
    pub todo_completed: Option<Option<DateTime<Utc>>>,
}

const NOTE_COLS: &str = "id, title, owner_id, notebook_id, is_todo, todo_due, todo_completed, \
                         created_at, updated_at, deleted_at";

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct NoteShare {
    pub note_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

/// One collaborative line: an independently versioned entity with soft-delete.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Line {
    pub id: Uuid,
    pub note_id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub vv: Json<VersionVector>,
    pub last_writer: String,
}

/// The versioned order of a note's lines (`NoteLines` in the design doc).
#[derive(Debug, Clone)]
pub struct NoteOrder {
    pub note_id: Uuid,
    pub order: Vec<Uuid>,
    pub updated_at: DateTime<Utc>,
    pub vv: VersionVector,
    pub last_writer: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserDevice {
    pub id: Uuid,
    pub user_id: Uuid,
    pub device_name: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

/// One journal row as fetched for delivery: the sequence number, the device
/// that pushed it (echo suppression) and the opaque `Change` payload.
#[derive(Debug, Clone)]
pub struct ChangeRow {
    pub seq: i64,
    pub origin_device_id: Uuid,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct Store {
    pool: Pool<Postgres>,
}

impl Store {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    // ── Users ────────────────────────────────────────────────────────────────

    pub async fn create_user(
        &self,
        email: &str,
        password_hash: &str,
        display_name: &str,
    ) -> Result<User, AppError> {
        let user = sqlx::query_as::<_, User>(
            r#"INSERT INTO users (id, email, password_hash, display_name)
               VALUES ($1, $2, $3, $4)
               RETURNING id, email, password_hash, display_name, created_at"#,
        )
        .bind(Uuid::new_v4())
        .bind(email)
        .bind(password_hash)
        .bind(display_name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.is_unique_violation() => AppError::Conflict,
            _ => AppError::from(e),
        })?;
        Ok(user)
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        let user = sqlx::query_as::<_, User>(
            r#"SELECT id, email, password_hash, display_name, created_at
               FROM users WHERE email = $1"#,
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(user)
    }

    pub async fn get_user_by_id(&self, id: Uuid) -> Result<Option<User>, AppError> {
        let user = sqlx::query_as::<_, User>(
            r#"SELECT id, email, password_hash, display_name, created_at
               FROM users WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(user)
    }

    // ── Devices ──────────────────────────────────────────────────────────────

    pub async fn create_device(
        &self,
        user_id: Uuid,
        device_name: &str,
    ) -> Result<UserDevice, AppError> {
        let device = sqlx::query_as::<_, UserDevice>(
            r#"INSERT INTO user_devices (id, user_id, device_name)
               VALUES ($1, $2, $3)
               RETURNING id, user_id, device_name, created_at, last_seen_at"#,
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(device_name)
        .fetch_one(&self.pool)
        .await?;
        Ok(device)
    }

    pub async fn get_device(&self, id: Uuid) -> Result<Option<UserDevice>, AppError> {
        let device = sqlx::query_as::<_, UserDevice>(
            r#"SELECT id, user_id, device_name, created_at, last_seen_at
               FROM user_devices WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(device)
    }

    pub async fn list_devices_by_user(&self, user_id: Uuid) -> Result<Vec<UserDevice>, AppError> {
        let devices = sqlx::query_as::<_, UserDevice>(
            r#"SELECT id, user_id, device_name, created_at, last_seen_at
               FROM user_devices WHERE user_id = $1 ORDER BY created_at"#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(devices)
    }

    pub async fn touch_device(&self, device_id: Uuid) -> Result<(), AppError> {
        sqlx::query("UPDATE user_devices SET last_seen_at = now() WHERE id = $1")
            .bind(device_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Change journal ───────────────────────────────────────────────────────

    /// Append a batch of changes to the user's journal. Duplicate re-sends of
    /// the same `(batch_id, batch_index)` are silently skipped, so a client
    /// that retries a batch after a reconnect never creates duplicate rows.
    ///
    /// Returns the sequence numbers actually inserted (empty for a pure
    /// duplicate batch, in which case the caller should skip the fan-out).
    pub async fn append_changes(
        &self,
        user_id: Uuid,
        origin_device_id: Uuid,
        sync_device_id: &str,
        batch_id: Uuid,
        payloads: &[serde_json::Value],
    ) -> Result<Vec<i64>, AppError> {
        let mut tx = self.pool.begin().await?;
        let mut seqs = Vec::with_capacity(payloads.len());
        for (idx, payload) in payloads.iter().enumerate() {
            let row = sqlx::query(
                r#"INSERT INTO changes
                       (user_id, origin_device_id, batch_id, batch_index, sync_device_id, payload)
                   VALUES ($1, $2, $3, $4, $5, $6)
                   ON CONFLICT (batch_id, batch_index) DO NOTHING
                   RETURNING seq"#,
            )
            .bind(user_id)
            .bind(origin_device_id)
            .bind(batch_id)
            .bind(idx as i32)
            .bind(sync_device_id)
            .bind(payload)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(row) = row {
                seqs.push(row.get::<i64, _>("seq"));
            }
        }
        tx.commit().await?;
        Ok(seqs)
    }

    /// Fetch up to `limit` journal rows for `user_id` with `seq > after_seq`,
    /// in sequence order. Rows from every device are returned (including the
    /// caller's own) so the delivery cursor can advance past them; the caller
    /// filters out its own rows before sending.
    pub async fn changes_after(
        &self,
        user_id: Uuid,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<ChangeRow>, AppError> {
        let rows = sqlx::query(
            r#"SELECT seq, origin_device_id, payload
               FROM changes
               WHERE user_id = $1 AND seq > $2
               ORDER BY seq
               LIMIT $3"#,
        )
        .bind(user_id)
        .bind(after_seq)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ChangeRow {
                seq: r.get("seq"),
                origin_device_id: r.get("origin_device_id"),
                payload: r.get("payload"),
            })
            .collect())
    }

    // ── Delivery cursors ─────────────────────────────────────────────────────

    pub async fn get_cursor(&self, device_id: Uuid) -> Result<i64, AppError> {
        let row = sqlx::query("SELECT last_seq FROM device_cursors WHERE device_id = $1")
            .bind(device_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<i64, _>("last_seq")).unwrap_or(0))
    }

    /// Advance a device's delivery cursor. `GREATEST` guards against a stale
    /// connection racing a newer one backwards.
    pub async fn advance_cursor(&self, device_id: Uuid, seq: i64) -> Result<(), AppError> {
        sqlx::query(
            r#"INSERT INTO device_cursors (device_id, last_seq, updated_at)
               VALUES ($1, $2, now())
               ON CONFLICT (device_id) DO UPDATE
               SET last_seq = GREATEST(device_cursors.last_seq, EXCLUDED.last_seq),
                   updated_at = now()"#,
        )
        .bind(device_id)
        .bind(seq)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Retention ────────────────────────────────────────────────────────────

    /// Delete journal rows older than `older_than` that every device of the
    /// owning user has already passed (seq <= the user's minimum cursor). A
    /// device that has never connected holds the minimum at 0 and blocks
    /// pruning for that user — the conservative choice: nothing is deleted
    /// that some device may still need.
    pub async fn prune_delivered_changes(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, AppError> {
        let result = sqlx::query(
            r#"DELETE FROM changes c
               WHERE c.received_at < $1
                 AND c.seq <= (
                     SELECT COALESCE(MIN(COALESCE(dc.last_seq, 0)), 0)
                     FROM user_devices d
                     LEFT JOIN device_cursors dc ON dc.device_id = d.id
                     WHERE d.user_id = c.user_id
                 )"#,
        )
        .bind(older_than)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    // ── Notes ────────────────────────────────────────────────────────────────

    /// Create a note and its (empty) versioned line order in one transaction.
    ///
    /// `id` may be supplied by the client (a keeplin daemon keeps its local
    /// note id when it uploads a note); a duplicate id maps to `Conflict`.
    pub async fn create_note(
        &self,
        id: Option<Uuid>,
        title: &str,
        owner_id: Uuid,
    ) -> Result<Note, AppError> {
        let mut tx = self.pool.begin().await?;
        let note = sqlx::query_as::<_, Note>(&format!(
            "INSERT INTO notes (id, title, owner_id) VALUES ($1, $2, $3) RETURNING {NOTE_COLS}"
        ))
        .bind(id.unwrap_or_else(Uuid::new_v4))
        .bind(title)
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.is_unique_violation() => AppError::Conflict,
            _ => AppError::from(e),
        })?;
        sqlx::query(
            r#"INSERT INTO note_line_order (note_id, order_json, updated_at, vv, last_writer)
               VALUES ($1, '[]', now(), '{}', $2)"#,
        )
        .bind(note.id)
        .bind(owner_id.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(note)
    }

    pub async fn get_note(&self, id: Uuid) -> Result<Option<Note>, AppError> {
        let note = sqlx::query_as::<_, Note>(&format!(
            "SELECT {NOTE_COLS} FROM notes WHERE id = $1 AND deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(note)
    }

    /// Notes the user owns plus notes shared with them.
    pub async fn list_notes_for_user(&self, user_id: Uuid) -> Result<Vec<Note>, AppError> {
        let notes = sqlx::query_as::<_, Note>(
            r#"SELECT n.id, n.title, n.owner_id, n.notebook_id, n.is_todo, n.todo_due,
                      n.todo_completed, n.created_at, n.updated_at, n.deleted_at
               FROM notes n
               LEFT JOIN note_shares s ON s.note_id = n.id AND s.user_id = $1
               WHERE n.deleted_at IS NULL AND (n.owner_id = $1 OR s.user_id IS NOT NULL)
               ORDER BY n.updated_at DESC"#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(notes)
    }

    /// Apply a partial metadata update; absent fields stay untouched.
    pub async fn update_note_meta(
        &self,
        id: Uuid,
        patch: &NotePatch,
    ) -> Result<Option<Note>, AppError> {
        let note = sqlx::query_as::<_, Note>(&format!(
            r#"UPDATE notes SET
                   title = COALESCE($2, title),
                   notebook_id = CASE WHEN $3 THEN $4 ELSE notebook_id END,
                   is_todo = COALESCE($5, is_todo),
                   todo_due = CASE WHEN $6 THEN $7 ELSE todo_due END,
                   todo_completed = CASE WHEN $8 THEN $9 ELSE todo_completed END,
                   updated_at = now()
               WHERE id = $1 AND deleted_at IS NULL
               RETURNING {NOTE_COLS}"#
        ))
        .bind(id)
        .bind(patch.title.as_deref())
        .bind(patch.notebook_id.is_some())
        .bind(patch.notebook_id.flatten())
        .bind(patch.is_todo)
        .bind(patch.todo_due.is_some())
        .bind(patch.todo_due.flatten())
        .bind(patch.todo_completed.is_some())
        .bind(patch.todo_completed.flatten())
        .fetch_optional(&self.pool)
        .await?;
        Ok(note)
    }

    pub async fn soft_delete_note(&self, id: Uuid) -> Result<Option<Note>, AppError> {
        let note = sqlx::query_as::<_, Note>(&format!(
            r#"UPDATE notes SET deleted_at = now(), updated_at = now()
               WHERE id = $1 AND deleted_at IS NULL
               RETURNING {NOTE_COLS}"#
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(note)
    }

    // ── Shares ───────────────────────────────────────────────────────────────

    pub async fn create_or_update_share(
        &self,
        note_id: Uuid,
        user_id: Uuid,
        role: &str,
    ) -> Result<NoteShare, AppError> {
        let share = sqlx::query_as::<_, NoteShare>(
            r#"INSERT INTO note_shares (note_id, user_id, role)
               VALUES ($1, $2, $3)
               ON CONFLICT (note_id, user_id) DO UPDATE SET role = EXCLUDED.role
               RETURNING note_id, user_id, role, created_at"#,
        )
        .bind(note_id)
        .bind(user_id)
        .bind(role)
        .fetch_one(&self.pool)
        .await?;
        Ok(share)
    }

    pub async fn get_share(
        &self,
        note_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<NoteShare>, AppError> {
        let share = sqlx::query_as::<_, NoteShare>(
            r#"SELECT note_id, user_id, role, created_at
               FROM note_shares WHERE note_id = $1 AND user_id = $2"#,
        )
        .bind(note_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(share)
    }

    pub async fn delete_share(&self, note_id: Uuid, user_id: Uuid) -> Result<(), AppError> {
        sqlx::query("DELETE FROM note_shares WHERE note_id = $1 AND user_id = $2")
            .bind(note_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Lines ────────────────────────────────────────────────────────────────

    pub async fn get_line(&self, id: Uuid) -> Result<Option<Line>, AppError> {
        let line = sqlx::query_as::<_, Line>(
            r#"SELECT id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer
               FROM lines WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(line)
    }

    /// Every line of the note, tombstones included (snapshots need them).
    pub async fn list_lines(&self, note_id: Uuid) -> Result<Vec<Line>, AppError> {
        let lines = sqlx::query_as::<_, Line>(
            r#"SELECT id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer
               FROM lines WHERE note_id = $1"#,
        )
        .bind(note_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(lines)
    }

    pub async fn insert_line(
        &self,
        id: Uuid,
        note_id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Line, AppError> {
        let line = sqlx::query_as::<_, Line>(
            r#"INSERT INTO lines (id, note_id, content, created_at, updated_at, vv, last_writer)
               VALUES ($1, $2, $3, now(), $4, $5, $6)
               RETURNING id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer"#,
        )
        .bind(id)
        .bind(note_id)
        .bind(content)
        .bind(updated_at)
        .bind(Json(vv))
        .bind(last_writer)
        .fetch_one(&self.pool)
        .await?;
        Ok(line)
    }

    /// Overwrite a line's content + version metadata (an applied `Update`).
    /// Also clears `deleted_at`: a causally newer edit revives a tombstone,
    /// same as keeplin-core's note semantics.
    pub async fn update_line(
        &self,
        id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Option<Line>, AppError> {
        let line = sqlx::query_as::<_, Line>(
            r#"UPDATE lines
               SET content = $2, vv = $3, last_writer = $4, updated_at = $5, deleted_at = NULL
               WHERE id = $1
               RETURNING id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer"#,
        )
        .bind(id)
        .bind(content)
        .bind(Json(vv))
        .bind(last_writer)
        .bind(updated_at)
        .fetch_optional(&self.pool)
        .await?;
        Ok(line)
    }

    /// Tombstone a line (an applied `Delete`). The row is kept for
    /// convergence and remains in the note's order until garbage collection.
    pub async fn soft_delete_line(
        &self,
        id: Uuid,
        deleted_at: DateTime<Utc>,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Option<Line>, AppError> {
        let line = sqlx::query_as::<_, Line>(
            r#"UPDATE lines
               SET deleted_at = $2, vv = $3, last_writer = $4, updated_at = $5
               WHERE id = $1
               RETURNING id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer"#,
        )
        .bind(id)
        .bind(deleted_at)
        .bind(Json(vv))
        .bind(last_writer)
        .bind(updated_at)
        .fetch_optional(&self.pool)
        .await?;
        Ok(line)
    }

    // ── Line order (the NoteLines entity) ────────────────────────────────────

    pub async fn get_note_order(&self, note_id: Uuid) -> Result<Option<NoteOrder>, AppError> {
        let row = sqlx::query(
            r#"SELECT note_id, order_json, updated_at, vv, last_writer
               FROM note_line_order WHERE note_id = $1"#,
        )
        .bind(note_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| NoteOrder {
            note_id: r.get("note_id"),
            order: r.get::<Json<Vec<Uuid>>, _>("order_json").0,
            updated_at: r.get("updated_at"),
            vv: r.get::<Json<VersionVector>, _>("vv").0,
            last_writer: r.get("last_writer"),
        }))
    }

    pub async fn set_note_order(
        &self,
        note_id: Uuid,
        order: &[Uuid],
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"UPDATE note_line_order
               SET order_json = $2, vv = $3, last_writer = $4, updated_at = $5
               WHERE note_id = $1"#,
        )
        .bind(note_id)
        .bind(Json(order))
        .bind(Json(vv))
        .bind(last_writer)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
