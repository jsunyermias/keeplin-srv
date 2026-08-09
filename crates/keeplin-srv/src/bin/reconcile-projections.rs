// md:Overview
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use keeplin_srv::{config::Config, projection, state::AppState};
use uuid::Uuid;

// md:fn main
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env();
    anyhow::ensure!(
        config.db_max_connections >= 2,
        "DB_MAX_CONNECTIONS must be at least 2 while projection workers hold a claim connection"
    );
    let mut user = None;
    let mut from = None;
    let mut to = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let value = arguments.next().context("every option requires a value")?;
        match argument.as_str() {
            "--user" => user = Some(value.parse::<Uuid>().context("invalid --user UUID")?),
            "--from" => {
                from = Some(
                    value
                        .parse::<DateTime<Utc>>()
                        .context("invalid --from timestamp")?,
                )
            }
            "--to" => {
                to = Some(
                    value
                        .parse::<DateTime<Utc>>()
                        .context("invalid --to timestamp")?,
                )
            }
            _ => anyhow::bail!("unknown option {argument}"),
        }
    }
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .acquire_timeout(Duration::from_secs(config.db_acquire_timeout_secs))
        .connect(&config.database_url)
        .await
        .context("connect to PostgreSQL")?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    let state = AppState::new(config, pool);
    let queued = projection::reconcile(&state.store, user, from, to).await?;
    projection::drain_available(&state, user, usize::MAX).await;
    let stats = projection::stats(&state.store).await?;
    println!(
        "queued={queued} outstanding={} retrying={} dead_lettered={}",
        stats.outstanding, stats.retrying, stats.dead_lettered
    );
    Ok(())
}
