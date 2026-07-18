// md:Overview
use chrono::{DateTime, Utc};
use keeplin_core::storage::note_log::VersionVector;
use serde::Serialize;
use sqlx::{types::Json, Pool, Postgres, Row};
use uuid::Uuid;

use crate::error::AppError;

// md:PageCursor
#[derive(Debug, Clone, Copy)]
pub struct PageCursor {
    pub ts: DateTime<Utc>,
    pub id: Uuid,
}

// md:impl PageCursor
impl PageCursor {
    // md:impl PageCursor > fn new
    pub fn new(ts: DateTime<Utc>, id: Uuid) -> Self {
        Self { ts, id }
    }

    // md:impl PageCursor > fn encode
    pub fn encode(&self) -> String {
        format!("{}_{}", self.ts.timestamp_micros(), self.id)
    }

    // md:impl PageCursor > fn decode
    pub fn decode(token: &str) -> Option<Self> {
        let (micros, id) = token.split_once('_')?;
        let ts = DateTime::from_timestamp_micros(micros.parse().ok()?)?;
        Some(Self {
            ts,
            id: id.parse().ok()?,
        })
    }
}

// md:fn token_hash
fn token_hash(raw: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    format!("{:x}", hasher.finalize())
}

// md:fn split_cursor
fn split_cursor(cursor: Option<PageCursor>) -> (Option<DateTime<Utc>>, Option<Uuid>) {
    match cursor {
        Some(c) => (Some(c.ts), Some(c.id)),
        None => (None, None),
    }
}

// md:User
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub email_verified_at: Option<DateTime<Utc>>,
}

// md:Note
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

// md:NotePatch
#[derive(Debug, Default)]
pub struct NotePatch {
    pub title: Option<String>,
    pub notebook_id: Option<Option<Uuid>>,
    pub is_todo: Option<bool>,
    pub todo_due: Option<Option<DateTime<Utc>>>,
    pub todo_completed: Option<Option<DateTime<Utc>>>,
}

// md:NOTE_COLS
const NOTE_COLS: &str = "id, title, owner_id, notebook_id, is_todo, todo_due, todo_completed, \
                         created_at, updated_at, deleted_at";

// md:NoteShare
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct NoteShare {
    pub note_id: Uuid,
    pub user_id: Uuid,
    pub capabilities: i32,
    pub created_at: DateTime<Utc>,
}

// md:NotebookShare
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct NotebookShare {
    pub notebook_id: Uuid,
    pub user_id: Uuid,
    pub capabilities: i32,
    pub created_at: DateTime<Utc>,
}

// md:Line
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

// md:NoteOrder
#[derive(Debug, Clone)]
pub struct NoteOrder {
    pub note_id: Uuid,
    pub order: Vec<Uuid>,
    pub updated_at: DateTime<Utc>,
    pub vv: VersionVector,
    pub last_writer: String,
}

// md:CollabEvent
#[derive(Debug, Clone)]
pub struct CollabEvent {
    pub seq: i64,
    pub note_id: Uuid,
    pub origin_instance: Uuid,
    pub origin_conn: i64,
    pub user_id: Uuid,
    pub ops: serde_json::Value,
}

// md:PresenceRow
#[derive(Debug, Clone)]
pub struct PresenceRow {
    pub user_id: Uuid,
    pub display_name: String,
    pub cursor: Option<serde_json::Value>,
}

// md:UserDevice
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserDevice {
    pub id: Uuid,
    pub user_id: Uuid,
    pub device_name: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

// md:ChangeRow
#[derive(Debug, Clone)]
pub struct ChangeRow {
    pub seq: i64,
    pub origin_device_id: Uuid,
    pub payload: serde_json::Value,
}

// md:HistoryKind
#[derive(Debug, Clone, Copy)]
pub enum HistoryKind {
    Note,
    Notebook,
}

// md:impl HistoryKind
impl HistoryKind {
    // md:impl HistoryKind > fn snapshot_key
    fn snapshot_key(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Notebook => "notebook",
        }
    }

    // md:impl HistoryKind > fn upsert_ops
    fn upsert_ops(self) -> &'static [&'static str] {
        match self {
            Self::Note => &["note_create", "note_update", "create", "update"],
            Self::Notebook => &["notebook_create", "notebook_update"],
        }
    }

    // md:impl HistoryKind > fn delete_ops
    fn delete_ops(self) -> &'static [&'static str] {
        match self {
            Self::Note => &["note_delete", "delete"],
            Self::Notebook => &["notebook_delete"],
        }
    }
}

// md:EntityVersionRow
#[derive(Debug, Clone, Serialize)]
pub struct EntityVersionRow {
    pub timestamp: DateTime<Utc>,
    pub device_id: String,
    pub entity: Option<serde_json::Value>,
}

