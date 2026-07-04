use std::sync::Arc;

use anyhow::Context;
use keeplin_srv::{config::Config, http::router, state::AppState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("keeplin_srv=info".parse()?))
        .init();

    dotenvy::dotenv().ok();
    let config = Config::from_env();

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .context("connect to PostgreSQL")?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("run database migrations")?;

    let state = Arc::new(AppState::new(config.clone(), pool));

    if config.retention_days > 0 {
        tokio::spawn(prune_loop(state.clone(), config.retention_days));
    }

    let app = router(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port))
        .await
        .context("bind server address")?;

    tracing::info!(port = config.port, "Keeplin sync server listening");
    axum::serve(listener, app).await.context("run server")?;

    Ok(())
}

/// Once an hour, delete journal rows older than the retention window that
/// every device of their user has already received.
async fn prune_loop(state: Arc<AppState>, retention_days: u64) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        interval.tick().await;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
        match state.store.prune_delivered_changes(cutoff).await {
            Ok(0) => {}
            Ok(rows) => tracing::info!(rows, "pruned delivered changes"),
            Err(e) => tracing::warn!(error = %e, "prune failed"),
        }
    }
}
