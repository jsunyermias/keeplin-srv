//! One-off administrative re-encrypt pass for at-rest encryption
//! (issue keeplin#110 follow-up).
//!
//! When `AT_REST_KEY` is enabled on an existing deployment, rows written
//! before the key stay plaintext forever: the [`crate::crypto::Cipher`] reads
//! both forms, but nothing rewrites the old rows. This module scans the two
//! encrypted columns — `notes.title` and `lines.content` — selects the rows
//! that do **not** carry the `enc:v1:` tag, and rewrites them encrypted.
//!
//! Properties (all load-bearing for an administrative tool run against a
//! production database):
//!
//! - **Idempotent**: already-encrypted rows are never selected (`NOT LIKE
//!   'enc:v1:%'`), so re-running the pass is safe and a no-op once complete.
//! - **Batched**: rows are processed in keyset-paginated batches (`id > last`,
//!   `ORDER BY id`, `LIMIT batch_size`), one transaction per batch, so no
//!   transaction ever spans the whole table.
//! - **Resumable**: each batch commits independently; an interrupted run
//!   leaves the finished batches encrypted and the next run picks up the
//!   remaining plaintext rows (idempotence makes the restart free).
//! - **Safe against a live server**: every `UPDATE` is guarded with
//!   `AND <column> = <the plaintext we read>`. If the running server rewrote
//!   the row in between, the guard fails and the row is skipped — the server
//!   holds the same key, so the concurrent write is already encrypted.
//! - **`--dry-run`**: counts what would be rewritten without modifying
//!   anything (no `UPDATE` is issued at all).
//! - **Progress logging**: one `tracing` line per batch per table.

use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::crypto::{Cipher, ENC_PREFIX};
use crate::error::AppError;

/// Tuning/behaviour knobs for one pass.
#[derive(Debug, Clone)]
pub struct Options {
    /// Report what would change without writing anything.
    pub dry_run: bool,
    /// Rows per batch (one transaction each). Bounds transaction size and
    /// memory; 500 is a safe default for interactive use on a live database.
    pub batch_size: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            dry_run: false,
            batch_size: 500,
        }
    }
}

/// Outcome counters for one table.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TableStats {
    /// Plaintext rows seen (would-be rewrites in `--dry-run`).
    pub scanned: u64,
    /// Rows actually rewritten to `enc:v1:` (always 0 in `--dry-run`).
    pub rewritten: u64,
    /// Rows whose value changed under us between read and write (guard
    /// failed). The live server wrote them encrypted; nothing to do.
    pub skipped_concurrent: u64,
}

/// Outcome of a whole pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub notes_title: TableStats,
    pub lines_content: TableStats,
}

/// Run the re-encrypt pass over `notes.title` and `lines.content`.
///
/// Errors if the cipher is disabled: running without a key would be a no-op
/// that reports success, which is exactly the kind of silent misfire an
/// administrative tool must refuse.
pub async fn run(pool: &PgPool, cipher: &Cipher, opts: &Options) -> Result<Stats, AppError> {
    if !cipher.enabled() {
        return Err(AppError::Internal(
            "re-encrypt pass needs AT_REST_KEY set (the cipher is disabled)".into(),
        ));
    }
    let notes_title = reencrypt_column(pool, cipher, opts, "notes", "title").await?;
    let lines_content = reencrypt_column(pool, cipher, opts, "lines", "content").await?;
    Ok(Stats {
        notes_title,
        lines_content,
    })
}

/// Re-encrypt one `(table, column)` pair with keyset pagination on `id`.
///
/// `table`/`column` are compile-time literals from [`run`], never user input,
/// so interpolating them into the SQL is safe.
async fn reencrypt_column(
    pool: &PgPool,
    cipher: &Cipher,
    opts: &Options,
    table: &str,
    column: &str,
) -> Result<TableStats, AppError> {
    let mut stats = TableStats::default();
    // Uuid::nil() sorts before every real v4 id under Postgres uuid ordering.
    let mut last_id = Uuid::nil();
    loop {
        let select = format!(
            "SELECT id, {column} AS value FROM {table} \
             WHERE {column} NOT LIKE $1 AND id > $2 \
             ORDER BY id LIMIT $3",
        );
        let rows = sqlx::query(&select)
            .bind(format!("{ENC_PREFIX}%"))
            .bind(last_id)
            .bind(opts.batch_size)
            .fetch_all(pool)
            .await?;
        if rows.is_empty() {
            break;
        }
        stats.scanned += rows.len() as u64;
        last_id = rows.last().map(|r| r.get::<Uuid, _>("id")).unwrap();

        if !opts.dry_run {
            // One bounded transaction per batch: an interruption loses at most
            // this batch, and already-committed batches stay encrypted.
            let mut tx = pool.begin().await?;
            let update =
                format!("UPDATE {table} SET {column} = $1 WHERE id = $2 AND {column} = $3",);
            for row in &rows {
                let id: Uuid = row.get("id");
                let plaintext: String = row.get("value");
                let encrypted = cipher.encrypt(&plaintext)?;
                let result = sqlx::query(&update)
                    .bind(&encrypted)
                    .bind(id)
                    .bind(&plaintext)
                    .execute(&mut *tx)
                    .await?;
                if result.rows_affected() == 1 {
                    stats.rewritten += 1;
                } else {
                    // Concurrently modified: the live server (same key) already
                    // wrote the new value encrypted. Nothing to do.
                    stats.skipped_concurrent += 1;
                }
            }
            tx.commit().await?;
        }

        tracing::info!(
            table,
            column,
            batch = rows.len(),
            scanned = stats.scanned,
            rewritten = stats.rewritten,
            skipped_concurrent = stats.skipped_concurrent,
            dry_run = opts.dry_run,
            "re-encrypt batch done"
        );
    }
    tracing::info!(
        table,
        column,
        scanned = stats.scanned,
        rewritten = stats.rewritten,
        skipped_concurrent = stats.skipped_concurrent,
        dry_run = opts.dry_run,
        "re-encrypt column complete"
    );
    Ok(stats)
}