// md:Notebook
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Notebook {
    pub id: Uuid,
    pub title: String,
    pub alias: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

// md:Tag
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Tag {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

// md:ResourceMeta
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

// md:fn incoming_wins
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

// md:Store
#[derive(Clone)]
pub struct Store {
    pool: Pool<Postgres>,
    cipher: crate::crypto::Cipher,
}

// md:impl Store
impl Store {
    // md:impl Store > fn new
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self {
            pool,
            cipher: crate::crypto::Cipher::from_key(None).expect("null key never fails"),
        }
    }

    // md:impl Store > fn with_cipher
    pub fn with_cipher(pool: Pool<Postgres>, cipher: crate::crypto::Cipher) -> Self {
        Self { pool, cipher }
    }

    // md:impl Store > fn create_user
    pub async fn create_user(
        &self,
        email: &str,
        password_hash: &str,
        display_name: &str,
    ) -> Result<User, AppError> {
        let user = sqlx::query_as::<_, User>(
            r#"INSERT INTO users (id, email, password_hash, display_name)
               VALUES ($1, $2, $3, $4)
               RETURNING id, email, password_hash, display_name, created_at, email_verified_at"#,
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

    // md:impl Store > fn get_user_by_email
    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        let user = sqlx::query_as::<_, User>(
            r#"SELECT id, email, password_hash, display_name, created_at, email_verified_at
               FROM users WHERE email = $1"#,
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(user)
    }

    // md:impl Store > fn get_user_by_id
    pub async fn get_user_by_id(&self, id: Uuid) -> Result<Option<User>, AppError> {
        let user = sqlx::query_as::<_, User>(
            r#"SELECT id, email, password_hash, display_name, created_at, email_verified_at
               FROM users WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(user)
    }

    // md:impl Store > fn update_password
    pub async fn update_password(&self, id: Uuid, password_hash: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
            .bind(id)
            .bind(password_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // md:impl Store > fn delete_user
    pub async fn delete_user(&self, id: Uuid) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // md:impl Store > fn login_locked
    pub async fn login_locked(&self, email: &str) -> Result<bool, AppError> {
        let locked: Option<bool> = sqlx::query_scalar(
            "SELECT COALESCE(locked_until > now(), false) FROM login_attempts WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(locked.unwrap_or(false))
    }

    // md:impl Store > fn record_login_failure
    pub async fn record_login_failure(
        &self,
        email: &str,
        max_failures: i32,
        lockout_secs: u64,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"INSERT INTO login_attempts (email, failed_count, last_failed_at, locked_until)
               VALUES ($1, 1, now(),
                       CASE WHEN 1 >= $2 THEN now() + $3 * interval '1 second' END)
               ON CONFLICT (email) DO UPDATE SET
                   failed_count = CASE
                       WHEN login_attempts.last_failed_at < now() - $3 * interval '1 second' THEN 1
                       ELSE login_attempts.failed_count + 1 END,
                   last_failed_at = now(),
                   locked_until = CASE
                       WHEN (CASE
                           WHEN login_attempts.last_failed_at < now() - $3 * interval '1 second' THEN 1
                           ELSE login_attempts.failed_count + 1 END) >= $2
                       THEN now() + $3 * interval '1 second' END"#,
        )
        .bind(email)
        .bind(max_failures)
        .bind(lockout_secs as f64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // md:impl Store > fn clear_login_failures
    pub async fn clear_login_failures(&self, email: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM login_attempts WHERE email = $1")
            .bind(email)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // md:impl Store > fn prune_login_attempts
    pub async fn prune_login_attempts(&self, older_than: DateTime<Utc>) -> Result<u64, AppError> {
        let r = sqlx::query("DELETE FROM login_attempts WHERE last_failed_at < $1")
            .bind(older_than)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    // md:impl Store > fn create_email_token
    pub async fn create_email_token(
        &self,
        user_id: Uuid,
        kind: crate::mail::MailKind,
        ttl_secs: u64,
    ) -> Result<(String, DateTime<Utc>), AppError> {
        use aes_gcm::aead::rand_core::RngCore;
        use base64::Engine as _;
        const MAX_LIVE_EMAIL_TOKENS: i64 = 5;
        let live: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM email_tokens
               WHERE user_id = $1 AND kind = $2 AND used_at IS NULL AND expires_at > now()"#,
        )
        .bind(user_id)
        .bind(kind.as_str())
        .fetch_one(&self.pool)
        .await?;
        if live >= MAX_LIVE_EMAIL_TOKENS {
            return Err(AppError::TooManyAttempts);
        }
        let mut raw = [0u8; 32];
        aes_gcm::aead::OsRng.fill_bytes(&mut raw);
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        let expires_at = Utc::now() + chrono::Duration::seconds(ttl_secs as i64);
        sqlx::query(
            r#"INSERT INTO email_tokens (id, user_id, kind, token_hash, expires_at)
               VALUES ($1, $2, $3, $4, $5)"#,
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(kind.as_str())
        .bind(token_hash(&token))
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok((token, expires_at))
    }

    // md:impl Store > fn consume_email_token
    pub async fn consume_email_token(
        &self,
        kind: crate::mail::MailKind,
        raw_token: &str,
    ) -> Result<Option<Uuid>, AppError> {
        let user_id: Option<Uuid> = sqlx::query_scalar(
            r#"UPDATE email_tokens SET used_at = now()
               WHERE token_hash = $1 AND kind = $2
                 AND used_at IS NULL AND expires_at > now()
               RETURNING user_id"#,
        )
        .bind(token_hash(raw_token))
        .bind(kind.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(user_id)
    }

    // md:impl Store > fn mark_email_verified
    pub async fn mark_email_verified(&self, user_id: Uuid) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // md:impl Store > fn prune_email_tokens
    pub async fn prune_email_tokens(&self, older_than: DateTime<Utc>) -> Result<u64, AppError> {
        let r = sqlx::query("DELETE FROM email_tokens WHERE expires_at < $1")
            .bind(older_than)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    // md:impl Store > fn create_device
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

    // md:impl Store > fn get_device
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

    // md:impl Store > fn list_devices_by_user
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

    // md:impl Store > fn delete_device
    pub async fn delete_device(&self, id: Uuid, user_id: Uuid) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM user_devices WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // md:impl Store > fn delete_all_devices
    pub async fn delete_all_devices(&self, user_id: Uuid) -> Result<u64, AppError> {
        let result = sqlx::query("DELETE FROM user_devices WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    // md:impl Store > fn touch_device
    pub async fn touch_device(&self, device_id: Uuid) -> Result<(), AppError> {
        sqlx::query("UPDATE user_devices SET last_seen_at = now() WHERE id = $1")
            .bind(device_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // md:impl Store > fn append_changes
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
                   ON CONFLICT (user_id, batch_id, batch_index) DO NOTHING
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

    // md:impl Store > fn changes_after
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

    // md:impl Store > fn entity_history
    pub async fn entity_history(
        &self,
        kind: HistoryKind,
        entity_id: Uuid,
        limit: i64,
        not_before: Option<DateTime<Utc>>,
        authored_not_before: Option<DateTime<Utc>>,
        user_scope: Option<Uuid>,
    ) -> Result<Vec<EntityVersionRow>, AppError> {
        let upsert_ops: Vec<String> = kind.upsert_ops().iter().map(|s| s.to_string()).collect();
        let delete_ops: Vec<String> = kind.delete_ops().iter().map(|s| s.to_string()).collect();
        let rows = sqlx::query(&format!(
            r#"SELECT payload, sync_device_id, received_at
               FROM changes
               WHERE ((payload->>'op' = ANY($2) AND payload->'{key}'->>'id' = $1)
                   OR (payload->>'op' = ANY($3) AND payload->>'id' = $1))
                 AND ($4::timestamptz IS NULL OR received_at >= $4)
                 AND ($6::uuid IS NULL OR user_id = $6)
                 AND ($7::timestamptz IS NULL OR COALESCE(
                        keeplin_try_timestamptz(
                            CASE WHEN payload->>'op' = ANY($3) THEN payload->>'deleted_at'
                                 ELSE payload->'{key}'->>'updated_at' END),
                        received_at) >= $7)
               ORDER BY seq DESC
               LIMIT $5"#,
            key = kind.snapshot_key(),
        ))
        .bind(entity_id.to_string())
        .bind(&upsert_ops)
        .bind(&delete_ops)
        .bind(not_before)
        .bind(limit)
        .bind(user_scope)
        .bind(authored_not_before)
        .fetch_all(&self.pool)
        .await?;

        let parse_ts =
            |v: &serde_json::Value| -> Option<DateTime<Utc>> { v.as_str()?.parse().ok() };
        Ok(rows
            .into_iter()
            .map(|row| {
                let payload: serde_json::Value = row.get("payload");
                let received_at: DateTime<Utc> = row.get("received_at");
                let op = payload["op"].as_str().unwrap_or_default();
                let (timestamp, entity) = if kind.delete_ops().contains(&op) {
                    (parse_ts(&payload["deleted_at"]), None)
                } else {
                    let snapshot = payload[kind.snapshot_key()].clone();
                    (parse_ts(&snapshot["updated_at"]), Some(snapshot))
                };
                EntityVersionRow {
                    timestamp: timestamp.unwrap_or(received_at),
                    device_id: row.get("sync_device_id"),
                    entity,
                }
            })
            .collect())
    }

    // md:impl Store > fn get_cursor
    pub async fn get_cursor(&self, device_id: Uuid) -> Result<i64, AppError> {
        let row = sqlx::query("SELECT last_seq FROM device_cursors WHERE device_id = $1")
            .bind(device_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<i64, _>("last_seq")).unwrap_or(0))
    }

    // md:impl Store > fn advance_cursor
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

    // md:impl Store > fn prune_delivered_changes
    pub async fn prune_delivered_changes(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, AppError> {
        let result = sqlx::query(
            r#"DELETE FROM changes c
               WHERE c.received_at < $1
                 AND c.seq <= (
                     SELECT COALESCE(MIN(dc.last_seq), 0)
                     FROM user_devices d
                     JOIN device_cursors dc ON dc.device_id = d.id
                     WHERE d.user_id = c.user_id
                 )"#,
        )
        .bind(older_than)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    // md:impl Store > fn purge_deleted_resource_blobs
    pub async fn purge_deleted_resource_blobs(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, AppError> {
        let result = sqlx::query(
            r#"DELETE FROM resource_blobs rb
               USING resources r
               WHERE rb.resource_id = r.id
                 AND r.deleted_at IS NOT NULL
                 AND r.deleted_at < $1"#,
        )
        .bind(older_than)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    // md:impl Store > fn gc_line_tombstones
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
            let mut tx = self.pool.begin().await?;
            let existing: Option<(Json<Vec<Uuid>>,)> = sqlx::query_as(
                "SELECT order_json FROM note_line_order WHERE note_id = $1 FOR UPDATE",
            )
            .bind(note_id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some((order_json,)) = existing {
                let kept: Vec<Uuid> = order_json
                    .0
                    .into_iter()
                    .filter(|id| !dead.contains(id))
                    .collect();
                sqlx::query("UPDATE note_line_order SET order_json = $2 WHERE note_id = $1")
                    .bind(note_id)
                    .bind(Json(kept))
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        Ok(rows.len() as u64)
    }

    // md:impl Store > fn ping
    pub async fn ping(&self) -> Result<(), AppError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    // md:impl Store > fn counts
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

    // md:impl Store > fn create_note
    pub async fn create_note(
        &self,
        id: Option<Uuid>,
        title: &str,
        owner_id: Uuid,
    ) -> Result<Note, AppError> {
        let mut tx = self.pool.begin().await?;
        let mut note = sqlx::query_as::<_, Note>(&format!(
            "INSERT INTO notes (id, title, owner_id) VALUES ($1, $2, $3) RETURNING {NOTE_COLS}"
        ))
        .bind(id.unwrap_or_else(Uuid::new_v4))
        .bind(self.cipher.encrypt(title)?)
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.is_unique_violation() => AppError::Conflict,
            _ => AppError::from(e),
        })?;
        note.title = title.to_string();
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

    // md:impl Store > fn get_note
    pub async fn get_note(&self, id: Uuid) -> Result<Option<Note>, AppError> {
        let mut note = sqlx::query_as::<_, Note>(&format!(
            "SELECT {NOTE_COLS} FROM notes WHERE id = $1 AND deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(note) = note.as_mut() {
            note.title = self.cipher.decrypt(&note.title)?;
        }
        Ok(note)
    }

    // md:impl Store > fn list_notes_for_user
    pub async fn list_notes_for_user(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
        cursor: Option<PageCursor>,
    ) -> Result<Vec<Note>, AppError> {
        let (cur_ts, cur_id) = split_cursor(cursor);
        let notes = sqlx::query_as::<_, Note>(
            r#"SELECT n.id, n.title, n.owner_id, n.notebook_id, n.is_todo, n.todo_due,
                      n.todo_completed, n.created_at, n.updated_at, n.deleted_at
               FROM notes n
               LEFT JOIN note_shares s ON s.note_id = n.id AND s.user_id = $1
               LEFT JOIN notebooks nb
                      ON nb.id = n.notebook_id AND nb.user_id = $1 AND nb.deleted_at IS NULL
               WHERE n.deleted_at IS NULL
                 AND (n.owner_id = $1 OR s.user_id IS NOT NULL OR nb.id IS NOT NULL)
                 AND ($3::timestamptz IS NULL OR (n.updated_at, n.id) < ($3, $4))
               ORDER BY n.updated_at DESC, n.id DESC
               LIMIT $2"#,
        )
        .bind(user_id)
        .bind(limit.unwrap_or(i64::MAX))
        .bind(cur_ts)
        .bind(cur_id)
        .fetch_all(&self.pool)
        .await?;
        let mut notes = notes;
        for note in notes.iter_mut() {
            note.title = self.cipher.decrypt(&note.title)?;
        }
        Ok(notes)
    }

    // md:impl Store > fn update_note_meta
    pub async fn update_note_meta(
        &self,
        id: Uuid,
        patch: &NotePatch,
    ) -> Result<Option<Note>, AppError> {
        let enc_title = patch
            .title
            .as_deref()
            .map(|t| self.cipher.encrypt(t))
            .transpose()?;
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
        .bind(enc_title.as_deref())
        .bind(patch.notebook_id.is_some())
        .bind(patch.notebook_id.flatten())
        .bind(patch.is_todo)
        .bind(patch.todo_due.is_some())
        .bind(patch.todo_due.flatten())
        .bind(patch.todo_completed.is_some())
        .bind(patch.todo_completed.flatten())
        .fetch_optional(&self.pool)
        .await?;
        self.decrypt_note_title(note)
    }

    // md:impl Store > fn decrypt_note_title
    fn decrypt_note_title(&self, note: Option<Note>) -> Result<Option<Note>, AppError> {
        match note {
            Some(mut n) => {
                n.title = self.cipher.decrypt(&n.title)?;
                Ok(Some(n))
            }
            None => Ok(None),
        }
    }

    // md:impl Store > fn soft_delete_note
    pub async fn soft_delete_note(&self, id: Uuid) -> Result<Option<Note>, AppError> {
        let note = sqlx::query_as::<_, Note>(&format!(
            r#"UPDATE notes SET deleted_at = now(), updated_at = now()
               WHERE id = $1 AND deleted_at IS NULL
               RETURNING {NOTE_COLS}"#
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        self.decrypt_note_title(note)
    }

    // md:impl Store > fn set_note_owner
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
        self.decrypt_note_title(note)
    }

    // md:impl Store > fn create_or_update_share
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

    // md:impl Store > fn get_share
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

    // md:impl Store > fn list_shares
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

    // md:impl Store > fn delete_share
    pub async fn delete_share(&self, note_id: Uuid, user_id: Uuid) -> Result<(), AppError> {
        sqlx::query("DELETE FROM note_shares WHERE note_id = $1 AND user_id = $2")
            .bind(note_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // md:impl Store > fn notebook_owner
    pub async fn notebook_owner(&self, notebook_id: Uuid) -> Result<Option<Uuid>, AppError> {
        let owner: Option<(Uuid,)> =
            sqlx::query_as("SELECT user_id FROM notebooks WHERE id = $1 AND deleted_at IS NULL")
                .bind(notebook_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(owner.map(|r| r.0))
    }

    // md:impl Store > fn set_notebook_owner
    pub async fn set_notebook_owner(
        &self,
        notebook_id: Uuid,
        new_owner: Uuid,
    ) -> Result<Option<Uuid>, AppError> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "UPDATE notebooks SET user_id = $2, updated_at = now()
             WHERE id = $1 AND deleted_at IS NULL RETURNING id",
        )
        .bind(notebook_id)
        .bind(new_owner)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    // md:impl Store > fn get_notebook_share
    pub async fn get_notebook_share(
        &self,
        notebook_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<NotebookShare>, AppError> {
        let share = sqlx::query_as::<_, NotebookShare>(
            r#"SELECT notebook_id, user_id, capabilities, created_at
               FROM notebook_shares WHERE notebook_id = $1 AND user_id = $2"#,
        )
        .bind(notebook_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(share)
    }

    // md:impl Store > fn list_notebook_shares
    pub async fn list_notebook_shares(
        &self,
        notebook_id: Uuid,
    ) -> Result<Vec<NotebookShare>, AppError> {
        let shares = sqlx::query_as::<_, NotebookShare>(
            r#"SELECT notebook_id, user_id, capabilities, created_at
               FROM notebook_shares WHERE notebook_id = $1 ORDER BY created_at"#,
        )
        .bind(notebook_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(shares)
    }

    // md:impl Store > fn create_or_update_notebook_share
    pub async fn create_or_update_notebook_share(
        &self,
        notebook_id: Uuid,
        user_id: Uuid,
        capabilities: i32,
    ) -> Result<NotebookShare, AppError> {
        let mut tx = self.pool.begin().await?;
        let share = sqlx::query_as::<_, NotebookShare>(
            r#"INSERT INTO notebook_shares (notebook_id, user_id, capabilities)
               VALUES ($1, $2, $3)
               ON CONFLICT (notebook_id, user_id) DO UPDATE SET capabilities = EXCLUDED.capabilities
               RETURNING notebook_id, user_id, capabilities, created_at"#,
        )
        .bind(notebook_id)
        .bind(user_id)
        .bind(capabilities)
        .fetch_one(&mut *tx)
        .await?;
        cascade_notebook_to_notes_tx(&mut tx, notebook_id).await?;
        tx.commit().await?;
        Ok(share)
    }

    // md:impl Store > fn delete_notebook_share
    pub async fn delete_notebook_share(
        &self,
        notebook_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM notebook_shares WHERE notebook_id = $1 AND user_id = $2")
            .bind(notebook_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        cascade_notebook_to_notes_tx(&mut tx, notebook_id).await?;
        tx.commit().await?;
        Ok(())
    }

    // md:impl Store > fn cascade_notebook_to_notes
    pub async fn cascade_notebook_to_notes(&self, notebook_id: Uuid) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        cascade_notebook_to_notes_tx(&mut tx, notebook_id).await?;
        tx.commit().await?;
        Ok(())
    }

    // md:impl Store > fn apply_notebook_shares_to_note
    pub async fn apply_notebook_shares_to_note(
        &self,
        note_id: Uuid,
        notebook_id: Uuid,
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        replace_note_shares_from_notebook_tx(&mut tx, note_id, notebook_id).await?;
        tx.commit().await?;
        Ok(())
    }

    // md:impl Store > fn get_line
    pub async fn get_line(&self, id: Uuid) -> Result<Option<Line>, AppError> {
        self.get_line_on(&self.pool, id).await
    }

    // md:impl Store > fn get_line_on
    pub async fn get_line_on<'e, E>(&self, exec: E, id: Uuid) -> Result<Option<Line>, AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let mut line = sqlx::query_as::<_, Line>(
            r#"SELECT id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer
               FROM lines WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(exec)
        .await?;
        if let Some(line) = line.as_mut() {
            line.content = self.cipher.decrypt(&line.content)?;
        }
        Ok(line)
    }

    // md:impl Store > fn list_lines
    pub async fn list_lines(&self, note_id: Uuid) -> Result<Vec<Line>, AppError> {
        let mut lines = sqlx::query_as::<_, Line>(
            r#"SELECT id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer
               FROM lines WHERE note_id = $1"#,
        )
        .bind(note_id)
        .fetch_all(&self.pool)
        .await?;
        for line in lines.iter_mut() {
            line.content = self.cipher.decrypt(&line.content)?;
        }
        Ok(lines)
    }

    // md:impl Store > fn insert_line
    pub async fn insert_line(
        &self,
        id: Uuid,
        note_id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Line, AppError> {
        self.insert_line_on(
            &self.pool,
            id,
            note_id,
            content,
            vv,
            last_writer,
            updated_at,
        )
        .await
    }

    // md:impl Store > fn insert_line_on
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_line_on<'e, E>(
        &self,
        exec: E,
        id: Uuid,
        note_id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Line, AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let mut line = sqlx::query_as::<_, Line>(
            r#"INSERT INTO lines (id, note_id, content, created_at, updated_at, vv, last_writer)
               VALUES ($1, $2, $3, now(), $4, $5, $6)
               RETURNING id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer"#,
        )
        .bind(id)
        .bind(note_id)
        .bind(self.cipher.encrypt(content)?)
        .bind(updated_at)
        .bind(Json(vv))
        .bind(last_writer)
        .fetch_one(exec)
        .await?;
        line.content = content.to_string();
        Ok(line)
    }

    // md:impl Store > fn update_line
    pub async fn update_line(
        &self,
        id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Option<Line>, AppError> {
        self.update_line_on(&self.pool, id, content, vv, last_writer, updated_at)
            .await
    }

    // md:impl Store > fn update_line_on
    pub async fn update_line_on<'e, E>(
        &self,
        exec: E,
        id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Option<Line>, AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let mut line = sqlx::query_as::<_, Line>(
            r#"UPDATE lines
               SET content = $2, vv = $3, last_writer = $4, updated_at = $5, deleted_at = NULL
               WHERE id = $1
               RETURNING id, note_id, content, created_at, updated_at, deleted_at, vv, last_writer"#,
        )
        .bind(id)
        .bind(self.cipher.encrypt(content)?)
        .bind(Json(vv))
        .bind(last_writer)
        .bind(updated_at)
        .fetch_optional(exec)
        .await?;
        if let Some(line) = line.as_mut() {
            line.content = content.to_string();
        }
        Ok(line)
    }

    // md:impl Store > fn soft_delete_line
    pub async fn soft_delete_line(
        &self,
        id: Uuid,
        deleted_at: DateTime<Utc>,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Option<Line>, AppError> {
        self.soft_delete_line_on(&self.pool, id, deleted_at, vv, last_writer, updated_at)
            .await
    }

    // md:impl Store > fn soft_delete_line_on
    pub async fn soft_delete_line_on<'e, E>(
        &self,
        exec: E,
        id: Uuid,
        deleted_at: DateTime<Utc>,
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<Option<Line>, AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let mut line = sqlx::query_as::<_, Line>(
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
        .fetch_optional(exec)
        .await?;
        if let Some(line) = line.as_mut() {
            line.content = self.cipher.decrypt(&line.content)?;
        }
        Ok(line)
    }

    // md:impl Store > fn get_note_order
    pub async fn get_note_order(&self, note_id: Uuid) -> Result<Option<NoteOrder>, AppError> {
        self.get_note_order_on(&self.pool, note_id).await
    }

    // md:impl Store > fn get_note_order_on
    pub async fn get_note_order_on<'e, E>(
        &self,
        exec: E,
        note_id: Uuid,
    ) -> Result<Option<NoteOrder>, AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query(
            r#"SELECT note_id, order_json, updated_at, vv, last_writer
               FROM note_line_order WHERE note_id = $1"#,
        )
        .bind(note_id)
        .fetch_optional(exec)
        .await?;
        Ok(row.map(|r| NoteOrder {
            note_id: r.get("note_id"),
            order: r.get::<Json<Vec<Uuid>>, _>("order_json").0,
            updated_at: r.get("updated_at"),
            vv: r.get::<Json<VersionVector>, _>("vv").0,
            last_writer: r.get("last_writer"),
        }))
    }

    // md:impl Store > fn set_note_order
    pub async fn set_note_order(
        &self,
        note_id: Uuid,
        order: &[Uuid],
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.set_note_order_on(&self.pool, note_id, order, vv, last_writer, updated_at)
            .await
    }

    // md:impl Store > fn set_note_order_on
    pub async fn set_note_order_on<'e, E>(
        &self,
        exec: E,
        note_id: Uuid,
        order: &[Uuid],
        vv: &VersionVector,
        last_writer: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<(), AppError>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
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
        .execute(exec)
        .await?;
        Ok(())
    }

    // md:impl Store > fn pool
    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }

    // md:impl Store > fn notify
    pub async fn notify(&self, channel: &str, payload: &str) -> Result<(), AppError> {
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(channel)
            .bind(payload)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // md:impl Store > fn lock_note_order
    pub async fn lock_note_order(
        &self,
        note_id: Uuid,
    ) -> Result<sqlx::Transaction<'static, Postgres>, AppError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(note_id.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    // md:impl Store > fn insert_collab_event
    pub async fn insert_collab_event(
        &self,
        note_id: Uuid,
        origin_instance: Uuid,
        origin_conn: i64,
        user_id: Uuid,
        ops: &serde_json::Value,
    ) -> Result<i64, AppError> {
        let seq: i64 = sqlx::query_scalar(
            r#"INSERT INTO collab_events (note_id, origin_instance, origin_conn, user_id, ops)
               VALUES ($1, $2, $3, $4, $5) RETURNING seq"#,
        )
        .bind(note_id)
        .bind(origin_instance)
        .bind(origin_conn)
        .bind(user_id)
        .bind(Json(ops))
        .fetch_one(&self.pool)
        .await?;
        Ok(seq)
    }

    // md:impl Store > fn get_collab_event
    pub async fn get_collab_event(&self, seq: i64) -> Result<Option<CollabEvent>, AppError> {
        let row = sqlx::query(
            r#"SELECT note_id, origin_instance, origin_conn, user_id, ops
               FROM collab_events WHERE seq = $1"#,
        )
        .bind(seq)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| CollabEvent {
            seq,
            note_id: r.get("note_id"),
            origin_instance: r.get("origin_instance"),
            origin_conn: r.get("origin_conn"),
            user_id: r.get("user_id"),
            ops: r.get::<Json<serde_json::Value>, _>("ops").0,
        }))
    }

    // md:impl Store > fn prune_collab_events
    pub async fn prune_collab_events(&self, older_than: DateTime<Utc>) -> Result<u64, AppError> {
        let r = sqlx::query("DELETE FROM collab_events WHERE created_at < $1")
            .bind(older_than)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    // md:impl Store > fn upsert_presence
    pub async fn upsert_presence(
        &self,
        note_id: Uuid,
        instance_id: Uuid,
        conn_id: i64,
        user_id: Uuid,
        display_name: &str,
        cursor: Option<&serde_json::Value>,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"INSERT INTO collab_presence
                   (note_id, instance_id, conn_id, user_id, display_name, cursor, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, now())
               ON CONFLICT (note_id, instance_id, conn_id)
               DO UPDATE SET cursor = EXCLUDED.cursor,
                             display_name = EXCLUDED.display_name,
                             updated_at = now()"#,
        )
        .bind(note_id)
        .bind(instance_id)
        .bind(conn_id)
        .bind(user_id)
        .bind(display_name)
        .bind(cursor.map(|c| Json(c.clone())))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // md:impl Store > fn delete_presence
    pub async fn delete_presence(
        &self,
        note_id: Uuid,
        instance_id: Uuid,
        conn_id: i64,
    ) -> Result<(), AppError> {
        sqlx::query(
            "DELETE FROM collab_presence WHERE note_id = $1 AND instance_id = $2 AND conn_id = $3",
        )
        .bind(note_id)
        .bind(instance_id)
        .bind(conn_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // md:impl Store > fn list_presence
    pub async fn list_presence(&self, note_id: Uuid) -> Result<Vec<PresenceRow>, AppError> {
        let rows = sqlx::query(
            r#"SELECT user_id, display_name, cursor
               FROM collab_presence WHERE note_id = $1"#,
        )
        .bind(note_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| PresenceRow {
                user_id: r.get("user_id"),
                display_name: r.get("display_name"),
                cursor: r
                    .get::<Option<Json<serde_json::Value>>, _>("cursor")
                    .map(|c| c.0),
            })
            .collect())
    }

    // md:impl Store > fn touch_instance_presence
    pub async fn touch_instance_presence(&self, instance_id: Uuid) -> Result<(), AppError> {
        sqlx::query("UPDATE collab_presence SET updated_at = now() WHERE instance_id = $1")
            .bind(instance_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // md:impl Store > fn sweep_presence
    pub async fn sweep_presence(&self, older_than: DateTime<Utc>) -> Result<u64, AppError> {
        let r = sqlx::query("DELETE FROM collab_presence WHERE updated_at < $1")
            .bind(older_than)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    // md:impl Store > fn delete_instance_presence
    pub async fn delete_instance_presence(&self, instance_id: Uuid) -> Result<u64, AppError> {
        let r = sqlx::query("DELETE FROM collab_presence WHERE instance_id = $1")
            .bind(instance_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    // md:impl Store > fn upsert_notebook
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

    // md:impl Store > fn delete_notebook
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

    // md:impl Store > fn upsert_tag
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

    // md:impl Store > fn delete_tag
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

    // md:impl Store > fn upsert_note_tag
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

    // md:impl Store > fn upsert_resource_meta
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

    // md:impl Store > fn delete_resource
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

    // md:impl Store > fn put_resource_blob
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

    // md:impl Store > fn get_resource_blob
    pub async fn get_resource_blob(&self, resource_id: Uuid) -> Result<Option<Vec<u8>>, AppError> {
        let row = sqlx::query("SELECT data FROM resource_blobs WHERE resource_id = $1")
            .bind(resource_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<Vec<u8>, _>("data")))
    }

    // md:impl Store > fn resource_owned_by
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

    // md:impl Store > fn list_notebooks
    pub async fn list_notebooks(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
        cursor: Option<PageCursor>,
    ) -> Result<Vec<Notebook>, AppError> {
        let (cur_ts, cur_id) = split_cursor(cursor);
        Ok(sqlx::query_as::<_, Notebook>(
            "SELECT id, title, alias, created_at, updated_at, deleted_at
             FROM notebooks
             WHERE user_id = $1 AND deleted_at IS NULL
               AND ($3::timestamptz IS NULL OR (created_at, id) > ($3, $4))
             ORDER BY created_at, id
             LIMIT $2",
        )
        .bind(user_id)
        .bind(limit.unwrap_or(i64::MAX))
        .bind(cur_ts)
        .bind(cur_id)
        .fetch_all(&self.pool)
        .await?)
    }

    // md:impl Store > fn list_tags
    pub async fn list_tags(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
        cursor: Option<PageCursor>,
    ) -> Result<Vec<Tag>, AppError> {
        let (cur_ts, cur_id) = split_cursor(cursor);
        Ok(sqlx::query_as::<_, Tag>(
            "SELECT id, title, created_at, updated_at, deleted_at
             FROM tags
             WHERE user_id = $1 AND deleted_at IS NULL
               AND ($3::timestamptz IS NULL OR (created_at, id) > ($3, $4))
             ORDER BY created_at, id
             LIMIT $2",
        )
        .bind(user_id)
        .bind(limit.unwrap_or(i64::MAX))
        .bind(cur_ts)
        .bind(cur_id)
        .fetch_all(&self.pool)
        .await?)
    }

    // md:impl Store > fn list_resources
    pub async fn list_resources(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
        cursor: Option<PageCursor>,
    ) -> Result<Vec<ResourceMeta>, AppError> {
        let (cur_ts, cur_id) = split_cursor(cursor);
        Ok(sqlx::query_as::<_, ResourceMeta>(
            "SELECT id, title, mime_type, file_name, size, created_at, deleted_at
             FROM resources
             WHERE user_id = $1 AND deleted_at IS NULL
               AND ($3::timestamptz IS NULL OR (created_at, id) > ($3, $4))
             ORDER BY created_at, id
             LIMIT $2",
        )
        .bind(user_id)
        .bind(limit.unwrap_or(i64::MAX))
        .bind(cur_ts)
        .bind(cur_id)
        .fetch_all(&self.pool)
        .await?)
    }

    // md:impl Store > fn list_note_tag_ids
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

    // md:impl Store > fn user_blob_bytes_excluding
    pub async fn user_blob_bytes_excluding(
        &self,
        user_id: Uuid,
        exclude: Uuid,
    ) -> Result<i64, AppError> {
        let bytes: i64 = sqlx::query_scalar(
            r#"SELECT COALESCE(SUM(octet_length(rb.data)), 0)::bigint
               FROM resource_blobs rb
               JOIN resources r ON r.id = rb.resource_id
               WHERE r.user_id = $1 AND r.deleted_at IS NULL AND rb.resource_id <> $2"#,
        )
        .bind(user_id)
        .bind(exclude)
        .fetch_one(&self.pool)
        .await?;
        Ok(bytes)
    }

    // md:impl Store > fn count_live_notes_for_user
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

// md:fn replace_note_shares_from_notebook_tx
async fn replace_note_shares_from_notebook_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    note_id: Uuid,
    notebook_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM note_shares WHERE note_id = $1")
        .bind(note_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        r#"INSERT INTO note_shares (note_id, user_id, capabilities)
           SELECT $1, user_id, capabilities FROM notebook_shares WHERE notebook_id = $2"#,
    )
    .bind(note_id)
    .bind(notebook_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// md:fn cascade_notebook_to_notes_tx
async fn cascade_notebook_to_notes_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    notebook_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM note_shares WHERE note_id IN
         (SELECT id FROM notes WHERE notebook_id = $1 AND deleted_at IS NULL)",
    )
    .bind(notebook_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO note_shares (note_id, user_id, capabilities)
           SELECT n.id, ns.user_id, ns.capabilities
           FROM notes n
           JOIN notebook_shares ns ON ns.notebook_id = n.notebook_id
           WHERE n.notebook_id = $1 AND n.deleted_at IS NULL"#,
    )
    .bind(notebook_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
