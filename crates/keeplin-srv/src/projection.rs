// md:Overview
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use keeplin_core::models::Change;
use sqlx::{Row, Transaction};
use uuid::Uuid;

use crate::{error::AppError, state::AppState, store::Store};

// md:ProjectionQueueStats
#[derive(Debug, Clone, Copy)]
pub struct ProjectionQueueStats {
    pub outstanding: i64,
    pub retrying: i64,
    pub dead_lettered: i64,
    pub oldest_outstanding_seconds: i64,
}

// md:ClaimedJob
struct ClaimedJob {
    user_id: Uuid,
    batch_id: Uuid,
    batch_index: i32,
    payload: serde_json::Value,
    transaction: Transaction<'static, sqlx::Postgres>,
}

// md:fn is_serialization_failure
fn is_serialization_failure(error: &AppError) -> bool {
    matches!(error, AppError::Database(sqlx::Error::Database(database)) if database.code().as_deref() == Some("40001"))
}

// md:fn is_permanent_database_failure
fn is_permanent_database_failure(error: &AppError) -> bool {
    matches!(error, AppError::Internal(message) if message.starts_with("invalid projection payload:"))
        || matches!(error, AppError::Database(sqlx::Error::Database(database)) if matches!(database.code().as_deref(), Some("22001") | Some("22003") | Some("22007") | Some("22008") | Some("22P02")))
}

// md:fn apply_change
async fn apply_change(store: &Store, user_id: Uuid, change: Change) -> Result<(), AppError> {
    match change {
        Change::NotebookCreate { notebook } | Change::NotebookUpdate { notebook } => {
            for attempt in 0..3 {
                match store.upsert_notebook(user_id, &notebook).await {
                    Err(error) if is_serialization_failure(&error) && attempt < 2 => {}
                    result => {
                        result?;
                        break;
                    }
                }
            }
        }
        Change::NotebookDelete {
            id,
            deleted_at,
            vv,
            last_writer,
        } => {
            for attempt in 0..3 {
                match store
                    .delete_notebook(user_id, id, deleted_at, &vv, &last_writer)
                    .await
                {
                    Err(error) if is_serialization_failure(&error) && attempt < 2 => {}
                    result => {
                        result?;
                        break;
                    }
                }
            }
        }
        Change::TagCreate { tag } | Change::TagUpdate { tag } => {
            store.upsert_tag(user_id, &tag).await?;
        }
        Change::TagDelete {
            id,
            deleted_at,
            vv,
            last_writer,
        } => {
            store
                .delete_tag(user_id, id, deleted_at, &vv, &last_writer)
                .await?;
        }
        Change::NoteTagAdd {
            note_id,
            tag_id,
            updated_at,
            vv,
            last_writer,
        } => {
            store
                .upsert_note_tag(
                    user_id,
                    note_id,
                    tag_id,
                    updated_at,
                    None,
                    &vv,
                    &last_writer,
                )
                .await?;
        }
        Change::NoteTagRemove {
            note_id,
            tag_id,
            updated_at,
            vv,
            last_writer,
        } => {
            store
                .upsert_note_tag(
                    user_id,
                    note_id,
                    tag_id,
                    updated_at,
                    Some(updated_at),
                    &vv,
                    &last_writer,
                )
                .await?;
        }
        Change::ResourceCreate { resource, data } => {
            store
                .apply_resource_create(user_id, &resource, data.as_deref())
                .await?;
        }
        Change::ResourceDelete {
            id,
            deleted_at,
            vv,
            last_writer,
        } => {
            store
                .delete_resource(user_id, id, deleted_at, &vv, &last_writer)
                .await?;
        }
        Change::NoteCreate { .. } | Change::NoteUpdate { .. } | Change::NoteDelete { .. } => {}
    }
    Ok(())
}

// md:fn claim_one
async fn claim_one(
    state: &AppState,
    user: Option<Uuid>,
    batch: Option<Uuid>,
) -> Result<Option<ClaimedJob>, AppError> {
    let mut transaction = state.store.pool().begin().await?;
    let row = sqlx::query(
        r#"SELECT pj.user_id, pj.batch_id, pj.batch_index, c.payload
           FROM projection_jobs pj
           JOIN changes c USING (user_id, batch_id, batch_index)
           WHERE pj.state IN ('pending', 'retry')
             AND pj.available_at <= now()
             AND ($1::uuid IS NULL OR pj.user_id = $1)
             AND ($2::uuid IS NULL OR pj.batch_id = $2)
           ORDER BY pj.available_at, pj.created_at, pj.batch_index
           FOR UPDATE OF pj SKIP LOCKED
           LIMIT 1"#,
    )
    .bind(user)
    .bind(batch)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(row) = row else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let mut job = ClaimedJob {
        user_id: row.try_get("user_id")?,
        batch_id: row.try_get("batch_id")?,
        batch_index: row.try_get("batch_index")?,
        payload: row.try_get("payload")?,
        transaction,
    };
    sqlx::query(
        "UPDATE projection_jobs SET leased_by = $4, leased_until = now() + interval '30 seconds', updated_at = now() WHERE user_id = $1 AND batch_id = $2 AND batch_index = $3",
    )
    .bind(job.user_id)
    .bind(job.batch_id)
    .bind(job.batch_index)
    .bind(state.instance_id)
    .execute(&mut *job.transaction)
    .await?;
    Ok(Some(job))
}

