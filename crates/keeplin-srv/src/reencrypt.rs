// md:Overview
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::crypto::{Cipher, ENC_PREFIX};
use crate::error::AppError;

// md:Options
#[derive(Debug, Clone)]
pub struct Options {
    pub dry_run: bool,
    pub batch_size: i64,
}

// md:impl Default for Options
impl Default for Options {
    fn default() -> Self {
        Self {
            dry_run: false,
            batch_size: 500,
        }
    }
}

// md:TableStats
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TableStats {
    pub scanned: u64,
    pub rewritten: u64,
    pub skipped_concurrent: u64,
}

// md:Stats
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub notes_title: TableStats,
    pub lines_content: TableStats,
}

// md:fn run
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

// md:fn reencrypt_column
async fn reencrypt_column(
    pool: &PgPool,
    cipher: &Cipher,
    opts: &Options,
    table: &str,
    column: &str,
) -> Result<TableStats, AppError> {
    let mut stats = TableStats::default();
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
