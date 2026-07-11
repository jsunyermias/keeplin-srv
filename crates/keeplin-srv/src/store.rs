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
    /// The grantee's capability bitmask (see `permissions::Capabilities`), already normalised.
    pub capabilities: i32,
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

/// A notebook as served over REST (metadata only; `vv`/`last_writer` are
/// internal to resolution and not exposed).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Notebook {
    pub id: Uuid,
    pub title: String,
    pub alias: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Tag {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Resource metadata as served over REST. The binary payload is fetched
/// separately from `resource_blobs` via `GET /api/resources/:id/data`.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ResourceMeta {
    pub id: Uuid,
    pub title: String,
    pub mime_type: String,
    pub file_name: String,
    pub size: i64,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Decide whether an `incoming` versioned write should replace the stored one,
/// reusing keeplin-core's exact resolution (dominates + `(timestamp, device)`
/// tiebreak) so the server converges to the same winner as every client.
fn incoming_wins(
    local_vv: &VersionVector,
    local_ts: DateTime<Utc>,
    local_writer: &str,
    incoming_vv: &VersionVector,
    incoming_ts: DateTime<Utc>,
    incoming_writer: &str,
) -> bool {
    use keeplin_core::storage::note_log::{resolve, Winner};
    matches!(
        resolve(
            local_vv,
            local_ts,
            local_writer,
            incoming_vv,
            incoming_ts,
            incoming_writer,
        ),
        Winner::Incoming
    )
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

    /// Delete one of `user_id`'s devices, revoking its token immediately
    /// (the auth middleware and both WebSocket handshakes re-check device
    /// existence). Returns whether a row was deleted.
    pub async fn delete_device(&self, id: Uuid, user_id: Uuid) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM user_devices WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
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

    /// Compact old line tombstones (design §6.4): delete lines soft-deleted
    /// before `older_than` and drop their ids from each note's order. By then
    /// every device has long converged past the delete (snapshots rebuild all
    /// client state), so the tombstone no longer needs to compete in
    /// resolution. Returns the number of lines reclaimed.
    pub async fn gc_line_tombstones(&self, older_than: DateTime<Utc>) -> Result<u64, AppError> {
        let rows = sqlx::query(
            r#"DELETE FROM lines
               WHERE deleted_at IS NOT NULL AND deleted_at < $1
               RETURNING id, note_id"#,
        )
        .bind(older_than)
        .fetch_all(&self.pool)
        .await?;
        if rows.is_empty() {
            return Ok(0);
        }
        let mut by_note: std::collections::HashMap<Uuid, Vec<Uuid>> =
            std::collections::HashMap::new();
        for row in &rows {
            by_note
                .entry(row.get("note_id"))
                .or_default()
                .push(row.get("id"));
        }
        for (note_id, dead) in by_note {
            if let Some(order) = self.get_note_order(note_id).await? {
                let kept: Vec<Uuid> = order
                    .order
                    .into_iter()
                    .filter(|id| !dead.contains(id))
                    .collect();
                // Only the membership changes; the order's version metadata is
                // untouched (compaction is not an edit).
                sqlx::query("UPDATE note_line_order SET order_json = $2 WHERE note_id = $1")
                    .bind(note_id)
                    .bind(Json(kept))
                    .execute(&self.pool)
                    .await?;
            }
        }
        Ok(rows.len() as u64)
    }

    /// Aggregate row counts for `/api/metrics`.
    pub async fn counts(&self) -> Result<(i64, i64, i64, i64), AppError> {
        let row = sqlx::query(
            r#"SELECT
                 (SELECT count(*) FROM users) AS users,
                 (SELECT count(*) FROM notes WHERE deleted_at IS NULL) AS notes,
                 (SELECT count(*) FROM lines) AS lines,
                 (SELECT count(*) FROM lines WHERE deleted_at IS NOT NULL) AS tombstones"#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok((
            row.get("users"),
            row.get("notes"),
            row.get("lines"),
            row.get("tombstones"),
        ))
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

    /// Transfer a note's ownership to `new_owner` (ownership is separate from capability
    /// grants and transferable only by the current owner — enforced at the HTTP layer).
    pub async fn set_note_owner(
        &self,
        id: Uuid,
        new_owner: Uuid,
    ) -> Result<Option<Note>, AppError> {
        let note = sqlx::query_as::<_, Note>(&format!(
            r#"UPDATE notes SET owner_id = $2, updated_at = now()
               WHERE id = $1 AND deleted_at IS NULL
               RETURNING {NOTE_COLS}"#
        ))
        .bind(id)
        .bind(new_owner)
        .fetch_optional(&self.pool)
        .await?;
        Ok(note)
    }

    // ── Shares ───────────────────────────────────────────────────────────────

    pub async fn create_or_update_share(
        &self,
        note_id: Uuid,
        user_id: Uuid,
        capabilities: i32,
    ) -> Result<NoteShare, AppError> {
        let share = sqlx::query_as::<_, NoteShare>(
            r#"INSERT INTO note_shares (note_id, user_id, capabilities)
               VALUES ($1, $2, $3)
               ON CONFLICT (note_id, user_id) DO UPDATE SET capabilities = EXCLUDED.capabilities
               RETURNING note_id, user_id, capabilities, created_at"#,
        )
        .bind(note_id)
        .bind(user_id)
        .bind(capabilities)
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
            r#"SELECT note_id, user_id, capabilities, created_at
               FROM note_shares WHERE note_id = $1 AND user_id = $2"#,
        )
        .bind(note_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(share)
    }

    /// List every share on a note (for owners/`share_read` grantees to see who has access).
    pub async fn list_shares(&self, note_id: Uuid) -> Result<Vec<NoteShare>, AppError> {
        let shares = sqlx::query_as::<_, NoteShare>(
            r#"SELECT note_id, user_id, capabilities, created_at
               FROM note_shares WHERE note_id = $1 ORDER BY created_at"#,
        )
        .bind(note_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(shares)
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

    // ── Domain entities materialised from the relay ──────────────────────────
    //
    // notebooks, tags, note↔tag associations and resource metadata arrive as
    // `Change`s over `/api/sync`; the relay materialises them here so the server
    // is their durable source of truth (the client DB is a cache). Every write
    // resolves against the stored row with the exact keeplin-core rule, under a
    // `SELECT … FOR UPDATE` lock so concurrent updates to the same entity are
    // serialised. Each entity id is created on a single device, so the
    // not-yet-present branch cannot race another creator.

    /// Apply a notebook create/update if it wins resolution. Returns whether it
    /// was written.
    pub async fn upsert_notebook(
        &self,
        user_id: Uuid,
        nb: &keeplin_core::models::Notebook,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        if let Some(row) = sqlx::query(
            "SELECT vv, updated_at, last_writer FROM notebooks WHERE id = $1 FOR UPDATE",
        )
        .bind(nb.id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("updated_at"),
                &row.get::<String, _>("last_writer"),
                &nb.vv,
                nb.updated_at,
                &nb.last_writer,
            ) {
                return Ok(false);
            }
        }
        sqlx::query(
            r#"INSERT INTO notebooks
                   (id, user_id, title, alias, created_at, updated_at, deleted_at, vv, last_writer)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               ON CONFLICT (id) DO UPDATE SET
                   title = EXCLUDED.title, alias = EXCLUDED.alias,
                   updated_at = EXCLUDED.updated_at, deleted_at = EXCLUDED.deleted_at,
                   vv = EXCLUDED.vv, last_writer = EXCLUDED.last_writer"#,
        )
        .bind(nb.id)
        .bind(user_id)
        .bind(&nb.title)
        .bind(&nb.alias)
        .bind(nb.created_at)
        .bind(nb.updated_at)
        .bind(nb.deleted_at)
        .bind(Json(&nb.vv))
        .bind(&nb.last_writer)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Apply a notebook tombstone if it wins. An unknown notebook gets a minimal
    /// tombstone so a later stale create/update cannot resurrect it.
    pub async fn delete_notebook(
        &self,
        user_id: Uuid,
        id: Uuid,
        deleted_at: DateTime<Utc>,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        let existed = if let Some(row) = sqlx::query(
            "SELECT vv, updated_at, last_writer FROM notebooks WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("updated_at"),
                &row.get::<String, _>("last_writer"),
                vv,
                deleted_at,
                last_writer,
            ) {
                return Ok(false);
            }
            true
        } else {
            false
        };
        if existed {
            sqlx::query(
                "UPDATE notebooks SET deleted_at = $2, updated_at = $2, vv = $3, last_writer = $4 WHERE id = $1",
            )
            .bind(id).bind(deleted_at).bind(Json(vv)).bind(last_writer)
            .execute(&mut *tx).await?;
        } else {
            sqlx::query(
                r#"INSERT INTO notebooks (id, user_id, title, created_at, updated_at, deleted_at, vv, last_writer)
                   VALUES ($1, $2, '', $3, $3, $3, $4, $5)
                   ON CONFLICT (id) DO NOTHING"#,
            )
            .bind(id).bind(user_id).bind(deleted_at).bind(Json(vv)).bind(last_writer)
            .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn upsert_tag(
        &self,
        user_id: Uuid,
        tag: &keeplin_core::models::Tag,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        if let Some(row) =
            sqlx::query("SELECT vv, updated_at, last_writer FROM tags WHERE id = $1 FOR UPDATE")
                .bind(tag.id)
                .fetch_optional(&mut *tx)
                .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("updated_at"),
                &row.get::<String, _>("last_writer"),
                &tag.vv,
                tag.updated_at,
                &tag.last_writer,
            ) {
                return Ok(false);
            }
        }
        sqlx::query(
            r#"INSERT INTO tags (id, user_id, title, created_at, updated_at, deleted_at, vv, last_writer)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (id) DO UPDATE SET
                   title = EXCLUDED.title, updated_at = EXCLUDED.updated_at,
                   deleted_at = EXCLUDED.deleted_at, vv = EXCLUDED.vv,
                   last_writer = EXCLUDED.last_writer"#,
        )
        .bind(tag.id)
        .bind(user_id)
        .bind(&tag.title)
        .bind(tag.created_at)
        .bind(tag.updated_at)
        .bind(tag.deleted_at)
        .bind(Json(&tag.vv))
        .bind(&tag.last_writer)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn delete_tag(
        &self,
        user_id: Uuid,
        id: Uuid,
        deleted_at: DateTime<Utc>,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        let existed = if let Some(row) =
            sqlx::query("SELECT vv, updated_at, last_writer FROM tags WHERE id = $1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("updated_at"),
                &row.get::<String, _>("last_writer"),
                vv,
                deleted_at,
                last_writer,
            ) {
                return Ok(false);
            }
            true
        } else {
            false
        };
        if existed {
            sqlx::query(
                "UPDATE tags SET deleted_at = $2, updated_at = $2, vv = $3, last_writer = $4 WHERE id = $1",
            )
            .bind(id).bind(deleted_at).bind(Json(vv)).bind(last_writer)
            .execute(&mut *tx).await?;
        } else {
            sqlx::query(
                r#"INSERT INTO tags (id, user_id, title, created_at, updated_at, deleted_at, vv, last_writer)
                   VALUES ($1, $2, '', $3, $3, $3, $4, $5)
                   ON CONFLICT (id) DO NOTHING"#,
            )
            .bind(id).bind(user_id).bind(deleted_at).bind(Json(vv)).bind(last_writer)
            .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Apply a note↔tag add (`deleted_at = None`) or remove (`Some`) if it wins.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_note_tag(
        &self,
        user_id: Uuid,
        note_id: Uuid,
        tag_id: Uuid,
        updated_at: DateTime<Utc>,
        deleted_at: Option<DateTime<Utc>>,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        if let Some(row) = sqlx::query(
            "SELECT vv, updated_at, last_writer FROM note_tags
             WHERE user_id = $1 AND note_id = $2 AND tag_id = $3 FOR UPDATE",
        )
        .bind(user_id)
        .bind(note_id)
        .bind(tag_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("updated_at"),
                &row.get::<String, _>("last_writer"),
                vv,
                updated_at,
                last_writer,
            ) {
                return Ok(false);
            }
        }
        sqlx::query(
            r#"INSERT INTO note_tags (user_id, note_id, tag_id, updated_at, deleted_at, vv, last_writer)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (user_id, note_id, tag_id) DO UPDATE SET
                   updated_at = EXCLUDED.updated_at, deleted_at = EXCLUDED.deleted_at,
                   vv = EXCLUDED.vv, last_writer = EXCLUDED.last_writer"#,
        )
        .bind(user_id)
        .bind(note_id)
        .bind(tag_id)
        .bind(updated_at)
        .bind(deleted_at)
        .bind(Json(vv))
        .bind(last_writer)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Apply resource metadata (create) if it wins. The binary payload is
    /// uploaded separately; resolution uses `deleted_at ?? created_at` as the
    /// timestamp, matching keeplin-core (resources carry no `updated_at`).
    pub async fn upsert_resource_meta(
        &self,
        user_id: Uuid,
        r: &keeplin_core::models::Resource,
    ) -> Result<bool, AppError> {
        let incoming_ts = r.deleted_at.unwrap_or(r.created_at);
        let mut tx = self.pool.begin().await?;
        if let Some(row) = sqlx::query(
            "SELECT vv, COALESCE(deleted_at, created_at) AS ts, last_writer
             FROM resources WHERE id = $1 FOR UPDATE",
        )
        .bind(r.id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let lvv = row.get::<Json<VersionVector>, _>("vv").0;
            if !incoming_wins(
                &lvv,
                row.get("ts"),
                &row.get::<String, _>("last_writer"),
                &r.vv,
                incoming_ts,
                &r.last_writer,
            ) {
                return Ok(false);
            }
        }
        sqlx::query(
            r#"INSERT INTO resources
                   (id, user_id, title, mime_type, file_name, size, created_at, deleted_at, vv, last_writer)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (id) DO UPDATE SET
                   title = EXCLUDED.title, mime_type = EXCLUDED.mime_type,
                   file_name = EXCLUDED.file_name, size = EXCLUDED.size,
                   deleted_at = EXCLUDED.deleted_at, vv = EXCLUDED.vv,
                   last_writer = EXCLUDED.last_writer"#,
        )
        .bind(r.id)
        .bind(user_id)
        .bind(&r.title)
        .bind(&r.mime_type)
        .bind(&r.file_name)
        .bind(r.size as i64)
        .bind(r.created_at)
        .bind(r.deleted_at)
        .bind(Json(&r.vv))
        .bind(&r.last_writer)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn delete_resource(
        &self,
        id: Uuid,
        deleted_at: DateTime<Utc>,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        let Some(row) = sqlx::query(
            "SELECT vv, COALESCE(deleted_at, created_at) AS ts, last_writer
             FROM resources WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        else {
            // Unknown resource: nothing to tombstone here. A later create will
            // arrive with its own vv and resolve normally.
            return Ok(false);
        };
        let lvv = row.get::<Json<VersionVector>, _>("vv").0;
        if !incoming_wins(
            &lvv,
            row.get("ts"),
            &row.get::<String, _>("last_writer"),
            vv,
            deleted_at,
            last_writer,
        ) {
            return Ok(false);
        }
        sqlx::query(
            "UPDATE resources SET deleted_at = $2, vv = $3, last_writer = $4 WHERE id = $1",
        )
        .bind(id)
        .bind(deleted_at)
        .bind(Json(vv))
        .bind(last_writer)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Store (or replace) a resource's binary payload. The resource metadata
    /// must already exist (enforced by the FK).
    pub async fn put_resource_blob(&self, resource_id: Uuid, data: &[u8]) -> Result<(), AppError> {
        sqlx::query(
            r#"INSERT INTO resource_blobs (resource_id, data) VALUES ($1, $2)
               ON CONFLICT (resource_id) DO UPDATE SET data = EXCLUDED.data"#,
        )
        .bind(resource_id)
        .bind(data)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_resource_blob(&self, resource_id: Uuid) -> Result<Option<Vec<u8>>, AppError> {
        let row = sqlx::query("SELECT data FROM resource_blobs WHERE resource_id = $1")
            .bind(resource_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<Vec<u8>, _>("data")))
    }

    /// Does a resource metadata row exist for this user (used to authorise a
    /// blob upload/download)?
    pub async fn resource_owned_by(
        &self,
        resource_id: Uuid,
        user_id: Uuid,
    ) -> Result<bool, AppError> {
        let row = sqlx::query("SELECT 1 FROM resources WHERE id = $1 AND user_id = $2")
            .bind(resource_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    // ── Domain entity reads (cold rehydration / queries) ─────────────────────

    pub async fn list_notebooks(&self, user_id: Uuid) -> Result<Vec<Notebook>, AppError> {
        Ok(sqlx::query_as::<_, Notebook>(
            "SELECT id, title, alias, created_at, updated_at, deleted_at
             FROM notebooks WHERE user_id = $1 AND deleted_at IS NULL ORDER BY created_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn list_tags(&self, user_id: Uuid) -> Result<Vec<Tag>, AppError> {
        Ok(sqlx::query_as::<_, Tag>(
            "SELECT id, title, created_at, updated_at, deleted_at
             FROM tags WHERE user_id = $1 AND deleted_at IS NULL ORDER BY created_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn list_resources(&self, user_id: Uuid) -> Result<Vec<ResourceMeta>, AppError> {
        Ok(sqlx::query_as::<_, ResourceMeta>(
            "SELECT id, title, mime_type, file_name, size, created_at, deleted_at
             FROM resources WHERE user_id = $1 AND deleted_at IS NULL ORDER BY created_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Live tag ids attached to a note (association present and both ends live).
    pub async fn list_note_tag_ids(
        &self,
        user_id: Uuid,
        note_id: Uuid,
    ) -> Result<Vec<Uuid>, AppError> {
        let rows = sqlx::query(
            r#"SELECT nt.tag_id FROM note_tags nt
               JOIN tags t ON t.id = nt.tag_id
               WHERE nt.user_id = $1 AND nt.note_id = $2
                 AND nt.deleted_at IS NULL AND t.deleted_at IS NULL
               ORDER BY nt.updated_at"#,
        )
        .bind(user_id)
        .bind(note_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| r.get::<Uuid, _>("tag_id"))
            .collect())
    }

    // ── Per-user quotas ──────────────────────────────────────────────────────

    /// Total bytes of the user's resource binaries, excluding one resource id.
    /// Excluding the resource being written means an overwrite is measured by
    /// its new size, not double-counted.
    pub async fn user_blob_bytes_excluding(
        &self,
        user_id: Uuid,
        exclude: Uuid,
    ) -> Result<i64, AppError> {
        let bytes: i64 = sqlx::query_scalar(
            r#"SELECT COALESCE(SUM(octet_length(rb.data)), 0)::bigint
               FROM resource_blobs rb
               JOIN resources r ON r.id = rb.resource_id
               WHERE r.user_id = $1 AND rb.resource_id <> $2"#,
        )
        .bind(user_id)
        .bind(exclude)
        .fetch_one(&self.pool)
        .await?;
        Ok(bytes)
    }

    /// Number of the user's live (non-deleted) owned notes.
    pub async fn count_live_notes_for_user(&self, user_id: Uuid) -> Result<i64, AppError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notes WHERE owner_id = $1 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }
}
