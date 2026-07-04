use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Pool, Postgres, Row};
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
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

    pub async fn create_user(&self, email: &str, password_hash: &str) -> Result<User, AppError> {
        let user = sqlx::query_as::<_, User>(
            r#"INSERT INTO users (id, email, password_hash)
               VALUES ($1, $2, $3)
               RETURNING id, email, password_hash, created_at"#,
        )
        .bind(Uuid::new_v4())
        .bind(email)
        .bind(password_hash)
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
            "SELECT id, email, password_hash, created_at FROM users WHERE email = $1",
        )
        .bind(email)
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
}