// md:fn finish_job
async fn finish_job(
    state: &AppState,
    mut job: ClaimedJob,
    result: Result<(), AppError>,
) -> Result<(), AppError> {
    match result {
        Ok(()) => {
            sqlx::query("DELETE FROM projection_jobs WHERE user_id = $1 AND batch_id = $2 AND batch_index = $3 AND leased_by = $4")
                .bind(job.user_id).bind(job.batch_id).bind(job.batch_index).bind(state.instance_id)
                .execute(&mut *job.transaction).await?;
        }
        Err(error) if is_serialization_failure(&error) => {
            sqlx::query("UPDATE projection_jobs SET state = 'retry', available_at = now() + interval '1 second', leased_by = NULL, leased_until = NULL, last_error = $4, updated_at = now() WHERE user_id = $1 AND batch_id = $2 AND batch_index = $3 AND leased_by = $5")
                .bind(job.user_id).bind(job.batch_id).bind(job.batch_index).bind(error.to_string()).bind(state.instance_id)
                .execute(&mut *job.transaction).await?;
        }
        Err(error) => {
            let permanent = is_permanent_database_failure(&error);
            sqlx::query("UPDATE projection_jobs SET attempts = attempts + 1, state = CASE WHEN $4 OR attempts + 1 >= 5 THEN 'dead_letter' ELSE 'retry' END, available_at = now() + make_interval(secs => LEAST(16, (1 << LEAST(attempts, 4)))::double precision), leased_by = NULL, leased_until = NULL, last_error = $5, updated_at = now() WHERE user_id = $1 AND batch_id = $2 AND batch_index = $3 AND leased_by = $6")
                .bind(job.user_id).bind(job.batch_id).bind(job.batch_index).bind(permanent).bind(error.to_string()).bind(state.instance_id)
                .execute(&mut *job.transaction).await?;
        }
    }
    job.transaction.commit().await?;
    Ok(())
}

// md:fn drain_available
pub async fn drain_available(state: &AppState, user: Option<Uuid>, limit: usize) {
    for _ in 0..limit {
        let job = match claim_one(state, user, None).await {
            Ok(Some(job)) => job,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, "projection claim failed");
                break;
            }
        };
        let result = match serde_json::from_value(job.payload.clone()) {
            Ok(change) => apply_change(&state.store, job.user_id, change).await,
            Err(error) => Err(AppError::Internal(format!(
                "invalid projection payload: {error}"
            ))),
        };
        if let Err(error) = finish_job(state, job, result).await {
            tracing::warn!(%error, "projection job state update failed");
        }
    }
}

// md:fn drain_batch
pub async fn drain_batch(state: &AppState, user_id: Uuid, batch_id: Uuid) {
    if let Err(error) = sqlx::query(
        "UPDATE projection_jobs SET available_at = now(), updated_at = now() WHERE user_id = $1 AND batch_id = $2 AND state IN ('pending', 'retry')",
    )
    .bind(user_id)
    .bind(batch_id)
    .execute(state.store.pool())
    .await
    {
        tracing::warn!(%error, %user_id, %batch_id, "duplicate batch projection scheduling failed");
        return;
    }
    loop {
        let job = match claim_one(state, Some(user_id), Some(batch_id)).await {
            Ok(Some(job)) => job,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, %user_id, %batch_id, "duplicate batch projection claim failed");
                break;
            }
        };
        let result = match serde_json::from_value(job.payload.clone()) {
            Ok(change) => apply_change(&state.store, job.user_id, change).await,
            Err(error) => Err(AppError::Internal(format!(
                "invalid projection payload: {error}"
            ))),
        };
        if let Err(error) = finish_job(state, job, result).await {
            tracing::warn!(%error, %user_id, %batch_id, "duplicate batch projection state update failed");
        }
    }
}

// md:fn worker
pub async fn worker(state: Arc<AppState>) {
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tick.tick().await;
        drain_available(&state, None, 64).await;
    }
}

// md:fn reconcile
pub async fn reconcile(
    store: &Store,
    user: Option<Uuid>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
) -> Result<u64, AppError> {
    let result = sqlx::query(
        r#"INSERT INTO projection_jobs (user_id, batch_id, batch_index)
           SELECT c.user_id, c.batch_id, c.batch_index FROM changes c
           WHERE ($1::uuid IS NULL OR c.user_id = $1)
             AND ($2::timestamptz IS NULL OR c.received_at >= $2)
             AND ($3::timestamptz IS NULL OR c.received_at < $3)
             AND c.payload->>'op' NOT IN ('note_create', 'note_update', 'note_delete')
           ON CONFLICT (user_id, batch_id, batch_index) DO UPDATE
           SET state = 'pending', attempts = 0, available_at = now(), leased_by = NULL,
               leased_until = NULL, last_error = NULL, updated_at = now()
           WHERE projection_jobs.state = 'dead_letter'"#,
    )
    .bind(user)
    .bind(from)
    .bind(to)
    .execute(store.pool())
    .await?;
    Ok(result.rows_affected())
}

// md:fn stats
pub async fn stats(store: &Store) -> Result<ProjectionQueueStats, AppError> {
    let row = sqlx::query("SELECT COUNT(*) FILTER (WHERE pj.state IN ('pending', 'retry')) AS outstanding, COUNT(*) FILTER (WHERE pj.state = 'retry') AS retrying, COUNT(*) FILTER (WHERE pj.state = 'dead_letter') AS dead_lettered, COALESCE(EXTRACT(EPOCH FROM now() - MIN(c.received_at) FILTER (WHERE pj.state IN ('pending', 'retry')))::bigint, 0) AS oldest FROM projection_jobs pj JOIN changes c USING (user_id, batch_id, batch_index)")
        .fetch_one(store.pool()).await?;
    Ok(ProjectionQueueStats {
        outstanding: row.get("outstanding"),
        retrying: row.get("retrying"),
        dead_lettered: row.get("dead_lettered"),
        oldest_outstanding_seconds: row.get("oldest"),
    })
}
